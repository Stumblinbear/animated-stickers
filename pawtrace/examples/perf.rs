//! Per-stage wall-time harness over the fixture PSDs: runs the full pipeline
//! per layer exactly as `pipeline::run` does (stages timed individually), plus
//! the document SVG serialization the CLI export emits.
//!
//! Run: `cargo run --release --example perf`

use std::path::Path;
use std::time::Instant;

use image::{GrayImage, RgbaImage};
use pawtrace::config::Config;
use pawtrace::output::{self, SvgLayer};
use pawtrace::profiles::ProfileStack;
use pawtrace::trace::TracedPath;
use pawtrace::{palette, pipeline, psd_import, raster, regions};

const FIXTURES: &[&str] = &["seff_deer_a.psd", "seff_deer_b.psd"];
const ITERS: usize = 4;

const STAGES: &[&str] = &[
    "crop to alpha",
    "prepare (resize+flatten)",
    "palette extract",
    "remap (+cleanup)",
    "segment + absorb",
    "region report",
    "shapes + trace + fit",
    "simplify",
    "svg string",
];

#[derive(Default, Clone)]
struct Times([f64; STAGES.len()]);

impl Times {
    fn add(&mut self, stage: usize, t: Instant) {
        self.0[stage] += t.elapsed().as_secs_f64() * 1000.0;
    }

    fn total(&self) -> f64 {
        self.0.iter().sum()
    }
}

struct Traced {
    name: String,
    stroke: Option<output::Stroke>,
    colors: Vec<(String, Vec<TracedPath>)>,
}

/// pipeline::run, stage-timed, byte-identical output.
fn run_layer(
    src: &RgbaImage,
    cfg: &Config,
    doc_dim: u32,
    times: &mut Times,
) -> Vec<(String, Vec<TracedPath>)> {
    let t = Instant::now();
    let cropped = pipeline::crop_to_alpha(src, cfg);

    times.add(0, t);

    let Some((src, ox, oy)) = cropped else {
        return Vec::new();
    };

    let pins = pipeline::scale_pins(&[], (ox, oy), cfg.scale, (src.width(), src.height()));

    let (alpha, regs): (GrayImage, Vec<regions::Region>) = if let Some(color) =
        raster::uniform_color(&src, cfg.alpha_threshold)
    {
        let t = Instant::now();
        let alpha = raster::scale_alpha(&src, cfg);
        times.add(1, t);

        let t = Instant::now();
        let regs = regions::from_mask(&alpha, color);
        times.add(4, t);

        (alpha, regs)
    } else {
        let t = Instant::now();
        let prep = raster::prepare(&src, &raster::PrepParams::of(cfg));
        times.add(1, t);

        let t = Instant::now();
        let plan = palette::Partition::build(&src, cfg).plan(&palette::SelectParams::of(cfg));
        times.add(2, t);

        let t = Instant::now();
        let quant = palette::remap_constrained(&prep.flat, &prep.alpha, &plan, cfg.scale);
        let quant = if cfg.color_cleanup > 0 {
            palette::label_smooth(&quant, &prep.alpha, cfg.color_cleanup)
        } else {
            quant
        };
        times.add(3, t);

        let t = Instant::now();
        let regs = regions::segment_absorbed(&quant, &prep.alpha, &regions::SegmentParams::of(cfg));
        times.add(4, t);

        (prep.alpha, regs)
    };

    // The plan is shared by the report and the trace, as in the GUI; its
    // build cost lands in the shapes+trace stage.
    let t = Instant::now();
    let plan = regions::merge_plan(&regs, &alpha, &regions::PlanParams::of(cfg), doc_dim, &pins);
    times.add(6, t);

    // The GUI recomputes this report on every pipeline rerun; timed here even
    // though pipeline::run itself never calls it.
    let t = Instant::now();
    let report = regions::report_of(&plan);
    times.add(5, t);
    std::hint::black_box(&report);

    let t = Instant::now();
    let traced = pipeline::trace_planned(&plan, &alpha, cfg);
    times.add(6, t);
    let t = Instant::now();
    let mut out = pipeline::simplify_paths(traced, &pipeline::SimplifyParams::of(cfg));
    times.add(7, t);

    let (sx, sy) = ((ox * cfg.scale) as f64, (oy * cfg.scale) as f64);
    for (_, paths) in &mut out {
        for p in paths {
            p.translate(sx, sy);
        }
    }

    out
}

fn main() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut grand_total = 0.0;

    for &fixture in FIXTURES {
        let psd_path = fixtures_dir.join(fixture);
        let profiles = ProfileStack::load_near(&psd_path);
        let bytes = std::fs::read(&psd_path).expect("fixture readable");
        let layers = psd_import::layers(&bytes).expect("psd parses");
        let (w, h) = (layers[0].1.width(), layers[0].1.height());
        let doc_scale = profiles.resolve("").0.scale;

        // Byte-identity check: the stage-by-stage replica must match
        // pipeline::run exactly, or the timings measure the wrong code.
        for (name, img) in &layers {
            let (cfg, _) = profiles.resolve(name);
            let mut times = Times::default();
            let mine = run_layer(img, &cfg, w.max(h), &mut times);
            let real = pipeline::run(img, &cfg, w.max(h), (0, 0), &[]).unwrap();

            assert_eq!(
                format!("{mine:?}"),
                format!("{real:?}"),
                "harness diverged on {name}"
            );
        }

        let mut best: Option<Times> = None;

        for iter in 0..=ITERS {
            let mut times = Times::default();

            let traced: Vec<Traced> = layers
                .iter()
                .map(|(name, img)| {
                    let (cfg, _) = profiles.resolve(name);
                    let mut colors = run_layer(img, &cfg, w.max(h), &mut times);
                    let ratio = doc_scale as f64 / cfg.scale as f64;

                    if ratio != 1.0 {
                        for (_, paths) in &mut colors {
                            for p in paths {
                                p.scale(ratio);
                            }
                        }
                    }

                    Traced {
                        name: name.clone(),
                        stroke: output::stroke_of(&cfg),
                        colors,
                    }
                })
                .collect();

            let svg_layers: Vec<SvgLayer> = traced
                .iter()
                .map(|l| SvgLayer {
                    name: &l.name,
                    stroke: l.stroke.as_ref(),
                    colors: &l.colors,
                })
                .collect();

            let t = Instant::now();
            let svg = output::svg(w, h, doc_scale, 0.0, &svg_layers);
            times.add(8, t);
            std::hint::black_box(&svg);

            // Iteration 0 is warmup (page faults, pool spin-up); keep the
            // fastest of the rest as the least-noisy estimate.
            if iter > 0 && best.as_ref().is_none_or(|b| times.total() < b.total()) {
                best = Some(times);
            }
        }

        let best = best.unwrap();

        println!("=== {fixture} ({} layers, {w}x{h})", layers.len());

        for (i, name) in STAGES.iter().enumerate() {
            println!(
                "  {name:26} {:8.1} ms  {:4.1}%",
                best.0[i],
                100.0 * best.0[i] / best.total()
            );
        }
        println!("  {:26} {:8.1} ms", "total", best.total());
        grand_total += best.total();
    }
    println!("=== grand total {grand_total:8.1} ms");
    // Splits shapes/trace and segment/absorb, summed over every run above
    // (warmup and identity checks included), so only the ratios matter.
    pawtrace::timing::report();
}
