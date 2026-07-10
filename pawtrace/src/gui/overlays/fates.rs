//! The fates overlay: the trace-fate tint composited over the segmentation on
//! the Regions view. The tint raster is built in compute-land (see
//! [`crate::gui::compute`]) and drawn here as one image over the art, so the
//! per-pixel work stays off the draw path and a pin edit re-tints without
//! rebuilding the segmentation raster.

use super::OverlayCtx;
use crate::gui::compute::Img;
use crate::gui::msg::Msg;
use crate::gui::phases::SubView;
use crate::gui::view::viewport::Viewport;
use iced::advanced::image as core_image;
use iced::mouse;
use iced::widget::canvas::{Frame, Geometry, Program};
use iced::{Element, Length, Rectangle, Size};

/// The fate tint over the segmentation, or nothing when the Regions view is not
/// showing or no region has a non-surviving fate.
pub fn overlay<'a>(ctx: &OverlayCtx<'a>) -> Option<Element<'a, Msg>> {
    if ctx.subview != Some(SubView::Regions) {
        return None;
    }
    let tint = FateTint {
        tint: ctx.fate_tint?.clone(),
        zoom: ctx.zoom,
        pan: ctx.pan,
        factor: ctx.factor,
    };
    Some(
        iced::widget::canvas(tint)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    )
}

struct FateTint {
    tint: Img,
    zoom: Option<f32>,
    pan: iced::Vector,
    /// Screen-raster px per crop px, matching the preview so the tint lands on
    /// the same rectangle as the segmentation it colors.
    factor: f32,
}

impl Program<Msg> for FateTint {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = self.tint.size;
        let dims = (w as f32 / self.factor, h as f32 / self.factor);
        let vp = Viewport::resolve(bounds.size(), dims, self.zoom, self.pan);
        let rect = Rectangle::new(vp.origin, Size::new(dims.0 * vp.zoom, dims.1 * vp.zoom));
        // Match the preview's filter so the tint's region edges align with the
        // segmentation's.
        let filter = if vp.zoom / self.factor >= 3.0 {
            core_image::FilterMethod::Nearest
        } else {
            core_image::FilterMethod::Linear
        };
        frame.draw_image(
            rect,
            core_image::Image::new(self.tint.handle.clone()).filter_method(filter),
        );
        vec![frame.into_geometry()]
    }
}
