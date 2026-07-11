//! Raster helpers turning pipeline output into display images: the raw handle
//! wrapper and the mask composite.

use super::Img;
use iced::widget::image as iced_image;
use image::RgbaImage;

pub(super) fn rgba_img(img: &RgbaImage) -> Img {
    Img {
        handle: iced_image::Handle::from_rgba(img.width(), img.height(), img.as_raw().clone()),
        size: img.dimensions(),
    }
}

/// RGB plus its alpha mask as displayable RGBA: pixels outside the mask
/// become fully transparent instead of exposing the meaningless zero fill.
pub(super) fn masked(img: &image::RgbImage, alpha: &image::GrayImage) -> RgbaImage {
    let mut out = RgbaImage::new(img.width(), img.height());

    for (o, (p, a)) in out.pixels_mut().zip(img.pixels().zip(alpha.pixels())) {
        o.0 = [p.0[0], p.0[1], p.0[2], a.0[0]];
    }

    out
}
