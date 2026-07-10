//! Raster helpers turning pipeline output into display images.

use super::Img;
use crate::{pipeline, regions};
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

/// Regions view: each region painted in its own quantized color, so the image
/// reads as the art. The trace-fate tint ([`fate_tint_handle`]) and the pin
/// markers are drawn as overlays over this, not baked in, so a pin edit
/// re-tints without rebuilding this raster.
pub(super) fn regions_handle(regs: &[regions::Region], (w, h): (u32, u32)) -> Img {
    let mut bytes = vec![0u8; (w * h * 4) as usize];
    for r in regs {
        for &(px, py) in &r.pixels {
            let idx = (((r.y0 + py) * w + (r.x0 + px)) * 4) as usize;
            bytes[idx..idx + 3].copy_from_slice(&r.color);
            bytes[idx + 3] = 255;
        }
    }
    Img {
        handle: iced_image::Handle::from_rgba(w, h, bytes),
        size: (w, h),
    }
}

/// Trace-fate tint over the segmentation, for the fates overlay to composite on
/// the Regions view. Transparent everywhere except regions the trace will not
/// keep as their own shape: red marks a culled region (below the speckle floor,
/// no neighbor to merge into, unpinned; it vanishes silently), orange one the
/// speckle merge folds into a neighbor (it survives as pixels, losing its color
/// and path). Returns `None` when every region survives, so the overlay draws
/// nothing.
pub(super) fn fate_tint_handle(
    regs: &[regions::Region],
    (w, h): (u32, u32),
    fates: &[regions::Fate],
) -> Option<Img> {
    // Straight alpha matching the old baked 0.5 blend once composited over the
    // segmentation: 128/255 ≈ 0.5.
    const TINT_A: u8 = 128;
    let mut bytes = vec![0u8; (w * h * 4) as usize];
    let mut any = false;
    for (i, r) in regs.iter().enumerate() {
        let tint = match fates.get(i) {
            Some(regions::Fate::Culled) => [230, 55, 45],
            Some(regions::Fate::MergedInto(_)) => [240, 150, 40],
            _ => continue,
        };
        any = true;
        for &(px, py) in &r.pixels {
            let idx = (((r.y0 + py) * w + (r.x0 + px)) * 4) as usize;
            bytes[idx..idx + 3].copy_from_slice(&tint);
            bytes[idx + 3] = TINT_A;
        }
    }
    any.then(|| Img {
        handle: iced_image::Handle::from_rgba(w, h, bytes),
        size: (w, h),
    })
}

/// Renders the smooth-and-corners debug view: each contour as a thin
/// polyline (blue, or green over stretches fit at the slackened seam
/// tolerance) with an orange dot on every corner vertex, on transparent
/// backing. Coordinates are in scaled space, so the viewBox matches.
pub(super) fn render_debug(
    contours: &[pipeline::DebugContour],
    w: u32,
    h: u32,
    scale: u32,
) -> Option<Img> {
    let (vw, vh) = (w * scale, h * scale);
    let mut s = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {vw} {vh}">"#);
    let line = scale as f32 * 0.5;
    for c in contours {
        if c.points.len() < 2 {
            continue;
        }
        let mut d = String::new();
        for (x, y) in &c.points {
            d += &format!("{x:.1},{y:.1} ");
        }
        // Close the ring back to its first vertex.
        let (fx, fy) = c.points[0];
        d += &format!("{fx:.1},{fy:.1}");
        s += &format!(
            r##"<polyline points="{d}" fill="none" stroke="#6ea8ff" stroke-width="{line}"/>"##
        );
        // Overlay each edge touching a slack vertex, so slackened seams read
        // apart from the base outline. Drawn on top at the same width.
        let n = c.points.len();
        for i in 0..n {
            let j = (i + 1) % n;
            if !c.slack.get(i).copied().unwrap_or(false) && !c.slack.get(j).copied().unwrap_or(false)
            {
                continue;
            }
            let (x1, y1) = c.points[i];
            let (x2, y2) = c.points[j];
            s += &format!(
                r##"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="#48d597" stroke-width="{line}"/>"##
            );
        }
    }
    let dot = scale as f32 * 1.2;
    for c in contours {
        for (x, y) in &c.corners {
            s += &format!(r##"<circle cx="{x:.1}" cy="{y:.1}" r="{dot}" fill="#ff9d3c"/>"##);
        }
    }
    s += "</svg>";
    render_svg(&s, w * 2, h * 2)
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
