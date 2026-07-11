//! Raster helpers turning pipeline output into display images: the raw handle
//! wrapper, the mask composite, and the SVG rasterizer.

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

pub(super) fn render_svg(svg: &str, w: u32, h: u32) -> Option<Img> {
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &Default::default()).ok()?;
    let sz = tree.size();
    let scale = (w as f32 / sz.width()).min(h as f32 / sz.height());
    let (pw, ph) = ((sz.width() * scale) as u32, (sz.height() * scale) as u32);
    let mut pix = resvg::tiny_skia::Pixmap::new(pw.max(1), ph.max(1))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pix.as_mut(),
    );
    let size = (pix.width(), pix.height());
    Some(Img {
        handle: iced_image::Handle::from_rgba(size.0, size.1, pix.take()),
        size,
    })
}
