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

use super::app::App;
use super::view::theme::theme;
use super::view::view;

/// The icon font the window registers via `.font(...)`. Without it every glyph
/// drawn from it renders as tofu, so the snapshot must load it too.
const LUCIDE: &[u8] = include_bytes!("../../assets/lucide.ttf");

/// The app state a snapshot captures.
pub enum Scene<'a> {
    /// The empty startup screen, no document open.
    Empty,
    /// A document loaded from `path`, focused with its top layer selected. Only
    /// the synchronous focus steps run, so the panels populate but the preview
    /// and stage images stay blank; the compute pipeline needs the iced
    /// runtime, which a headless render does not drive.
    Document(&'a Path),
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
    let app = match scene {
        Scene::Empty => App::default(),
        Scene::Document(path) => App::with_document(path)
            .with_context(|| format!("load fixture {}", path.display()))?,
    };

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
