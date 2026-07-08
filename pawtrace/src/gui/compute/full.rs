//! Whole-document rendering and export: every layer traced under its matched
//! profile, then positioned in document space. The trace itself is
//! layer-local, shared with the stage strip through the memo. The
//! document-scale ratio and position translation are applied here, at use
//! time.

use super::memo::StageKeys;
use super::render::render_svg;
use super::{DocStats, FullResult, LayerTrace};
use crate::config::Config;
use crate::gui::doc::{Doc, Layer, LayerFlags};
use crate::gui::ids::LayerId;
use crate::{output, pipeline, profiles};
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// A newly computed layer trace to fold into the memo. `fit_key` is set only
/// when simplify is off, where the pre-transform trace is both the fit and the
/// simplify result.
#[derive(Debug, Clone)]
pub(in crate::gui) struct FullMerge {
    pub(in crate::gui) layer: LayerId,
    pub(in crate::gui) simplify_key: u64,
    pub(in crate::gui) fit_key: Option<u64>,
    pub(in crate::gui) trace: Arc<LayerTrace>,
}

/// One enabled layer's trace and the config it was traced under.
struct Entry {
    idx: usize,
    cfg: Config,
    /// Layer-local paths at the layer's own scale, before the document
    /// transform.
    pre: Arc<LayerTrace>,
    computed: bool,
}

/// Positions a layer-local trace in the document: scales from the layer's
/// supersample space into the document's, then translates to the layer's
/// document position.
fn place(pre: &LayerTrace, cfg: &Config, doc_scale: u32, offset: (u32, u32)) -> LayerTrace {
    let mut colors = pre.clone();
    let ratio = doc_scale as f64 / cfg.scale as f64;
    let (dx, dy) = ((offset.0 * doc_scale) as f64, (offset.1 * doc_scale) as f64);
    for (_, paths) in &mut colors {
        for p in paths {
            if ratio != 1.0 {
                p.scale(ratio);
            }
            p.translate(dx, dy);
        }
    }
    colors
}

/// One layer through the pipeline, positioned in document space.
fn trace_layer(l: &Layer, cfg: &Config, doc_scale: u32, doc_dim: u32) -> Result<LayerTrace> {
    let pre = pipeline::run(&l.img, cfg, doc_dim, l.offset)?;
    Ok(place(&pre, cfg, doc_scale, l.offset))
}

/// Full document render. Excluded layers are skipped entirely; hidden layers
/// are traced (their stats and memo entries stay current) but left out of the
/// composite. `snap[i]` is the layer's cached pre-transform trace when it is
/// unchanged, so a profile edit re-traces only that profile's layers.
pub(super) fn render_full(
    layers: &[Layer],
    flags: &[LayerFlags],
    size: (u32, u32),
    profiles: &profiles::ProfileStack,
    doc_dim: u32,
    snap: Vec<Option<Arc<LayerTrace>>>,
) -> Result<Box<FullResult>> {
    let doc_scale = profiles.resolve("").0.scale;
    use rayon::prelude::*;
    let entries: Vec<Option<Entry>> = layers
        .par_iter()
        .enumerate()
        .map(|(i, l)| {
            if !flags[i].enabled {
                return Ok(None);
            }
            let cfg = profiles.resolve(&l.name).0;
            let (pre, computed) = match &snap[i] {
                Some(t) => (t.clone(), false),
                None => (Arc::new(pipeline::run(&l.img, &cfg, doc_dim, l.offset)?), true),
            };
            Ok(Some(Entry { idx: i, cfg, pre, computed }))
        })
        .collect::<Result<Vec<_>>>()?;

    // Counts come from the layer-local trace: the document transform is a
    // scale and translate, so it leaves path and anchor counts unchanged.
    let mut anchors = vec![0usize; layers.len()];
    let (mut shapes, mut total) = (0usize, 0usize);
    for e in entries.iter().flatten() {
        let a: usize = e.pre.iter().flat_map(|(_, ps)| ps.iter()).map(|p| p.cubics.len()).sum();
        anchors[e.idx] = a;
        total += a;
        shapes += e.pre.iter().map(|(_, ps)| ps.len()).sum::<usize>();
    }
    let stats = DocStats { shapes, anchors: total };

    let placed: Vec<(usize, LayerTrace, Option<output::Stroke>)> = entries
        .iter()
        .flatten()
        .map(|e| (e.idx, place(&e.pre, &e.cfg, doc_scale, layers[e.idx].offset), output::stroke_of(&e.cfg)))
        .collect();
    let svg_layers: Vec<output::SvgLayer> = placed
        .iter()
        .filter(|(i, _, _)| flags[*i].visible)
        .map(|(i, colors, stroke)| output::SvgLayer {
            name: &layers[*i].name,
            stroke: stroke.as_ref(),
            colors,
        })
        .collect();
    let svg = output::svg(size.0, size.1, doc_scale, 0.0, &svg_layers);
    let img = render_svg(&svg, size.0, size.1).ok_or_else(|| anyhow!("full preview render failed"))?;

    let merges = entries
        .iter()
        .flatten()
        .filter(|e| e.computed)
        .map(|e| {
            let k = StageKeys::of(&e.cfg);
            FullMerge {
                layer: LayerId(e.idx),
                simplify_key: k.simplify,
                fit_key: (e.cfg.simplify <= 0.0).then_some(k.fit),
                trace: e.pre.clone(),
            }
        })
        .collect();

    Ok(Box::new(FullResult { img, stats, anchors, merges }))
}

/// Batch export: Tailmovin JSON next to each document. Excluded layers are
/// omitted; hidden layers export normally.
pub(crate) fn export_doc(
    doc: &Doc,
    profiles: &profiles::ProfileStack,
) -> Result<std::path::PathBuf> {
    let doc_dim = doc.size.0.max(doc.size.1);
    let doc_scale = profiles.resolve("").0.scale;
    use rayon::prelude::*;
    let included: Vec<&Layer> = doc
        .layers
        .iter()
        .zip(&doc.flags)
        .filter(|(_, f)| f.enabled)
        .map(|(l, _)| l)
        .collect();
    let traced: Vec<(String, Option<output::Stroke>, LayerTrace)> = included
        .par_iter()
        .map(|l| {
            let (cfg, _) = profiles.resolve(&l.name);
            Ok((
                l.name.clone(),
                output::stroke_of(&cfg),
                trace_layer(l, &cfg, doc_scale, doc_dim)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let scale = doc_scale as f64;
    let out = output::Doc {
        width: doc.size.0,
        height: doc.size.1,
        layers: traced
            .into_iter()
            .map(|(name, stroke, colors)| output::Layer {
                name,
                stroke,
                colors: colors
                    .into_iter()
                    .map(|(hex, paths)| output::ColorGroup {
                        hex,
                        paths: paths.iter().map(|p| output::to_json_path(p, scale)).collect(),
                    })
                    .collect(),
            })
            .collect(),
    };
    let path = doc.path.with_extension("json");
    std::fs::write(&path, serde_json::to_vec_pretty(&out)?)?;
    Ok(path)
}
