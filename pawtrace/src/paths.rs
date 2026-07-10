//! Where pawtrace keeps its global, cross-project state (the profile library
//! and the recents list).

use std::path::PathBuf;

/// The app's global-config directory, where the profile library and recents
/// live. `%APPDATA%\pawtrace` on Windows, `$XDG_CONFIG_HOME/pawtrace` (falling
/// back to `~/.config/pawtrace`) elsewhere. `None` when neither the config
/// directory nor a home directory can be determined.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("pawtrace"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|base| base.join("pawtrace"))
    }
}
