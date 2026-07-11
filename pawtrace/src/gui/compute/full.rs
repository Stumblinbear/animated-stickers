//! Whole-document rendering and export: every layer traced under its matched
//! profile, then positioned in document space. The trace itself is
//! layer-local, shared with the stage strip through the memo. The
//! document-scale ratio and position translation are applied here, at use
//! time.

use super::cache::ShapeCache;
use super::render::render_svg;
use super::stages::{LayerStages, PlanCtx};
use super::{DocStats, FullError, FullResult, LayerTrace};
use crate::config::Config;
use crate::gui::doc::{Doc, Layer, LayerInputs, LayerOutputs};
use crate::gui::ids::LayerId;
use crate::{output, pipeline, profiles};
use anyhow::Result;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// One layer through the pipeline, positioned in document space.
fn trace_layer(
    l: &Layer,
    cfg: &Config,
    doc_scale: u32,
    doc_dim: u32,
    pins: &[[u32; 2]],
) -> Result<LayerTrace> {
    let pre = pipeline::run(&l.img, cfg, doc_dim, l.offset, pins)?;
    Ok(output::place(&pre, cfg.scale, doc_scale, l.offset))
}

/// Full document render. Excluded layers are skipped entirely; hidden layers
/// are traced (their stats and stage slots stay current) but left out of the
/// composite. Each enabled layer runs the same staged chain the strip driver
/// runs, against `slots` cloned from the cache, so a layer already traced hits
/// the shared per-shape fit cache. The filled slot sets ride home in the result
/// for reinstalling.
pub(super) fn render_full(
    layers: &[Layer],
    inputs: &FxHashMap<LayerId, LayerInputs>,
    size: (u32, u32),
    profiles: &profiles::ProfileStack,
    doc_dim: u32,
    mut slots: FxHashMap<LayerId, LayerStages>,
    shape_cache: ShapeCache,
) -> std::result::Result<Box<FullResult>, FullError> {
    use rayon::prelude::*;

    let doc_scale = profiles.resolve("").0.scale;

    let jobs: Vec<(usize, LayerStages)> = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| inputs[&l.id].enabled)
        .map(|(i, l)| (i, slots.remove(&l.id).unwrap_or_default()))
        .collect();

    // One filled slot set and layer-local trace per enabled layer. The chain is
    // sequential within a layer; the layers run in parallel.
    let done: Vec<(usize, Config, LayerStages, Arc<LayerTrace>)> = jobs
        .into_par_iter()
        .map(|(i, mut slot)| {
            let l = &layers[i];
            let cfg = profiles.resolve(&l.name).0;
            let plan_ctx = PlanCtx {
                offset: l.offset,
                dims: l.img.dimensions(),
                doc_dim,
            };
            let pre = slot.trace(&l.img, &cfg, &inputs[&l.id].pins, plan_ctx, &shape_cache);
            (i, cfg, slot, pre)
        })
        .collect();

    // Counts come from the layer-local trace: the document transform is a
    // scale and translate, so it leaves path and anchor counts unchanged.
    let mut outputs: FxHashMap<LayerId, LayerOutputs> = FxHashMap::default();

    let (mut shapes, mut total) = (0usize, 0usize);

    for (i, _, _, pre) in &done {
        let a: usize = pre
            .iter()
            .flat_map(|(_, ps)| ps.iter())
            .map(|p| p.cubics.len())
            .sum();

        outputs.insert(layers[*i].id, LayerOutputs { anchors: a });

        total += a;
        shapes += pre.iter().map(|(_, ps)| ps.len()).sum::<usize>();
    }

    let stats = DocStats {
        shapes,
        anchors: total,
    };

    let placed: Vec<(usize, LayerTrace, Option<output::Stroke>)> = done
        .iter()
        .map(|(i, cfg, _, pre)| {
            (
                *i,
                output::place(pre, cfg.scale, doc_scale, layers[*i].offset),
                output::stroke_of(cfg),
            )
        })
        .collect();

    let svg_layers: Vec<output::SvgLayer> = placed
        .iter()
        .filter(|(i, _, _)| inputs[&layers[*i].id].visible)
        .map(|(i, colors, stroke)| output::SvgLayer {
            name: &layers[*i].name,
            stroke: stroke.as_ref(),
            colors,
        })
        .collect();

    let svg = output::svg(size.0, size.1, doc_scale, 0.0, &svg_layers);
    let img = render_svg(&svg, size.0, size.1).ok_or_else(|| FullError {
        msg: "full preview render failed".into(),
    })?;

    let stages: FxHashMap<LayerId, LayerStages> = done
        .into_iter()
        .map(|(i, _, slot, _)| (layers[i].id, slot))
        .collect();

    Ok(Box::new(FullResult {
        img,
        stats,
        outputs,
        stages,
    }))
}

/// Batch export: Tailmovin JSON next to each document. Excluded layers are
/// omitted; hidden layers export normally.
pub(crate) fn export_doc(
    doc: &Doc,
    profiles: &profiles::ProfileStack,
) -> Result<std::path::PathBuf> {
    use rayon::prelude::*;

    let included: Vec<&Layer> = doc
        .layers
        .iter()
        .filter(|l| doc.inputs[&l.id].enabled)
        .collect();

    let doc_dim = doc.size.0.max(doc.size.1);
    let doc_scale = profiles.resolve("").0.scale;

    let traced: Vec<(String, Option<output::Stroke>, LayerTrace)> = included
        .par_iter()
        .map(|l| {
            let (cfg, _) = profiles.resolve(&l.name);
            Ok((
                l.name.clone(),
                output::stroke_of(&cfg),
                trace_layer(l, &cfg, doc_scale, doc_dim, &doc.inputs[&l.id].pins)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let out = output::doc(doc.size.0, doc.size.1, doc_scale, traced);

    let path = doc.path.with_extension("json");
    std::fs::write(&path, serde_json::to_vec_pretty(&out)?)?;
    Ok(path)
}
