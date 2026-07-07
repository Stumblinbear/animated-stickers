use anyhow::Result;
use clap::Parser;
use pawtrace::{config::Config, output, pipeline, profiles::ProfileStack, psd_import};

#[derive(Parser)]
#[command(about = "PSD/PNG -> traced vectors (Tailmovin JSON / SVG)")]
struct Cli {
    /// Omit to open the GUI (requires the "gui" build).
    input: Option<std::path::PathBuf>,
    #[arg(short, long)]
    output: Option<std::path::PathBuf>,
    #[arg(long, default_value = "json")]
    format: String, // json | svg
    #[arg(long, default_value_t = 5.0)]
    detail: f32,
    #[arg(long, default_value_t = 1.15)]
    alphamax: f64,
    #[cfg(feature = "gui")]
    #[arg(long)]
    gui: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    #[cfg(feature = "gui")]
    {
        let want_gui = cli.gui || cli.input.is_none();
        if want_gui {
            return pawtrace::gui::run(cli.input.iter().cloned().collect()).map_err(Into::into);
        }
    }
    let Some(input) = cli.input.clone() else {
        anyhow::bail!("input file required (GUI builds open a window instead: --features gui)");
    };

    let profiles = ProfileStack::load_near(&input);
    // CLI flags override profiles only when explicitly passed — clap doesn't
    // expose "was it passed" for defaulted args cleanly; simplest honest rule:
    // profiles win unless the CLI value differs from the built-in default.
    let apply_cli = |mut c: Config| -> Config {
        let d = Config::default();
        if (cli.detail - d.detail).abs() > f32::EPSILON {
            c.detail = cli.detail;
        }
        if (cli.alphamax - d.alphamax).abs() > f64::EPSILON {
            c.alphamax = cli.alphamax;
        }
        c
    };

    // PSD: one layer per art layer. PNG: single anonymous layer.
    let layers: Vec<(String, image::RgbaImage)> = if input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("psd"))
    {
        psd_import::layers(&std::fs::read(&input)?)?
    } else {
        vec![("layer".into(), image::open(&input)?.to_rgba8())]
    };

    let (w, h) = (layers[0].1.width(), layers[0].1.height());
    // Doc-level cfg for output scaling.
    let cfg = apply_cli(profiles.resolve("").0);
    // Layers are independent; trace them in parallel. collect preserves
    // input order, so paint order is unaffected.
    use rayon::prelude::*;
    let traced = layers
        .par_iter()
        .map(|(name, img)| {
            let (layer_cfg, matched) = profiles.resolve(name);
            let layer_cfg = apply_cli(layer_cfg);
            eprintln!(
                "tracing layer: {name}  [profile: {}]",
                matched.as_deref().unwrap_or("default")
            );
            let mut colors = pipeline::run(img, &layer_cfg, w.max(h), (0, 0))?;
            // Layers trace in their own scale space; output assembles in the
            // document's. A per-layer scale override needs converting or it
            // lands displaced and mis-sized.
            let ratio = cfg.scale as f64 / layer_cfg.scale as f64;
            if ratio != 1.0 {
                for (_, paths) in &mut colors {
                    for p in paths {
                        p.scale(ratio);
                    }
                }
            }
            Ok((name.clone(), output::stroke_of(&layer_cfg), colors))
        })
        .collect::<Result<Vec<_>>>()?;

    let out_path = cli
        .output
        .unwrap_or_else(|| input.with_extension(if cli.format == "svg" { "svg" } else { "json" }));
    match cli.format.as_str() {
        "svg" => {
            let layers: Vec<output::SvgLayer> = traced
                .iter()
                .map(|(name, stroke, colors)| output::SvgLayer {
                    name,
                    stroke: stroke.as_ref(),
                    colors,
                })
                .collect();
            std::fs::write(&out_path, output::svg(w, h, cfg.scale, 0.0, &layers))?
        }
        _ => {
            let doc = output::Doc {
                width: w,
                height: h,
                layers: traced
                    .into_iter()
                    .map(|(name, stroke, colors)| output::Layer {
                        name,
                        stroke,
                        colors: colors
                            .into_iter()
                            .map(|(hex, paths)| output::ColorGroup {
                                hex,
                                paths: paths
                                    .iter()
                                    .map(|p| output::to_json_path(p, cfg.scale as f64))
                                    .collect(),
                            })
                            .collect(),
                    })
                    .collect(),
            };
            std::fs::write(&out_path, serde_json::to_vec_pretty(&doc)?)?;
        }
    }
    eprintln!("wrote {}", out_path.display());
    if std::env::var_os("PAWTRACE_TIMING").is_some() {
        pawtrace::timing::report();
    }
    Ok(())
}
