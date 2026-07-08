//! Opening, selecting, and closing documents, plus saving profiles and
//! batch export.

use crate::gui::app::App;
use crate::gui::compute;
use crate::gui::doc;
use crate::gui::msg::{FileMsg, Msg};
use crate::profiles;
use iced::Task;

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
                let Some(dir) = rfd::AsyncFileDialog::new().pick_folder().await else {
                    return Vec::new();
                };
                doc::scan_folder(dir.path())
            },
            |paths| Msg::File(FileMsg::Opened(paths)),
        ),
        FileMsg::Opened(paths) => {
            for p in paths {
                match doc::load_doc(&p) {
                    Ok(d) => app.docs.push(d),
                    Err(e) => app.status = format!("{}: {e}", p.display()),
                }
            }
            if app.docs.is_empty() {
                Task::none()
            } else {
                app.select_doc(app.docs.len() - 1)
            }
        }
        FileMsg::SelectDoc(i) => app.select_doc(i),
        FileMsg::CloseDoc(i) => close_doc(app, i),
        FileMsg::SaveProfiles => save_profiles(app),
        FileMsg::ExportAll => export_all(app),
    }
}

/// Closes document `i` and re-focuses a neighbor, initializing it if it has
/// not been shown yet. The project tier stays loaded for other open tabs.
/// `usize::MAX` closes the selected tab, which the keyboard shortcut cannot
/// name directly.
fn close_doc(app: &mut App, i: usize) -> Task<Msg> {
    let i = if i == usize::MAX { app.selected_doc } else { i };
    if i >= app.docs.len() {
        return Task::none();
    }
    app.docs.remove(i);
    if app.docs.is_empty() {
        app.selected_doc = 0;
        return Task::none();
    }
    if app.selected_doc >= app.docs.len() {
        app.selected_doc = app.docs.len() - 1;
    } else if app.selected_doc > i {
        app.selected_doc -= 1;
    }
    app.select_doc(app.selected_doc)
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
    match profiles::global_path() {
        Some(p) => match write_tier(&app.global_profiles, &p) {
            Ok(()) => saved += 1,
            Err(e) => errors.push(e.to_string()),
        },
        None => errors.push("no APPDATA or HOME for the global library".into()),
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
