//! The preview canvas: the active image over a transparency checkerboard,
//! with drag-to-pan and cursor-anchored scroll-to-zoom. Pin presses are
//! published in the shown view's image pixels; the pan/zoom gestures are
//! resolved here and published as a viewport.
//!
//! Raster views (the stage images) draw through an iced [`canvas`]; the vector
//! views draw the whole visible scene in one [`vello`](super::vector) GPU pass
//! behind a [`shader`](iced::widget::shader) widget. The two share this module's
//! gesture handling, so pan, zoom, and tool presses behave identically whichever
//! is shown.

use super::checkerboard::Checkerboard;
use super::vector::VectorPrimitive;
use super::viewport::Viewport;
use crate::gui::app::App;
use crate::gui::compute::Art;
use crate::gui::msg::{CanvasMsg, Msg};
use crate::gui::overlays::{self, OverlayCtx};
use crate::gui::tools::Tool;
use iced::advanced::image as core_image;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::shader;
use iced::widget::Stack;
use iced::{Element, Length, Point, Rectangle, Size, Vector};

const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 32.0;
/// Screen-pixel travel below which a Select press-and-release counts as a
/// click rather than a pan.
const CLICK_SLOP: f32 = 4.0;

/// Builds the preview widget over the document's active image and viewport,
/// with the registered state overlays composited on top in [`overlays::ALL`]
/// order.
pub fn preview(app: &App) -> Element<'_, Msg> {
    let program = Preview {
        art: app.active_art(),
        zoom: app.session().and_then(|s| s.zoom()),
        pan: app.session().map(|s| s.pan()).unwrap_or(Vector::ZERO),
        tool: app.tools.active,
        doc_view: app.session().is_some_and(|s| s.is_doc_view()),
    };

    // Vector views rasterize on the GPU behind a shader widget; raster views
    // blit their image through a canvas. Both carry the same gestures.
    let art: Element<'_, Msg> = if matches!(program.art, Some(Art::Vector(_))) {
        shader(program).width(Length::Fill).height(Length::Fill).into()
    } else {
        iced::widget::canvas(program)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };

    let checker = iced::widget::canvas(Checkerboard)
        .width(Length::Fill)
        .height(Length::Fill);

    let ctx = OverlayCtx::from_app(app);

    let mut stack = Stack::new().push(checker).push(art);

    for overlay in overlays::ALL {
        if let Some(element) = overlay(&ctx) {
            stack = stack.push(element);
        }
    }

    stack.into()
}

struct Preview<'a> {
    /// What to draw for the active view, raster or vector; `None` before it has
    /// been rendered.
    art: Option<Art<'a>>,
    zoom: Option<f32>,
    pan: Vector,
    tool: Tool,
    /// Whether the active view is the whole-document composite, the only view
    /// on which a Select click hit-tests layers.
    doc_view: bool,
}

#[derive(Default)]
struct State {
    /// Last cursor position while panning; `None` when not dragging.
    last: Option<Point>,
    panning: bool,
    /// A pin press is in progress, so cursor moves drag the tool.
    tool_active: bool,
    /// Screen position where a Select-tool left press began; a release near it
    /// is a click, farther away was a pan.
    press: Option<Point>,
}

impl Preview<'_> {
    /// The active art's crop-space dimensions, the size the viewport places it
    /// against so every view resolves to the same on-screen rectangle.
    fn canonical(&self) -> Option<(f32, f32)> {
        Some(self.art.as_ref()?.dims())
    }

    /// A `SelectAt` action when a Select press released near where it began on
    /// the Document view, carrying the release point in document px; `None`
    /// when the gesture was a pan, off the Document view, or before the
    /// composite exists to map the point.
    fn click_on_release(
        &self,
        press: Option<Point>,
        cursor: mouse::Cursor,
        bounds: Rectangle,
    ) -> Option<shader::Action<Msg>> {
        let press = press?;
        let released = cursor.position_in(bounds)?;

        if !self.doc_view
            || (released.x - press.x).abs() >= CLICK_SLOP
            || (released.y - press.y).abs() >= CLICK_SLOP
        {
            return None;
        }

        let (cw, ch) = self.canonical()?;
        let vp = Viewport::resolve(bounds.size(), (cw, ch), self.zoom, self.pan);
        let doc_px = vp.to_image(released);

        Some(shader::Action::publish(Msg::Canvas(CanvasMsg::SelectAt(
            doc_px,
        ))))
    }

    /// The shared pan/zoom/tool gesture handler. `canvas::Event` and
    /// `shader::Event` are the same core event, and both programs publish the
    /// same [`shader::Action`], so raster and vector previews resolve gestures
    /// through this one path.
    fn gesture(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Msg>> {
        // A release outside the canvas still ends a drag; handling it before
        // the cursor-in-bounds gate keeps a pan from sticking to the cursor
        // when it re-enters with no button held.
        if let canvas::Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            if state.panning || state.tool_active {
                let was_tool = state.tool_active;
                let press = state.press.take();

                state.panning = false;
                state.tool_active = false;
                state.last = None;

                if was_tool {
                    return Some(shader::Action::publish(Msg::Canvas(CanvasMsg::ToolRelease)));
                }

                return self.click_on_release(press, cursor, bounds);
            }
        }

        let pos = cursor.position_in(bounds)?;
        let (cw, ch) = self.canonical()?;
        let vp = Viewport::resolve(bounds.size(), (cw, ch), self.zoom, self.pan);
        let zoom = vp.zoom;
        let to_img = |p: Point| vp.to_image(p);

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let mid = matches!(button, mouse::Button::Middle);
                let left = matches!(button, mouse::Button::Left);
                let captures = left && self.tool.captures_press();

                if mid || (left && !captures) {
                    state.panning = true;
                    state.last = Some(pos);
                    // Only a left non-capturing press can become a click; a
                    // middle-button pan leaves `press` clear so its release
                    // never selects.
                    state.press = (left && !captures).then_some(pos);

                    return Some(shader::Action::capture());
                }

                if captures {
                    let ip = to_img(pos);

                    if ip.x >= 0.0 && ip.y >= 0.0 && ip.x < cw && ip.y < ch {
                        state.tool_active = true;

                        return Some(
                            shader::Action::publish(Msg::Canvas(CanvasMsg::ToolPress(ip)))
                                .and_capture(),
                        );
                    }
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.panning {
                    let last = state.last?;

                    state.last = Some(pos);

                    let pan = self.pan + Vector::new(pos.x - last.x, pos.y - last.y);

                    return Some(shader::Action::publish(Msg::Canvas(CanvasMsg::SetViewport {
                        zoom,
                        pan,
                    })));
                }

                if state.tool_active {
                    return Some(shader::Action::publish(Msg::Canvas(CanvasMsg::ToolDrag(
                        to_img(pos),
                    ))));
                }

                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };

                if amount == 0.0 {
                    return None;
                }

                let new_zoom = (zoom * (1.0 + amount * 0.15).max(0.1)).clamp(ZOOM_MIN, ZOOM_MAX);
                // Keep the crop point under the cursor fixed as zoom changes.
                let ip = to_img(pos);
                let anchored =
                    Viewport::resolve(bounds.size(), (cw, ch), Some(new_zoom), Vector::ZERO);
                let s = anchored.to_screen(ip.x, ip.y);
                let pan = Vector::new(pos.x - s.x, pos.y - s.y);

                Some(
                    shader::Action::publish(Msg::Canvas(CanvasMsg::SetViewport {
                        zoom: new_zoom,
                        pan,
                    }))
                    .and_capture(),
                )
            }
            _ => None,
        }
    }

    /// The cursor shown over the preview: a grabbing hand while panning, else
    /// the active tool's cursor when the pointer is over the pane.
    fn interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.panning {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            self.tool.cursor()
        } else {
            mouse::Interaction::default()
        }
    }

    /// Draws the active raster image at the live viewport, or nothing when the
    /// active view is not a raster (the vector views draw through the shader
    /// widget) or has not rendered yet.
    fn draw_raster(&self, renderer: &iced::Renderer, bounds: Rectangle) -> Vec<Geometry> {
        let Some(Art::Raster { img, factor }) = &self.art else {
            return Vec::new();
        };

        let dims = self.art.as_ref().map(Art::dims).unwrap_or((1.0, 1.0));
        let live = Viewport::resolve(bounds.size(), dims, self.zoom, self.pan);
        let zoom = live.zoom;

        // One frame sized to the pane clips anything past the edge so the art
        // never bleeds over adjacent panes.
        let mut frame = Frame::new(renderer, bounds.size());
        let disp = Rectangle::new(live.origin, Size::new(dims.0 * zoom, dims.1 * zoom));

        // The raster is denser than crop space by `factor`, so its per-pixel
        // screen size is the crop zoom over that.
        let filter = if zoom / factor >= 3.0 {
            core_image::FilterMethod::Nearest
        } else {
            core_image::FilterMethod::Linear
        };

        frame.draw_image(
            disp,
            core_image::Image::new(img.handle.clone()).filter_method(filter),
        );

        vec![frame.into_geometry()]
    }
}

impl canvas::Program<Msg> for Preview<'_> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Msg>> {
        self.gesture(state, event, bounds, cursor)
    }

    fn draw(
        &self,
        _state: &State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        self.draw_raster(renderer, bounds)
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        self.interaction(state, bounds, cursor)
    }
}

impl shader::Program<Msg> for Preview<'_> {
    type State = State;
    type Primitive = VectorPrimitive;

    fn update(
        &self,
        state: &mut State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Msg>> {
        self.gesture(state, event, bounds, cursor)
    }

    fn draw(&self, _state: &State, _cursor: mouse::Cursor, bounds: Rectangle) -> VectorPrimitive {
        // The live viewport places the art at its centered, panned position and
        // resolves the concrete zoom; the primitive derives the physical
        // transform from this and the render viewport's scale factor.
        let dims = self.art.as_ref().map(Art::dims).unwrap_or((1.0, 1.0));
        let live = Viewport::resolve(bounds.size(), dims, self.zoom, self.pan);

        let scene = match &self.art {
            Some(Art::Vector(scene)) => scene.clone(),
            // preview() only wraps a vector view in the shader widget, so this
            // is unreachable; an empty scene renders transparent if it is hit.
            _ => crate::gui::compute::VectorScene::empty(),
        };

        VectorPrimitive::new(scene, (live.origin.x, live.origin.y), live.zoom)
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        self.interaction(state, bounds, cursor)
    }
}
