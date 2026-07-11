//! Whole-document rendering and export: every layer traced under its matched
//! profile, then positioned in document space. The trace itself is
//! layer-local, shared with the stage strip through the memo. The
//! document-scale ratio and position translation are applied here, at use
//! time.

use super::cache::ShapeCache;
use super::stages::{LayerStages, PlanCtx};
use super::{layer_bboxes, DocStats, FullResult, LayerTrace, VectorLayer, VectorScene};
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
) -> Box<FullResult> {
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

    // Only visible layers enter the composite, bottom-first in stack order (the
    // enabled-layer order `done` preserves). Each layer's placed trace and
    // stroke are the same the SVG export emits, so the preview and the export
    // agree.
    let scene_layers: Vec<VectorLayer> = done
        .iter()
        .filter(|(i, _, _, _)| inputs[&layers[*i].id].visible)
        .map(|(i, cfg, _, pre)| {
            let placed = output::place(pre, cfg.scale, doc_scale, layers[*i].offset);
            let bboxes = layer_bboxes(&placed);

            VectorLayer {
                colors: Arc::new(placed),
                bboxes: Arc::new(bboxes),
                stroke: output::stroke_of(cfg),
            }
        })
        .collect();

    let scene = VectorScene {
        dims: size,
        scale: doc_scale,
        layers: scene_layers,
    };

    let stages: FxHashMap<LayerId, LayerStages> = done
        .into_iter()
        .map(|(i, _, slot, _)| (layers[i].id, slot))
        .collect();

    Box::new(FullResult {
        scene,
        stats,
        outputs,
        stages,
    })
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

#[cfg(test)]
mod tests {
    use crate::output;
    use crate::trace::TracedPath;

    // The Document scene stores each layer placed by output::place at the doc
    // scale, so dividing a placed coordinate by that scale yields the layer's
    // position in document source px: its layer-local point over the layer scale,
    // shifted by the layer offset. The preview's viewport, sized in doc source
    // px, then maps it to screen exactly as the stage views map a layer trace.
    #[test]
    fn placed_scene_coordinate_is_document_source_px() {
        let (layer_scale, doc_scale) = (2u32, 6u32);
        let offset = (3u32, 4u32);
        let colors = vec![(
            "#000000".to_string(),
            vec![TracedPath {
                start: (2.0, 5.0),
                cubics: vec![],
            }],
        )];

        let placed = output::place(&colors, layer_scale, doc_scale, offset);
        let p = &placed[0].1[0];
        let s = doc_scale as f64;

        let want_x = 2.0 / layer_scale as f64 + offset.0 as f64;
        let want_y = 5.0 / layer_scale as f64 + offset.1 as f64;
        assert!((p.start.0 / s - want_x).abs() < 1e-9);
        assert!((p.start.1 / s - want_y).abs() < 1e-9);
    }
}
