//! The Lock tool: click a swatch on the Palette view to lock or unlock its
//! color, protecting it from being merged away. Locked colors are a document
//! property the tool writes, never tool state. Offered only on the Palette
//! sub-view, where the swatches are shown.

use crate::gui::app::App;
use crate::gui::msg::{Msg, StripView};
use crate::gui::phases::SubView;
use crate::gui::view::icons;
use iced::{Point, Task};

pub const ICON: char = icons::PIPETTE;

/// Offered only when the Palette sub-view is shown.
pub fn applies(_view: StripView, sub: Option<SubView>) -> bool {
    sub == Some(SubView::Palette)
}

/// Locks or unlocks the palette color under source-crop px `p`.
pub fn press(app: &mut App, p: Point) -> Task<Msg> {
    let Some(sess) = app.session() else {
        return Task::none();
    };

    let Some(q) = &sess.preview.remap_px else {
        return Task::none();
    };

    // The remap raster is the crop supersampled by `scale`; map crop px into it.
    let s = sess.cfg.scale as f32;
    let (x, y) = ((p.x * s) as u32, (p.y * s) as u32);
    if x >= q.width() || y >= q.height() {
        return Task::none();
    }

    let px = q.get_pixel(x, y).0;
    let c = [px[0], px[1], px[2]];
    if px[3] != 0 && sess.preview.palette.contains(&c) {
        app.toggle_lock(c)
    } else {
        Task::none()
    }
}
