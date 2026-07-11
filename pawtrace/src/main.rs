use anyhow::Result;
use clap::{Parser, ValueEnum};
use pawtrace::{config::Config, output, pipeline, profiles::ProfileStack, psd_import};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Json,
    Svg,
}

#[derive(Parser)]
#[command(about = "PSD/PNG -> traced vectors (Tailmovin JSON / SVG)")]
struct Cli {
    /// Omit to open the GUI.
    input: Option<std::path::PathBuf>,
    #[arg(short, long)]
    output: Option<std::path::PathBuf>,
    #[arg(long, value_enum, default_value = "json")]
    format: Format,
    #[arg(long)]
    detail: Option<f32>,
    #[arg(long)]
    alphamax: Option<f64>,
    #[arg(long)]
    gui: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.gui || cli.input.is_none() {
        return pawtrace::gui::run(cli.input.iter().cloned().collect()).map_err(Into::into);
    }
    let input = cli.input.clone().expect("input present when not opening the GUI");

    let profiles = ProfileStack::load_near(&input);
    let apply_cli = |mut c: Config| -> Config {
        if let Some(v) = cli.detail {
            c.detail = v;
        }
        if let Some(v) = cli.alphamax {
            c.alphamax = v;
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
            let colors = pipeline::run(img, &layer_cfg, w.max(h), (0, 0), &[])?;
            // Layers trace in their own scale space; output assembles in the
            // document's. A per-layer scale override needs converting or it
            // lands displaced and mis-sized.
            let colors = output::place(&colors, layer_cfg.scale, cfg.scale, (0, 0));
            Ok((name.clone(), output::stroke_of(&layer_cfg), colors))
        })
        .collect::<Result<Vec<_>>>()?;

    let out_path = cli.output.unwrap_or_else(|| {
        input.with_extension(match cli.format {
            Format::Svg => "svg",
            Format::Json => "json",
        })
    });
    match cli.format {
        Format::Svg => {
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
        Format::Json => {
            let doc = output::doc(w, h, cfg.scale, traced);
            std::fs::write(&out_path, serde_json::to_vec_pretty(&doc)?)?;
        }
    }
    eprintln!("wrote {}", out_path.display());
    if std::env::var_os("PAWTRACE_TIMING").is_some() {
        pawtrace::timing::report();
    }
    Ok(())
}
