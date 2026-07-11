//! Headless screenshots of the pawtrace GUI, for iterating on its design
//! without launching a window. Delegates to `pawtrace::gui::write_snapshot`,
//! which renders `App::view()` through iced_test's headless renderer, then
//! decodes each PNG to confirm it is non-trivial.
//!
//! Run: `cargo run --example uishot [output_dir]`.
//!
//! Document-loaded scenes populate the panels, but their preview and stage
//! images stay blank. The compute pipeline is async and a headless render does
//! not drive it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use pawtrace::gui::{write_snapshot, Scene};

/// The design canvas the snapshots are laid out at, in logical pixels. The app
/// has no fixed window size; this is a representative editing size.
const CANVAS: (f32, f32) = (1600.0, 1000.0);

/// Fixtures tried in order for the loaded-document shots; the first that exists
/// is used, so a checkout without them still produces the document-free shots.
const FIXTURES: &[&str] = &["fixtures/blushiboi.psd", "fixtures/happy king.psd"];

fn main() -> Result<()> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/uishot"));
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create output dir {}", out_dir.display()))?;

    for (name, scene) in [("empty", Scene::Empty), ("welcome", Scene::Welcome)] {
        let path = write_snapshot(&out_dir, name, scene, CANVAS)?;
        verify(&path)?;
    }

    match FIXTURES.iter().map(Path::new).find(|p| p.exists()) {
        Some(fixture) => {
            for (name, scene) in [
                ("document", Scene::Document(fixture)),
                ("phase", Scene::Phase(fixture)),
                ("failure", Scene::Failure(fixture)),
                ("library", Scene::Library(fixture)),
            ] {
                let path = write_snapshot(&out_dir, name, scene, CANVAS)?;
                verify(&path)?;
            }
        }
        None => eprintln!("no fixture found; skipping the loaded-document shots"),
    }

    Ok(())
}

/// Decodes the PNG and asserts it has content: more than one distinct pixel, so
/// a blank or single-color render fails loudly. Prints its dimensions and size.
fn verify(path: &Path) -> Result<()> {
    let rgba = image::open(path)
        .with_context(|| format!("decode {}", path.display()))?
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    let bytes = std::fs::metadata(path)?.len();
    let first = rgba.pixels().next().copied();
    if !rgba.pixels().any(|p| Some(*p) != first) {
        return Err(anyhow!("{} is a single flat color", path.display()));
    }
    println!("{}  {w}x{h}  {bytes} bytes", path.display());
    Ok(())
}
