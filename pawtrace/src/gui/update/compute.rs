//! Routes streamed stage parts and full-render results into the document they
//! belong to, merging each computed value into that document's memo and
//! discarding parts superseded by a newer run while still settling the
//! document's in-flight bookkeeping.

use crate::gui::app::App;
use crate::gui::compute::{FullError, FullResult, StagePart};
use crate::gui::ids::DocId;
use crate::gui::msg::{ComputeMsg, Msg};
use crate::gui::phases::Stage;
use iced::Task;

pub(super) fn update(app: &mut App, msg: ComputeMsg) -> Task<Msg> {
    match msg {
        ComputeMsg::StagePart(doc, generation, part) => stage_part(app, doc, generation, part),
        ComputeMsg::FullReady(doc, generation, result) => full_ready(app, doc, generation, result),
    }
}

fn stage_part(app: &mut App, doc: DocId, generation: u64, part: StagePart) -> Task<Msg> {
    let done = matches!(part, StagePart::Simplify(..));
    let (dirty, queued);
    let selected = doc == app.selected_doc;
    // A part for a document that has since closed resolves to no position and
    // drops, so a dead stream never touches whichever document slid into its
    // old tab-strip slot.
    let Some(pos) = app.doc_pos(doc) else {
        return Task::none();
    };
    {
        let d = &mut app.docs[pos];
        let s = &mut d.session;
        if generation == s.stage_gen {
            let layer = s.selected_layer;
            let keys = s.stage_keys;
            match part {
                StagePart::Source(img) => {
                    s.stages.source = Some(img);
                    s.stage_pending[Stage::Source] = false;
                }
                StagePart::Flat(img, prep) => {
                    s.stages.flat = Some(img);
                    s.memo.put_prep(layer, keys.prep, prep);
                    s.stage_pending[Stage::Flatten] = false;
                }
                StagePart::Detect(detect) => {
                    s.memo.put_detect(layer, keys.detect, detect);
                }
                StagePart::Plan(plan) => {
                    s.memo.put_plan(layer, keys.regions_view, plan);
                }
                StagePart::Remap(img, px, quant, pal) => {
                    s.stages.remap = Some(img);
                    s.stages.palette = (*pal).clone();
                    s.stages.quant_px = Some(px);
                    s.memo.put_quant(layer, keys.quant, quant);
                    s.memo.put_palette(layer, keys.quant, pal);
                    s.stage_pending[Stage::Remap] = false;
                }
                StagePart::Regions(img, count, regs) => {
                    s.stages.regions = Some(img);
                    s.stages.region_count = count;
                    s.memo.put_regions(layer, keys.regions, regs);
                    s.stage_pending[Stage::Regions] = false;
                }
                StagePart::Fates(tint, report) => {
                    s.stages.fate_tint = tint;
                    s.stages.region_report = Some(report);
                }
                StagePart::Contours(img) => {
                    s.stages.contours = img.clone();
                    s.memo.put_smooth(layer, keys.fit, img);
                    s.stage_pending[Stage::Contours] = false;
                }
                StagePart::Fit(img, anchors, fit) => {
                    s.stages.render = img;
                    s.stages.anchor_count = anchors;
                    s.memo.put_fit(layer, keys.fit, fit);
                    s.stage_pending[Stage::Fit] = false;
                }
                StagePart::Simplify(img, anchors, simpl, shown) => {
                    s.stages.simplified = img;
                    s.stages.simplify_anchor_count = anchors;
                    s.memo.put_simplify(layer, keys.simplify, simpl);
                    s.stages.shown = Some(shown);
                    s.stage_pending[Stage::Simplify] = false;
                }
            }
        }
        if !done {
            return Task::none();
        }
        s.stages_running = false;
        // A background document only settles `stages_running`; its latches
        // stay set so select_doc can relaunch what the edit still owes.
        if !selected {
            return Task::none();
        }
        dirty = s.stages_dirty;
        queued = s.full_queued;
        if dirty {
            s.stages_dirty = false;
        } else if queued {
            s.full_queued = false;
        }
    }
    if dirty {
        app.spawn_stages()
    } else if queued {
        app.spawn_full()
    } else {
        Task::none()
    }
}

fn full_ready(
    app: &mut App,
    doc: DocId,
    generation: u64,
    result: Result<Box<FullResult>, FullError>,
) -> Task<Msg> {
    let mut err = None;
    let dirty;
    let selected = doc == app.selected_doc;
    // A result for a since-closed document resolves to no position and drops,
    // leaving the document now at its old slot untouched.
    let Some(pos) = app.doc_pos(doc) else {
        return Task::none();
    };
    {
        let d = &mut app.docs[pos];
        let s = &mut d.session;
        if generation == s.full_gen {
            match result {
                Ok(r) => {
                    let r = *r;
                    for m in r.merges {
                        s.memo.put_simplify(m.layer, m.simplify_key, m.trace.clone());
                        if let Some(fk) = m.fit_key {
                            s.memo.put_fit(m.layer, fk, m.trace);
                        }
                    }
                    s.layer_outputs = r.outputs;
                    s.full_preview = Some(r.img);
                    s.full_stats = Some(r.stats);
                    // A successful render clears any prior failure treatment.
                    s.trace_error = None;
                }
                Err(e) => err = Some(e),
            }
        }
        s.full_busy = false;
        // A background document keeps its dirty latch for select_doc to
        // honor later.
        if !selected {
            dirty = false;
        } else {
            dirty = s.full_dirty;
            if dirty {
                s.full_dirty = false;
            }
        }
    }
    if let Some(e) = err {
        // Attribute the red treatment to the layer that failed, falling back to
        // the selected layer when the failure names none. The render does not
        // report which stage failed, so the phase is the final one, Curves.
        if selected {
            if let Some(s) = app.session_mut() {
                let layer = e.layer.unwrap_or(s.selected_layer);
                s.trace_error = Some(crate::gui::app::LayerError {
                    layer,
                    phase: crate::gui::msg::Phase::Curves,
                    human: "The document render could not be produced.".into(),
                    raw: e.msg.clone(),
                    fix: None,
                });
            }
        }
        app.status = e.msg;
    }
    if dirty {
        app.spawn_full()
    } else {
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::compute::FullError;
    use crate::gui::doc::{Doc, Layer, LayerInputs};
    use crate::gui::ids::{DocId, LayerId};
    use image::RgbaImage;
    use std::sync::Arc;

    fn doc_with_id(id: DocId) -> Doc {
        let lid = LayerId::from_raw(0);
        Doc {
            id,
            path: "test.png".into(),
            size: (4, 4),
            layers: Arc::new(vec![Layer {
                id: lid,
                name: "layer".into(),
                img: RgbaImage::new(4, 4),
                offset: (0, 0),
            }]),
            inputs: [(lid, LayerInputs::default())].into_iter().collect(),
            session: Default::default(),
        }
    }

    // Closing a tab mid-stream once cleared the in-flight flag of whichever
    // document slid into the closed tab's index, because the stream routed by
    // position. With identity routing the dead stream resolves to no document
    // and drops, leaving the surviving document's bookkeeping intact.
    #[test]
    fn a_closed_documents_stream_leaves_the_surviving_tab_untouched() {
        let mut app = App::default();
        let gone = DocId::from_raw(1);
        let survivor = DocId::from_raw(2);
        app.docs.push(doc_with_id(gone));
        app.docs.push(doc_with_id(survivor));
        app.selected_doc = survivor;
        {
            let s = &mut app.docs[1].session;
            s.full_busy = true;
            s.full_gen = 7;
        }

        // Close `gone`: `survivor` slides from slot 1 into slot 0, the index the
        // in-flight stream started under.
        app.docs.remove(0);
        assert_eq!(app.doc_pos(gone), None);
        assert_eq!(app.doc_pos(survivor), Some(0));

        // `gone`'s final full result arrives after the close.
        let err = FullError { layer: Some(LayerId::from_raw(0)), msg: "dead stream".into() };
        let _ = update(&mut app, ComputeMsg::FullReady(gone, 1, Err(err)));

        assert!(
            app.docs[0].session.full_busy,
            "the dead stream must not clear the surviving document's in-flight flag",
        );
    }
}
