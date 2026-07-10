//! Headless PNG snapshots of the editor, for iterating on its design without a
//! window. The widget tree is laid out and rasterized through iced_test's
//! headless renderer (wgpu when a GPU is present, tiny-skia otherwise), so no
//! window is needed.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use iced::Size;
use iced_test::core::Settings;
use iced_test::Simulator;

use super::app::{App, ErrorFix, LayerError};
use super::fields::Field;
use super::msg::{Phase, StripView};
use super::tools::Tool;
use super::recents::{RecentEntry, Timestamp};
use super::view::theme::theme;
use super::view::view;

/// The icon font the window registers via `.font(...)`. Without it every glyph
/// drawn from it renders as tofu, so the snapshot must load it too.
const LUCIDE: &[u8] = include_bytes!("../../assets/lucide.ttf");

/// The app state a snapshot captures.
pub enum Scene<'a> {
    /// The empty startup screen, no document open.
    Empty,
    /// The welcome screen with a seeded recents list, so the panel is populated
    /// without touching the user's real recents file.
    Welcome,
    /// A document loaded from `path`, focused with its top layer selected. Only
    /// the synchronous focus steps run, so the panels populate but the preview
    /// and stage images stay blank; the compute pipeline needs the iced
    /// runtime, which a headless render does not drive.
    Document(&'a Path),
    /// A document on the Colors phase view with the sub-view panel open and the
    /// Heat brush fly-out showing.
    Phase(&'a Path),
    /// A document with a synthetic trace failure on its top layer, for the
    /// coordinated red failure treatment.
    Failure(&'a Path),
    /// A document with the profile-library modal open over it.
    Library(&'a Path),
}

/// Builds the app state for `scene`.
fn build(scene: Scene) -> Result<App> {
    Ok(match scene {
        Scene::Empty => App::default(),
        Scene::Welcome => {
            let mut app = App::default();
            app.welcome.recents = sample_recents();
            app
        }
        Scene::Document(path) => load(path)?,
        Scene::Phase(path) => {
            let mut app = load(path)?;
            app.tools.active = Tool::Heat;
            if let Some(s) = app.session_mut() {
                s.view = StripView::Phase(Phase::Colors);
                s.expanded = Some(Phase::Colors);
            }
            app
        }
        Scene::Failure(path) => {
            let mut app = load(path)?;
            let layer = app.session().map(|s| s.selected_layer).unwrap_or_else(super::ids::LayerId::new);
            if let Some(s) = app.session_mut() {
                s.view = StripView::Phase(Phase::Curves);
                s.expanded = Some(Phase::Curves);
                s.trace_error = Some(LayerError {
                    layer,
                    phase: Phase::Curves,
                    human: "The region fit produced no valid geometry \u{2014} likely too few \
                            opaque pixels after the alpha threshold."
                        .into(),
                    raw: "pipeline: fit stage returned 0 paths (alpha_threshold=128)".into(),
                    fix: Some(ErrorFix {
                        label: "Lower alpha threshold".into(),
                        field: Field::AlphaThreshold,
                        value: 96.0,
                    }),
                });
            }
            app
        }
        Scene::Library(path) => {
            let mut app = load(path)?;
            // Seed a few global templates so the modal is not empty, then open it.
            let g = &mut app.global_profiles;
            for (k, hex) in [("sylvie", "c88a4a"), ("lineart", "8a8a94"), ("flat-cel", "4a6ad0"), ("rex-scales", "5a9a4a")] {
                let ov = crate::profiles::Overrides {
                    locked: Some(vec![format!("#{hex}")]),
                    ..Default::default()
                };
                g.profiles.insert(k.to_string(), ov);
            }
            app.profile_ui.library_open = true;
            app
        }
    })
}

fn load(path: &Path) -> Result<App> {
    App::with_document(path).with_context(|| format!("load fixture {}", path.display()))
}

/// A few made-up recent entries so the welcome snapshot is not empty.
fn sample_recents() -> Vec<RecentEntry> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let hour = 3600;
    let at = |h: u64| Timestamp::from_unix(now - h * hour);
    vec![
        RecentEntry { path: "~/art/sylvie_set".into(), folder: true, opened: at(2), pinned: true },
        RecentEntry { path: "~/art/sylvie_set/sylvie_ref.psd".into(), folder: false, opened: at(3), pinned: false },
        RecentEntry { path: "~/art/sylvie_set/hero_pose.psd".into(), folder: false, opened: at(30), pinned: false },
        RecentEntry { path: "~/art/sylvie_set/tail_swish.png".into(), folder: false, opened: at(32), pinned: false },
        RecentEntry { path: "~/commissions/rex/rex_turnaround.psd".into(), folder: false, opened: at(74), pinned: false },
    ]
}

/// Renders `scene` at `size` logical pixels and writes a PNG into `dir`.
///
/// The file is named `<name>-<renderer>.png`: iced_test appends the renderer's
/// name to the stem. Returns the path written. Any existing PNG for `name` is
/// removed first, so repeated calls regenerate rather than compare.
///
/// # Errors
///
/// Returns an error if `scene` is a document that fails to load, if the
/// headless renderer cannot be created, or if the PNG cannot be written.
pub fn write_snapshot(dir: &Path, name: &str, scene: Scene, size: (f32, f32)) -> Result<PathBuf> {
    let app = build(scene)?;

    let settings = Settings {
        fonts: vec![Cow::Borrowed(LUCIDE)],
        ..Settings::default()
    };
    let mut sim: Simulator<_> =
        Simulator::with_size(settings, Size::new(size.0, size.1), view(&app));
    let snapshot = sim
        .snapshot(&theme())
        .map_err(|e| anyhow!("render {name}: {e:?}"))?;

    // matches_image only writes when its target is absent, so a stale shot from
    // a prior run would otherwise be kept and compared instead of replaced.
    clear_prior(dir, name)?;
    snapshot
        .matches_image(dir.join(format!("{name}.png")))
        .map_err(|e| anyhow!("write {name}: {e:?}"))?;
    find_written(dir, name)
}

/// Whether `path` is `dir/<name>-<something>.png`.
fn is_shot(path: &Path, name: &str) -> bool {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))
        && stem.strip_prefix(name).is_some_and(|rest| rest.starts_with('-'))
}

fn clear_prior(dir: &Path, name: &str) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if is_shot(&path, name) {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn find_written(dir: &Path, name: &str) -> Result<PathBuf> {
    std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .find(|p| is_shot(p, name))
        .ok_or_else(|| anyhow!("no PNG written for {name}"))
}
