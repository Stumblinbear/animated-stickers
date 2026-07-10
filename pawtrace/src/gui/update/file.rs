//! Opening, selecting, and closing documents, plus saving profiles and
//! batch export.

use crate::gui::app::App;
use crate::gui::compute;
use crate::gui::doc;
use crate::gui::ids::DocId;
use crate::gui::msg::{FileMsg, Msg};
use crate::profiles;
use iced::Task;
use std::path::PathBuf;

pub(super) fn update(app: &mut App, msg: FileMsg) -> Task<Msg> {
    match msg {
        FileMsg::OpenFiles => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("art", &["psd", "png"])
                    .pick_files()
                    .await
                    .map(|fs| fs.iter().map(|f| f.path().to_path_buf()).collect())
                    .unwrap_or_default()
            },
            |paths| Msg::File(FileMsg::Opened(paths)),
        ),
        FileMsg::OpenFolder => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .pick_folder()
                    .await
                    .map(|d| d.path().to_path_buf())
            },
            |dir| match dir {
                Some(d) => Msg::File(FileMsg::OpenedFolder(d)),
                None => Msg::File(FileMsg::Opened(Vec::new())),
            },
        ),
        FileMsg::Opened(paths) => {
            let opened = load_docs(app, &paths);
            app.remember_recents(&opened, false);
            focus_last(app)
        }
        FileMsg::OpenedFolder(dir) => {
            // The folder is the recent, not the files it holds, so batch-opening
            // it doesn't flood the Files tab with its contents.
            app.remember_recents(std::slice::from_ref(&dir), true);
            let files = doc::scan_folder(&dir);
            load_docs(app, &files);
            focus_last(app)
        }
        FileMsg::SelectDoc(i) => app.select_doc(i),
        FileMsg::CloseDoc(id) => close_doc(app, id),
        FileMsg::SaveProfiles => save_profiles(app),
        FileMsg::ExportAll => export_all(app),
    }
}

/// Loads each path as a document, appending the ones that open and reporting
/// each failure to the status line. Returns the paths that opened.
fn load_docs(app: &mut App, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut opened = Vec::new();
    for p in paths {
        match doc::load_doc(p) {
            Ok(d) => {
                app.docs.push(d);
                opened.push(p.clone());
            }
            Err(e) => app.status = format!("{}: {e}", p.display()),
        }
    }
    opened
}

/// Focuses the last open document, or does nothing when none are open.
fn focus_last(app: &mut App) -> Task<Msg> {
    if app.docs.is_empty() {
        Task::none()
    } else {
        app.select_doc(app.docs.len() - 1)
    }
}

/// Closes the document identified by `id`, or the selected one when `None`,
/// and re-focuses a neighbor, initializing it if it has not been shown yet.
/// The project tier stays loaded for other open tabs. A no-op when `id` names
/// no open document.
fn close_doc(app: &mut App, id: Option<DocId>) -> Task<Msg> {
    let id = id.unwrap_or(app.selected_doc);
    let Some(i) = app.doc_pos(id) else {
        return Task::none();
    };
    app.docs.remove(i);
    if app.docs.is_empty() {
        // `selected_doc` now resolves to no document; the welcome screen shows.
        return Task::none();
    }
    // Re-focus: the selected document if it survived the close, otherwise the
    // neighbor that slid into the closed tab's slot (clamped to the new last).
    let pos = app.doc_pos(app.selected_doc).unwrap_or_else(|| i.min(app.docs.len() - 1));
    app.select_doc(pos)
}

fn save_profiles(app: &mut App) -> Task<Msg> {
    let mut errors = Vec::new();
    let mut saved = 0;
    for (dir, tier) in &app.projects {
        match write_tier(tier, &dir.join("pawtrace.toml")) {
            Ok(()) => saved += 1,
            Err(e) => errors.push(e.to_string()),
        }
    }
    match profiles::save_global(&app.global_profiles) {
        Ok(()) => saved += 1,
        Err(e) => errors.push(e.to_string()),
    }
    app.status = if errors.is_empty() {
        format!("saved {saved} profile file(s)")
    } else {
        errors.join("; ")
    };
    Task::none()
}

fn export_all(app: &mut App) -> Task<Msg> {
    let mut written = 0;
    for i in 0..app.docs.len() {
        let stack = app.stack(i).to_owned();
        match compute::export_doc(&app.docs[i], &stack) {
            Ok(p) => {
                written += 1;
                app.status = format!("wrote {}", p.display());
            }
            Err(e) => app.status = format!("{}: {e}", app.docs[i].path.display()),
        }
    }
    if written == app.docs.len() {
        app.status = format!("exported {written} document(s)");
    }
    Task::none()
}

fn write_tier(tier: &profiles::Profiles, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = toml::to_string_pretty(tier)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, s)
}
