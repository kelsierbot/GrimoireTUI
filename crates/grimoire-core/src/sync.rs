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

// ── other programs' conflict copies ─────────────────────────────────

/// Who left a conflict copy beside a scene. Grimoire parks its own; every
/// sync app has its own way of keeping the version that lost, and each one
/// is that app telling you two devices wrote the same scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Grimoire,
    Dropbox,
    PCloud,
    Box,
    OneDrive,
    ICloud,
    Syncthing,
    GoogleDrive,
    /// A numbered copy ("Scene (1)") from a sync app that couldn't be told
    /// apart from its neighbours by the name alone.
    SyncApp,
}

impl Source {
    /// The program, as a person would name it.
    pub fn name(self) -> &'static str {
        match self {
            Source::Grimoire => "Grimoire",
            Source::Dropbox => "Dropbox",
            Source::PCloud => "pCloud",
            Source::Box => "Box",
            Source::OneDrive => "OneDrive",
            Source::ICloud => "iCloud",
            Source::Syncthing => "Syncthing",
            Source::GoogleDrive => "Google Drive",
            Source::SyncApp => "your sync app",
        }
    }

    /// The tag the tree shows where a word count would be.
    pub fn label(self) -> &'static str {
        match self {
            Source::Grimoire => "parked copy",
            Source::Dropbox => "Dropbox copy",
            Source::PCloud => "pCloud copy",
            Source::Box => "Box copy",
            Source::OneDrive => "OneDrive copy",
            Source::ICloud => "iCloud copy",
            Source::Syncthing => "Syncthing copy",
            Source::GoogleDrive => "Google Drive copy",
            Source::SyncApp => "sync copy",
        }
    }
}

/// A conflict copy of another file in the same folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyOf {
    pub source: Source,
    /// The original's file stem: "01-Scene-One".
    pub original: String,
    /// What the copy's name adds after the original's stem, kept as it is
    /// when the original is renamed or moved, so the copy follows it:
    /// " (Josh's conflicted copy 2026-09-25)".
    pub suffix: String,
}

/// Which sync app a book lives in, from the folders around it, for the
/// copies whose names several apps share ("Scene (1)", "… conflicted copy").
pub fn provider_of(path: &Path) -> Option<Source> {
    path.ancestors().find_map(|a| {
        let name = a.file_name()?.to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if lower == "dropbox" || lower.starts_with("dropbox (") || lower.starts_with("dropbox-") {
            Some(Source::Dropbox)
        } else if lower == "box" || lower == "box drive" || lower.starts_with("box-box") {
            Some(Source::Box)
        } else if lower == "google drive"
            || lower == "my drive"
            || lower.starts_with("googledrive-")
        {
            Some(Source::GoogleDrive)
        } else if lower == "pclouddrive" || lower == "pcloud drive" || lower == "pcloud sync" {
            Some(Source::PCloud)
        } else if lower.starts_with("onedrive") {
            Some(Source::OneDrive)
        } else if lower == "mobile documents" || lower == "icloud drive" || lower == "iclouddrive" {
            Some(Source::ICloud)
        } else {
            None
        }
    })
}

/// Is the file stem `stem` a conflict copy, and of what? `has` says whether
/// a sibling `.md` with a given stem exists. The names that could be an
/// ordinary title — "Part (1)", "Draft 2", "Notes-NYC2" — only count when the
/// file they'd be a copy of is right there beside them; the unmistakable
/// ones ("… conflicted copy …", ".sync-conflict-…") always do. A copy of a
/// copy resolves to the first original. `hint` is [`provider_of`] the book.
pub fn copy_of(stem: &str, has: &dyn Fn(&str) -> bool, hint: Option<Source>) -> Option<CopyOf> {
    let mut base = stem;
    let mut outer = None;
    while let Some((source, shorter)) = strip_once(base, has, hint) {
        outer.get_or_insert(source);
        base = shorter;
    }
    let source = outer?;
    Some(CopyOf {
        source,
        original: base.to_string(),
        suffix: stem[base.len()..].to_string(),
    })
}

/// The conflict copy at `path`, judged against the files beside it.
pub fn copy_of_path(path: &Path) -> Option<CopyOf> {
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let stems = md_stems(path.parent()?);
    copy_of(&stem, &|s| stems.contains(s), provider_of(path))
}

/// The stems of the `.md` files in `dir`, hidden ones left out.
pub fn md_stems(dir: &Path) -> std::collections::HashSet<String> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                .filter(|s| !s.starts_with('.'))
                .collect()
        })
        .unwrap_or_default()
}

/// Every conflict copy of the scene at `path`, beside it on disk, with what
/// makes each one a copy.
pub fn copies_of(path: &Path) -> Vec<(PathBuf, CopyOf)> {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem()) else {
        return Vec::new();
    };
    let stem = stem.to_string_lossy().to_string();
    let stems = md_stems(dir);
    let hint = provider_of(path);
    let mut out: Vec<(PathBuf, CopyOf)> = stems
        .iter()
        .filter(|s| **s != stem)
        .filter_map(|s| {
            copy_of(s, &|x| stems.contains(x), hint)
                .filter(|c| c.original == stem)
                .map(|c| (dir.join(format!("{s}.md")), c))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// One layer of copy-naming taken off the end of `stem`, if there is one.
fn strip_once<'a>(
    stem: &'a str,
    has: &dyn Fn(&str) -> bool,
    hint: Option<Source>,
) -> Option<(Source, &'a str)> {
    // Grimoire's own: "<scene> (from <machine>, <when>)", maybe " 2" after.
    if let Some(i) = stem.rfind(" (from ") {
        let tail = stem[i + 7..].trim_end_matches(|c: char| c.is_ascii_digit());
        let tail = tail.trim_end();
        if tail.ends_with(')') && tail.contains(", ") && i > 0 {
            return Some((Source::Grimoire, &stem[..i]));
        }
    }
    // Syncthing: "<name>.sync-conflict-20260925-140233-ABCDEFG".
    if let Some(i) = stem.rfind(".sync-conflict-") {
        let rest = &stem[i + ".sync-conflict-".len()..];
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() >= 3
            && parts[0].len() == 8
            && parts[0].chars().all(|c| c.is_ascii_digit())
            && parts[1].len() == 6
            && parts[1].chars().all(|c| c.is_ascii_digit())
            && i > 0
        {
            return Some((Source::Syncthing, &stem[..i]));
        }
    }
    // pCloud and Google Drive's bracketed tags.
    for (tag, source) in [
        (" [conflicted]", Source::PCloud),
        ("[conflicted]", Source::PCloud),
        (" [Conflict]", Source::GoogleDrive),
        ("[Conflict]", Source::GoogleDrive),
        (".conflicted", Source::PCloud),
    ] {
        if let Some(base) = stem.strip_suffix(tag).filter(|b| !b.is_empty()) {
            return Some((source, base));
        }
    }
    // Google Drive: "<name>_conf(1)".
    if let Some(i) = stem.rfind("_conf(")
        && stem.ends_with(')')
        && stem[i + 6..stem.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit())
        && stem.len() > i + 7
        && i > 0
    {
        return Some((Source::GoogleDrive, &stem[..i]));
    }
    // "<name> (…)": the tag inside the last parentheses says who.
    if stem.ends_with(')')
        && let Some(i) = stem.rfind(" (")
        && i > 0
    {
        let base = &stem[..i];
        let inner = &stem[i + 2..stem.len() - 1];
        let lower = inner.to_lowercase();
        if lower.contains("conflicted copy") {
            // Dropbox's shape; Box names the same way in some versions.
            let source = if hint == Some(Source::Box) {
                Source::Box
            } else {
                Source::Dropbox
            };
            return Some((source, base));
        }
        if lower.contains("copie en conflit") {
            return Some((Source::Dropbox, base));
        }
        let plain = lower
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .trim_end();
        if matches!(
            plain,
            "case conflict" | "unicode encoding conflict" | "whitespace conflict"
        ) {
            return Some((Source::Dropbox, base));
        }
        if plain == "conflicted" {
            return Some((Source::PCloud, base));
        }
        // Ambiguous: only beside the original.
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) && has(base) {
            let source = match hint {
                Some(s @ (Source::Box | Source::GoogleDrive)) => s,
                _ => Source::SyncApp,
            };
            return Some((source, base));
        }
        if inner.contains('@') && !inner.contains(' ') && has(base) {
            return Some((Source::Box, base));
        }
    }
    // iCloud: "<name> 2", beside the original.
    if let Some(i) = stem.rfind(' ') {
        let n = &stem[i + 1..];
        if !n.is_empty()
            && n.len() <= 3
            && n.chars().all(|c| c.is_ascii_digit())
            && n != "0"
            && n != "1"
            && i > 0
            && has(&stem[..i])
        {
            return Some((Source::ICloud, &stem[..i]));
        }
    }
    // OneDrive: "<name>-DEVICENAME", beside the original. A device name is
    // how Windows spells a computer: capitals and digits, up to fifteen, so
    // an ordinary title word ("-One") never reads as one.
    for (i, _) in stem.match_indices('-').rev() {
        let device = &stem[i + 1..];
        let looks = (2..=15).contains(&device.len())
            && device
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
            && device.chars().any(|c| c.is_ascii_uppercase())
            && !device.starts_with('-')
            && (device.len() >= 5 || device.chars().any(|c| c.is_ascii_digit()));
        if looks && i > 0 && has(&stem[..i]) {
            return Some((Source::OneDrive, &stem[..i]));
        }
    }
    None
}

/// The shape a parked version has: Grimoire's own "<scene> (from <machine>,
/// <when>).md", or any sync app's conflict copy of a file beside it.
pub fn is_conflict_copy(path: &Path) -> bool {
    copy_of_path(path).is_some()
}

/// Put a parked version away once the writer has settled the conflict: into
/// the book's trash, never deleted. It refuses anything that is not a
/// conflict copy, so a bug in a front end can never ask this to remove a
/// scene. Returns where it went.
pub fn drop_conflict_copy(path: &Path) -> Result<Option<PathBuf>> {
    if !is_conflict_copy(path) {
        anyhow::bail!("{} is not a conflict copy", path.display());
    }
    if !path.exists() {
        return Ok(None);
    }
    let root = path
        .ancestors()
        .skip(1)
        .find(|a| a.join("novel.toml").is_file())
        .with_context(|| format!("{} isn't inside a book", path.display()))?;
    crate::project::trash(root, path).map(Some)
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
