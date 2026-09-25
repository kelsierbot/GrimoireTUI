//! Not overwriting a scene that changed somewhere else.
//!
//! Grimoire has always assumed it is the only thing touching the book. The
//! moment the same folder lives on a desktop and a phone that stops being
//! true: you write a paragraph on the sofa, the file lands on the laptop that
//! still has the morning's version open, and a save there would quietly throw
//! the paragraph away.
//!
//! So a scene remembers what the file looked like when it was read. Saving
//! checks that the file on disk still looks that way. If it doesn't, nothing
//! is written and the caller is handed both versions to sort out — which is
//! the one thing a writing app must never get wrong.

use crate::project::write_atomic;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a file looked like when it was read: its size and a hash of its bytes.
/// Size alone would miss an edit that happens to be the same length; the hash
/// alone would mean reading the whole file twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub hash: u64,
}

impl Fingerprint {
    pub fn of(text: &str) -> Fingerprint {
        Fingerprint::of_bytes(text.as_bytes())
    }

    pub fn of_bytes(bytes: &[u8]) -> Fingerprint {
        Fingerprint {
            len: bytes.len() as u64,
            hash: fnv1a(bytes),
        }
    }

    /// The file as it is on disk right now, or `None` if it isn't there —
    /// which is itself meaningful: a scene that has never been written. Bytes,
    /// not text, so a file that isn't UTF-8 still has a fingerprint.
    pub fn read(path: &Path) -> Option<Fingerprint> {
        fs::read(path).ok().map(|b| Fingerprint::of_bytes(&b))
    }
}

/// The cheap half of noticing a change: size and modification time, from a
/// `stat` rather than a read. When these match what was last seen, the file
/// is taken as unchanged; when they don't, the [`Fingerprint`] decides (a
/// sync client touches the time without changing a word more often than not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl Stat {
    pub fn of(path: &Path) -> Option<Stat> {
        let m = fs::metadata(path).ok()?;
        m.is_file().then(|| Stat {
            len: m.len(),
            modified: m.modified().ok(),
        })
    }
}

/// FNV-1a. Not cryptographic, and it doesn't need to be: this only has to
/// notice that a file changed, never resist someone trying to fool it.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[derive(Debug, PartialEq)]
pub enum SaveOutcome {
    /// The file matched what we last read, so the new text is on disk.
    Written,
    /// Something else wrote to the file first. Nothing was saved; here is what
    /// is on disk now, so the caller can show both and let the writer choose.
    Conflict { on_disk: String },
}

/// Save `text` to `path`, but only if the file still matches `seen` — the
/// fingerprint taken when the scene was read. Pass `None` for a scene that did
/// not exist on disk yet; then anything already there counts as a conflict.
pub fn save_guarded(path: &Path, text: &str, seen: Option<Fingerprint>) -> Result<SaveOutcome> {
    let now = Fingerprint::read(path);
    if now != seen {
        let on_disk = fs::read_to_string(path).unwrap_or_default();
        return Ok(SaveOutcome::Conflict { on_disk });
    }
    write_atomic(path, text)?;
    Ok(SaveOutcome::Written)
}

/// Keep the losing version rather than discarding it: write it beside the
/// scene as "The Crossing (from bazzite, 2026-09-17 15-04).md". A writer can
/// open both in Grimoire and merge by hand — nothing is ever thrown away for
/// them.
pub fn write_conflict_copy(path: &Path, text: &str) -> Result<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    park(dir, path, text)
}

/// Write `text` into `dir` under the conflict-copy name for `scene`. Two
/// clashes in the same minute get "… 2", "… 3": a parked version must never
/// land on top of another one.
fn park(dir: &Path, scene: &Path, text: &str) -> Result<PathBuf> {
    let stem = scene
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "scene".into());
    let ext = scene
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "md".into());
    let when = chrono::Local::now().format("%Y-%m-%d %H-%M");
    let machine = crate::resume::machine_name();
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let base = format!("{stem} (from {machine}, {when})");
    let mut copy = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while copy.exists() {
        copy = dir.join(format!("{base} {n}.{ext}"));
        n += 1;
    }
    write_atomic(&copy, text)?;
    Ok(copy)
}

/// A scene that vanished from disk (deleted or moved by something else) while
/// this session still had unsaved words in it. Writing them back to the old
/// path would resurrect a file someone meant to remove, so they go to the
/// book's trash instead, under the conflict-copy name: on screen, and nothing
/// lost.
pub fn park_in_trash(root: &Path, scene: &Path, text: &str) -> Result<PathBuf> {
    park(&crate::project::trash_dir(root), scene, text)
}

/// The shape a parked version has: "<scene> (from <machine>, <when>).md".
pub fn is_conflict_copy(path: &Path) -> bool {
    path.file_stem()
        .map(|s| s.to_string_lossy().contains(" (from "))
        .unwrap_or(false)
}

/// Throw away a parked version once the writer has settled the conflict. It
/// refuses anything that is not a conflict copy: a bug in a front end should
/// never be able to ask this to delete a scene.
pub fn drop_conflict_copy(path: &Path) -> Result<()> {
    if !is_conflict_copy(path) {
        anyhow::bail!("{} is not a conflict copy", path.display());
    }
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-sync-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_untouched_file_saves() {
        let d = scratch("plain");
        let f = d.join("scene.md");
        fs::write(&f, "morning version\n").unwrap();
        let seen = Fingerprint::read(&f);
        assert_eq!(
            save_guarded(&f, "evening version\n", seen).unwrap(),
            SaveOutcome::Written
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "evening version\n");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_file_changed_elsewhere_is_never_overwritten() {
        let d = scratch("conflict");
        let f = d.join("scene.md");
        fs::write(&f, "morning version\n").unwrap();
        let seen = Fingerprint::read(&f);

        // the phone saved while this copy was open
        fs::write(&f, "the paragraph written on the sofa\n").unwrap();

        let out = save_guarded(&f, "evening version\n", seen).unwrap();
        assert_eq!(
            out,
            SaveOutcome::Conflict {
                on_disk: "the paragraph written on the sofa\n".into()
            }
        );
        // the sofa's words are still there, untouched
        assert_eq!(
            fs::read_to_string(&f).unwrap(),
            "the paragraph written on the sofa\n"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_edit_of_the_same_length_is_still_caught() {
        let d = scratch("samelen");
        let f = d.join("scene.md");
        fs::write(&f, "cat\n").unwrap();
        let seen = Fingerprint::read(&f);
        fs::write(&f, "dog\n").unwrap(); // same byte count, different words
        assert!(matches!(
            save_guarded(&f, "owl\n", seen).unwrap(),
            SaveOutcome::Conflict { .. }
        ));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_scene_that_is_new_on_disk_writes_and_a_surprise_file_does_not() {
        let d = scratch("new");
        let f = d.join("new-scene.md");
        assert_eq!(
            save_guarded(&f, "first words\n", None).unwrap(),
            SaveOutcome::Written
        );

        let g = d.join("already-there.md");
        fs::write(&g, "someone else got here first\n").unwrap();
        assert!(matches!(
            save_guarded(&g, "first words\n", None).unwrap(),
            SaveOutcome::Conflict { .. }
        ));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn two_clashes_in_one_minute_park_two_copies() {
        let d = scratch("twice");
        let f = d.join("Gravel.md");
        fs::write(&f, "theirs\n").unwrap();
        let a = write_conflict_copy(&f, "first parked\n").unwrap();
        let b = write_conflict_copy(&f, "second parked\n").unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read_to_string(&a).unwrap(), "first parked\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "second parked\n");
        assert!(is_conflict_copy(&a) && is_conflict_copy(&b));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_file_that_is_not_utf8_still_has_a_fingerprint() {
        let d = scratch("latin1");
        let f = d.join("note.md");
        fs::write(&f, b"caf\xe9\n").unwrap();
        let fp = Fingerprint::read(&f).expect("bytes, not text");
        assert_eq!(fp, Fingerprint::of_bytes(b"caf\xe9\n"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_losing_version_is_kept_beside_the_scene() {
        let d = scratch("copy");
        let f = d.join("The Crossing.md");
        fs::write(&f, "what the phone wrote\n").unwrap();
        let copy = write_conflict_copy(&f, "what this machine had\n").unwrap();
        assert_eq!(
            fs::read_to_string(&copy).unwrap(),
            "what this machine had\n"
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "what the phone wrote\n");
        let name = copy.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("The Crossing (from "), "{name}");
        assert!(name.ends_with(".md"), "{name}");
        fs::remove_dir_all(&d).unwrap();
    }
}
