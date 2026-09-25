//! Grimoire's heart: the book on disk, and everything that reads or changes it.
//!
//! No terminal, no windows, no audio — nothing here knows how it is being
//! shown. That is what lets the same code back the terminal app on a desktop
//! and an Android app on a phone, with the book itself staying plain Markdown
//! either way.

pub mod atomic;
pub mod codex;
pub mod cork;
pub mod create;
pub mod editor;
pub mod export;
pub mod history;
pub mod manuscript;
pub mod notes;
pub mod paths;
pub mod project;
pub mod recovery;
pub mod resume;
pub mod revision;
pub mod search;
pub mod sessions;
pub mod settings;
pub mod spell;
pub mod sync;
