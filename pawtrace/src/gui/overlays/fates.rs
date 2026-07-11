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
        dims: ctx.dims?,
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
    /// The shown art's crop-space dimensions, matching the preview so the tint
    /// lands on the same rectangle as the segmentation it colors.
    dims: (f32, f32),
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
        let vp = Viewport::resolve(bounds.size(), self.dims, self.zoom, self.pan);
        let rect = Rectangle::new(
            vp.origin,
            Size::new(self.dims.0 * vp.zoom, self.dims.1 * vp.zoom),
        );

        // The tint raster spans the same crop rectangle as the segmentation, so
        // its density is its pixel width over the crop width. Match the preview's
        // filter at that density so the region edges align.
        let factor = self.tint.size.0 as f32 / self.dims.0;

        let filter = if vp.zoom / factor >= 3.0 {
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
