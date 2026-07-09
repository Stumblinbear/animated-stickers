/// One perceptual constant drives derived params (ported from vectorize.py).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Smallest feature (px at output/512 scale) worth keeping.
    pub detail: f32, // default 5.0
    /// Supersample factor. 3 keeps ~1px linework alive through the mode filter.
    pub scale: u32, // default 3
    pub alpha_threshold: u8, // default 128 (50%)
    /// Palette safety cap; extraction self-terminates below it.
    pub max_colors: usize, // default 24
    /// Pre-quantization mode-filter kernel (odd, scaled px). 0 = off.
    pub mode_filter: u32, // default 0
    /// Majority-vote kernel (odd, scaled px) over the quantized colors,
    /// after remap. Settles jagged or speckled boundaries where two similar
    /// palette colors were assigned noisily. 0 = off.
    pub color_cleanup: u32, // default 0
    /// Moving-average window applied to pixel-boundary vertices before curve
    /// fitting, corners pinned, as a multiple of the supersample scale (the
    /// wobble's own wavelength). Larger rounds off more of the pixel
    /// staircase. 0 = none.
    pub smoothing: f32, // default 1.0 (== one scale of window)
    /// OKLab ΔE under which a thin region is absorbed into a spatially
    /// adjacent one. Collapses gradient transition bands, which are thin and
    /// low-contrast to their neighbors by construction; linework survives on
    /// contrast, solid blobs on width. 0 = off.
    pub absorb_dist: f32, // default 0.08
    /// Multiplies the band-thickness ceiling for absorption. Absorption
    /// only takes bands under 2.5*scale mean and 4*scale max width; this
    /// scales both, so >1 absorbs chunkier bands and <1 only the thinnest.
    /// Deliberate features cross the raised ceiling too, so pin any this
    /// erases.
    pub absorb_aggr: f32, // default 1.0
    /// OKLab ΔE under which two adjacent thin regions merge as segments of
    /// one stroke. Quantizing shaded artwork cuts a drawn line (an outline
    /// crossing a gradient) into pieces of interchangeable colors, each
    /// otherwise traced as its own path with a visible joint. 0 = off.
    pub stroke_merge_dist: f32, // default 0.08 (OKLab ΔE)
    /// Max width, in source px, a region may have and still count as a
    /// stroke segment for [`stroke_merge_dist`](Self::stroke_merge_dist).
    /// Wide regions are gradient banding or fills, kept apart so their
    /// structure survives. 0 = off.
    pub stroke_merge_width: f32, // default 4.0 (source px)
    /// Colors seeded into the palette unconditionally (no count floor, no
    /// merge test); other candidates merge toward them. Locked from the GUI
    /// or per profile.
    pub locked: Vec<[u8; 3]>, // default empty
    /// Representative bands a gradient family consolidates to in region-first
    /// extraction: a run of adjacent features whose colors ramp collinearly is
    /// kept as at most this many colors spanning it, the rest remapped to the
    /// nearest kept band. Permissive by default because deliberate banding is a
    /// style an artist dials up per profile, not a defect to collapse; only a
    /// family longer than this loses bands.
    pub gradient_bands: u32, // default 6
    /// Points in document source px. Any region containing one survives the
    /// speckle floor: a pin marks a small feature (a tooth, a glint) as
    /// deliberate, and outlives re-segmentation because whatever region
    /// holds the point is what gets kept.
    pub pins: Vec<[u32; 2]>, // default empty
    /// Curve smoothing ("Simplify"): corner threshold; 4/3 = no corners.
    pub alphamax: f64, // default 1.15
    pub opttolerance: f64,   // default 0.4 (saturates quickly; alphamax is the lever)
    /// Fit-tolerance multiplier for boundary stretches whose far side is a
    /// low-contrast neighboring region: those seams fit at `opttolerance *
    /// seam_slack`, everything else (transparency, high-contrast neighbors) at
    /// base `opttolerance`. A neighbor counts as low-contrast when its color
    /// sits within `2 * stroke_merge_dist` (OKLab ΔE) of the shape's own
    /// color. 1.0 = off (base tolerance everywhere).
    pub seam_slack: f64, // default 1.0 (off)
    /// Final anchor-reduction pass: removes any anchor whose deletion keeps
    /// the path within this many scaled px, merging its two segments into
    /// one cubic. Corners survive. Independent of opttolerance, which sets
    /// the initial fit density. 0 = off.
    pub simplify: f64, // default 0
    /// Centered stroke on every traced path of the layer, in source px.
    /// 0 = none. The sticker outline hosted by Fill layers: a "* Fill"
    /// profile sets it once for every matte.
    pub stroke_width: f32, // default 0
    pub stroke_color: [u8; 3], // default white
}

impl Default for Config {
    fn default() -> Self {
        Self {
            detail: 5.0,
            scale: 3,
            alpha_threshold: 128,
            max_colors: 24,
            locked: Vec::new(),
            gradient_bands: 6,
            pins: Vec::new(),
            mode_filter: 0,
            color_cleanup: 0,
            smoothing: 1.0,
            absorb_dist: 0.08,
            absorb_aggr: 1.0,
            stroke_merge_dist: 0.08,
            stroke_merge_width: 4.0,
            alphamax: 1.15,
            opttolerance: 0.4,
            seam_slack: 1.0,
            simplify: 0.0,
            stroke_width: 0.0,
            stroke_color: [255, 255, 255],
        }
    }
}

impl Config {
    /// Area (scaled px^2) of the smallest visible feature; drives palette
    /// floor and speckle (turd) size. `dim` = max(source W,H).
    pub fn detail_area_scaled(&self, dim: u32) -> f32 {
        let d = self.detail * self.scale as f32 * (512.0 / dim.max(1) as f32);
        d * d
    }
    pub fn turdsize(&self, dim: u32) -> u32 {
        ((self.detail_area_scaled(dim) * 0.5) as u32).max(4)
    }
    /// Turn angle (radians) at or above which a boundary vertex is a corner.
    pub fn corner_threshold(&self) -> f64 {
        (self.alphamax * 90.0).to_radians()
    }
    /// Boundary-smoothing window radius in scaled px.
    pub fn smooth_radius(&self) -> usize {
        (self.smoothing.max(0.0) * self.scale as f32).round() as usize
    }
}

/// sRGB [0,255] -> OKLab (Ottosson 2020). Perceptually uniform: Euclidean
/// distance here tracks visual difference, unlike RGB or redmean. Used for
/// ALL color comparisons (palette dedup, key guard, per-pixel remap) so the
/// whole pipeline agrees on what "different colors" means.
pub fn srgb_to_oklab(c: [u8; 3]) -> [f32; 3] {
    // The channel input is 8-bit, so the three powf calls (most of the
    // conversion's cost, per-pixel in feature detection and remap) fold into
    // one 256-entry table.
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
    let (r, g, b) = (LIN[c[0] as usize], LIN[c[1] as usize], LIN[c[2] as usize]);
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    ]
}

/// Perceptual color distance (OKLab ΔE), living roughly on 0..1.
pub fn color_dist(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (srgb_to_oklab(a), srgb_to_oklab(b));
    let d0 = la[0] - lb[0];
    let d1 = la[1] - lb[1];
    let d2 = la[2] - lb[2];
    (d0 * d0 + d1 * d1 + d2 * d2).sqrt()
}
