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
