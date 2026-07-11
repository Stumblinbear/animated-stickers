//! Routes streamed stage parts and full-render results into the document they
//! belong to, merging each computed value into that document's memo and
//! discarding parts superseded by a newer run while still settling the
//! document's in-flight bookkeeping.

use crate::gui::app::App;
use crate::gui::compute::{FullResult, StagePart};
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
    let done = matches!(part, StagePart::Done(..));
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
            // A live generation's stream is fixed to this session's selected
            // layer: any layer switch bumps the generation and orphans the run,
            // so a part that passes the gate belongs to the selected layer.
            let layer = s.selected_layer;

            match part {
                StagePart::Source(img) => {
                    s.preview.source = Some(img);
                    s.stage_pending[Stage::Source] = false;
                }
                StagePart::Flat(img, key, prep) => {
                    s.preview.flat = Some(img);
                    s.stages.stages_mut(layer).prep.install(key, prep);
                    s.stage_pending[Stage::Flatten] = false;
                }
                StagePart::Detect(key, detect) => {
                    s.stages.stages_mut(layer).detect.install(key, detect);
                }
                StagePart::Remap(img, px, key, out) => {
                    s.preview.remap = Some(img);
                    s.preview.remap_px = Some(px);
                    s.stages.stages_mut(layer).remap.install(key, out);
                    s.stage_pending[Stage::Remap] = false;
                }
                StagePart::Regions(img, key, regs) => {
                    s.preview.regions = Some(img);
                    s.stages.stages_mut(layer).regions.install(key, regs);
                    s.stage_pending[Stage::Regions] = false;
                }
                StagePart::Fates(tint, report, key, plan) => {
                    s.preview.fate_tint = tint;
                    s.preview.region_report = Some(report);
                    s.stages.stages_mut(layer).plan.install(key, plan);
                }
                StagePart::Shapes(key, shapes) => {
                    s.stages.stages_mut(layer).shapes.install(key, shapes);
                }
                StagePart::Fit(key, out) => {
                    s.stages.stages_mut(layer).fit.install(key, out);
                    s.stage_pending[Stage::Fit] = false;
                }
                StagePart::Simplify(key, out) => {
                    s.stages.stages_mut(layer).simplify.install(key, out);
                    s.stage_pending[Stage::Simplify] = false;
                }
                StagePart::Unchanged(stage) => {
                    s.stage_pending[stage] = false;
                }
                StagePart::Done(shown) => {
                    s.preview.shown = Some(*shown);
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

fn full_ready(app: &mut App, doc: DocId, generation: u64, result: Box<FullResult>) -> Task<Msg> {
    let dirty;
    let selected = doc == app.selected_doc;

    // A result for a since-closed document resolves to no position and drops,
    // leaving the document now at its old slot untouched.
    let Some(pos) = app.doc_pos(doc) else {
        return Task::none();
    };

    let d = &mut app.docs[pos];
    let s = &mut d.session;

    if generation == s.full_gen {
        let r = *result;

        // Reinstall each layer's filled slots. A concurrent strip worker may
        // install its own set for the selected layer over this one; both hold
        // the same values for the same inputs.
        for (layer, slots) in r.stages {
            s.stages.install(layer, slots);
        }

        s.layer_outputs = r.outputs;
        s.full_scene = Some(r.scene);
        s.full_stats = Some(r.stats);
        // A successful render clears any prior failure treatment.
        s.trace_error = None;
    }

    s.full_busy = false;

    // A background document keeps its dirty latch for select_doc to honor later.
    if !selected {
        dirty = false;
    } else {
        dirty = s.full_dirty;

        if dirty {
            s.full_dirty = false;
        }
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
    use crate::config::Config;
    use crate::gui::compute::{Artifact, FitInputs, TraceOutput};
    use crate::gui::doc::{Doc, Layer, LayerInputs};
    use crate::gui::ids::{DocId, LayerId};
    use crate::trace::{ContourParams, FitParams};
    use image::RgbaImage;
    use std::sync::Arc;

    /// A fit part whose trace carries the distinctive `scale`, so a reader can
    /// tell whether this exact value reached the memo.
    fn fit_part(scale: u32) -> StagePart {
        let cfg = Config::default();
        let key = FitInputs {
            shapes: Artifact::new(Arc::new(Vec::new())),
            contour: ContourParams::of(&cfg),
            fit: FitParams::of(&cfg),
        };
        let out = TraceOutput {
            trace: Arc::new(Vec::new()),
            scale,
        };
        StagePart::Fit(key, out)
    }

    /// An app holding one document, its selected layer live under stage
    /// generation `gen`, ready to receive that generation's stage parts.
    fn app_streaming(doc: DocId, layer: LayerId, gen: u64) -> App {
        let mut app = App::default();
        let mut d = doc_with_id(doc);
        d.session.selected_layer = layer;
        d.session.stages_running = true;
        d.session.stage_gen = gen;
        app.docs.push(d);
        app.selected_doc = doc;
        app
    }

    // The core of the refactor: a fit part installs its memo value the moment it
    // arrives, so the selected layer's fit memo (the value stage_scene and the
    // anchors overlay read) reflects it before any Done part completes the run.
    #[test]
    fn a_fit_part_installs_its_memo_before_done() {
        let doc = DocId::from_raw(0);
        let layer = LayerId::from_raw(0);
        let mut app = app_streaming(doc, layer, 5);

        let _ = update(&mut app, ComputeMsg::StagePart(doc, 5, fit_part(7)));

        let s = &app.docs[0].session;
        assert!(s.stages_running, "the run is still live, no Done seen");
        let out = s
            .stages
            .peek(layer)
            .and_then(|st| st.fit.current())
            .expect("the fit memo holds the installed value mid-stream");
        assert_eq!(out.scale, 7, "the exact streamed trace is visible");
    }

    // A part from a superseded generation is discarded by the generation gate,
    // so it must not install into the session memo.
    #[test]
    fn a_superseded_generation_part_does_not_install() {
        let doc = DocId::from_raw(0);
        let layer = LayerId::from_raw(0);
        // The live generation is 6; a stray part from generation 5 arrives late.
        let mut app = app_streaming(doc, layer, 6);

        let _ = update(&mut app, ComputeMsg::StagePart(doc, 5, fit_part(7)));

        let s = &app.docs[0].session;
        assert!(
            s.stages
                .peek(layer)
                .is_none_or(|st| st.fit.current().is_none()),
            "a superseded part leaves the memo untouched"
        );
    }

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
        use crate::gui::compute::{DocStats, VectorScene};
        let result = Box::new(FullResult {
            scene: VectorScene {
                dims: (4, 4),
                scale: 1,
                layers: vec![],
            },
            stats: DocStats {
                shapes: 0,
                anchors: 0,
            },
            outputs: Default::default(),
            stages: Default::default(),
        });
        let _ = update(&mut app, ComputeMsg::FullReady(gone, 1, result));

        assert!(
            app.docs[0].session.full_busy,
            "the dead stream must not clear the surviving document's in-flight flag",
        );
    }
}
