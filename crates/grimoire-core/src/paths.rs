//! Where things live on disk, for the parts of Grimoire that have no terminal.
//!
//! Split out of `main.rs` when the core became its own crate, so the library
//! and every front end that uses it agree on one answer.

use std::path::PathBuf;

/// The user's home folder. `$HOME` on macOS and Linux; on Windows, where
/// `HOME` usually isn't set at all, the profile folder (`C:\Users\name`).
/// On Android there is no home in that sense: the app passes its own base
/// directory instead, so nothing here assumes this path is writable.
pub fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

static DATA_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Where Grimoire keeps things of its own that don't belong in a book and
/// don't travel with it — per machine. The platform's usual place:
/// `$XDG_DATA_HOME/grimoire` or `~/.local/share/grimoire` on Linux,
/// `~/Library/Application Support/grimoire` on macOS, `%APPDATA%\grimoire` on
/// Windows. A front end with its own idea (the phone) sets it with
/// [`set_data_dir`], and so do tests.
pub fn data_dir() -> PathBuf {
    if let Some(d) = DATA_DIR.read().ok().and_then(|d| d.clone()) {
        return d;
    }
    if cfg!(target_os = "macos") {
        return home().join("Library/Application Support/grimoire");
    }
    if cfg!(windows)
        && let Some(app) = std::env::var_os("APPDATA")
    {
        return PathBuf::from(app).join("grimoire");
    }
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x).join("grimoire"),
        None => home().join(".local/share/grimoire"),
    }
}

/// Use `dir` for [`data_dir`] from now on.
pub fn set_data_dir(dir: PathBuf) {
    if let Ok(mut d) = DATA_DIR.write() {
        *d = Some(dir);
    }
}
