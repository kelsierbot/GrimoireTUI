//! What a front end asks for, in the shapes a front end wants.
//!
//! The terminal app drives `grimoire-core` directly, holding a whole `Project`
//! in memory for as long as the session lasts. A phone can't work that way: it
//! gets backgrounded mid-sentence, and every screen is a separate ask. So this
//! layer turns the book into small, serialisable answers — a shelf of books, an
//! outline, one scene's text — and takes each save on its own, guarded against
//! the copy on another device having moved on ([`grimoire_core::sync`]).
//!
//! Nothing here knows about Tauri, Android, or a window. That keeps it testable
//! on a laptop, which is where these tests run.

use anyhow::{Context, Result};
use grimoire_core::project::{Kind, Project};
use grimoire_core::search;
use grimoire_core::sync::{self, Fingerprint, SaveOutcome};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One book on the shelf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub title: String,
    pub path: PathBuf,
    /// What the shelf shows beside the title, so a book reads as a body of
    /// work rather than a folder name.
    pub words: usize,
    pub scenes: usize,
}

/// A scene as the outline lists it: enough to show a row and open it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub title: String,
    /// "Act One › Chapter One" — where this scene sits in the book.
    pub place: String,
    pub path: PathBuf,
    pub words: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub title: String,
    pub words: usize,
    pub scenes: Vec<Row>,
}

/// A scene opened for editing. `seen` is what the file looked like at that
/// moment; hand it back when saving and nothing written elsewhere can be lost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub path: PathBuf,
    pub title: String,
    pub text: String,
    pub seen: Stamp,
}

/// A [`Fingerprint`] in a form that survives a trip through JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub len: u64,
    pub hash: u64,
    /// False when the scene had no file yet, so an existing file is a surprise.
    pub existed: bool,
}

impl Stamp {
    fn of(fp: Option<Fingerprint>) -> Stamp {
        match fp {
            Some(f) => Stamp { len: f.len, hash: f.hash, existed: true },
            None => Stamp { len: 0, hash: 0, existed: false },
        }
    }

    fn fingerprint(self) -> Option<Fingerprint> {
        self.existed.then_some(Fingerprint { len: self.len, hash: self.hash })
    }
}

/// The answer to a save, in the terms the writer cares about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Saved {
    /// On disk. `seen` replaces the old stamp for the next save.
    Ok { seen: Stamp, words: usize },
    /// Another device wrote this scene first. Nothing was overwritten; both
    /// versions are here, and `kept` is where this device's words were parked
    /// so they cannot be lost while the writer decides.
    Conflict { theirs: String, kept: PathBuf },
}

/// Every book directly under `base` — a folder is a book when it has the
/// `novel.toml` that `Project::scaffold` writes.
pub fn books(base: &Path) -> Vec<Book> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(base) else { return out };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.join("novel.toml").is_file() {
            continue;
        }
        let loaded = Project::load(&dir).ok();
        let title = loaded
            .as_ref()
            .map(|p| p.meta.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let (words, scenes) = match loaded {
            Some(p) => {
                let live: Vec<_> = (0..p.nodes.len())
                    .filter(|&i| p.nodes[i].kind == Kind::Scene && p.nodes[i].in_manuscript && !p.in_trash(i))
                    .collect();
                (live.iter().map(|&i| p.nodes[i].words()).sum(), live.len())
            }
            None => (0, 0),
        };
        out.push(Book { title, path: dir, words, scenes });
    }
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

/// Start a new book under `base`, named for its folder.
pub fn create_book(base: &Path, name: &str) -> Result<Book> {
    let dir = base.join(name);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    grimoire_core::project::scaffold(&dir)?;
    let made = books(base).into_iter().find(|b| b.path == dir);
    Ok(made.unwrap_or(Book { title: name.to_string(), path: dir, words: 0, scenes: 0 }))
}

/// Every manuscript scene in book order, with where it sits and how long it is.
pub fn outline(book: &Path) -> Result<Outline> {
    let p = Project::load(book).with_context(|| format!("opening {}", book.display()))?;
    let parents = p.parents();
    let mut scenes = Vec::new();
    let mut words = 0;
    for (i, n) in p.nodes.iter().enumerate() {
        if n.kind != Kind::Scene || !n.in_manuscript || p.in_trash(i) {
            continue;
        }
        words += n.words();
        scenes.push(Row {
            title: n.title.clone(),
            place: place_above(&p, &parents, i),
            path: n.path.clone(),
            words: n.words(),
        });
    }
    Ok(Outline { title: p.meta.title.clone(), words, scenes })
}

/// The prose count, the way the outline counts it: frontmatter is a scene's
/// notes to itself, not words of the novel. Counting the whole file here would
/// have the editor and the outline disagree about the same scene.
fn body_words(text: &str) -> usize {
    let (_front, body) = grimoire_core::project::split_frontmatter(text);
    body.split_whitespace().count()
}

/// "Act One › Chapter One" — `place_of` includes the scene itself, which the
/// row already shows, so the last step is dropped.
fn place_above(p: &Project, parents: &[Option<usize>], idx: usize) -> String {
    let full = search::place_of(p, parents, idx);
    match full.rsplit_once(" › ") {
        Some((above, _scene)) => above.to_string(),
        None => String::new(),
    }
}

/// Open one scene for editing.
pub fn read_scene(path: &Path) -> Result<Scene> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Ok(Scene { path: path.to_path_buf(), title, text, seen: Stamp::of(Fingerprint::read(path)) })
}

/// Save one scene. If the file changed since it was read, nothing is
/// overwritten: this device's text is written beside it as a conflict copy and
/// both versions come back for the writer to settle.
pub fn save_scene(path: &Path, text: &str, seen: Stamp) -> Result<Saved> {
    match sync::save_guarded(path, text, seen.fingerprint())? {
        SaveOutcome::Written => Ok(Saved::Ok {
            seen: Stamp::of(Fingerprint::read(path)),
            words: body_words(text),
        }),
        SaveOutcome::Conflict { on_disk } => {
            let kept = sync::write_conflict_copy(path, text)?;
            Ok(Saved::Conflict { theirs: on_disk, kept })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-app-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_book_appears_on_the_shelf_with_an_outline() {
        let base = shelf("shelf");
        assert!(books(&base).is_empty(), "an empty folder holds no books");

        let book = create_book(&base, "The Crossing").unwrap();
        let shelved = books(&base);
        assert_eq!(shelved.len(), 1);
        assert_eq!(shelved[0].path, book.path);
        assert_eq!(shelved[0].words, 0, "a new book has no words yet");
        assert!(shelved[0].scenes > 0, "but it does have scenes waiting");

        let outline = outline(&book.path).unwrap();
        assert!(!outline.scenes.is_empty(), "a scaffolded book has scenes to open");
        assert!(outline.scenes.iter().all(|r| r.path.exists()));
        // a fresh book is empty, so every row starts at zero words
        assert_eq!(outline.words, 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_scene_round_trips_and_the_outline_counts_it() {
        let base = shelf("roundtrip");
        let book = create_book(&base, "The Crossing").unwrap();
        let first = outline(&book.path).unwrap().scenes[0].clone();

        let opened = read_scene(&first.path).unwrap();
        let written = format!("{}They crossed at dawn, and the river said nothing.\n", opened.text);
        let saved = save_scene(&first.path, &written, opened.seen).unwrap();
        let Saved::Ok { seen, words } = saved else { panic!("expected a clean save") };
        assert_eq!(words, 9);

        assert_eq!(read_scene(&first.path).unwrap().text, written);
        assert_eq!(outline(&book.path).unwrap().words, 9, "the outline counts what was written");

        // saving again with the stamp the save handed back still works
        assert!(matches!(
            save_scene(&first.path, "Second pass.\n", seen).unwrap(),
            Saved::Ok { .. }
        ));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_phones_words_survive_a_save_from_a_stale_desktop() {
        let base = shelf("conflict");
        let book = create_book(&base, "The Crossing").unwrap();
        let scene = outline(&book.path).unwrap().scenes[0].path.clone();

        // the desktop opened the scene…
        let desktop = read_scene(&scene).unwrap();
        // …the phone saved to the same file…
        fs::write(&scene, "the paragraph written on the sofa\n").unwrap();
        // …and now the desktop saves its own version
        let saved = save_scene(&scene, "the desktop's version\n", desktop.seen).unwrap();

        let Saved::Conflict { theirs, kept } = saved else { panic!("expected a conflict") };
        assert_eq!(theirs, "the paragraph written on the sofa\n");
        assert_eq!(fs::read_to_string(&scene).unwrap(), "the paragraph written on the sofa\n", "the phone's words stay put");
        assert_eq!(fs::read_to_string(&kept).unwrap(), "the desktop's version\n", "the desktop's words are kept, not dropped");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_book_without_novel_toml_is_not_a_book() {
        let base = shelf("notabook");
        fs::create_dir_all(base.join("holiday photos")).unwrap();
        fs::write(base.join("holiday photos/beach.md"), "not a manuscript").unwrap();
        assert!(books(&base).is_empty());
        fs::remove_dir_all(&base).unwrap();
    }
}
