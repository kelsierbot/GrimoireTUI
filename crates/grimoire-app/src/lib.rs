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
    /// A version parked by a guarded save. It is a real scene and stays
    /// visible — nothing is hidden from a writer — but the outline says so,
    /// because two rows with the same name and different words is the one
    /// thing guaranteed to get the wrong one edited. (It got me, testing.)
    pub conflict_copy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub title: String,
    pub words: usize,
    pub scenes: Vec<Row>,
}

/// A scene opened for editing. `text` is the prose alone: a writer on a phone
/// should never be able to tap into the frontmatter and type through their own
/// `title:` line, which is exactly what happened the first time this ran on a
/// device. The frontmatter rides along untouched and is put back on save.
///
/// `seen` is what the whole file looked like when it was read; hand it back
/// when saving and nothing written elsewhere can be lost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub path: PathBuf,
    pub title: String,
    pub text: String,
    /// The scene's own notes — title, pov, status, synopsis — kept aside.
    pub front: Option<String>,
    pub seen: Stamp,
}

/// A [`Fingerprint`] in a form that survives a trip through JSON.
///
/// The hash travels as hex, not as a number. A front end in JavaScript holds
/// every number as a double, so a 64-bit hash would come back subtly changed —
/// and a stamp that no longer matches itself makes the first save of every
/// scene look like a conflict. (It did, on the first Android build.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub len: u64,
    pub hash: String,
    /// False when the scene had no file yet, so an existing file is a surprise.
    pub existed: bool,
}

impl Stamp {
    fn of(fp: Option<Fingerprint>) -> Stamp {
        match fp {
            Some(f) => Stamp { len: f.len, hash: format!("{:016x}", f.hash), existed: true },
            None => Stamp { len: 0, hash: String::new(), existed: false },
        }
    }

    fn fingerprint(&self) -> Option<Fingerprint> {
        if !self.existed {
            return None;
        }
        u64::from_str_radix(&self.hash, 16).ok().map(|hash| Fingerprint { len: self.len, hash })
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
            conflict_copy: is_conflict_copy(&n.path),
        });
    }
    Ok(Outline { title: p.meta.title.clone(), words, scenes })
}

/// The shape `write_conflict_copy` gives a parked version: "<scene> (from
/// <machine>, <when>).md".
fn is_conflict_copy(path: &Path) -> bool {
    path.file_stem()
        .map(|s| s.to_string_lossy().contains(" (from "))
        .unwrap_or(false)
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

/// Open one scene for editing: prose in `text`, frontmatter set aside.
pub fn read_scene(path: &Path) -> Result<Scene> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (front, body) = grimoire_core::project::split_frontmatter(&raw);
    let title = front
        .as_deref()
        .and_then(|f| front_title(f))
        .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
    Ok(Scene { path: path.to_path_buf(), title, text: body, front, seen: Stamp::of(Fingerprint::read(path)) })
}

/// `title: "Scene One"` out of the frontmatter, so the bar shows the scene's
/// name rather than its file name.
fn front_title(front: &str) -> Option<String> {
    front.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "title").then(|| v.trim().trim_matches('"').to_string())
    }).filter(|t| !t.is_empty())
}

/// Prose plus the frontmatter it came with, in the same shape the terminal app
/// writes (see `Node::file_text`), so a scene edited on either one reads
/// identically on the other.
fn whole_file(front: Option<&str>, body: &str) -> String {
    let mut out = String::new();
    if let Some(f) = front {
        out.push_str("---\n");
        out.push_str(f);
        if !f.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---\n\n");
    }
    out.push_str(body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Save one scene. If the file changed since it was read, nothing is
/// overwritten: this device's text is written beside it as a conflict copy and
/// both versions come back for the writer to settle.
pub fn save_scene(path: &Path, text: &str, front: Option<&str>, seen: &Stamp) -> Result<Saved> {
    let whole = whole_file(front, text);
    match sync::save_guarded(path, &whole, seen.fingerprint())? {
        SaveOutcome::Written => Ok(Saved::Ok {
            seen: Stamp::of(Fingerprint::read(path)),
            words: text.split_whitespace().count(),
        }),
        SaveOutcome::Conflict { on_disk } => {
            let kept = sync::write_conflict_copy(path, &whole)?;
            // the writer is comparing two versions of their prose, so show
            // prose: frontmatter in that panel is noise they cannot act on
            let (_front, theirs) = grimoire_core::project::split_frontmatter(&on_disk);
            Ok(Saved::Conflict { theirs, kept })
        }
    }
}

/// Throw away a parked version once the writer has settled the conflict —
/// either because they took this device's words into the scene (so the copy is
/// now a duplicate that would sit in the outline pretending to be a lost
/// draft), or because they chose the other device and said to discard these.
///
/// It refuses anything that is not a conflict copy. A bug in a front end
/// should never be able to ask this function to delete a scene.
pub fn drop_conflict_copy(path: &Path) -> Result<()> {
    if !is_conflict_copy(path) {
        anyhow::bail!("{} is not a conflict copy", path.display());
    }
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// Where the writer stopped, ready for the one button that matters on a
/// shelf: the scene, where it sits in the book, and whose hands were last on
/// it. The phone and the desktop write the same `resume.md`, so picking up a
/// sentence on the sofa needs no thinking about which file that was.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resuming {
    pub book: PathBuf,
    pub book_title: String,
    pub path: PathBuf,
    pub title: String,
    pub place: String,
    pub paragraph: usize,
    pub machine: String,
    /// Already a phrase, not a timestamp: "Thursday 18 September, 11:47 am".
    pub when: String,
}

/// The most recent stopping point across every book on the shelf. `None` when
/// nothing has been written yet, or when the scene it names has since gone —
/// a button that opens nothing is worse than no button.
pub fn resuming(base: &Path) -> Option<Resuming> {
    let mut best: Option<(chrono::DateTime<chrono::Local>, Resuming)> = None;
    for book in books(base) {
        let Some(r) = grimoire_core::resume::read(&book.path) else { continue };
        let scene = book.path.join(&r.scene);
        if !scene.is_file() {
            continue;
        }
        let row = outline(&book.path).ok().and_then(|o| o.scenes.into_iter().find(|s| s.path == scene));
        let here = Resuming {
            book: book.path.clone(),
            book_title: book.title.clone(),
            title: row.as_ref().map(|s| s.title.clone()).unwrap_or_else(|| scene_title(&scene)),
            place: row.map(|s| s.place).unwrap_or_default(),
            path: scene,
            paragraph: r.line + 1,
            machine: r.machine.clone(),
            when: r.when.format("%A %-d %B, %-I:%M %P").to_string(),
        };
        if best.as_ref().is_none_or(|(w, _)| r.when > *w) {
            best = Some((r.when, here));
        }
    }
    best.map(|(_, r)| r)
}

/// Remember this scene as the place to pick up. Called when a scene is saved,
/// so the desktop's shelf offers the paragraph the phone was just in.
pub fn mark_place(book: &Path, scene: &Path, line: usize) -> Result<()> {
    let p = Project::load(book)?;
    let place = outline(book)
        .ok()
        .and_then(|o| o.scenes.into_iter().find(|s| s.path == scene).map(|s| s.place))
        .unwrap_or_default();
    let r = grimoire_core::resume::Resume {
        scene: grimoire_core::resume::relative(book, scene),
        line,
        column: 0,
        machine: grimoire_core::resume::machine_name(),
        when: chrono::Local::now(),
    };
    grimoire_core::resume::write(book, &r, &p.meta.title, &place)
}

/// A scene's name when the outline cannot supply one: its file name, tidied.
fn scene_title(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// Where the books live.
///
/// A phone app that owns one private folder is the easy answer, and it was the
/// right first answer: no permissions, nothing to explain. But a manuscript
/// that only exists inside one app is a manuscript with no way home. So the
/// shelf is a setting — point it at a folder in an Obsidian vault, a pCloud
/// folder, anywhere something else already syncs, and Grimoire will work there
/// instead. Everything above this line already assumes files can change
/// underneath it, which is exactly what a synced folder does.
pub mod shelf {
    use super::*;

    /// Remembered in the front end's config directory, not in the books: a
    /// shelf that pointed at itself would be lost the moment it moved.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Saved {
        root: PathBuf,
    }

    fn settings(config: &Path) -> PathBuf {
        config.join("shelf.json")
    }

    /// The chosen shelf, or `fallback` when the writer has never chosen one —
    /// or when what they chose has since gone (an unplugged drive, a vault
    /// folder deleted on the desktop). A missing shelf must not be a dead app.
    pub fn root(config: &Path, fallback: &Path) -> PathBuf {
        let chosen = fs::read_to_string(settings(config))
            .ok()
            .and_then(|t| serde_json::from_str::<Saved>(&t).ok())
            .map(|s| s.root);
        match chosen {
            Some(dir) if dir.is_dir() => dir,
            _ => fallback.to_path_buf(),
        }
    }

    /// Choose a shelf. The folder is made if it does not exist and checked by
    /// actually writing to it: on Android a path can look perfectly ordinary
    /// and still be refused, and finding that out at the first save would mean
    /// finding it out with a paragraph in hand.
    pub fn choose(config: &Path, dir: &Path) -> Result<PathBuf> {
        fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
        let probe = dir.join(".grimoire-write-test");
        fs::write(&probe, b"grimoire")
            .with_context(|| format!("{} cannot be written to", dir.display()))?;
        let _ = fs::remove_file(&probe);

        fs::create_dir_all(config)?;
        let text = serde_json::to_string_pretty(&Saved { root: dir.to_path_buf() })?;
        grimoire_core::project::write_atomic(&settings(config), &text)?;
        Ok(dir.to_path_buf())
    }

    /// One row in the folder picker.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Folder {
        pub name: String,
        pub path: PathBuf,
        /// Books already sitting in it, so a folder that holds work says so
        /// before it is chosen.
        pub books: usize,
        /// An Obsidian vault. Worth naming out loud: it is the folder most
        /// likely to be the right answer, and the one whose contents another
        /// app is also writing.
        pub vault: bool,
    }

    /// The folders inside `dir`, for browsing. Hidden folders are left out —
    /// they are never where a writer keeps a novel — but they are read for
    /// what they say about their parent.
    pub fn folders(dir: &Path) -> Result<Vec<Folder>> {
        let mut out = Vec::new();
        let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            out.push(describe(&path, name));
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    fn describe(path: &Path, name: String) -> Folder {
        Folder {
            books: books(path).len(),
            vault: path.join(".obsidian").is_dir(),
            name,
            path: path.to_path_buf(),
        }
    }

    /// Somewhere to start browsing: the places a writer's folders actually
    /// live on this device, each under the name they would use for it, and
    /// skipping any that are not there.
    pub fn places(roots: &[(PathBuf, String)]) -> Vec<Folder> {
        roots.iter().filter(|(p, _)| p.is_dir()).map(|(p, name)| describe(p, name.clone())).collect()
    }

    /// Take the books with you when the shelf moves. Anything that is not a
    /// book is left alone: the new folder may be someone else's vault, full of
    /// notes that are none of Grimoire's business.
    pub fn move_books(from: &Path, to: &Path) -> Result<usize> {
        if from == to {
            return Ok(0);
        }
        fs::create_dir_all(to)?;
        let mut moved = 0;
        for book in books(from) {
            let Some(name) = book.path.file_name() else { continue };
            let target = to.join(name);
            if target.exists() {
                continue; // never write over a book already standing there
            }
            match fs::rename(&book.path, &target) {
                Ok(()) => moved += 1,
                // across devices rename fails; a phone's private folder and a
                // vault on shared storage are exactly that case
                Err(_) => {
                    copy_dir(&book.path, &target)?;
                    fs::remove_dir_all(&book.path)?;
                    moved += 1;
                }
            }
        }
        Ok(moved)
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<()> {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)?.flatten() {
            let path = entry.path();
            let target = to.join(entry.file_name());
            if path.is_dir() {
                copy_dir(&path, &target)?;
            } else {
                fs::copy(&path, &target)?;
            }
        }
        Ok(())
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
        assert!(!opened.text.contains("title:"), "the editor is handed prose, never frontmatter");
        let written = format!("{}They crossed at dawn, and the river said nothing.\n", opened.text);
        let saved = save_scene(&first.path, &written, opened.front.as_deref(), &opened.seen).unwrap();
        let Saved::Ok { seen, words } = saved else { panic!("expected a clean save") };
        assert_eq!(words, 9);

        assert_eq!(read_scene(&first.path).unwrap().text, written);
        assert_eq!(outline(&book.path).unwrap().words, 9, "the outline counts what was written");

        // saving again with the stamp the save handed back still works
        assert!(matches!(
            save_scene(&first.path, "Second pass.\n", opened.front.as_deref(), &seen).unwrap(),
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
        let saved = save_scene(&scene, "the desktop's version", desktop.front.as_deref(), &desktop.seen).unwrap();

        let Saved::Conflict { theirs, kept } = saved else { panic!("expected a conflict") };
        assert_eq!(theirs, "the paragraph written on the sofa\n");
        assert!(!theirs.contains("title:"), "the other version is shown as prose, not as a file");
        assert_eq!(fs::read_to_string(&scene).unwrap(), "the paragraph written on the sofa\n", "the phone's words stay put");

        // and the outline says which row is the parked version
        let rows = outline(&book.path).unwrap().scenes;
        let parked: Vec<_> = rows.iter().filter(|r| r.conflict_copy).collect();
        assert_eq!(parked.len(), 1, "the copy shows up, marked");
        assert_eq!(parked[0].path, kept);
        // the copy is a whole scene file, not a loose fragment, so it opens in
        // Grimoire like any other scene
        let copy = fs::read_to_string(&kept).unwrap();
        assert!(copy.contains("the desktop's version"), "the desktop's words are kept, not dropped");
        assert!(copy.starts_with("---\n"), "and it keeps the scene's frontmatter: {copy}");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_stamp_survives_the_trip_through_json_a_front_end_makes() {
        let base = shelf("json");
        let book = create_book(&base, "The Crossing").unwrap();
        let scene = outline(&book.path).unwrap().scenes[0].path.clone();
        let opened = read_scene(&scene).unwrap();

        // exactly what Tauri does between Rust and the web view
        let wire = serde_json::to_string(&opened.seen).unwrap();
        let back: Stamp = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, opened.seen, "the stamp must come back identical");

        // and a save with the round-tripped stamp is a save, not a conflict
        assert!(
            matches!(save_scene(&scene, "First words on the phone.\n", opened.front.as_deref(), &back).unwrap(), Saved::Ok { .. }),
            "a scene opened and saved on one device must never look like a conflict"
        );
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

    #[test]
    fn a_parked_copy_can_be_dropped_but_a_scene_cannot() {
        let base = shelf("drop");
        let dir = base.join("book");
        fs::create_dir_all(&dir).unwrap();
        let scene = dir.join("01-Scene-One.md");
        let copy = dir.join("01-Scene-One (from sofa, 2026-09-18 11-38).md");
        fs::write(&scene, "the real scene\n").unwrap();
        fs::write(&copy, "what the phone had\n").unwrap();

        drop_conflict_copy(&copy).unwrap();
        assert!(!copy.exists(), "the parked copy is gone");

        assert!(drop_conflict_copy(&scene).is_err(), "a scene must never be deletable this way");
        assert!(scene.exists(), "and it is still there");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_shelf_offers_the_newest_stopping_point() {
        let base = shelf("resume");
        let book = create_book(&base, "The Crossing").unwrap();
        let o = outline(&book.path).unwrap();
        let scene = o.scenes[1].path.clone();

        assert!(resuming(&base).is_none(), "nothing written yet, so no button");

        mark_place(&book.path, &scene, 41).unwrap();
        let r = resuming(&base).expect("a place to pick up");
        assert_eq!(r.path, scene);
        assert_eq!(r.book_title, "The Crossing");
        assert_eq!(r.paragraph, 42);
        assert!(!r.place.is_empty(), "the card says where in the book: {:?}", r.place);
        assert!(r.when.contains(','), "a phrase, not a timestamp: {}", r.when);

        // a scene that has since been deleted must not leave a dead button
        fs::remove_file(&scene).unwrap();
        assert!(resuming(&base).is_none());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_shelf_is_remembered_and_falls_back_when_it_is_gone() {
        let base = shelf("settings");
        let config = base.join("config");
        let fallback = base.join("private");
        fs::create_dir_all(&fallback).unwrap();
        let vault = base.join("vault").join("Novels");

        assert_eq!(shelf::root(&config, &fallback), fallback, "never chosen: the private folder");

        shelf::choose(&config, &vault).unwrap();
        assert_eq!(shelf::root(&config, &fallback), vault, "chosen: the vault folder");

        fs::remove_dir_all(base.join("vault")).unwrap();
        assert_eq!(shelf::root(&config, &fallback), fallback, "vault gone: back to the private folder, not a dead app");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_picker_shows_folders_and_says_which_hold_work() {
        let base = shelf("picker");
        fs::create_dir_all(base.join("Vault").join(".obsidian")).unwrap();
        fs::create_dir_all(base.join("Empty")).unwrap();
        fs::write(base.join("not-a-folder.md"), "x").unwrap();
        fs::create_dir_all(base.join(".hidden")).unwrap();
        create_book(&base.join("Vault"), "The Crossing").unwrap();

        let seen = shelf::folders(&base).unwrap();
        let names: Vec<_> = seen.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["Empty", "Vault"], "files and hidden folders are not places to put a book");

        let vault = seen.iter().find(|f| f.name == "Vault").unwrap();
        assert!(vault.vault, "an Obsidian vault is worth naming");
        assert_eq!(vault.books, 1);
        assert_eq!(seen.iter().find(|f| f.name == "Empty").unwrap().books, 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn books_follow_the_shelf_and_nothing_else_does() {
        let base = shelf("move");
        let from = base.join("private");
        let to = base.join("vault");
        fs::create_dir_all(&from).unwrap();
        create_book(&from, "The Crossing").unwrap();
        fs::create_dir_all(to.join("Someone Elses Notes")).unwrap();
        fs::write(to.join("Someone Elses Notes").join("note.md"), "not mine").unwrap();

        assert_eq!(shelf::move_books(&from, &to).unwrap(), 1);
        assert!(to.join("The Crossing").join("novel.toml").is_file(), "the book arrived whole");
        assert!(!from.join("The Crossing").exists(), "and did not stay behind");
        assert_eq!(fs::read_to_string(to.join("Someone Elses Notes").join("note.md")).unwrap(), "not mine");
        assert_eq!(books(&to).len(), 1);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn places_are_named_the_way_a_person_names_them() {
        let base = shelf("places");
        let docs = base.join("Documents");
        fs::create_dir_all(&docs).unwrap();
        let missing = base.join("Nowhere");

        let named = vec![
            (docs.clone(), "Documents".to_string()),
            (base.clone(), "Phone storage".to_string()),
            (missing, "Gone".to_string()),
        ];
        let seen = shelf::places(&named);
        let names: Vec<_> = seen.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["Documents", "Phone storage"], "a folder that is not there is not a place to offer");
        fs::remove_dir_all(&base).unwrap();
    }
}
