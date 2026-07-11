//! The preview canvas: the active image over a transparency checkerboard,
//! with drag-to-pan and cursor-anchored scroll-to-zoom. Pin presses are
//! published in the shown view's image pixels; the pan/zoom gestures are
//! resolved here and published as a viewport.

use super::checkerboard::checkerboard;
use super::viewport::Viewport;
use crate::color::Srgb;
use crate::gui::app::App;
use crate::gui::compute::{Art, ArtKey, VectorScene};
use crate::gui::msg::{CanvasMsg, Msg};
use crate::gui::overlays::{self, OverlayCtx};
use crate::gui::tools::Tool;
use iced::advanced::image as core_image;
use iced::mouse;
use iced::widget::canvas::fill::Rule;
use iced::widget::canvas::{
    Action, Cache, Event, Fill, Frame, Geometry, Path, Program, Stroke, Style,
};
use iced::widget::Stack;
use iced::{Color, Element, Length, Point, Rectangle, Size, Vector};
use std::cell::Cell;

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

    let base = iced::widget::canvas(program)
        .width(Length::Fill)
        .height(Length::Fill);

    let ctx = OverlayCtx::from_app(app);

    let mut stack = Stack::new().push(base);

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
    /// The checkerboard and art, redrawn only when the viewport or image
    /// changes so other widgets' animation frames don't re-tessellate them.
    statics: Cache,
    /// The inputs `statics` was last drawn for. A mismatch clears the cache.
    statics_key: Cell<Option<StaticKey>>,
}

/// The viewport inputs that determine the cached static layers. When any
/// differ from the last draw, the checkerboard and art are stale.
#[derive(Clone, Copy, PartialEq)]
struct StaticKey {
    art: Option<ArtKey>,
    zoom: Option<f32>,
    pan: (f32, f32),
    bounds: (f32, f32),
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
    ) -> Option<Action<Msg>> {
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

        Some(Action::publish(Msg::Canvas(CanvasMsg::SelectAt(doc_px))))
    }
}

impl Program<Msg> for Preview<'_> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Msg>> {
        // A release outside the canvas still ends a drag; handling it before
        // the cursor-in-bounds gate keeps a pan from sticking to the cursor
        // when it re-enters with no button held.
        if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            if state.panning || state.tool_active {
                let was_tool = state.tool_active;
                let press = state.press.take();

                state.panning = false;
                state.tool_active = false;
                state.last = None;

                if was_tool {
                    return Some(Action::publish(Msg::Canvas(CanvasMsg::ToolRelease)));
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
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
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

                    return Some(Action::capture());
                }

                if captures {
                    let ip = to_img(pos);

                    if ip.x >= 0.0 && ip.y >= 0.0 && ip.x < cw && ip.y < ch {
                        state.tool_active = true;

                        return Some(
                            Action::publish(Msg::Canvas(CanvasMsg::ToolPress(ip))).and_capture(),
                        );
                    }
                }
                None
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.panning {
                    let last = state.last?;

                    state.last = Some(pos);

                    let pan = self.pan + Vector::new(pos.x - last.x, pos.y - last.y);

                    return Some(Action::publish(Msg::Canvas(CanvasMsg::SetViewport {
                        zoom,
                        pan,
                    })));
                }

                if state.tool_active {
                    return Some(Action::publish(Msg::Canvas(CanvasMsg::ToolDrag(to_img(
                        pos,
                    )))));
                }

                None
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
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
                    Action::publish(Msg::Canvas(CanvasMsg::SetViewport {
                        zoom: new_zoom,
                        pan,
                    }))
                    .and_capture(),
                )
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let key = StaticKey {
            art: self.art.as_ref().map(Art::key),
            zoom: self.zoom,
            pan: (self.pan.x, self.pan.y),
            bounds: (bounds.width, bounds.height),
        };

        if state.statics_key.get() != Some(key) {
            state.statics.clear();
            state.statics_key.set(Some(key));
        }

        let statics = state.statics.draw(renderer, bounds.size(), |frame| {
            checkerboard(
                frame,
                Point::ORIGIN,
                bounds.width,
                bounds.height,
                bounds.size(),
            );

            if let Some(art) = &self.art {
                let dims = art.dims();
                let vp = Viewport::resolve(bounds.size(), dims, self.zoom, self.pan);

                match art {
                    Art::Raster { img, factor } => {
                        let disp = Rectangle::new(
                            vp.origin,
                            Size::new(dims.0 * vp.zoom, dims.1 * vp.zoom),
                        );

                        // The raster is denser than crop space by `factor`, so
                        // its per-pixel screen size is the crop zoom over that.
                        let filter = if vp.zoom / factor >= 3.0 {
                            core_image::FilterMethod::Nearest
                        } else {
                            core_image::FilterMethod::Linear
                        };

                        frame.draw_image(
                            disp,
                            core_image::Image::new(img.handle.clone()).filter_method(filter),
                        );
                    }

                    Art::Vector(scene) => draw_vector(frame, scene, &vp),
                }
            }
        });
        vec![statics]
    }

    fn mouse_interaction(
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
}

/// Screen position of a supersample-space point: dividing by `scale` gives crop
/// px, which `vp` maps to screen, baking the zoom into the coordinate.
fn scene_to_screen(scale: u32, vp: &Viewport, (x, y): (f64, f64)) -> Point {
    let s = scale as f32;
    vp.to_screen(x as f32 / s, y as f32 / s)
}

/// Fills each color run of `scene` under the nonzero rule, bottom layer first,
/// stroking runs whose layer carries a stroke over their own fill so a later
/// run covers an earlier run's stroke exactly as the SVG export paints them.
fn draw_vector(frame: &mut Frame, scene: &VectorScene, vp: &Viewport) {
    let pt = |p: (f64, f64)| scene_to_screen(scene.scale, vp, p);

    for layer in &scene.layers {
        // A source-px stroke width scales with the crop-space zoom to stay a
        // fixed source-px width on screen.
        let stroke = layer
            .stroke
            .as_ref()
            .and_then(|st| Some((Srgb::from_hex(&st.hex)?, st.width * vp.zoom)));

        for (hex, paths) in layer.colors.iter() {
            let path = Path::new(|b| {
                for p in paths {
                    b.move_to(pt(p.start));
                    for &(c1, c2, end) in &p.cubics {
                        b.bezier_curve_to(pt(c1), pt(c2), pt(end));
                    }
                    b.close();
                }
            });

            if let Some(fill) = Srgb::from_hex(hex) {
                frame.fill(
                    &path,
                    Fill {
                        style: Style::Solid(to_color(fill)),
                        rule: Rule::NonZero,
                    },
                );
            }

            if let Some((color, width)) = stroke {
                frame.stroke(
                    &path,
                    Stroke::default()
                        .with_color(to_color(color))
                        .with_width(width),
                );
            }
        }
    }
}

fn to_color(c: Srgb) -> Color {
    Color::from_rgb8(c.r(), c.g(), c.b())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::compute::{LayerTrace, VectorLayer};
    use std::sync::Arc;

    fn vp() -> Viewport {
        // An explicit zoom keeps the transform independent of canvas bounds.
        Viewport::resolve(
            Size::new(100.0, 100.0),
            (10.0, 10.0),
            Some(4.0),
            Vector::ZERO,
        )
    }

    // A supersample coordinate is divided by the scale to reach crop px, then
    // placed by the same viewport the anchors overlay uses, so a filled path and
    // its anchor dots land on identical screen points.
    #[test]
    fn scene_point_divides_by_scale_then_places() {
        let vp = vp();
        let scale = 5;

        // (20, 35) at scale 5 is crop px (4, 7).
        assert_eq!(
            scene_to_screen(scale, &vp, (20.0, 35.0)),
            vp.to_screen(4.0, 7.0)
        );

        // The origin maps to the art's screen origin.
        assert_eq!(scene_to_screen(scale, &vp, (0.0, 0.0)), vp.origin);
    }

    fn scene(colors: Arc<LayerTrace>) -> VectorScene {
        VectorScene {
            dims: (10, 10),
            scale: 5,
            layers: vec![VectorLayer {
                colors,
                stroke: None,
            }],
        }
    }

    // The geometry cache keys on art content: the same trace `Arc` keeps the key
    // fixed (so an unrelated animation frame reuses the tessellation), a new
    // trace `Arc` moves it, and a stroke change moves it even on the same trace.
    #[test]
    fn vector_art_key_tracks_content_not_frames() {
        let trace: Arc<LayerTrace> = Arc::new(vec![]);
        let a = Art::Vector(scene(trace.clone()));
        let b = Art::Vector(scene(trace.clone()));
        assert_eq!(a.key(), b.key(), "same trace Arc keeps the cache key");

        let c = Art::Vector(scene(Arc::new(vec![])));
        assert_ne!(a.key(), c.key(), "a fresh trace Arc moves the key");

        let stroked = Art::Vector(VectorScene {
            dims: (10, 10),
            scale: 5,
            layers: vec![VectorLayer {
                colors: trace,
                stroke: Some(crate::output::Stroke {
                    hex: "#ffffff".into(),
                    width: 4.0,
                }),
            }],
        });
        assert_ne!(a.key(), stroked.key(), "adding a stroke moves the key");
    }
}
