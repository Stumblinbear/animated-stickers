//! The color vocabulary shared across the pipeline, config, and GUI: sRGB
//! byte colors with their hex form, and OKLab, the space every perceptual
//! comparison runs in.

/// An sRGB color, one byte per channel.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Srgb(pub [u8; 3]);

impl Srgb {
    /// Parses "#rrggbb" or "rrggbb"; `None` on anything malformed.
    pub fn from_hex(s: &str) -> Option<Srgb> {
        let s = s.trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Srgb([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
    }

    /// The "#rrggbb" form.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }

    /// The red channel.
    pub fn r(self) -> u8 {
        self.0[0]
    }

    /// The green channel.
    pub fn g(self) -> u8 {
        self.0[1]
    }

    /// The blue channel.
    pub fn b(self) -> u8 {
        self.0[2]
    }

    /// Perceptual distance to `o` (OKLab ΔE), living roughly on 0..1.
    pub fn dist(self, o: Srgb) -> f32 {
        Lab::from(self).dist(Lab::from(o))
    }

}

impl From<Srgb> for image::Rgb<u8> {
    fn from(c: Srgb) -> image::Rgb<u8> {
        image::Rgb(c.0)
    }
}

/// The color as a fully opaque pixel.
impl From<Srgb> for image::Rgba<u8> {
    fn from(c: Srgb) -> image::Rgba<u8> {
        image::Rgba([c.0[0], c.0[1], c.0[2], 255])
    }
}

/// A color in OKLab (Ottosson 2020), perceptually uniform: Euclidean distance
/// here tracks visual difference, unlike RGB or redmean. Wrapping the triple
/// keeps an sRGB byte color from entering a ΔE comparison unconverted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Lab(pub [f32; 3]);

impl From<Srgb> for Lab {
    fn from(srgb: Srgb) -> Lab {
        // The channel input is 8-bit, so the three powf calls (most of the
        // conversion's cost, per-pixel in feature detection and remap) fold
        // into one 256-entry table.
        static LIN: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
            std::array::from_fn(|u| {
                let x = u as f32 / 255.0;
                if x >= 0.04045 {
                    ((x + 0.055) / 1.055).powf(2.4)
                } else {
                    x / 12.92
                }
            })
        });
        let c = srgb.0;
        let (r, g, b) = (LIN[c[0] as usize], LIN[c[1] as usize], LIN[c[2] as usize]);
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
        let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
        Lab([
            0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
        ])
    }
}

impl Lab {
    /// Euclidean ΔE to `o`.
    pub fn dist(self, o: Lab) -> f32 {
        self.dist2(o).sqrt()
    }

    /// Squared ΔE to `o`, saving the sqrt where only comparisons happen.
    pub fn dist2(self, o: Lab) -> f32 {
        let (d0, d1, d2) = (self.0[0] - o.0[0], self.0[1] - o.0[1], self.0[2] - o.0[2]);
        d0 * d0 + d1 * d1 + d2 * d2
    }

    /// Distance to the segment between `a` and `b`.
    pub fn seg_dev(self, a: Lab, b: Lab) -> f32 {
        let ab = [b.0[0] - a.0[0], b.0[1] - a.0[1], b.0[2] - a.0[2]];
        let ap = [self.0[0] - a.0[0], self.0[1] - a.0[1], self.0[2] - a.0[2]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        let t = if len2 > 0.0 {
            ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let on = Lab([a.0[0] + t * ab[0], a.0[1] + t * ab[1], a.0[2] + t * ab[2]]);
        self.dist(on)
    }

    /// Whether this color reads as an anti-alias mixture of `a` and `b`:
    /// within `dev` of the segment between them and at least `jnd` away from
    /// each endpoint, so a mark sharing an endpoint's color never matches.
    pub fn blend_between(self, a: Lab, b: Lab, jnd: f32, dev: f32) -> bool {
        if self.dist(a) <= jnd || self.dist(b) <= jnd {
            return false;
        }
        self.seg_dev(a, b) < dev
    }
}
