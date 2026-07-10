//! The recent-files store shown on the welcome screen, persisted across
//! sessions in the app's global config directory. Loading is best-effort: a
//! missing or malformed file yields an empty list rather than an error.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A moment in time, as whole seconds since the Unix epoch. Serializes as a
/// bare integer, so the stored list stays a plain number.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The current time.
    pub fn now() -> Self {
        Timestamp(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        )
    }

    /// A timestamp at `secs` since the Unix epoch.
    pub const fn from_unix(secs: u64) -> Self {
        Timestamp(secs)
    }

    /// A short "time since" label: `just now`, `2h ago`, `yesterday`, `3d ago`,
    /// or a week count.
    pub fn ago(self) -> String {
        let secs = Timestamp::now().0.saturating_sub(self.0);
        let mins = secs / 60;
        let hours = mins / 60;
        let days = hours / 24;
        if mins < 1 {
            "just now".into()
        } else if hours < 1 {
            format!("{mins}m ago")
        } else if days < 1 {
            format!("{hours}h ago")
        } else if days == 1 {
            "yesterday".into()
        } else if days < 7 {
            format!("{days}d ago")
        } else {
            format!("{}w ago", days / 7)
        }
    }
}

/// One remembered file or folder.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentEntry {
    pub path: PathBuf,
    /// Whether `path` is a folder opened as a batch, rather than a single file.
    #[serde(default)]
    pub folder: bool,
    /// When it was last opened, for the "2h ago" readout and newest-first order.
    #[serde(default)]
    pub opened: Timestamp,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct RecentsFile {
    #[serde(default)]
    entries: Vec<RecentEntry>,
}

/// The recents file, inside the app's global config directory.
fn recents_path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("recents.toml"))
}

/// Loads the persisted recents, newest-first, or an empty list when there is
/// none. Pinned entries sort ahead of the rest.
pub fn load() -> Vec<RecentEntry> {
    let mut entries = recents_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str::<RecentsFile>(&s).ok())
        .map(|f| f.entries)
        .unwrap_or_default();
    sort(&mut entries);
    entries
}

/// Persists `entries`, best-effort: a write failure is ignored, since recents
/// are a convenience, not project data.
pub fn save(entries: &[RecentEntry]) {
    let Some(path) = recents_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = RecentsFile { entries: entries.to_vec() };
    if let Ok(s) = toml::to_string_pretty(&file) {
        let _ = std::fs::write(path, s);
    }
}

/// Records `path` as just opened, moving it to the front (or inserting it) and
/// preserving its pinned flag. Paths are canonicalized so different spellings
/// of the same file collapse to one entry. Caps the list so it can't grow
/// without bound.
pub fn touch(entries: &mut Vec<RecentEntry>, path: &Path, folder: bool) {
    const CAP: usize = 40;
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let now = Timestamp::now();
    if let Some(e) = entries.iter_mut().find(|e| e.path == path) {
        e.opened = now;
        e.folder = folder;
    } else {
        entries.push(RecentEntry { path, folder, opened: now, pinned: false });
    }
    sort(entries);
    entries.truncate(CAP);
}

fn sort(entries: &mut [RecentEntry]) {
    entries.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.opened.cmp(&a.opened)));
}
