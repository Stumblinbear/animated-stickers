//! Widget tree: toolbar, the stage-card strip with per-stage settings and
//! tooltips, and the right panel (full preview, profile editor, layer list).
//!
//! Icons come from the bundled Lucide font (assets/lucide.ttf, ISC
//! license), drawn via the `icon` helper. Raw unicode symbols in label text
//! render as error boxes: iced's default font has no glyphs for arrows,
//! spinners, or emoji, and its fallback found nothing on Windows.

use super::{App, Field, Msg};
use iced::widget::rule::horizontal as horizontal_rule;
use iced::widget::{
    button, checkbox, column, container, image as iced_image, mouse_area, pick_list, row,
    scrollable, slider, text, text_input, tooltip,
};
use iced::{Element, Length};

const ICON_FONT: iced::Font = iced::Font::with_name("lucide");

// Codepoints from lucide.css; the font maps icons into the private-use area.
const I_FILES: char = '\u{e0c9}'; // file-plus
const I_FOLDER: char = '\u{e247}'; // folder-open
const I_EXPORT: char = '\u{e0b2}'; // download
const I_SAVE: char = '\u{e14d}'; // save
const I_LOADER: char = '\u{e109}'; // loader
const I_LOCK: char = '\u{e10b}'; // lock
const I_CHEVRON: char = '\u{e06f}'; // chevron-right
const I_FLAME: char = '\u{e0d2}'; // flame
const I_IMAGE: char = '\u{e0f6}'; // image

fn icon<'a>(code: char) -> iced::widget::Text<'a> {
    text(code.to_string()).font(ICON_FONT)
}

pub(super) const CARD_IMG_WIDTH: f32 = 380.0;

pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let labeled = |i: char, label: &'static str| {
        row![icon(i).size(14), text(label).size(14)]
            .spacing(6)
            .align_y(iced::Alignment::Center)
    };
    let toolbar = row![
        button(labeled(I_FILES, "Open files")).on_press(Msg::Open),
        button(labeled(I_FOLDER, "Open folder")).on_press(Msg::OpenFolder),
        pick_list(
            app.doc_names.clone(),
            app.doc_names.get(app.selected_doc).cloned(),
            Msg::SelectDoc
        )
        .placeholder("no documents"),
        button(labeled(I_EXPORT, "Export all (JSON)")).on_press(Msg::ExportAll),
        button(labeled(I_SAVE, "Save project")).on_press(Msg::SaveProfiles),
        text(&app.status).size(14),
    ]
    .spacing(10)
    .padding(8);

    let mut strip = column![text("Pipeline stages").size(16)]
        .spacing(14)
        .padding(12);
    strip = strip.push(stage_card(
        "1. Source",
        "The layer as painted, cropped to its art. Nothing to configure; \
         everything downstream starts here.",
        stage_visual(app.stages.source.clone()),
        app.stage_pending[0],
        column![].into(),
    ));
    strip = strip.push(stage_card(
        "2. Supersample & flatten",
        "Premultiplied bilinear upsample, then an alpha threshold decides \
         what is art. Soft edges spread into a continuum here, which is \
         what keeps them out of the palette later.",
        stage_visual(app.stages.flat.clone()),
        app.stage_pending[1],
        column![
            setting(
                "Scale",
                format!("{}x", app.cfg.scale),
                "Supersample factor. Boundary precision is 1/scale source \
                 pixels; cost grows with its square. 3 is calibrated; above \
                 4 rarely earns its cost.",
                Field::Scale,
                app.field_is_set(Field::Scale),
                slider(1.0..=6.0, app.cfg.scale as f64, |v| Msg::Set(Field::Scale, v)).step(1.0),
            ),
            setting(
                "Alpha threshold",
                format!("{}%", (app.cfg.alpha_threshold as f64 / 255.0 * 100.0).round()),
                "Opacity cutoff after upsampling: a pixel at or above this is \
                 art, below is background.",
                Field::AlphaThreshold,
                app.field_is_set(Field::AlphaThreshold),
                slider(
                    1.0..=100.0,
                    app.cfg.alpha_threshold as f64 / 255.0 * 100.0,
                    |pct| Msg::Set(Field::AlphaThreshold, pct / 100.0 * 255.0),
                )
                .step(1.0),
            ),
            setting(
                "Mode filter",
                if app.cfg.mode_filter == 0 {
                    "off".into()
                } else {
                    format!("{} px", app.cfg.mode_filter)
                },
                "Majority-vote denoise before quantization: kernel width in \
                 supersampled pixels (source px times the supersample \
                 scale), odd, 0 = off. Off by default: the smooth upsample \
                 plus perceptual remap already cover its job.",
                Field::ModeFilter,
                app.field_is_set(Field::ModeFilter),
                slider(0.0..=15.0, app.cfg.mode_filter as f64, |v| {
                    Msg::Set(Field::ModeFilter, v)
                })
                .step(1.0),
            ),
        ]
        .spacing(10)
        .into(),
    ));
    // Click a swatch to lock its color: locked colors keep their palette
    // slot through any settings change. Each swatch also shows the OKLab
    // distance to its nearest other palette entry, so near-duplicate slots
    // stand out.
    let pal = &app.stages.palette;
    let nearest_de = |i: usize| -> f32 {
        pal.iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, o)| crate::config::color_dist(pal[i], *o))
            .fold(f32::INFINITY, f32::min)
    };
    let swatches = pal.iter().enumerate().fold(row![].spacing(4), |r, (i, c)| {
        let locked = app.cfg.locked.contains(c);
        let hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        let top = if locked {
            row![icon(I_LOCK).size(10), text(hex).size(11)]
                .spacing(3)
                .align_y(iced::Alignment::Center)
        } else {
            row![text(hex).size(11)]
        };
        let de = nearest_de(i);
        let de_text = if de.is_finite() {
            format!("{de:.3}")
        } else {
            "-".into()
        };
        let label = column![top, text(de_text).size(9)].spacing(0);
        r.push(
            button(
                container(label)
                    .style({
                        let c = *c;
                        move |_: &iced::Theme| container::Style {
                            background: Some(iced::Color::from_rgb8(c[0], c[1], c[2]).into()),
                            text_color: Some(
                                if (c[0] as u32 + c[1] as u32 + c[2] as u32) > 380 {
                                    iced::Color::BLACK
                                } else {
                                    iced::Color::WHITE
                                },
                            ),
                            ..Default::default()
                        }
                    })
                    .padding(4),
            )
            .padding(if locked { 3 } else { 0 })
            .on_press(Msg::ToggleLock(*c)),
        )
    });
    strip = strip.push(stage_card(
        "3. Palette & remap",
        "Colors earn palette slots by error energy: pixel count times \
         squared distance to the nearest kept color. A gradient gets one \
         representative; small distinct features (linework, eyes) always \
         beat a blend's third step. Every art pixel then remaps to its \
         perceptually nearest slot. Click a swatch, or click a color right \
         on this image, to lock that color in. Each swatch's second line is \
         its OKLab distance to the nearest other slot.",
        mouse_area(stage_visual(app.stages.quant.clone()))
            .on_move(Msg::QuantHover)
            .on_press(Msg::QuantPick)
            .into(),
        app.stage_pending[2],
        column![
            setting(
                "Detail",
                format!("{:.1}px", app.cfg.detail),
                "Smallest feature worth keeping, in pixels at 512-canvas \
                 scale. Drives the palette floor and speckle removal.",
                Field::Detail,
                app.field_is_set(Field::Detail),
                slider(0.5..=24.0, app.cfg.detail as f64, |v| Msg::Set(Field::Detail, v))
                    .step(0.5),
            ),
            setting(
                "Max colors",
                format!("{}", app.cfg.max_colors),
                "Palette safety cap; extraction usually self-terminates \
                 below it.",
                Field::MaxColors,
                app.field_is_set(Field::MaxColors),
                slider(2.0..=64.0, app.cfg.max_colors as f64, |v| {
                    Msg::Set(Field::MaxColors, v)
                })
                .step(1.0),
            ),
            setting(
                "Merge distance",
                format!("{:.3} \u{394}E", app.cfg.merge_dist),
                "Colors within this perceptual distance (OKLab \u{394}E, \
                 roughly 0..1) merge into one slot. 0 keeps every distinct \
                 bucket. Too high erases soft shading and highlights; fur \
                 highlights sit around 0.037 \u{394}E from their base.",
                Field::MergeDist,
                app.field_is_set(Field::MergeDist),
                slider(0.0..=0.30, app.cfg.merge_dist as f64, |v| {
                    Msg::Set(Field::MergeDist, v)
                })
                .step(0.005),
            ),
            setting(
                "Gradient merge",
                format!("{:.4} \u{394}E", app.cfg.gradient_dist),
                "Candidates within this perceptual distance (OKLab \u{394}E) \
                 of the line between \
                 two kept colors merge as gradient interiors, however far \
                 they are from the endpoints. Distinct features survive: \
                 outlines and highlights are extrema, beyond every segment, \
                 and deliberate mid-tones are picked before their endpoints \
                 form a segment. Soft deliberate strokes sit within ~0.003 \
                 of a segment, so useful values are tiny. 0 disables.",
                Field::GradientDist,
                app.field_is_set(Field::GradientDist),
                slider(0.0..=0.05, app.cfg.gradient_dist as f64, |v| {
                    Msg::Set(Field::GradientDist, v)
                })
                .step(0.0005),
            ),
            setting(
                "Histogram buckets",
                format!(
                    "{} bits ({}-step)",
                    app.cfg.hist_bits,
                    256u32 >> app.cfg.hist_bits.clamp(3, 6)
                ),
                "Bucket granularity for palette candidates, bits per \
                 channel. Coarser pools soft airbrushed strokes into \
                 candidates that clear the detail floor; finer keeps close \
                 distinct colors apart. Range is a hard cap: finer than 6 \
                 bits costs hundreds of MB of histogram. Keep bucket width \
                 at or below merge distance.",
                Field::HistBits,
                app.field_is_set(Field::HistBits),
                slider(3.0..=6.0, app.cfg.hist_bits as f64, |v| {
                    Msg::Set(Field::HistBits, v)
                })
                .step(1.0),
            ),
            setting(
                "Color cleanup",
                if app.cfg.color_cleanup == 0 {
                    "off".into()
                } else {
                    format!("{} px", app.cfg.color_cleanup)
                },
                "Reassigns each pixel to the majority color in a window \
                 (kernel width in supersampled px, 0 = off). Cleans jagged \
                 or speckled edges where two similar palette colors (a dark \
                 speckled edges where two similar palette colors (a dark \
                 line on dark fur) got assigned noisily. Larger kernels also \
                 swallow 1px detail strokes, so raise it only when a \
                 boundary looks ragged.",
                Field::ColorCleanup,
                app.field_is_set(Field::ColorCleanup),
                slider(0.0..=9.0, app.cfg.color_cleanup as f64, |v| {
                    Msg::Set(Field::ColorCleanup, v)
                })
                .step(1.0),
            ),
            swatches,
        ]
        .spacing(10)
        .into(),
    ));
    strip = strip.push(stage_card(
        "4. Regions & absorption",
        "Connected same-color regions, each in its own quantized color, with \
         a fate tint on the ones the trace will not keep: red for a culled \
         region (below the speckle floor, no neighbor to merge into, so it \
         vanishes), orange for one the speckle merge folds into a neighbor \
         (its pixels survive, its color and path do not). Untinted regions \
         trace as their own shape. Hover a region for its area, the floor, \
         and its fate. Thin low-contrast bands between two regions are AA \
         transitions and are absorbed; islands (highlights), color extrema, \
         wide bands, and nested stroke families survive. Click a region to \
         pin it: pinned regions (white dots) skip speckle removal, keeping a \
         small deliberate feature like a tooth. Click again to unpin.",
        mouse_area(stage_visual(app.stages.regions.clone()))
            .on_move(Msg::RegionHover)
            .on_press(Msg::RegionPick)
            .into(),
        app.stage_pending[3],
        column![
            setting(
                "Absorb distance",
                format!("{:.3} \u{394}E", app.cfg.absorb_dist),
                "Bands within this perceptual distance (OKLab \u{394}E) of an \
                 adjacent region merge into it. 0 disables absorption.",
                Field::AbsorbDist,
                app.field_is_set(Field::AbsorbDist),
                slider(0.0..=0.30, app.cfg.absorb_dist as f64, |v| {
                    Msg::Set(Field::AbsorbDist, v)
                })
                .step(0.005),
            ),
            setting(
                "Absorb aggressiveness",
                format!("{:.2}x", app.cfg.absorb_aggr),
                "Scales how thick a band may be and still absorb. 1.0 is the \
                 baseline; the right value depends on the artwork, so raise \
                 it to swallow chunkier transitions and lower it to keep \
                 only the thinnest. It raises the ceiling for deliberate \
                 features too, so pin any it erases.",
                Field::AbsorbAggr,
                app.field_is_set(Field::AbsorbAggr),
                slider(0.0..=3.0, app.cfg.absorb_aggr as f64, |v| {
                    Msg::Set(Field::AbsorbAggr, v)
                })
                .step(0.05),
            ),
            setting(
                "Stroke merge distance",
                format!("{:.3} \u{394}E", app.cfg.stroke_merge_dist),
                "Adjacent thin regions within this perceptual distance \
                 (OKLab \u{394}E) fuse as segments of one stroke, reuniting \
                 linework that quantization cut into pieces. Wide regions \
                 never fuse, so gradient banding keeps its structure. 0 \
                 disables the merge.",
                Field::StrokeMergeDist,
                app.field_is_set(Field::StrokeMergeDist),
                slider(0.0..=0.30, app.cfg.stroke_merge_dist as f64, |v| {
                    Msg::Set(Field::StrokeMergeDist, v)
                })
                .step(0.005),
            ),
            setting(
                "Stroke merge width",
                format!("{:.1} px", app.cfg.stroke_merge_width),
                "How wide (source px) a region may be and still count as a \
                 stroke segment. Set it just above the artwork's line \
                 weight: lower keeps close-colored shapes apart, higher \
                 fuses chunkier linework.",
                Field::StrokeMergeWidth,
                app.field_is_set(Field::StrokeMergeWidth),
                slider(0.0..=12.0, app.cfg.stroke_merge_width as f64, |v| {
                    Msg::Set(Field::StrokeMergeWidth, v)
                })
                .step(0.5),
            ),
            text(match app.region_hover_info() {
                Some(info) => format!(
                    "{} regions, {} pinned  \u{b7}  {info}",
                    app.stages.region_count,
                    app.cfg.pins.len()
                ),
                None => format!(
                    "{} regions, {} pinned",
                    app.stages.region_count,
                    app.cfg.pins.len()
                ),
            })
            .size(13),
        ]
        .spacing(10)
        .into(),
    ));
    strip = strip.push(stage_card(
        "5. Smooth & corners",
        "The pixel boundary of each shape, averaged to shed the staircase \
         (blue outline), with the vertices kept as corners marked (orange \
         dots). This is exactly the geometry the fit runs on: corners get \
         independent tangents, everything else a smooth curve. Green stretches \
         are seams against a near-identical neighbor, fit at the slackened \
         seam tolerance.",
        stage_visual(app.stages.smooth.clone()),
        app.stage_pending[4],
        column![
            setting(
                "Corner angle",
                format!("{:.0}\u{b0}", app.cfg.alphamax * 90.0),
                "A vertex is a corner (fit segments meet with independent \
                 tangents) only where the path bends by at least this much. \
                 Higher keeps fewer corners and smooths more: 0\u{b0} makes \
                 every vertex a corner, 180\u{b0} keeps none. Fur tips \
                 survive at the 104\u{b0} default.",
                Field::Alphamax,
                app.field_is_set(Field::Alphamax),
                slider(0.0..=180.0, app.cfg.alphamax * 90.0, |deg| {
                    Msg::Set(Field::Alphamax, deg / 90.0)
                })
                .step(1.0),
            ),
            setting(
                "Edge smoothing",
                format!("{:.1}x", app.cfg.smoothing),
                "Averages pixel-boundary vertices before fitting, as a \
                 multiple of the supersample scale (corners stay pinned). \
                 Higher rounds off more of the pixel staircase; too high \
                 softens intended detail. 1.0 is the calibrated default, 0 \
                 = none.",
                Field::Smoothing,
                app.field_is_set(Field::Smoothing),
                slider(0.0..=4.0, app.cfg.smoothing as f64, |v| {
                    Msg::Set(Field::Smoothing, v)
                })
                .step(0.1),
            ),
        ]
        .spacing(10)
        .into(),
    ));
    strip = strip.push(stage_card(
        "6. Fit",
        "The smoothed boundary fitted with error-bounded cubic beziers. \
         Anchor count follows curvature, not path length. Fill layers can \
         host the sticker's white outline stroke here.",
        stage_visual(app.stages.render.clone()),
        app.stage_pending[5],
        column![
            setting(
                "Fit tolerance",
                format!("{:.2} px", app.cfg.opttolerance),
                "Max curve deviation in supersampled pixels (source px times \
                 the supersample scale). Higher = fewer anchors, looser fit; \
                 line widths start to wobble past ~1, and the high end \
                 trades shape fidelity for anchor count.",
                Field::Opttolerance,
                app.field_is_set(Field::Opttolerance),
                slider(0.05..=20.0, app.cfg.opttolerance, |v| {
                    Msg::Set(Field::Opttolerance, v)
                })
                .step(0.05),
            ),
            setting(
                "Seam slack",
                format!("{:.1}x", app.cfg.seam_slack),
                "Loosens the fit only along seams against a near-identical \
                 color (within twice the stroke merge distance): such seams \
                 are invisible, so they can carry fewer anchors. It multiplies \
                 the fit tolerance there; the silhouette and high-contrast \
                 edges keep the base tolerance. 1.0 disables.",
                Field::SeamSlack,
                app.field_is_set(Field::SeamSlack),
                slider(1.0..=4.0, app.cfg.seam_slack, |v| {
                    Msg::Set(Field::SeamSlack, v)
                })
                .step(0.1),
            ),
            setting(
                "Sticker stroke",
                format!("{:.1} px", app.cfg.stroke_width),
                "Centered stroke on every path of the layer, in source \
                 pixels. 0 = none. Fill layers host the white sticker \
                 outline: set it on a \"* Fill\" profile (the Illustrator \
                 flow used 11).",
                Field::StrokeWidth,
                app.field_is_set(Field::StrokeWidth),
                slider(0.0..=30.0, app.cfg.stroke_width as f64, |v| {
                    Msg::Set(Field::StrokeWidth, v)
                })
                .step(0.5),
            ),
            row![
                tooltip(
                    text("Stroke color").size(13).width(170),
                    container(
                        text("\"#rrggbb\"; applies once the hex is valid.").size(12)
                    )
                    .padding(6)
                    .max_width(320)
                    .style(container::rounded_box),
                    tooltip::Position::Bottom,
                ),
                text_input("#ffffff", &app.stroke_hex)
                    .on_input(Msg::StrokeHex)
                    .size(13)
                    .width(90),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            text(format!("{} anchors", app.stages.anchor_count)).size(13),
        ]
        .spacing(10)
        .into(),
    ));
    strip = strip.push(stage_card(
        "7. Simplify",
        "Optional final pass: removes anchors whose deletion keeps the path \
         within the tolerance, merging their segments into one curve. \
         Corners are preserved. This is separate from Fit tolerance, which \
         sets the initial anchor density; simplify then trims what remains. \
         0 disables.",
        stage_visual(app.stages.simplified.clone()),
        app.stage_pending[6],
        column![
            setting(
                "Simplify tolerance",
                format!("{:.2} px", app.cfg.simplify),
                "Max deviation in supersampled pixels (source px times the \
                 supersample scale) allowed when dropping an anchor. Higher \
                 removes more. 0 disables the pass.",
                Field::Simplify,
                app.field_is_set(Field::Simplify),
                slider(0.0..=20.0, app.cfg.simplify, |v| Msg::Set(Field::Simplify, v))
                    .step(0.1),
            ),
            text(format!(
                "{} anchors (from {})",
                app.stages.simplify_anchor_count, app.stages.anchor_count
            ))
            .size(13),
        ]
        .spacing(10)
        .into(),
    ));
    let center = scrollable(strip).height(Length::Fill).width(Length::Fill);

    // Right panel: full document on top, layer list beneath it.
    let full_header = row![
        icon(I_IMAGE).size(15),
        text("Full document").size(16),
        if app.full_busy {
            row![icon(I_LOADER).size(13), text("working...").size(13)].spacing(4)
        } else {
            row![]
        },
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let full_view: Element<'_, Msg> = match &app.full_preview {
        Some(h) => iced_image(h.clone()).width(Length::Fill).into(),
        None => text("rendering...").size(13).into(),
    };
    let full_stats = text(match &app.full_stats {
        Some(s) => format!(
            "{} layers, {} shapes, {} anchors",
            s.layers, s.shapes, s.anchors
        ),
        None => String::new(),
    })
    .size(13);

    // Topmost layer first, matching how layer panels read in art tools;
    // storage is bottom-first paint order. Anchor counts from the last full
    // render mark expensive layers relative to the mean.
    let mean_anchors = {
        let total: usize = app.layer_anchors.iter().sum();
        let n = app.layer_anchors.iter().filter(|&&a| a > 0).count().max(1);
        (total as f32 / n as f32).max(1.0)
    };
    let mut layer_list = column![].spacing(3);
    if let Some(doc) = app.doc() {
        for (i, l) in doc.layers.iter().enumerate().rev() {
            let matched = app
                .profiles
                .match_name(&l.name)
                .unwrap_or_else(|| "default".into());
            // A layer override is marked with a trailing "*" on its tag.
            let tag = if app.profiles.has_override(&l.name) {
                format!("[{matched} *]")
            } else {
                format!("[{matched}]")
            };
            let mut row_content = row![].spacing(4).align_y(iced::Alignment::Center);
            if i == app.selected_layer {
                row_content = row_content.push(icon(I_CHEVRON).size(12));
            }
            row_content = row_content
                .push(text(format!("{}  {tag}", l.name)).size(13).width(Length::Fill));
            let anchors = app.layer_anchors.get(i).copied().unwrap_or(0);
            if anchors > 0 {
                let heat = anchors as f32 / mean_anchors;
                let color = if heat >= 2.0 {
                    Some(iced::Color::from_rgb(0.95, 0.42, 0.35))
                } else if heat >= 1.25 {
                    Some(iced::Color::from_rgb(0.92, 0.72, 0.30))
                } else {
                    None
                };
                if heat >= 2.0 {
                    row_content = row_content
                        .push(icon(I_FLAME).size(11).color(color.unwrap()));
                }
                let mut count = text(format!("{anchors}")).size(12);
                if let Some(c) = color {
                    count = count.color(c);
                }
                row_content = row_content.push(count);
            }
            layer_list = layer_list.push(
                button(row_content)
                    .on_press(Msg::SelectLayer(i))
                    .width(Length::Fill)
                    .style(if i == app.selected_layer {
                        button::primary
                    } else {
                        button::text
                    }),
            );
        }
    }

    let layer_name = app
        .doc()
        .and_then(|d| d.layers.get(app.selected_layer))
        .map(|l| l.name.clone());
    let matched = layer_name
        .as_deref()
        .and_then(|l| app.profiles.match_name(l));

    // Profile-mode only: the pattern the edits target, plus whether it hits
    // the selected layer (so the preview would reflect the change).
    let profile_editor: Element<'_, Msg> = if app.edit_profile {
        let hint = if app.profile_input.trim().is_empty() {
            text("empty: writes to [default] (all layers)")
                .size(11)
                .color(iced::Color::from_rgb(0.6, 0.6, 0.65))
        } else if app.profile_input_matches_layer() {
            text("matches this layer")
                .size(11)
                .color(iced::Color::from_rgb(0.45, 0.78, 0.5))
        } else {
            text("does not match this layer")
                .size(11)
                .color(iced::Color::from_rgb(0.92, 0.62, 0.3))
        };
        column![
            row![
                tooltip(
                    text("Pattern").size(13).width(60),
                    container(
                        text(
                            "A case-sensitive glob against the whole layer \
                             name: \"*\" matches any characters, everything \
                             else is literal. Add spaces yourself for word \
                             boundaries: \"Deer *\" is a prefix, \"* Fill\" a \
                             suffix (won't hit \"Refill\"), \"Deer * Fill\" \
                             anchors both ends. The most specific pattern wins."
                        )
                        .size(12)
                    )
                    .padding(6)
                    .max_width(340)
                    .style(container::rounded_box),
                    tooltip::Position::Bottom,
                ),
                text_input("Deer *  ·  * Fill  ·  * Hand *", &app.profile_input)
                    .on_input(Msg::ProfileInput)
                    .size(13),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
            hint,
        ]
        .spacing(4)
        .into()
    } else {
        iced::widget::space().height(0).into()
    };

    let right = container(
        column![
            full_header,
            full_view,
            full_stats,
            horizontal_rule(1),
            text(match &layer_name {
                Some(l) => format!("Editing: {l}"),
                None => "No layer selected".into(),
            })
            .size(14),
            text(format!(
                "Applies now: {}",
                matched.as_deref().unwrap_or("default (no matching profile)")
            ))
            .size(13),
            tooltip(
                checkbox(app.edit_profile)
                    .label("Apply changes to a profile")
                    .on_toggle(Msg::EditProfile)
                    .size(16)
                    .text_size(13),
                container(
                    text(
                        "Off (default): a slider change writes a per-layer \
                         override for just this layer. On: it writes to the \
                         profile named below, for every layer that pattern \
                         matches. Toggling never moves a slider and never \
                         folds this layer's existing overrides into the \
                         profile."
                    )
                    .size(12)
                )
                .padding(6)
                .max_width(340)
                .style(container::rounded_box),
                tooltip::Position::Bottom,
            ),
            profile_editor,
            tooltip(
                checkbox(app.edit_global && app.edit_profile)
                    .label("Save that profile to the global library")
                    .on_toggle_maybe(app.edit_profile.then_some(Msg::EditGlobal))
                .size(16)
                .text_size(13),
                container(
                    text(
                        "In profile mode, send the edit to the per-user \
                         library shared by every project (Fill layers, eyes, \
                         borders) instead of this project's pawtrace.toml. \
                         The project file still wins a tie."
                    )
                    .size(12)
                )
                .padding(6)
                .max_width(340)
                .style(container::rounded_box),
                tooltip::Position::Bottom,
            ),
            button(text("Reset this layer's overrides").size(13)).on_press(Msg::ResetLayer),
            horizontal_rule(1),
            text("Layers (top to bottom)").size(14),
            scrollable(layer_list).height(Length::Fill),
        ]
        .spacing(10)
        .padding(10),
    )
    .width(360);

    column![toolbar, horizontal_rule(1), row![center, right]].into()
}

/// The default card visual: the stage image at card width.
fn stage_visual<'a>(img: Option<iced_image::Handle>) -> Element<'a, Msg> {
    match img {
        Some(h) => iced_image(h).width(CARD_IMG_WIDTH).into(),
        None => text("(no image)").into(),
    }
}

/// A stage card: visual on the left, title/description/settings on the right.
fn stage_card<'a>(
    title: &'a str,
    description: &'a str,
    visual: Element<'a, Msg>,
    pending: bool,
    settings: Element<'a, Msg>,
) -> Element<'a, Msg> {
    let header = row![
        text(title).size(18),
        if pending {
            row![icon(I_LOADER).size(13), text("updating...").size(13)].spacing(4)
        } else {
            row![]
        },
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    container(
        row![
            container(visual).width(CARD_IMG_WIDTH),
            column![header, text(description).size(13), settings]
                .spacing(8)
                .width(Length::Fill),
        ]
        .spacing(14),
    )
    .padding(12)
    .style(container::rounded_box)
    .into()
}

/// A labeled slider with its value and a tooltip. When `modified` (the field
/// is set at the current edit target), the label is tinted and a reset
/// control appears that clears just this field.
fn setting<'a>(
    label: &'a str,
    value: String,
    help: &'a str,
    field: Field,
    modified: bool,
    control: iced::widget::Slider<'a, f64, Msg>,
) -> Element<'a, Msg> {
    let mut label_text = text(format!("{label}: {value}")).size(13).width(150);
    if modified {
        // Accent so customized settings stand out from inherited ones.
        label_text = label_text.color(iced::Color::from_rgb(0.55, 0.78, 1.0));
    }
    let reset: Element<'a, Msg> = if modified {
        button(text("reset").size(11))
            .on_press(Msg::ResetField(field))
            .style(button::text)
            .padding(2)
            .into()
    } else {
        iced::widget::space().width(0).into()
    };
    row![
        tooltip(
            label_text,
            container(text(help).size(12))
                .padding(6)
                .max_width(320)
                .style(container::rounded_box),
            tooltip::Position::Bottom,
        ),
        reset,
        control.width(Length::Fill),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center)
    .into()
}
