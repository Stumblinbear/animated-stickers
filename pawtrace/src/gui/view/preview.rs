//! The preview canvas: the active image over a transparency checkerboard,
//! with drag-to-pan and cursor-anchored scroll-to-zoom. Pin presses are
//! published in the shown view's image pixels; the pan/zoom gestures are
//! resolved here and published as a viewport.

use super::overlays::pin_overlay;
use super::viewport::Viewport;
use crate::gui::app::App;
use crate::gui::compute::Img;
use crate::gui::msg::{CanvasMsg, Msg, Tool};
use iced::advanced::image as core_image;
use iced::mouse;
use iced::widget::canvas::{Action, Cache, Event, Frame, Geometry, Program};
use iced::widget::stack;
use iced::{Color, Element, Length, Point, Rectangle, Size, Vector};
use std::cell::Cell;

const CHECK_LIGHT: Color = Color {
    r: 0.16,
    g: 0.16,
    b: 0.18,
    a: 1.0,
};
const CHECK_DARK: Color = Color {
    r: 0.11,
    g: 0.11,
    b: 0.13,
    a: 1.0,
};
const TILE: f32 = 8.0;
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 32.0;

/// Builds the preview widget over the document's active image and viewport,
/// with the selected layer's pins overlaid on top.
pub fn preview(app: &App) -> Element<'_, Msg> {
    let program = Preview {
        img: app.active_image(),
        zoom: app.session().and_then(|s| s.zoom()),
        pan: app.session().map(|s| s.pan()).unwrap_or(Vector::ZERO),
        factor: app.view_density(),
        tool: app.tool,
    };
    let base = iced::widget::canvas(program)
        .width(Length::Fill)
        .height(Length::Fill);
    stack![base, pin_overlay(app)].into()
}

struct Preview<'a> {
    img: Option<&'a Img>,
    zoom: Option<f32>,
    pan: Vector,
    /// Screen-raster px per crop px for the shown view; the raster's size over
    /// this is the crop-space dimensions the viewport works in.
    factor: f32,
    tool: Tool,
}

#[derive(Default)]
struct State {
    /// Last cursor position while panning; `None` when not dragging.
    last: Option<Point>,
    panning: bool,
    /// A pin press is in progress, so cursor moves drag the tool.
    tool_active: bool,
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
    img: Option<core_image::Id>,
    zoom: Option<f32>,
    pan: (f32, f32),
    bounds: (f32, f32),
}

impl Preview<'_> {
    /// The shown raster's crop-space dimensions: raster pixels over the view's
    /// density, so every stage view resolves to the same on-screen rectangle.
    fn canonical(&self) -> Option<(f32, f32)> {
        let img = self.img?;
        Some((img.size.0 as f32 / self.factor, img.size.1 as f32 / self.factor))
    }

    /// The on-screen rectangle the art occupies for `size`, and the crop-space
    /// zoom it is drawn at. `None` with no image.
    fn art_rect(&self, size: Size) -> Option<(Rectangle, f32)> {
        let (cw, ch) = self.canonical()?;
        let vp = Viewport::resolve(size, (cw, ch), self.zoom, self.pan);
        Some((
            Rectangle::new(vp.origin, Size::new(cw * vp.zoom, ch * vp.zoom)),
            vp.zoom,
        ))
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
                state.panning = false;
                state.tool_active = false;
                state.last = None;
                return was_tool.then(|| Action::publish(Msg::Canvas(CanvasMsg::ToolRelease)));
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
                if mid || (left && self.tool == Tool::Select) {
                    state.panning = true;
                    state.last = Some(pos);
                    return Some(Action::capture());
                }
                if left && self.tool == Tool::Pin {
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
                    return Some(Action::publish(Msg::Canvas(CanvasMsg::SetViewport { zoom, pan })));
                }
                if state.tool_active {
                    return Some(Action::publish(Msg::Canvas(CanvasMsg::ToolDrag(to_img(pos)))));
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
                let anchored = Viewport::resolve(bounds.size(), (cw, ch), Some(new_zoom), Vector::ZERO);
                let s = anchored.to_screen(ip.x, ip.y);
                let pan = Vector::new(pos.x - s.x, pos.y - s.y);
                Some(
                    Action::publish(Msg::Canvas(CanvasMsg::SetViewport { zoom: new_zoom, pan }))
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
            img: self.img.map(|i| i.handle.id()),
            zoom: self.zoom,
            pan: (self.pan.x, self.pan.y),
            bounds: (bounds.width, bounds.height),
        };
        if state.statics_key.get() != Some(key) {
            state.statics.clear();
            state.statics_key.set(Some(key));
        }
        let statics = state.statics.draw(renderer, bounds.size(), |frame| {
            checkerboard(frame, Point::ORIGIN, bounds.width, bounds.height, bounds.size());
            if let (Some(img), Some((disp, zoom))) = (self.img, self.art_rect(bounds.size())) {
                // The raster is denser than crop space by `factor`, so its
                // per-pixel screen size is the crop zoom over that.
                let filter = if zoom / self.factor >= 3.0 {
                    core_image::FilterMethod::Nearest
                } else {
                    core_image::FilterMethod::Linear
                };
                frame.draw_image(
                    disp,
                    core_image::Image::new(img.handle.clone()).filter_method(filter),
                );
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
            match self.tool {
                Tool::Select => mouse::Interaction::Grab,
                Tool::Pin => mouse::Interaction::Crosshair,
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Fills the display rect with the two-tone alpha checker, iterating only the
/// tiles visible within `size` so a deep zoom stays cheap.
fn checkerboard(frame: &mut Frame, origin: Point, dw: f32, dh: f32, size: Size) {
    let x0 = origin.x.max(0.0);
    let y0 = origin.y.max(0.0);
    let x1 = (origin.x + dw).min(size.width);
    let y1 = (origin.y + dh).min(size.height);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    frame.fill_rectangle(Point::new(x0, y0), Size::new(x1 - x0, y1 - y0), CHECK_LIGHT);
    let col0 = ((x0 - origin.x) / TILE).floor() as i32;
    let col1 = ((x1 - origin.x) / TILE).ceil() as i32;
    let row0 = ((y0 - origin.y) / TILE).floor() as i32;
    let row1 = ((y1 - origin.y) / TILE).ceil() as i32;
    for row in row0..row1 {
        for col in col0..col1 {
            if (row + col) % 2 == 0 {
                continue;
            }
            let px = (origin.x + col as f32 * TILE).max(x0);
            let py = (origin.y + row as f32 * TILE).max(y0);
            let pw = (origin.x + (col + 1) as f32 * TILE).min(x1) - px;
            let ph = (origin.y + (row + 1) as f32 * TILE).min(y1) - py;
            if pw > 0.0 && ph > 0.0 {
                frame.fill_rectangle(Point::new(px, py), Size::new(pw, ph), CHECK_DARK);
            }
        }
    }
}
