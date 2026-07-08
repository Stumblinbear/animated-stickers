//! The image-to-screen transform shared by the preview canvas and its pin
//! overlay. Both map document image pixels onto the same on-screen rectangle;
//! resolving the fit, origin, and zoom in one place keeps the two sibling
//! canvases from drifting and desyncing pins from the art.

use iced::{Point, Size, Vector};

/// Where the art sits within a canvas of some bounds: its top-left corner on
/// screen and the zoom it is drawn at.
#[derive(Clone, Copy)]
pub struct Viewport {
    pub origin: Point,
    pub zoom: f32,
}

impl Viewport {
    /// Resolves the transform for an image of `dims` shown in `bounds` under an
    /// optional explicit `zoom` (`None` fits the image to the canvas) and a
    /// `pan` offset in screen pixels. `dims` and `zoom` share one space: pass
    /// crop-space dimensions to place the art independently of a raster's pixel
    /// density.
    pub fn resolve(bounds: Size, dims: (f32, f32), zoom: Option<f32>, pan: Vector) -> Self {
        let (iw, ih) = dims;
        let zoom = zoom.unwrap_or_else(|| fit(bounds, iw, ih));
        let origin = Point::new(
            bounds.width / 2.0 - iw * zoom / 2.0 + pan.x,
            bounds.height / 2.0 - ih * zoom / 2.0 + pan.y,
        );
        Self { origin, zoom }
    }

    /// Maps an image pixel to its position on screen.
    pub fn to_screen(self, px: f32, py: f32) -> Point {
        Point::new(self.origin.x + px * self.zoom, self.origin.y + py * self.zoom)
    }

    /// Maps an on-screen point back to image-pixel coordinates.
    pub fn to_image(self, p: Point) -> Point {
        Point::new((p.x - self.origin.x) / self.zoom, (p.y - self.origin.y) / self.zoom)
    }
}

/// The zoom that fits an `iw`×`ih` image within `bounds`, floored so a
/// degenerate size can't collapse the transform.
pub fn fit(bounds: Size, iw: f32, ih: f32) -> f32 {
    (bounds.width / iw).min(bounds.height / ih).max(0.01)
}
