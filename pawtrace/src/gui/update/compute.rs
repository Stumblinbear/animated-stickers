//! Routes streamed stage parts and full-render results into the document they
//! belong to, merging each computed value into that document's memo and
//! discarding parts superseded by a newer run while still settling the
//! document's in-flight bookkeeping.

use crate::gui::app::App;
use crate::gui::compute::{FullResult, StagePart};
use crate::gui::msg::{ComputeMsg, Msg};
use iced::Task;

pub(super) fn update(app: &mut App, msg: ComputeMsg) -> Task<Msg> {
    match msg {
        ComputeMsg::StagePart(doc, generation, part) => stage_part(app, doc, generation, part),
        ComputeMsg::FullReady(doc, generation, result) => full_ready(app, doc, generation, result),
    }
}

fn stage_part(app: &mut App, doc: usize, generation: u64, part: StagePart) -> Task<Msg> {
    let done = matches!(part, StagePart::Simplify(..));
    let (dirty, queued);
    let selected = doc == app.selected_doc;
    {
        let Some(d) = app.docs.get_mut(doc) else {
            return Task::none();
        };
        let s = &mut d.session;
        if generation == s.stage_gen {
            let layer = s.selected_layer;
            let keys = s.stage_keys;
            match part {
                StagePart::Source(img) => {
                    s.stages.source = Some(img);
                    s.stage_pending[0] = false;
                }
                StagePart::Flat(img, prep) => {
                    s.stages.flat = Some(img);
                    s.memo.put_prep(layer, keys.prep, prep);
                    s.stage_pending[1] = false;
                }
                StagePart::Quant(img, px, quant, pal) => {
                    s.stages.quant = Some(img);
                    s.stages.palette = (*pal).clone();
                    s.stages.quant_px = Some(px);
                    s.memo.put_quant(layer, keys.quant, quant);
                    s.memo.put_palette(layer, keys.quant, pal);
                    s.stage_pending[2] = false;
                }
                StagePart::Regions(img, count, report, regs) => {
                    s.stages.regions = Some(img);
                    s.stages.region_count = count;
                    s.stages.region_report = Some(report);
                    s.memo.put_regions(layer, keys.regions, regs);
                    s.stage_pending[3] = false;
                }
                StagePart::Smooth(img) => {
                    s.stages.smooth = img.clone();
                    s.memo.put_smooth(layer, keys.fit, img);
                    s.stage_pending[4] = false;
                }
                StagePart::Fit(img, anchors, fit) => {
                    s.stages.render = img;
                    s.stages.anchor_count = anchors;
                    s.memo.put_fit(layer, keys.fit, fit);
                    s.stage_pending[5] = false;
                }
                StagePart::Simplify(img, anchors, simpl, shown) => {
                    s.stages.simplified = img;
                    s.stages.simplify_anchor_count = anchors;
                    s.memo.put_simplify(layer, keys.simplify, simpl);
                    s.stages.shown = Some(shown);
                    s.stage_pending[6] = false;
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
    doc: usize,
    generation: u64,
    result: Result<Box<FullResult>, String>,
) -> Task<Msg> {
    let mut err = None;
    let dirty;
    let selected = doc == app.selected_doc;
    {
        let Some(d) = app.docs.get_mut(doc) else {
            return Task::none();
        };
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
                    s.layer_anchors = r.anchors;
                    s.full_preview = Some(r.img);
                    s.full_stats = Some(r.stats);
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
        app.status = e;
    }
    if dirty {
        app.spawn_full()
    } else {
        Task::none()
    }
}
