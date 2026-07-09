//! The numeric settings as a routable enum, with the plumbing that moves one
//! field between a live [`Config`] and a profile's sparse [`Overrides`].

use crate::config::Config;
use crate::profiles::Overrides;

/// One slider-adjustable pipeline parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Scale,
    AlphaThreshold,
    ModeFilter,
    Detail,
    MaxColors,
    ColorCleanup,
    Smoothing,
    AbsorbDist,
    AbsorbAggr,
    StrokeMergeDist,
    StrokeMergeWidth,
    Alphamax,
    Opttolerance,
    SeamSlack,
    Simplify,
    StrokeWidth,
}

/// Applies a slider value to the live config, converting to the field's type.
pub fn apply(cfg: &mut Config, field: Field, v: f64) {
    match field {
        Field::Scale => cfg.scale = v as u32,
        Field::AlphaThreshold => cfg.alpha_threshold = v as u8,
        Field::ModeFilter => cfg.mode_filter = v as u32,
        Field::Detail => cfg.detail = v as f32,
        Field::MaxColors => cfg.max_colors = v as usize,
        Field::ColorCleanup => cfg.color_cleanup = v as u32,
        Field::Smoothing => cfg.smoothing = v as f32,
        Field::AbsorbDist => cfg.absorb_dist = v as f32,
        Field::AbsorbAggr => cfg.absorb_aggr = v as f32,
        Field::StrokeMergeDist => cfg.stroke_merge_dist = v as f32,
        Field::StrokeMergeWidth => cfg.stroke_merge_width = v as f32,
        Field::Alphamax => cfg.alphamax = v,
        Field::Opttolerance => cfg.opttolerance = v,
        Field::SeamSlack => cfg.seam_slack = v,
        Field::Simplify => cfg.simplify = v,
        Field::StrokeWidth => cfg.stroke_width = v as f32,
    }
}

/// Sets `field`'s value from `cfg` into `ov`.
pub fn set(ov: &mut Overrides, field: Field, cfg: &Config) {
    match field {
        Field::Scale => ov.scale = Some(cfg.scale),
        Field::AlphaThreshold => ov.alpha_threshold = Some(cfg.alpha_threshold),
        Field::ModeFilter => ov.mode_filter = Some(cfg.mode_filter),
        Field::Detail => ov.detail = Some(cfg.detail),
        Field::MaxColors => ov.max_colors = Some(cfg.max_colors),
        Field::ColorCleanup => ov.color_cleanup = Some(cfg.color_cleanup),
        Field::Smoothing => ov.smoothing = Some(cfg.smoothing),
        Field::AbsorbDist => ov.absorb_dist = Some(cfg.absorb_dist),
        Field::AbsorbAggr => ov.absorb_aggr = Some(cfg.absorb_aggr),
        Field::StrokeMergeDist => ov.stroke_merge_dist = Some(cfg.stroke_merge_dist),
        Field::StrokeMergeWidth => ov.stroke_merge_width = Some(cfg.stroke_merge_width),
        Field::Alphamax => ov.alphamax = Some(cfg.alphamax),
        Field::Opttolerance => ov.opttolerance = Some(cfg.opttolerance),
        Field::SeamSlack => ov.seam_slack = Some(cfg.seam_slack),
        Field::Simplify => ov.simplify = Some(cfg.simplify),
        Field::StrokeWidth => ov.stroke_width = Some(cfg.stroke_width),
    }
}

/// Clears `field` from `ov`.
pub fn clear(ov: &mut Overrides, field: Field) {
    match field {
        Field::Scale => ov.scale = None,
        Field::AlphaThreshold => ov.alpha_threshold = None,
        Field::ModeFilter => ov.mode_filter = None,
        Field::Detail => ov.detail = None,
        Field::MaxColors => ov.max_colors = None,
        Field::ColorCleanup => ov.color_cleanup = None,
        Field::Smoothing => ov.smoothing = None,
        Field::AbsorbDist => ov.absorb_dist = None,
        Field::AbsorbAggr => ov.absorb_aggr = None,
        Field::StrokeMergeDist => ov.stroke_merge_dist = None,
        Field::StrokeMergeWidth => ov.stroke_merge_width = None,
        Field::Alphamax => ov.alphamax = None,
        Field::Opttolerance => ov.opttolerance = None,
        Field::SeamSlack => ov.seam_slack = None,
        Field::Simplify => ov.simplify = None,
        Field::StrokeWidth => ov.stroke_width = None,
    }
}

/// Whether `field` is set in `ov`.
pub fn is_set(ov: &Overrides, field: Field) -> bool {
    match field {
        Field::Scale => ov.scale.is_some(),
        Field::AlphaThreshold => ov.alpha_threshold.is_some(),
        Field::ModeFilter => ov.mode_filter.is_some(),
        Field::Detail => ov.detail.is_some(),
        Field::MaxColors => ov.max_colors.is_some(),
        Field::ColorCleanup => ov.color_cleanup.is_some(),
        Field::Smoothing => ov.smoothing.is_some(),
        Field::AbsorbDist => ov.absorb_dist.is_some(),
        Field::AbsorbAggr => ov.absorb_aggr.is_some(),
        Field::StrokeMergeDist => ov.stroke_merge_dist.is_some(),
        Field::StrokeMergeWidth => ov.stroke_merge_width.is_some(),
        Field::Alphamax => ov.alphamax.is_some(),
        Field::Opttolerance => ov.opttolerance.is_some(),
        Field::SeamSlack => ov.seam_slack.is_some(),
        Field::Simplify => ov.simplify.is_some(),
        Field::StrokeWidth => ov.stroke_width.is_some(),
    }
}

/// Every field, grouped by stage in strip order.
pub const ALL: [Field; 16] = [
    Field::Scale,
    Field::AlphaThreshold,
    Field::ModeFilter,
    Field::Detail,
    Field::MaxColors,
    Field::ColorCleanup,
    Field::Smoothing,
    Field::AbsorbDist,
    Field::AbsorbAggr,
    Field::StrokeMergeDist,
    Field::StrokeMergeWidth,
    Field::Alphamax,
    Field::Opttolerance,
    Field::SeamSlack,
    Field::Simplify,
    Field::StrokeWidth,
];
