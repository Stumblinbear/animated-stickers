//! iced UI (feature "gui"): a pipeline inspector and profile editor. One
//! vertically scrolling strip of stage cards for the selected layer, each
//! card pairing the stage's output image with the settings that act on that
//! stage (tooltips explain each). Edits write the selected profile's
//! overrides and re-run the pipeline in the background, each stage's card
//! refreshing the moment that stage finishes; the right panel keeps the
//! full document render and the layer list in view. Multiple files can be
//! open for batch export.
//!
//! Split: `doc` loads files, `worker` runs the pipeline off the UI thread,
//! `view` builds the widget tree; this module owns the state and routing.

mod doc;
mod view;
mod worker;

use crate::config::Config;
use crate::profiles;
use doc::Doc;
use iced::Task;
use worker::{FullResult, LayerTrace, StageCache, StageImages, StagePart, STAGE_COUNT};

pub use worker::DocStats;

pub fn run(initial: Vec<std::path::PathBuf>) -> iced::Result {
    iced::application(
        move || (App::default(), Task::done(Msg::Opened(initial.clone()))),
        update,
        view::view,
    )
    .title("Pawtrace")
    .theme(theme)
    .font(include_bytes!("../../assets/lucide.ttf").as_slice())
    .run()
}

fn theme(_: &App) -> iced::Theme {
    iced::Theme::TokyoNightStorm
}

#[derive(Default)]
pub struct App {
    docs: Vec<Doc>,
    doc_names: Vec<String>,
    selected_doc: usize,
    selected_layer: usize,
    profiles: profiles::ProfileStack,
    /// Opt-in: when set, a setting change writes to the selected layer's
    /// matching profile instead of a per-layer override. Never re-derives
    /// the displayed values, and never folds existing overrides in.
    edit_profile: bool,
    /// When set (and in profile mode), a profile edit writes into the global
    /// library instead of the project file.
    edit_global: bool,
    /// Profile-mode edit target: the pattern key edits write to, seeded to
    /// the selected layer's matching profile but freely editable (any prefix,
    /// or "*"-suffix). Typing a new pattern and editing a setting creates it.
    profile_input: String,
    /// Control values: always the selected layer's fully resolved Config.
    cfg: Config,
    /// Raw stroke-color text; cfg only updates on a valid "#rrggbb".
    stroke_hex: String,
    stages: StageImages,
    stage_cache: Option<StageCache>,
    full_cache: Vec<Option<(Config, std::sync::Arc<LayerTrace>)>>,
    /// Document scale `full_cache` entries are normalized to.
    full_cache_scale: u32,
    quant_hover: Option<iced::Point>,
    region_hover: Option<iced::Point>,
    stage_pending: [bool; STAGE_COUNT],
    stages_running: bool,
    stages_dirty: bool,
    stages_gen: u64,
    full_preview: Option<iced::widget::image::Handle>,
    full_stats: Option<DocStats>,
    /// Per-layer anchor counts from the last full render, for the layer
    /// list's hot-layer indicators.
    layer_anchors: Vec<usize>,
    full_busy: bool,
    full_dirty: bool,
    /// Full render requested but deferred until the stage strip finishes:
    /// both share the rayon pool, and letting the full render start first
    /// starves the strip's trace, so the layer being edited updates last.
    full_queued: bool,
    full_gen: u64,
    status: String,
}

/// What the profile controls currently write to.
enum EditTarget {
    /// The tier `[default]` section.
    Default,
    /// A named profile (a class of layers).
    Profile(String),
    /// A single layer's override, keyed on its exact name.
    Override(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Scale,
    AlphaThreshold,
    ModeFilter,
    Detail,
    MaxColors,
    MergeDist,
    GradientDist,
    HistBits,
    ColorCleanup,
    Smoothing,
    AbsorbDist,
    AbsorbAggr,
    StrokeMergeDist,
    StrokeMergeWidth,
    Alphamax,
    Opttolerance,
    SeamSlack,
    Simplify,
    StrokeWidth,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open,
    OpenFolder,
    Opened(Vec<std::path::PathBuf>),
    SelectDoc(String),
    SelectLayer(usize),
    Set(Field, f64),
    ResetField(Field),
    StrokeHex(String),
    ToggleLock([u8; 3]),
    QuantHover(iced::Point),
    QuantPick,
    RegionHover(iced::Point),
    RegionPick,
    StagePart(u64, StagePart),
    FullReady(u64, Result<Box<FullResult>, String>),
    EditProfile(bool),
    EditGlobal(bool),
    ProfileInput(String),
    ResetLayer,
    SaveProfiles,
    ExportAll,
}

fn update(app: &mut App, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Open => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("art", &["psd", "png"])
                    .pick_files()
                    .await
                    .map(|fs| fs.iter().map(|f| f.path().to_path_buf()).collect())
                    .unwrap_or_default()
            },
            Msg::Opened,
        ),
        Msg::OpenFolder => Task::perform(
            async {
                let Some(dir) = rfd::AsyncFileDialog::new().pick_folder().await else {
                    return Vec::new();
                };
                doc::scan_folder(dir.path())
            },
            Msg::Opened,
        ),
        Msg::Opened(paths) => {
            for p in paths {
                match doc::load_doc(&p) {
                    Ok(doc) => {
                        app.doc_names.push(doc::doc_label(&doc.path));
                        app.docs.push(doc);
                    }
                    Err(e) => app.status = format!("{}: {e}", p.display()),
                }
            }
            if !app.docs.is_empty() {
                app.select_doc(app.docs.len() - 1)
            } else {
                Task::none()
            }
        }
        Msg::SelectDoc(name) => {
            if let Some(i) = app.doc_names.iter().position(|n| *n == name) {
                app.full_preview = None;
                app.full_stats = None;
                app.select_doc(i)
            } else {
                Task::none()
            }
        }
        Msg::SelectLayer(i) => app.select_layer(i),
        Msg::Set(field, v) => {
            app.apply_field(field, v);
            app.write_field(field);
            app.preview_tasks()
        }
        Msg::ResetField(field) => {
            app.reset_field(field);
            app.preview_tasks()
        }
        Msg::StrokeHex(s) => {
            app.stroke_hex = s;
            let Some(c) = profiles::parse_hex(&app.stroke_hex) else {
                return Task::none();
            };
            if c == app.cfg.stroke_color {
                return Task::none();
            }
            app.cfg.stroke_color = c;
            app.write_stroke_color();
            app.preview_tasks()
        }
        Msg::ToggleLock(c) => app.toggle_lock(c),
        Msg::QuantHover(p) => {
            app.quant_hover = Some(p);
            Task::none()
        }
        Msg::QuantPick => {
            // Map the last hover position from displayed to image pixels:
            // the image widget is width-fit at CARD_IMG_WIDTH with aspect
            // preserved, so one scale factor covers both axes.
            let Some(p) = app.quant_hover else { return Task::none() };
            let Some(q) = &app.stages.quant_px else { return Task::none() };
            let scale = q.width() as f32 / view::CARD_IMG_WIDTH;
            let (x, y) = ((p.x * scale) as u32, (p.y * scale) as u32);
            if x >= q.width() || y >= q.height() {
                return Task::none();
            }
            let px = q.get_pixel(x, y).0;
            let c = [px[0], px[1], px[2]];
            if px[3] != 0 && app.stages.palette.contains(&c) {
                app.toggle_lock(c)
            } else {
                Task::none()
            }
        }
        Msg::RegionHover(p) => {
            app.region_hover = Some(p);
            Task::none()
        }
        Msg::RegionPick => match app.region_hover {
            Some(p) => app.toggle_pin(p),
            None => Task::none(),
        },
        Msg::StagePart(generation, part) => {
            let done = matches!(part, StagePart::Simplify(..));
            if generation == app.stages_gen {
                match part {
                    StagePart::Source(h) => {
                        app.stages.source = Some(h);
                        app.stage_pending[0] = false;
                    }
                    StagePart::Flat(h) => {
                        app.stages.flat = Some(h);
                        app.stage_pending[1] = false;
                    }
                    StagePart::Quant(h, pal, px) => {
                        app.stages.quant = Some(h);
                        app.stages.palette = pal;
                        app.stages.quant_px = Some(px);
                        app.stage_pending[2] = false;
                    }
                    StagePart::Regions(h, count, report) => {
                        app.stages.regions = Some(h);
                        app.stages.region_count = count;
                        app.stages.region_report = Some(report);
                        app.stage_pending[3] = false;
                    }
                    StagePart::Smooth(h) => {
                        app.stages.smooth = h;
                        app.stage_pending[4] = false;
                    }
                    StagePart::Fit(h, anchors) => {
                        app.stages.render = h;
                        app.stages.anchor_count = anchors;
                        app.stage_pending[5] = false;
                    }
                    StagePart::Simplify(h, anchors, cache) => {
                        app.stages.simplified = h;
                        app.stages.simplify_anchor_count = anchors;
                        app.stage_cache = Some(*cache);
                        app.stage_pending[6] = false;
                        app.status = format!(
                            "{} regions, {} anchors ({} after simplify)",
                            app.stages.region_count,
                            app.stages.anchor_count,
                            app.stages.simplify_anchor_count
                        );
                    }
                }
            }
            if done {
                app.stages_running = false;
                if app.stages_dirty {
                    // More edits arrived mid-run; the full render stays
                    // queued until the strip settles on the final state.
                    app.stages_dirty = false;
                    return app.spawn_stages();
                }
                if app.full_queued {
                    app.full_queued = false;
                    return app.spawn_full();
                }
            }
            Task::none()
        }
        Msg::FullReady(generation, result) => {
            if generation == app.full_gen {
                match result {
                    Ok(r) => {
                        app.full_preview = Some(r.handle);
                        app.full_stats = Some(r.stats);
                        app.layer_anchors = r
                            .cache
                            .iter()
                            .map(|e| {
                                e.as_ref().map_or(0, |(_, t)| {
                                    t.iter()
                                        .flat_map(|(_, paths)| paths.iter())
                                        .map(|p| p.cubics.len())
                                        .sum()
                                })
                            })
                            .collect();
                        app.full_cache = r.cache;
                        app.full_cache_scale = r.doc_scale;
                    }
                    Err(e) => app.status = e,
                }
            }
            app.full_busy = false;
            if app.full_dirty {
                app.full_dirty = false;
                app.spawn_full()
            } else {
                Task::none()
            }
        }
        // Toggling a write-mode flag never changes the displayed values (the
        // controls always show the layer's resolved config), only where the
        // next edit lands, so no recompute is needed.
        Msg::EditProfile(b) => {
            app.edit_profile = b;
            Task::none()
        }
        Msg::EditGlobal(b) => {
            app.edit_global = b;
            Task::none()
        }
        // Only redirects where the next profile edit lands; the displayed
        // values are the layer's, so nothing recomputes.
        Msg::ProfileInput(s) => {
            app.profile_input = s;
            Task::none()
        }
        Msg::ResetLayer => {
            if let Some(layer) = app.layer_name() {
                app.profiles.project.overrides.remove(&layer);
            }
            app.load_layer_into_controls();
            app.preview_tasks()
        }
        Msg::SaveProfiles => {
            let path = app
                .docs
                .get(app.selected_doc)
                .and_then(|d| d.path.parent())
                .map(|d| d.join("pawtrace.toml"))
                .unwrap_or_else(|| "pawtrace.toml".into());
            let project = write_tier(&app.profiles.project, &path);
            let global = match profiles::global_path() {
                Some(p) => write_tier(&app.profiles.global, &p),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no APPDATA or HOME for the global library",
                )),
            };
            app.status = match (project, global) {
                (Ok(()), Ok(())) => format!("saved {} + global library", path.display()),
                (Err(e), _) | (_, Err(e)) => e.to_string(),
            };
            Task::none()
        }
        Msg::ExportAll => {
            let mut written = 0;
            for doc in &app.docs {
                match worker::export_doc(doc, &app.profiles) {
                    Ok(p) => {
                        written += 1;
                        app.status = format!("wrote {}", p.display());
                    }
                    Err(e) => app.status = format!("{}: {e}", doc.path.display()),
                }
            }
            if written == app.docs.len() {
                app.status = format!("exported {written} document(s)");
            }
            Task::none()
        }
    }
}

fn write_tier(tier: &profiles::Profiles, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = toml::to_string_pretty(tier)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, s)
}

impl App {
    fn doc(&self) -> Option<&Doc> {
        self.docs.get(self.selected_doc)
    }

    /// Switch to document `i`: reload its profiles, drop caches tied to the
    /// old document, and select its topmost layer.
    fn select_doc(&mut self, i: usize) -> Task<Msg> {
        self.selected_doc = i;
        // Profiles live next to the art, per the CLI's convention.
        self.profiles = profiles::ProfileStack::load_near(&self.docs[i].path);
        self.stage_cache = None;
        self.full_cache.clear();
        self.layer_anchors.clear();
        // Storage is bottom-first paint order; the top of the visual stack
        // is the last index.
        let top = self.docs[i].layers.len().saturating_sub(1);
        Task::batch([self.select_layer(top), self.spawn_full()])
    }

    fn select_layer(&mut self, i: usize) -> Task<Msg> {
        self.selected_layer = i;
        // Seed the profile-edit target with the layer's current match; the
        // user can retype it to any pattern.
        self.profile_input = self
            .layer_name()
            .and_then(|l| self.profiles.match_name(&l))
            .unwrap_or_default();
        self.load_layer_into_controls();
        self.spawn_stages()
    }

    fn layer_name(&self) -> Option<String> {
        self.doc()
            .and_then(|d| d.layers.get(self.selected_layer))
            .map(|l| l.name.clone())
    }

    /// Where a setting change lands: the profile named in `profile_input` in
    /// profile mode (its tier `[default]` when that box is empty), otherwise
    /// a per-layer override.
    fn edit_target(&self) -> EditTarget {
        if self.edit_profile {
            let key = self.profile_input.trim();
            if key.is_empty() {
                EditTarget::Default
            } else {
                EditTarget::Profile(key.to_string())
            }
        } else {
            match self.layer_name() {
                Some(l) => EditTarget::Override(l),
                None => EditTarget::Default,
            }
        }
    }

    /// Whether the typed profile pattern matches the selected layer, so its
    /// edits actually affect this layer's preview.
    fn profile_input_matches_layer(&self) -> bool {
        let key = self.profile_input.trim();
        !key.is_empty()
            && self
                .layer_name()
                .is_some_and(|l| profiles::key_matches(key, &l))
    }

    /// The tier a profile edit writes to. Overrides are always project-side.
    fn scope(&self) -> profiles::Scope {
        if self.edit_profile && self.edit_global {
            profiles::Scope::Global
        } else {
            profiles::Scope::Project
        }
    }

    /// Load the selected layer's fully resolved config into the controls.
    /// This is the only thing the controls ever show, so switching write
    /// mode never moves a slider.
    fn load_layer_into_controls(&mut self) {
        self.cfg = match self.layer_name() {
            Some(l) => self.profiles.resolve(&l).0,
            None => Config::default(),
        };
        let c = self.cfg.stroke_color;
        self.stroke_hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    }

    /// The overrides map the current target writes into, created if absent.
    fn target_ov(&mut self) -> &mut profiles::Overrides {
        let scope = self.scope();
        match self.edit_target() {
            EditTarget::Override(layer) => {
                self.profiles.project.overrides.entry(layer).or_default()
            }
            EditTarget::Profile(key) => {
                self.profiles.tier_mut(scope).profiles.entry(key).or_default()
            }
            EditTarget::Default => &mut self.profiles.tier_mut(scope).default,
        }
    }

    /// The current target's overrides, if it exists, without creating it.
    fn target_ov_ref(&self) -> Option<&profiles::Overrides> {
        match self.edit_target() {
            EditTarget::Override(layer) => self.profiles.project.overrides.get(&layer),
            EditTarget::Profile(key) => self.profiles.tier(self.scope()).profiles.get(&key),
            EditTarget::Default => Some(&self.profiles.tier(self.scope()).default),
        }
    }

    /// Whether `field` is set at the current target (so the reset control
    /// has something to clear).
    fn field_is_set(&self, field: Field) -> bool {
        self.target_ov_ref().is_some_and(|ov| field_is_set(ov, field))
    }

    /// Clears the current layer's override of `field` (profile mode only), so
    /// a profile edit is not shadowed by an existing per-layer override.
    fn clear_layer_field(&mut self, field: Field) {
        if let Some(layer) = self.layer_name() {
            if let Some(o) = self.profiles.project.overrides.get_mut(&layer) {
                field_clear(o, field);
            }
        }
    }

    /// Writes one setting's current value to the edit target. In profile mode
    /// it also clears that setting from the layer's override, promoting it to
    /// the profile without touching the layer's other overrides.
    fn write_field(&mut self, field: Field) {
        let cfg = self.cfg.clone();
        field_set(self.target_ov(), field, &cfg);
        if self.edit_profile {
            self.clear_layer_field(field);
        }
    }

    /// Clears one setting from the edit target and re-resolves the controls.
    /// A layer override emptied of its last field is dropped, so no bare
    /// `[overrides."layer"]` lingers.
    fn reset_field(&mut self, field: Field) {
        field_clear(self.target_ov(), field);
        if let EditTarget::Override(layer) = self.edit_target() {
            if self.profiles.project.overrides.get(&layer) == Some(&profiles::Overrides::default()) {
                self.profiles.project.overrides.remove(&layer);
            }
        }
        self.load_layer_into_controls();
    }

    fn write_stroke_color(&mut self) {
        let c = self.cfg.stroke_color;
        self.target_ov().stroke_color = Some(format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]));
    }

    fn apply_field(&mut self, field: Field, v: f64) {
        match field {
            Field::Scale => self.cfg.scale = v as u32,
            Field::AlphaThreshold => self.cfg.alpha_threshold = v as u8,
            Field::ModeFilter => self.cfg.mode_filter = v as u32,
            Field::Detail => self.cfg.detail = v as f32,
            Field::MaxColors => self.cfg.max_colors = v as usize,
            Field::MergeDist => self.cfg.merge_dist = v as f32,
            Field::GradientDist => self.cfg.gradient_dist = v as f32,
            Field::HistBits => self.cfg.hist_bits = v as u32,
            Field::ColorCleanup => self.cfg.color_cleanup = v as u32,
            Field::Smoothing => self.cfg.smoothing = v as f32,
            Field::AbsorbDist => self.cfg.absorb_dist = v as f32,
            Field::AbsorbAggr => self.cfg.absorb_aggr = v as f32,
            Field::StrokeMergeDist => self.cfg.stroke_merge_dist = v as f32,
            Field::StrokeMergeWidth => self.cfg.stroke_merge_width = v as f32,
            Field::Alphamax => self.cfg.alphamax = v,
            Field::Opttolerance => self.cfg.opttolerance = v,
            Field::SeamSlack => self.cfg.seam_slack = v,
            Field::Simplify => self.cfg.simplify = v,
            Field::StrokeWidth => self.cfg.stroke_width = v as f32,
        }
    }

    /// Writes the current locked-color set to the edit target.
    fn write_locked(&mut self) {
        let hexes: Vec<String> = self
            .cfg
            .locked
            .iter()
            .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
            .collect();
        self.target_ov().locked = Some(hexes);
    }

    /// Writes the current pin set to the edit target.
    fn write_pins(&mut self) {
        let pins = self.cfg.pins.clone();
        self.target_ov().pins = Some(pins);
    }

    fn toggle_lock(&mut self, c: [u8; 3]) -> Task<Msg> {
        if let Some(i) = self.cfg.locked.iter().position(|&l| l == c) {
            self.cfg.locked.remove(i);
        } else {
            self.cfg.locked.push(c);
        }
        self.write_locked();
        self.preview_tasks()
    }

    fn preview_tasks(&mut self) -> Task<Msg> {
        self.full_queued = true;
        self.spawn_stages()
    }
}

/// Sets `field`'s value from `cfg` into `ov`.
fn field_set(ov: &mut profiles::Overrides, field: Field, cfg: &Config) {
    match field {
        Field::Scale => ov.scale = Some(cfg.scale),
        Field::AlphaThreshold => ov.alpha_threshold = Some(cfg.alpha_threshold),
        Field::ModeFilter => ov.mode_filter = Some(cfg.mode_filter),
        Field::Detail => ov.detail = Some(cfg.detail),
        Field::MaxColors => ov.max_colors = Some(cfg.max_colors),
        Field::MergeDist => ov.merge_dist = Some(cfg.merge_dist),
        Field::GradientDist => ov.gradient_dist = Some(cfg.gradient_dist),
        Field::HistBits => ov.hist_bits = Some(cfg.hist_bits),
        Field::ColorCleanup => ov.color_cleanup = Some(cfg.color_cleanup),
        Field::Smoothing => ov.smoothing = Some(cfg.smoothing),
        Field::AbsorbDist => ov.absorb_dist = Some(cfg.absorb_dist),
        Field::AbsorbAggr => ov.absorb_aggr = Some(cfg.absorb_aggr),
        Field::StrokeMergeDist => ov.stroke_merge_dist = Some(cfg.stroke_merge_dist),
        Field::StrokeMergeWidth => ov.stroke_merge_width = Some(cfg.stroke_merge_width),
        Field::Alphamax => ov.alphamax = Some(cfg.alphamax),
        Field::Opttolerance => ov.opttolerance = Some(cfg.opttolerance),
        Field::SeamSlack => ov.seam_slack = Some(cfg.seam_slack),
        Field::Simplify => ov.simplify = Some(cfg.simplify),
        Field::StrokeWidth => ov.stroke_width = Some(cfg.stroke_width),
    }
}

/// Clears `field` from `ov`.
fn field_clear(ov: &mut profiles::Overrides, field: Field) {
    match field {
        Field::Scale => ov.scale = None,
        Field::AlphaThreshold => ov.alpha_threshold = None,
        Field::ModeFilter => ov.mode_filter = None,
        Field::Detail => ov.detail = None,
        Field::MaxColors => ov.max_colors = None,
        Field::MergeDist => ov.merge_dist = None,
        Field::GradientDist => ov.gradient_dist = None,
        Field::HistBits => ov.hist_bits = None,
        Field::ColorCleanup => ov.color_cleanup = None,
        Field::Smoothing => ov.smoothing = None,
        Field::AbsorbDist => ov.absorb_dist = None,
        Field::AbsorbAggr => ov.absorb_aggr = None,
        Field::StrokeMergeDist => ov.stroke_merge_dist = None,
        Field::StrokeMergeWidth => ov.stroke_merge_width = None,
        Field::Alphamax => ov.alphamax = None,
        Field::Opttolerance => ov.opttolerance = None,
        Field::SeamSlack => ov.seam_slack = None,
        Field::Simplify => ov.simplify = None,
        Field::StrokeWidth => ov.stroke_width = None,
    }
}

/// Whether `field` is set in `ov`.
fn field_is_set(ov: &profiles::Overrides, field: Field) -> bool {
    match field {
        Field::Scale => ov.scale.is_some(),
        Field::AlphaThreshold => ov.alpha_threshold.is_some(),
        Field::ModeFilter => ov.mode_filter.is_some(),
        Field::Detail => ov.detail.is_some(),
        Field::MaxColors => ov.max_colors.is_some(),
        Field::MergeDist => ov.merge_dist.is_some(),
        Field::GradientDist => ov.gradient_dist.is_some(),
        Field::HistBits => ov.hist_bits.is_some(),
        Field::ColorCleanup => ov.color_cleanup.is_some(),
        Field::Smoothing => ov.smoothing.is_some(),
        Field::AbsorbDist => ov.absorb_dist.is_some(),
        Field::AbsorbAggr => ov.absorb_aggr.is_some(),
        Field::StrokeMergeDist => ov.stroke_merge_dist.is_some(),
        Field::StrokeMergeWidth => ov.stroke_merge_width.is_some(),
        Field::Alphamax => ov.alphamax.is_some(),
        Field::Opttolerance => ov.opttolerance.is_some(),
        Field::SeamSlack => ov.seam_slack.is_some(),
        Field::Simplify => ov.simplify.is_some(),
        Field::StrokeWidth => ov.stroke_width.is_some(),
    }
}
