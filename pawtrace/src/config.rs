use crate::color::Srgb;

/// One perceptual constant drives derived params (ported from vectorize.py).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Smallest feature (px at output/512 scale) worth keeping.
    pub detail: f32, // default 5.0
    /// Supersample factor: boundary precision is 1/scale source px, at
    /// quadratic cost.
    pub scale: u32, // default 3
    pub alpha_threshold: u8, // default 128 (50%)
    /// Palette safety cap; extraction self-terminates below it.
    pub max_colors: usize, // default 24
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
    pub locked: Vec<Srgb>, // default empty
    /// OKLab ΔE two shades must differ by to stay separate features. Feature
    /// consolidation merges adjacent detected regions, closest colors first,
    /// until every remaining gap reaches this; soft interiors and ramp bands
    /// sit under it and fold, authored color steps sit over it and survive.
    /// Lower preserves more banding in smooth gradients; higher merges shading
    /// more aggressively. The closest deliberate step measured on the goldens
    /// is 0.037, so the default stays under that.
    pub shade_split: f32, // default 0.03 (OKLab ΔE)
    /// Extra merge tolerance for tiny fragments during feature consolidation:
    /// a pair may merge while its ΔE stays under `shade_split + shade_noise /
    /// min(area)`. A small fragment's mean color is a noise-dominated estimate
    /// that cannot testify to a real color boundary; fragments of one noisy
    /// airbrush stroke measure ΔE 0.05-0.1 apart. Higher reunites noisier
    /// brushwork; lower preserves more faint small detail. At the default a
    /// 2 px fragment tolerates +0.07 and the boost fades past ~100 px.
    pub shade_noise: f32, // default 0.14 (OKLab ΔE · px)
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
    /// Couples the fit along boundary stretches shared by two sibling shapes:
    /// each shared stretch is canonicalized once and both shapes emit the
    /// identical curve there, so a fit wobble cannot open a hairline gap
    /// between them. Points where a third color meets a seam become anchors.
    /// Off fits every shape independently.
    pub seam_stitch: bool, // default true
    /// Final anchor-reduction pass: removes any anchor whose deletion keeps
    /// the path within this many scaled px, merging its two segments into
    /// one cubic. Corners survive. Independent of opttolerance, which sets
    /// the initial fit density. 0 = off.
    pub simplify: f64, // default 0
    /// Fraction of a thin feature's original width a simplify merge must leave
    /// intact: a merge that would sweep a boundary past this fraction of the
    /// way toward its opposite side is vetoed, so simplify never thins ink to a
    /// hairline. 0 disables the veto (simplify merges on tolerance alone).
    pub simplify_width_keep: f64, // default 0.6
    /// Centered stroke on every traced path of the layer, in source px.
    /// 0 = none. The sticker outline hosted by Fill layers: a "* Fill"
    /// profile sets it once for every matte.
    pub stroke_width: f32, // default 0
    pub stroke_color: Srgb, // default white
}

impl Default for Config {
    fn default() -> Self {
        Self {
            detail: 5.0,
            scale: 3,
            alpha_threshold: 128,
            max_colors: 24,
            locked: Vec::new(),
            shade_split: 0.03,
            shade_noise: 0.14,
            smoothing: 1.0,
            absorb_dist: 0.08,
            absorb_aggr: 1.0,
            stroke_merge_dist: 0.08,
            stroke_merge_width: 4.0,
            alphamax: 1.15,
            opttolerance: 0.4,
            seam_slack: 1.0,
            seam_stitch: true,
            simplify: 0.0,
            simplify_width_keep: 0.6,
            stroke_width: 0.0,
            stroke_color: Srgb([255, 255, 255]),
        }
    }
}

/// Area (scaled px^2) of the smallest visible feature at `detail`/`scale`;
/// drives the speckle (turd) floor. `dim` = max(source W,H).
pub fn detail_area_scaled(detail: f32, scale: u32, dim: u32) -> f32 {
    let d = detail * scale as f32 * (512.0 / dim.max(1) as f32);
    d * d
}

/// Speckle floor (scaled px): the area below which a region is a turd.
/// `dim` = max(source W,H).
pub fn turdsize(detail: f32, scale: u32, dim: u32) -> u32 {
    ((detail_area_scaled(detail, scale, dim) * 0.5) as u32).max(4)
}

/// Turn angle (radians) at or above which a boundary vertex is a corner.
pub fn corner_threshold(alphamax: f64) -> f64 {
    (alphamax * 90.0).to_radians()
}

/// Boundary-smoothing window radius in scaled px.
pub fn smooth_radius(smoothing: f32, scale: u32) -> usize {
    (smoothing.max(0.0) * scale as f32).round() as usize
}
