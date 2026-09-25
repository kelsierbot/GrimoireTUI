//! Manuscript model: a project is a directory tree of plain Markdown files.
//!
//! Structure on disk is the source of truth — no central index to conflict in git:
//!
//!   novel.toml                     project metadata
//!   Novel-Format.md                how the book is laid out
//!   manuscript/01-Page-One/01-Chapter-One/01-the-archive.md
//!   characters/wren.md
//!   places/  front-matter/  notes/  research/  template-sheets/
//!
//! Each of those is a section of the tree ([`Area`]), in that order, with the
//! trash last. Directories are containers (pages, chapters). `.md` files are
//! scenes, notes and documents.
//! Ordering comes from the filename; a leading `01-` is stripped for display.
//! Scene metadata lives in YAML frontmatter, which we preserve verbatim so
//! Obsidian and anything else can read and write it without us mangling it.

use crate::sync::{self, Fingerprint, SaveOutcome, Stat};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProjectMeta {
    pub title: String,
    pub author: String,
    pub draft: String,
    pub target_words: usize,
    pub daily_target: usize,
    /// What this book calls its largest division: "Part", or "Act", or "Book".
    /// Names new ones, and the app says it back to you everywhere.
    pub part_label: String,
    /// The order of the tree's sections, by [`Area::key`], when it's been
    /// rearranged. Missing ones follow in the usual order; the trash is last.
    pub sections: Vec<String>,
    /// The title page's contact block (`[contact]`).
    #[serde(deserialize_with = "crate::submission::lenient_contact")]
    pub contact: crate::submission::Contact,
    /// How the submission manuscript looks (`[manuscript]`).
    #[serde(deserialize_with = "crate::submission::lenient_manuscript")]
    pub manuscript: crate::submission::Manuscript,
    /// `[ebook]`: what the EPUB says about itself.
    #[serde(deserialize_with = "lenient_table")]
    pub ebook: EbookMeta,
    /// `[paperback]`: how the print edition is laid out.
    #[serde(deserialize_with = "lenient_table")]
    pub paperback: PaperbackMeta,
}

/// A table that can't be read as its type (a typo, a number where text
/// belongs) is taken as its defaults — never a reason to lose the rest of
/// novel.toml, title and all.
fn lenient_table<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    Ok(toml::Value::deserialize(d)
        .ok()
        .and_then(|v| v.try_into().ok())
        .unwrap_or_default())
}

/// `[ebook]` in novel.toml. Every field is optional; an EPUB is valid
/// without any of them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EbookMeta {
    /// BCP 47, e.g. `en`, `en-GB`, `fr`. Empty means `en`.
    pub language: String,
    pub publisher: String,
    /// With or without hyphens; carried as a second identifier.
    pub isbn: String,
    /// The blurb readers see in their library.
    pub description: String,
    pub subjects: Vec<String>,
    pub series: String,
    /// This book's place in `series`.
    pub series_number: Option<f64>,
    /// e.g. "All rights reserved."
    pub rights: String,
    /// Publication date, `YYYY-MM-DD` (or just the year).
    pub published: String,
    /// A cover image, relative to the book. Empty means look for
    /// `cover.jpg` / `cover.png` in the book or its Ebook front matter.
    pub cover: String,
}

impl EbookMeta {
    pub fn language(&self) -> &str {
        let l = self.language.trim();
        if l.is_empty() { "en" } else { l }
    }
}

/// `[paperback]` in novel.toml.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PaperbackMeta {
    /// `5x8`, `5.25x8`, `5.5x8.5` or `6x9` (inches).
    pub trim: String,
    /// The text face. Word and LibreOffice substitute when it's missing.
    pub font: String,
    /// Point size of the text.
    pub size: f64,
    /// What sits centred between scenes.
    pub ornament: String,
    /// A two-line drop cap on each chapter's first letter; otherwise its
    /// first words in small capitals.
    pub drop_cap: bool,
}

impl Default for PaperbackMeta {
    fn default() -> Self {
        Self {
            trim: "6x9".into(),
            font: "Garamond".into(),
            size: 11.0,
            ornament: "* * *".into(),
            drop_cap: false,
        }
    }
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            author: String::new(),
            draft: String::new(),
            target_words: 80_000,
            daily_target: 1_000,
            part_label: "Part".into(),
            sections: Vec::new(),
            contact: crate::submission::Contact::default(),
            manuscript: crate::submission::Manuscript::default(),
            ebook: EbookMeta::default(),
            paperback: PaperbackMeta::default(),
        }
    }
}

impl ProjectMeta {
    /// "Act" — capitalised, for naming a new one.
    pub fn part_word(&self) -> &str {
        let w = self.part_label.trim();
        if w.is_empty() { "Part" } else { w }
    }

    /// "act" — for a sentence.
    pub fn part_noun(&self) -> String {
        self.part_word().to_lowercase()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Container,
    Scene,
    /// A section of the book — Manuscript, Characters, Trash. It folds like a
    /// folder, but it can't be renamed, moved or deleted.
    Category,
}

/// The sections of a book, top to bottom, the way the tree lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// `Novel-Format.md`: one document, not a folder.
    Format,
    Manuscript,
    Characters,
    Places,
    FrontMatter,
    /// Pages after the story: about the author, also by, acknowledgements.
    BackMatter,
    Notes,
    Research,
    Templates,
    Trash,
}

impl Area {
    pub const ALL: [Area; 10] = [
        Area::Format,
        Area::Manuscript,
        Area::Characters,
        Area::Places,
        Area::FrontMatter,
        Area::BackMatter,
        Area::Notes,
        Area::Research,
        Area::Templates,
        Area::Trash,
    ];

    /// How novel.toml names it in `sections = [...]`.
    pub fn key(self) -> &'static str {
        match self {
            Area::Format => "format",
            Area::Manuscript => "manuscript",
            Area::Characters => "characters",
            Area::Places => "places",
            Area::FrontMatter => "front-matter",
            Area::BackMatter => "back-matter",
            Area::Notes => "notes",
            Area::Research => "research",
            Area::Templates => "template-sheets",
            Area::Trash => "trash",
        }
    }

    /// The sections in this book's order: as rearranged, then any not
    /// mentioned, and the trash always last.
    pub fn ordered(meta: &ProjectMeta) -> Vec<Area> {
        let mut out: Vec<Area> = Vec::new();
        for k in &meta.sections {
            if let Some(a) = Area::ALL.iter().copied().find(|a| a.key() == k.trim())
                && a != Area::Trash
                && !out.contains(&a)
            {
                out.push(a);
            }
        }
        for a in Area::ALL {
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out.retain(|&a| a != Area::Trash);
        out.push(Area::Trash);
        out
    }

    pub fn title(self) -> &'static str {
        match self {
            Area::Format => "Novel Format",
            Area::Manuscript => "Manuscript",
            Area::Characters => "Characters",
            Area::Places => "Places",
            Area::FrontMatter => "Front Matter",
            Area::BackMatter => "Back Matter",
            Area::Notes => "Notes",
            Area::Research => "Research",
            Area::Templates => "Template Sheets",
            Area::Trash => "Trash",
        }
    }

    /// The tree's symbol for it. Plain symbols, so every terminal font has them.
    pub fn icon(self) -> &'static str {
        match self {
            Area::Format => "ⓘ",
            Area::Manuscript => "▤",
            Area::Characters => "☺",
            Area::Places => "⌖",
            Area::FrontMatter => "❡",
            Area::BackMatter => "❧",
            Area::Notes => "≡",
            Area::Research => "✎",
            Area::Templates => "⊞",
            Area::Trash => "⌫",
        }
    }

    /// Where it lives on disk.
    pub fn path(self, root: &Path) -> PathBuf {
        match self {
            Area::Format => root.join("Novel-Format.md"),
            Area::Manuscript => root.join("manuscript"),
            Area::Characters => root.join("characters"),
            Area::Places => root.join("places"),
            Area::FrontMatter => root.join("front-matter"),
            Area::BackMatter => root.join("back-matter"),
            Area::Notes => root.join("notes"),
            Area::Research => root.join("research"),
            Area::Templates => root.join("template-sheets"),
            Area::Trash => trash_dir(root),
        }
    }

    /// The notebook: where the codex and the spellchecker learn the book's
    /// names. Template sheets aren't in it, or "Character Sketch" would be a
    /// character.
    pub fn is_notebook(self) -> bool {
        matches!(
            self,
            Area::Characters | Area::Places | Area::Notes | Area::Research
        )
    }
}

/// Why a file can't be read right now: a word for the tree, a sentence for
/// the status bar. Such a file is shown and kept, and never written until a
/// read succeeds again.
#[derive(Debug, Clone, PartialEq)]
pub struct Unavailable {
    pub tag: &'static str,
    pub why: String,
}

impl Unavailable {
    /// A read that failed: an online-only file with no connection (Dropbox,
    /// Google Drive, Box, OneDrive, pCloud Drive), a download that failed, a
    /// permission problem.
    pub fn from_io(e: &std::io::Error) -> Unavailable {
        let reason = e.to_string();
        let reason = match reason.find(" (os error") {
            Some(i) => reason[..i].to_string(),
            None => reason,
        };
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Unavailable {
                tag: "can't read",
                why: format!("it can't be read ({reason})"),
            }
        } else {
            Unavailable {
                tag: "offline",
                why: format!("it can't be read right now — not downloaded, or offline? ({reason})"),
            }
        }
    }

    /// iCloud's older placeholder: the file is replaced by a hidden
    /// `.Name.md.icloud` until it downloads.
    pub fn icloud() -> Unavailable {
        Unavailable {
            tag: "in iCloud",
            why: "it's in iCloud and hasn't downloaded yet".into(),
        }
    }

    /// Empty on disk where there were words: an online-only placeholder
    /// showing its size as nothing. Never taken for a scene cut to nothing.
    pub fn empty() -> Unavailable {
        Unavailable {
            tag: "downloading",
            why: "it's empty on disk — waiting for it to download".into(),
        }
    }
}

/// What the disk checks have seen of a scene between one look and the next.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiskState {
    /// Set while the file can't be read. The words last read stay in memory;
    /// the file is never written until a read works again.
    pub unavailable: Option<Unavailable>,
    /// Checks in a row that found the file missing. Once is a sync client
    /// replacing it (or a folder blinking out and back); twice is gone.
    pub missing: u8,
    /// A change seen on disk but not taken in yet, by its size and time. A
    /// second look that finds it the same takes it in; a client still writing
    /// the file would have moved on.
    pub pending: Option<Stat>,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    /// Which section of the book it sits in.
    pub area: Area,
    pub title: String,
    pub path: PathBuf,
    pub depth: usize,
    pub expanded: bool,
    pub children: Vec<usize>,
    pub in_manuscript: bool,
    /// Scrivener's "include in compile". `compile: false` in frontmatter keeps
    /// a scene in the tree but out of the finished manuscript.
    pub compile: bool,
    /// Lives under front-matter/ — compiled first, never counted in the draft.
    pub front_matter: bool,
    /// Raw frontmatter block, without the `---` fences. Round-tripped untouched.
    pub front: Option<String>,
    pub body: String,
    pub dirty: bool,
    pub pov: Option<String>,
    pub status: Option<String>,
    /// What the file on disk looked like when it was last read or written.
    /// A save only goes ahead while the disk still matches, so a change made
    /// by Obsidian, a sync client or the phone is never overwritten.
    pub seen: Option<Fingerprint>,
    /// Size and time at that moment: the cheap check before hashing.
    pub stat: Option<Stat>,
    /// The file isn't UTF-8 text. It is shown as read (a best guess), and
    /// never written — saving it would destroy the bytes that couldn't be read.
    pub read_only: bool,
    /// A version parked beside its scene after a clash — Grimoire's own
    /// ("… (from bazzite, …).md") or a sync app's conflicted copy. Shown,
    /// never compiled or counted.
    pub parked: bool,
    /// Another item in the same folder has this name but for capitals or
    /// accents (`wren.md` beside `Wren.md`). Linux keeps both; Dropbox, Box,
    /// Macs and Windows see one file. Shown so the writer can rename one —
    /// never renamed behind their back.
    pub clash: bool,
    /// Whose copy it is and of what, when `parked`.
    pub copy_of: Option<sync::CopyOf>,
    /// Between disk checks: unreadable, missing, or changing. See [`DiskState`].
    pub disk: DiskState,
}

impl Node {
    /// Take the file as it is on disk now: text, frontmatter and the fields
    /// read from it, and what it looked like. A file that isn't UTF-8 is read
    /// with the undecodable bytes replaced, and marked read-only so it is
    /// never written back — one odd file must not keep the book from opening.
    pub fn read_disk(&mut self) -> Result<()> {
        let stat = Stat::of(&self.path);
        let bytes =
            fs::read(&self.path).with_context(|| format!("reading {}", self.path.display()))?;
        self.take_bytes(bytes, stat);
        Ok(())
    }

    /// The file's `bytes`, just read, become the scene. `stat` is what it
    /// looked like just before; it is remembered only once it's old enough to
    /// be trusted ([`sync::settled`]).
    fn take_bytes(&mut self, bytes: Vec<u8>, stat: Option<Stat>) {
        let seen = Fingerprint::of_bytes(&bytes);
        let (raw, read_only) = match String::from_utf8(bytes) {
            Ok(text) => (text, false),
            Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
        };
        self.seen = Some(seen);
        self.stat = sync::settled(stat);
        self.read_only = read_only;
        self.disk = DiskState::default();
        self.adopt(&raw);
    }

    /// Unreadable since the book opened: no words in memory, nothing to show
    /// or count until it downloads.
    pub fn never_read(&self) -> bool {
        self.kind == Kind::Scene && self.seen.is_none() && self.disk.unavailable.is_some()
    }

    /// Replace this scene's text with `raw` (a whole file), as if just read.
    fn adopt(&mut self, raw: &str) {
        let (front, body) = split_frontmatter(raw);
        self.pov = front.as_deref().and_then(|f| front_get(f, "pov"));
        self.status = front.as_deref().and_then(|f| front_get(f, "status"));
        // A parked copy is there to be read and merged by hand, never
        // compiled into the book beside the scene it came from.
        self.compile = !self.parked
            && front
                .as_deref()
                .and_then(|f| front_get(f, "compile"))
                .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "no" | "0"))
                .unwrap_or(true);
        self.title = display_title(&self.path, front.as_deref());
        self.front = front;
        self.body = body;
        self.dirty = false;
    }

    /// After writing `text` ourselves: remember it as what the disk holds.
    /// The stat is kept only if the file still hashes to what was written, so
    /// a change landing in between is still noticed on the next look.
    fn wrote(&mut self, text: &str) {
        let fp = Fingerprint::of(text);
        self.seen = Some(fp);
        self.stat = if Fingerprint::read(&self.path) == Some(fp) {
            sync::settled(Stat::of(&self.path))
        } else {
            None
        };
        self.disk = DiskState::default();
        self.dirty = false;
    }

    /// Words in the prose — not counting `%% notes %%` or TKs, which never
    /// reach the book.
    pub fn words(&self) -> usize {
        crate::notes::count_words(&self.body)
    }

    /// A value from the scene's frontmatter, if it has one.
    pub fn meta(&self, key: &str) -> Option<String> {
        self.front.as_deref().and_then(|f| front_get(f, key))
    }

    /// Change one frontmatter value, leaving every other line of the block
    /// exactly as it was, and keep the fields read from it in step.
    pub fn set_meta(&mut self, key: &str, value: &str) {
        self.front = Some(set_front(self.front.as_deref(), key, value));
        let v = (!value.trim().is_empty()).then(|| value.trim().to_string());
        match key {
            "pov" => self.pov = v,
            "status" => self.status = v,
            _ => {}
        }
    }

    /// The scene as it belongs on disk: its frontmatter block, untouched, then
    /// the prose.
    pub fn file_text(&self) -> String {
        let mut out = String::new();
        if let Some(f) = &self.front {
            out.push_str("---\n");
            out.push_str(f);
            if !f.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("---\n\n");
        }
        out.push_str(&self.body);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }
}

/// What a save managed: the scenes written, and the ones that couldn't be
/// with the reason. Every dirty scene is attempted even if one fails.
#[derive(Debug, Default)]
pub struct SaveReport {
    pub saved: Vec<usize>,
    pub failed: Vec<(usize, String)>,
    /// Changed on disk by something else since it was read. The disk version
    /// is now the scene (in memory too); this session's words are parked at
    /// the path beside it.
    pub parked: Vec<(usize, PathBuf)>,
    /// Gone from disk (deleted or moved elsewhere). Its unsaved words are in
    /// the trash at the path given; the row is stale until the tree reloads.
    pub gone: Vec<(usize, PathBuf)>,
    /// Not written yet, and not failed: the file can't be read right now, or
    /// went missing for a moment. The words are still unsaved in memory; keep
    /// them in recovery and try again later.
    pub waiting: Vec<usize>,
}

/// What [`Project::check_disk`] found for one scene.
#[derive(Debug, Clone, PartialEq)]
pub enum DiskChange {
    Same,
    /// Changed elsewhere, nothing unsaved here: the new version is in memory.
    Adopted,
    /// Changed elsewhere while this session had unsaved words: the new
    /// version is in memory, and those words were parked at this path.
    Parked(PathBuf),
    /// Deleted or moved elsewhere, nothing unsaved here.
    Gone,
    /// Deleted or moved elsewhere with unsaved words, now in the trash here.
    GoneParked(PathBuf),
    /// The file can't be read right now (why): what's in memory stays, and
    /// nothing is written until it can be.
    Unavailable(Unavailable),
    /// Readable again — or for the first time, a scene that opened
    /// unreadable — and its words are in memory now.
    Available,
}

pub struct Project {
    pub root: PathBuf,
    pub meta: ProjectMeta,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    /// `novel.toml` couldn't be read or understood, so the book opened with
    /// the defaults (it is never rewritten from them): why, for the status bar.
    pub meta_unreadable: Option<String>,
}

impl Project {
    pub fn load(root: &Path) -> Result<Project> {
        // A novel.toml that can't be read (not downloaded yet) or parsed (a
        // sync caught halfway) opens the book on the defaults rather than not
        // at all. Nothing rewrites novel.toml from them: everything that
        // writes it reads it first.
        let meta_path = root.join("novel.toml");
        let (meta, meta_unreadable) = match fs::read_to_string(&meta_path) {
            Ok(s) => match toml::from_str::<ProjectMeta>(&s) {
                Ok(m) => (m, None),
                Err(e) => (
                    ProjectMeta::default(),
                    Some(format!(
                        "novel.toml didn't read as settings ({})",
                        e.message()
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (ProjectMeta::default(), None),
            Err(e) => (
                ProjectMeta::default(),
                Some(format!(
                    "novel.toml {}",
                    Unavailable::from_io(&e).why.trim_start_matches("it ")
                )),
            ),
        };

        let mut p = Project {
            root: root.to_path_buf(),
            meta,
            nodes: Vec::new(),
            roots: Vec::new(),
            meta_unreadable,
        };

        // Every section, in order, as a row that folds. The front matter sits
        // beside the draft, not inside it — Scrivener's arrangement, and the
        // reason it compiles without inflating the wordcount. The trash is last,
        // and folded: what's been deleted is still on screen, so nothing ever
        // simply disappears, but it counts for nothing and compiles into nothing.
        for area in Area::ordered(&p.meta) {
            let path = area.path(root);
            if area == Area::Format {
                if path.is_file() {
                    let idx = p.load_file(&path, 0, area, None);
                    p.roots.push(idx);
                }
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            let idx = p.push(Node {
                kind: Kind::Category,
                area,
                title: area.title().into(),
                path: path.clone(),
                depth: 0,
                expanded: area != Area::Trash,
                children: Vec::new(),
                in_manuscript: area == Area::Manuscript,
                compile: true,
                front_matter: area == Area::FrontMatter,
                front: None,
                body: String::new(),
                dirty: false,
                pov: None,
                status: None,
                seen: None,
                stat: None,
                read_only: false,
                parked: false,
                clash: false,
                copy_of: None,
                disk: DiskState::default(),
            });
            let kids = p.scan(idx, &path, 1, area);
            p.nodes[idx].children = kids;
            p.roots.push(idx);
        }
        p.mark_clashes();

        Ok(p)
    }

    /// Flag items whose names differ from a sibling's only by capitals or
    /// accents (see [`Node::clash`]).
    fn mark_clashes(&mut self) {
        let mut groups: Vec<Vec<usize>> = vec![self.roots.clone()];
        groups.extend(self.nodes.iter().map(|n| n.children.clone()));
        for siblings in groups {
            let folded: Vec<String> = siblings
                .iter()
                .map(|&i| {
                    let name = self.nodes[i].path.file_name().unwrap_or_default();
                    crate::names::fold(&name.to_string_lossy())
                })
                .collect();
            for (a, &i) in siblings.iter().enumerate() {
                if folded
                    .iter()
                    .enumerate()
                    .any(|(b, f)| b != a && *f == folded[a])
                {
                    self.nodes[i].clash = true;
                }
            }
        }
    }

    /// Is this row already in the trash? Then deleting it means for good.
    pub fn in_trash(&self, idx: usize) -> bool {
        self.nodes[idx].path.starts_with(trash_dir(&self.root))
    }

    /// The manuscript's top level — its pages, or its chapters in a book
    /// without pages — in order.
    pub fn manuscript(&self) -> Vec<usize> {
        self.roots
            .iter()
            .find(|&&r| {
                self.nodes[r].kind == Kind::Category && self.nodes[r].area == Area::Manuscript
            })
            .map(|&r| self.nodes[r].children.clone())
            .unwrap_or_default()
    }

    fn push(&mut self, n: Node) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    /// The folder `dir` (node `at`) and everything under it. A folder or file
    /// that can't be read becomes a row that says so rather than stopping the
    /// book opening: an online-only file with no connection is a normal thing
    /// to have in a synced book.
    fn scan(&mut self, at: usize, dir: &Path, depth: usize, area: Area) -> Vec<usize> {
        let rd = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                self.nodes[at].disk.unavailable = Some(Unavailable::from_io(&e));
                return Vec::new();
            }
        };
        // Sorted by the name each entry stands for: an iCloud stub
        // `.01-Scene.md.icloud` takes the place of `01-Scene.md`.
        let mut entries: Vec<(std::ffi::OsString, PathBuf, bool)> = Vec::new();
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name();
            let path = e.path();
            if let Some(real) = icloud_stub_for(&path) {
                if !real.exists() {
                    let key = real.file_name().unwrap_or_default().to_os_string();
                    entries.push((key, real, true));
                }
                continue;
            }
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            entries.push((name, path, false));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        // Conflict copies are told from the files beside them: "Part (1)" is
        // a copy only where "Part" is there too.
        let stems: std::collections::HashSet<String> = entries
            .iter()
            .map(|(_, p, _)| p)
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .collect();
        let hint = sync::provider_of(dir);

        let mut out = Vec::new();
        for (_, path, stub) in entries {
            if stub {
                out.push(self.placeholder(&path, depth, area, Unavailable::icloud(), None));
            } else if path.is_dir() {
                let idx = self.push(Node {
                    kind: Kind::Container,
                    area,
                    title: display_title(&path, None),
                    path: path.clone(),
                    depth,
                    // A novel tree is small. Start it open; collapsing is a
                    // deliberate act, not something to make the user undo. The
                    // front matter's per-format folders are the exception: they
                    // matter at the end, not while writing.
                    expanded: !(matches!(area, Area::FrontMatter | Area::BackMatter) && depth == 1),
                    children: Vec::new(),
                    in_manuscript: area == Area::Manuscript,
                    compile: true,
                    front_matter: area == Area::FrontMatter,
                    front: None,
                    body: String::new(),
                    dirty: false,
                    pov: None,
                    status: None,
                    seen: None,
                    stat: None,
                    read_only: false,
                    parked: false,
                    clash: false,
                    copy_of: None,
                    disk: DiskState::default(),
                });
                let kids = self.scan(idx, &path, depth + 1, area);
                self.nodes[idx].children = kids;
                out.push(idx);
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let copy = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .and_then(|stem| sync::copy_of(&stem, &|x| stems.contains(x), hint));
                out.push(self.load_file(&path, depth, area, copy));
            }
        }
        self.copies_after_originals(out)
    }

    /// Each conflict copy right after the scene it's a copy of, however its
    /// name happens to sort ("Scene (1)" sorts before "Scene.md"), so the two
    /// versions are always read side by side.
    fn copies_after_originals(&self, out: Vec<usize>) -> Vec<usize> {
        let stem = |i: usize| {
            self.nodes[i]
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        let original_of = |i: usize| {
            self.nodes[i]
                .copy_of
                .as_ref()
                .map(|c| c.original.clone())
                .filter(|o| {
                    out.iter()
                        .any(|&j| self.nodes[j].copy_of.is_none() && stem(j) == *o)
                })
        };
        let mut ordered = Vec::with_capacity(out.len());
        for &i in &out {
            if original_of(i).is_some() {
                continue;
            }
            ordered.push(i);
            if self.nodes[i].kind == Kind::Scene && self.nodes[i].copy_of.is_none() {
                let me = stem(i);
                ordered.extend(
                    out.iter()
                        .copied()
                        .filter(|&j| original_of(j).as_deref() == Some(me.as_str())),
                );
            }
        }
        ordered
    }

    /// A scene read from disk — or, when it can't be read, a row that stands
    /// in for it: titled from its filename, read-only, never written, never
    /// counted, and read properly once it can be.
    fn load_file(
        &mut self,
        path: &Path,
        depth: usize,
        area: Area,
        copy_of: Option<sync::CopyOf>,
    ) -> usize {
        let mut node = Node {
            kind: Kind::Scene,
            area,
            title: String::new(),
            path: path.to_path_buf(),
            depth,
            expanded: false,
            children: Vec::new(),
            in_manuscript: area == Area::Manuscript,
            compile: true,
            front_matter: area == Area::FrontMatter,
            front: None,
            body: String::new(),
            dirty: false,
            pov: None,
            status: None,
            seen: None,
            stat: None,
            read_only: false,
            parked: copy_of.is_some(),
            copy_of,
            clash: false,
            disk: DiskState::default(),
        };
        let stat = Stat::of(path);
        match sync::probe(path) {
            sync::Probe::Bytes(bytes) => {
                node.take_bytes(bytes, stat);
                self.push(node)
            }
            sync::Probe::Unreadable(e) => {
                self.placeholder(path, depth, area, Unavailable::from_io(&e), node.copy_of)
            }
            // Gone between listing and reading: a sync client's delete. The
            // next look at the tree takes it out.
            sync::Probe::Missing => {
                let gone = std::io::Error::from(std::io::ErrorKind::NotFound);
                self.placeholder(path, depth, area, Unavailable::from_io(&gone), node.copy_of)
            }
        }
    }

    fn placeholder(
        &mut self,
        path: &Path,
        depth: usize,
        area: Area,
        why: Unavailable,
        copy_of: Option<sync::CopyOf>,
    ) -> usize {
        self.push(Node {
            kind: Kind::Scene,
            area,
            title: display_title(path, None),
            path: path.to_path_buf(),
            depth,
            expanded: false,
            children: Vec::new(),
            in_manuscript: area == Area::Manuscript,
            compile: true,
            front_matter: area == Area::FrontMatter,
            front: None,
            body: String::new(),
            dirty: false,
            pov: None,
            status: None,
            seen: None,
            stat: None,
            read_only: true,
            parked: copy_of.is_some(),
            copy_of,
            clash: false,
            disk: DiskState {
                unavailable: Some(why),
                ..DiskState::default()
            },
        })
    }

    /// Each node's parent, for walking up the tree.
    pub fn parents(&self) -> Vec<Option<usize>> {
        let mut out = vec![None; self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            for &c in &n.children {
                out[c] = Some(i);
            }
        }
        out
    }

    /// Words in this node, summing descendants for containers.
    pub fn subtree_words(&self, i: usize) -> usize {
        let n = &self.nodes[i];
        match n.kind {
            Kind::Scene if n.parked => 0,
            Kind::Scene => n.words(),
            _ => n.children.iter().map(|&c| self.subtree_words(c)).sum(),
        }
    }

    pub fn total_words(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.kind == Kind::Scene && n.in_manuscript && !n.parked)
            .map(|n| n.words())
            .sum()
    }

    /// Flattened list of visible rows, honouring collapse state.
    pub fn visible(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for &r in &self.roots {
            self.walk(r, &mut out);
        }
        out
    }

    fn walk(&self, i: usize, out: &mut Vec<usize>) {
        out.push(i);
        let n = &self.nodes[i];
        if n.expanded {
            for &c in &n.children {
                self.walk(c, out);
            }
        }
    }

    /// Write every changed scene, each through a temporary file so a crash
    /// mid-save can never leave half a scene behind — and only over the
    /// version this session last read. A scene changed on disk since then
    /// keeps the disk's version and this session's words are parked beside
    /// it; one that vanished has them parked in the trash. A scene whose file
    /// can't be read right now, or is missing for just a moment, waits: its
    /// words stay unsaved (and the caller keeps them in recovery) rather than
    /// be written over something that can't be checked. Nothing written
    /// elsewhere is ever overwritten, and nothing typed here is ever dropped.
    pub fn save_dirty(&mut self) -> SaveReport {
        let mut report = SaveReport::default();
        for i in 0..self.nodes.len() {
            if !(self.nodes[i].dirty && self.nodes[i].kind == Kind::Scene) {
                continue;
            }
            if self.nodes[i].read_only {
                // Never written; the app refuses edits to it in the first place.
                self.nodes[i].dirty = false;
                continue;
            }
            if self.nodes[i].disk.unavailable.is_some() {
                report.waiting.push(i);
                continue;
            }
            let text = self.nodes[i].file_text();
            let path = self.nodes[i].path.clone();
            // Missing only since the last look: a sync client replacing it, or
            // a folder blinking out. The disk check decides, on a second look.
            if self.nodes[i].seen.is_some() && Stat::of(&path).is_none() {
                if icloud_stub(&path).exists() {
                    self.nodes[i].disk.unavailable = Some(Unavailable::icloud());
                    report.waiting.push(i);
                    continue;
                }
                if self.nodes[i].disk.missing < 2 {
                    report.waiting.push(i);
                    continue;
                }
            }
            if let sync::Probe::Unreadable(e) = sync::probe(&path) {
                self.nodes[i].disk.unavailable = Some(Unavailable::from_io(&e));
                report.waiting.push(i);
                continue;
            }
            match sync::save_guarded(&path, &text, self.nodes[i].seen) {
                Ok(SaveOutcome::Written) => {
                    self.nodes[i].wrote(&text);
                    report.saved.push(i);
                }
                Ok(SaveOutcome::Conflict { .. }) => match self.park_unsaved(i, &text) {
                    Ok(DiskChange::GoneParked(copy)) => report.gone.push((i, copy)),
                    Ok(DiskChange::Parked(copy)) => report.parked.push((i, copy)),
                    Ok(_) => report.saved.push(i),
                    Err(e) => report.failed.push((i, short_reason(&e))),
                },
                // The reason, not the path: "Permission denied", "No space left".
                Err(e) => report.failed.push((i, short_reason(&e))),
            }
        }
        report
    }

    /// Scene `i` has unsaved `text` and the disk has moved on: keep both.
    /// Once the copy is written the words are safe, so the scene is no longer
    /// unsaved even if the new version can't be read yet — otherwise every
    /// later look would park the same words again.
    fn park_unsaved(&mut self, i: usize, text: &str) -> Result<DiskChange> {
        let path = self.nodes[i].path.clone();
        if Stat::of(&path).is_none() {
            let copy = sync::park_in_trash(&self.root, &path, text)?;
            self.nodes[i].dirty = false;
            return Ok(DiskChange::GoneParked(copy));
        }
        let copy = sync::write_conflict_copy(&path, text)?;
        // The words are safe in the copy now: whatever reading the disk's
        // version finds, this scene has nothing unsaved left — or the next
        // check (every two seconds) would park the same words again.
        let stat = Stat::of(&path);
        match sync::probe(&path) {
            sync::Probe::Bytes(bytes) => self.nodes[i].take_bytes(bytes, stat),
            sync::Probe::Unreadable(e) => {
                let n = &mut self.nodes[i];
                n.dirty = false;
                n.stat = None;
                n.disk.unavailable = Some(Unavailable::from_io(&e));
            }
            sync::Probe::Missing => {
                let n = &mut self.nodes[i];
                n.dirty = false;
                n.stat = None;
            }
        }
        Ok(DiskChange::Parked(copy))
    }

    /// Has scene `i` changed on disk since it was read or saved here? The
    /// size and time are checked first; the bytes are hashed only when those
    /// moved, or when the time was too recent to trust ([`sync::settled`]), or
    /// always with `force` (opening a scene). A change is taken in only once a
    /// second look finds it settled — a sync client may still be writing it —
    /// and if this session had unsaved words in the scene, those are parked
    /// beside it first. A file missing once is given a second look before
    /// it's taken as deleted, a file that can't be read is left as it was
    /// (see [`Unavailable`]), and an empty file where there were words is an
    /// online-only placeholder, not a scene cut to nothing. Call with the
    /// editor's text already flushed into the node.
    pub fn check_disk(&mut self, i: usize, force: bool) -> Result<DiskChange> {
        if self.nodes[i].kind != Kind::Scene {
            return Ok(DiskChange::Same);
        }
        let path = self.nodes[i].path.clone();
        let Some(stat) = Stat::of(&path) else {
            return self.missing_on_disk(i);
        };
        self.nodes[i].disk.missing = 0;
        let n = &self.nodes[i];
        if !force && n.disk.unavailable.is_none() && n.stat == Some(stat) {
            return Ok(DiskChange::Same);
        }
        let bytes = match sync::probe(&path) {
            sync::Probe::Bytes(b) => b,
            sync::Probe::Missing => return self.missing_on_disk(i),
            sync::Probe::Unreadable(e) => {
                return Ok(self.mark_unavailable(i, Unavailable::from_io(&e)));
            }
        };
        let n = &self.nodes[i];
        let had_words = n.seen.is_some_and(|s| s.len > 0) || !n.body.trim().is_empty();
        if bytes.is_empty() && had_words {
            return Ok(self.mark_unavailable(i, Unavailable::empty()));
        }
        let fp = Fingerprint::of_bytes(&bytes);
        if n.seen == Some(fp) {
            let n = &mut self.nodes[i];
            let back = n.disk.unavailable.take().is_some();
            n.disk.pending = None;
            n.stat = sync::settled(Some(stat));
            return Ok(if back {
                DiskChange::Available
            } else {
                DiskChange::Same
            });
        }
        // Changed elsewhere. Wait for it to settle unless asked not to.
        if !force && self.nodes[i].disk.pending != Some(stat) {
            self.nodes[i].disk.pending = Some(stat);
            return Ok(DiskChange::Same);
        }
        let n = &self.nodes[i];
        let first_read = n.seen.is_none();
        let was_unavailable = n.disk.unavailable.is_some();
        if n.dirty && !n.read_only {
            let text = n.file_text();
            let copy = sync::write_conflict_copy(&path, &text)?;
            self.nodes[i].take_bytes(bytes, Some(stat));
            return Ok(DiskChange::Parked(copy));
        }
        self.nodes[i].take_bytes(bytes, Some(stat));
        Ok(if first_read || was_unavailable {
            DiskChange::Available
        } else {
            DiskChange::Adopted
        })
    }

    /// Scene `i`'s file wasn't there just now. iCloud's stub standing in for
    /// it means not downloaded, not deleted; otherwise a first miss is given
    /// the benefit of the doubt (a client replacing the file, a folder
    /// renamed away and back), and only a second, confirmed by a fresh look,
    /// makes it gone — with any unsaved words parked in the trash.
    fn missing_on_disk(&mut self, i: usize) -> Result<DiskChange> {
        let path = self.nodes[i].path.clone();
        if icloud_stub(&path).exists() {
            return Ok(self.mark_unavailable(i, Unavailable::icloud()));
        }
        let n = &mut self.nodes[i];
        n.disk.missing = n.disk.missing.saturating_add(1);
        if n.disk.missing < 2 {
            return Ok(DiskChange::Same);
        }
        if Stat::of(&path).is_some() {
            self.nodes[i].disk.missing = 0;
            return Ok(DiskChange::Same);
        }
        let n = &self.nodes[i];
        if n.dirty && !n.read_only {
            let text = n.file_text();
            let copy = sync::park_in_trash(&self.root, &path, &text)?;
            self.nodes[i].dirty = false;
            return Ok(DiskChange::GoneParked(copy));
        }
        Ok(DiskChange::Gone)
    }

    /// Scene `i` can't be read: keep what's in memory, never write it, and
    /// say so once (not on every look).
    fn mark_unavailable(&mut self, i: usize, why: Unavailable) -> DiskChange {
        let d = &mut self.nodes[i].disk;
        d.pending = None;
        if d.unavailable.as_ref() == Some(&why) {
            return DiskChange::Same;
        }
        d.unavailable = Some(why.clone());
        DiskChange::Unavailable(why)
    }

    /// Refuse to compile or export a manuscript with scenes that have never
    /// been readable here: the result would be a book with holes in it and no
    /// sign of them.
    pub fn ensure_whole(&self) -> Result<()> {
        let missing = self.missing_from_manuscript();
        if missing.is_empty() {
            return Ok(());
        }
        let shown: Vec<&str> = missing.iter().take(3).map(|s| s.as_str()).collect();
        let more = if missing.len() > 3 {
            format!(" and {} more", missing.len() - 3)
        } else {
            String::new()
        };
        anyhow::bail!(
            "{}{more} can't be read yet (not downloaded, or offline?) — nothing was written; try again once they've downloaded",
            shown.join(", ")
        )
    }

    /// Scenes in the manuscript that have never been readable here: a compile
    /// or export without them would be a book with holes and no warning.
    pub fn missing_from_manuscript(&self) -> Vec<String> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| n.in_manuscript && n.never_read() && !n.parked && !self.in_trash(*i))
            .map(|(_, n)| n.title.clone())
            .collect()
    }

    /// Has something else added or removed files or folders since the tree
    /// was read? Only names are compared — no file is opened.
    pub fn tree_is_stale(&self) -> bool {
        let mut on_disk = std::collections::BTreeSet::new();
        for area in Area::ordered(&self.meta) {
            let path = area.path(&self.root);
            if area == Area::Format {
                if path.is_file() {
                    on_disk.insert(path);
                }
                continue;
            }
            if path.is_dir() {
                on_disk.insert(path.clone());
                list_tree(&path, &mut on_disk);
            }
        }
        let in_tree: std::collections::BTreeSet<PathBuf> =
            self.nodes.iter().map(|n| n.path.clone()).collect();
        on_disk != in_tree
    }

    pub fn dirty_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.dirty).count()
    }
}

/// Every folder and `.md` file under `dir`, as [`Project::load`] would read
/// them (hidden names skipped).
fn list_tree(dir: &Path, out: &mut std::collections::BTreeSet<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if let Some(real) = icloud_stub_for(&e.path()) {
            if !real.exists() {
                out.insert(real);
            }
            continue;
        }
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            out.insert(path.clone());
            list_tree(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            out.insert(path);
        }
    }
}

/// "Permission denied" rather than "writing /long/path: Permission denied (os error 13)".
fn short_reason(e: &anyhow::Error) -> String {
    let root = e.root_cause().to_string();
    match root.find(" (os error") {
        Some(i) => root[..i].to_string(),
        None => root,
    }
}

/// Write a whole file so that it is either the old version or the new one,
/// never a torn mix — see [`crate::atomic`].
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    crate::atomic::write_text(path, text)
}

/// Split a `---` fenced YAML frontmatter block off the front of a file.
pub fn split_frontmatter(raw: &str) -> (Option<String>, String) {
    let s = raw.strip_prefix("\u{feff}").unwrap_or(raw);
    let Some(rest) = s.strip_prefix("---\n") else {
        return (None, s.to_string());
    };
    // Find the closing fence at the start of a line.
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            let front = rest[..offset].to_string();
            let body = rest[offset + line.len()..]
                .trim_start_matches('\n')
                .to_string();
            return (Some(front), body);
        }
        offset += line.len();
    }
    (None, s.to_string())
}

/// Pull a scalar value out of a raw frontmatter block. Deliberately naive —
/// we only read a few known keys and never rewrite the block.
fn front_get(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.trim() == key {
            let v = v.trim().trim_matches('"').trim_matches('\'').trim();
            if v.is_empty() {
                return None;
            }
            return Some(v.to_string());
        }
    }
    None
}

/// Set `key: value` in a frontmatter block. The first line for that key is
/// replaced; if there isn't one, it goes at the end. Nothing else changes — not
/// the order, the comments, or keys Grimoire has never heard of.
pub fn set_front(front: Option<&str>, key: &str, value: &str) -> String {
    let value = value.trim();
    let needs_quotes = value.contains(':')
        || value.contains('#')
        || value.starts_with([
            '"', '\'', '[', '{', '-', '&', '*', '!', '|', '>', '%', '@', '`',
        ]);
    let line = if value.is_empty() {
        format!("{key}:")
    } else if needs_quotes {
        format!(
            "{key}: \"{}\"",
            value.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else {
        format!("{key}: {value}")
    };
    let mut out = String::new();
    let mut done = false;
    for l in front.unwrap_or("").lines() {
        let is_key =
            !l.starts_with([' ', '\t']) && l.split_once(':').is_some_and(|(k, _)| k.trim() == key);
        if is_key && !done {
            out.push_str(&line);
            done = true;
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    if !done {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// `01-the-archive.md` -> `The Archive`, unless frontmatter names it.
fn display_title(path: &Path, front: Option<&str>) -> String {
    if let Some(f) = front
        && let Some(t) = front_get(f, "title")
    {
        return t;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let stripped = stem
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '-' && *c != '_' && *c != ' ')
        .map(|(i, _)| &stem[i..])
        .unwrap_or(&stem);

    let words: Vec<String> = stripped
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    rejoin_numbers(&words)
}

/// A dash is a word separator on disk, so "Chapter Twenty-Seven" comes back as
/// three words. Only one pairing is ever meant as a hyphen — a tens word
/// followed by a units word — so put that one back and leave everything else
/// alone. "The Archive" stays two words; "Chapter Twenty-Seven" stays hyphenated.
fn rejoin_numbers(words: &[String]) -> String {
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    const UNITS: [&str; 9] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let w = words[i].to_lowercase();
        let next = words.get(i + 1).map(|n| n.to_lowercase());
        match next {
            Some(n) if TENS.contains(&w.as_str()) && UNITS.contains(&n.as_str()) => {
                out.push(format!("{}-{}", words[i], words[i + 1]));
                i += 2;
            }
            _ => {
                out.push(words[i].clone());
                i += 1;
            }
        }
    }
    out.join(" ")
}

/// Three parts, nine chapters each, three scenes in every chapter.
pub const PARTS: usize = 3;
pub const CHAPTERS_PER_PART: usize = 9;
pub const SCENES_PER_CHAPTER: usize = 3;

/// The front matter keeps one folder per edition, because a paperback, an
/// ebook and a submission each open differently.
pub const FRONT_MATTER_FORMATS: [&str; 3] = ["Manuscript Format", "Paperback", "Ebook"];

/// Back matter goes only into the reader editions: a submission ends at END.
pub const BACK_MATTER_FORMATS: [&str; 2] = ["Paperback", "Ebook"];

/// Where deleted things go. Inside `.grimoire/`, which is gitignored, but the
/// tree shows it so nothing ever just vanishes.
pub fn trash_dir(root: &Path) -> PathBuf {
    root.join(".grimoire/trash")
}

/// Create a new book: every section, the three-part shape with every chapter
/// named and three empty scenes in each, starter front matter for each
/// edition, and the template sheets.
pub fn scaffold(root: &Path) -> Result<()> {
    if root.join("manuscript").is_dir() {
        anyhow::bail!("{} already contains a manuscript/", root.display());
    }

    let title = root
        .file_name()
        .map(|s| display_title(Path::new(s), None))
        .unwrap_or_else(|| "Untitled".into());

    write_new(
        &root.join("novel.toml"),
        &format!(
            "title = \"{title}\"\nauthor = \"\"\ndraft = \"1\"\ntarget_words = 80000\ndaily_target = 1000\n\n# What this book calls its largest division: Part, Act, Book…\npart_label = \"Part\"\n"
        ),
    )?;

    let mut chapter = 0usize;
    for part in 1..=PARTS {
        let part_dir = root.join("manuscript").join(numbered_dir(
            part,
            &crate::manuscript::numbered("Part", part),
        ));
        for c in 1..=CHAPTERS_PER_PART {
            chapter += 1;
            // Chapters are numbered straight through the book, the way the
            // finished manuscript numbers them, not restarted in each part.
            let ch_dir = part_dir.join(numbered_dir(
                c,
                &crate::manuscript::numbered("Chapter", chapter),
            ));
            fs::create_dir_all(&ch_dir)
                .with_context(|| format!("creating {}", ch_dir.display()))?;
            for s in 1..=SCENES_PER_CHAPTER {
                let name = crate::manuscript::numbered("Scene", s);
                let path = ch_dir.join(format!("{}.md", numbered_dir(s, &name)));
                write_new(
                    &path,
                    &format!(
                        "---\ntitle: \"{name}\"\npov:\nstatus: outline\nsynopsis:\ntarget: 1500\n---\n\n"
                    ),
                )?;
            }
        }
    }

    add_sections(root, true)?;
    write_new(&root.join(".gitignore"), ".grimoire/\n")?;
    Ok(())
}

/// Make whichever sections are missing, with their starter documents. A new
/// book (`fresh`) also gets starter front matter; an existing one only gets
/// the empty per-edition folders, so nothing new turns up in its exports.
fn add_sections(root: &Path, fresh: bool) -> Result<Vec<String>> {
    let mut added = Vec::new();
    for area in Area::ALL {
        let path = area.path(root);
        if area == Area::Format {
            if !path.exists() {
                write_new(&path, starter::NOVEL_FORMAT)?;
                added.push(area.title().to_string());
            }
            continue;
        }
        if !path.is_dir() {
            fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
            if area != Area::Trash {
                added.push(area.title().to_string());
            }
            if area == Area::Templates {
                write_new(
                    &path.join("01-Character-Sketch.md"),
                    starter::CHARACTER_SKETCH,
                )?;
                write_new(&path.join("02-Setting-Sketch.md"), starter::SETTING_SKETCH)?;
            }
        }
    }

    let fm = Area::FrontMatter.path(root);
    for (i, edition) in FRONT_MATTER_FORMATS.iter().enumerate() {
        if has_folder(&fm, edition) {
            continue;
        }
        let dir = fm.join(numbered_dir(i + 1, edition));
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        if fresh {
            if *edition == "Manuscript Format" {
                write_new(&dir.join("01-Title-Page.md"), starter::SUBMISSION_TITLE)?;
            } else {
                write_new(&dir.join("01-Title-Page.md"), starter::TITLE_PAGE)?;
                write_new(&dir.join("02-Copyright.md"), starter::COPYRIGHT)?;
                write_new(&dir.join("03-Dedication.md"), starter::DEDICATION)?;
            }
        }
    }

    let bm = Area::BackMatter.path(root);
    for (i, edition) in BACK_MATTER_FORMATS.iter().enumerate() {
        if has_folder(&bm, edition) {
            continue;
        }
        let dir = bm.join(numbered_dir(i + 1, edition));
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        if fresh {
            write_new(
                &dir.join("01-Acknowledgements.md"),
                starter::ACKNOWLEDGEMENTS,
            )?;
            write_new(
                &dir.join("02-About-the-Author.md"),
                starter::ABOUT_THE_AUTHOR,
            )?;
            write_new(&dir.join("03-Also-By.md"), starter::ALSO_BY)?;
        }
    }

    let research = Area::Research.path(root);
    if !has_folder(&research, "Sample Output") {
        let dir = research.join(numbered_dir(next_number(&research)?, "Sample Output"));
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Ok(added)
}

/// Is there a folder in `dir` that reads as `name`, numbered or not?
fn has_folder(dir: &Path, name: &str) -> bool {
    fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten()
            .any(|e| e.path().is_dir() && display_title(&e.path(), None).eq_ignore_ascii_case(name))
    })
}

/// Bring a book made before sections into their shape, once, when it opens.
/// Nothing is deleted and nothing is overwritten: the notebook's Characters,
/// Places and Research folders move up to be sections of their own, a Notes
/// folder inside the notes empties into them, missing sections are made, and
/// `[[links]]` follow every move. A book's parts keep their names.
/// Returns what it did, for the status line; empty when the book was already
/// in shape.
pub fn upgrade(root: &Path) -> Result<Vec<String>> {
    if !root.join("manuscript").is_dir() {
        return Ok(Vec::new());
    }
    let mut done = Vec::new();
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();

    // Out of the notebook, into sections.
    let notes = Area::Notes.path(root);
    for dir in tree_entries(&notes).into_iter().filter(|p| p.is_dir()) {
        let area = match display_title(&dir, None).to_lowercase().as_str() {
            "characters" => Area::Characters,
            "places" => Area::Places,
            "research" => Area::Research,
            "notes" => Area::Notes,
            _ => continue,
        };
        let target = area.path(root);
        let empty = |p: &Path| fs::read_dir(p).is_ok_and(|mut rd| rd.next().is_none());
        if area != Area::Notes && (!target.exists() || empty(&target)) {
            if target.exists() {
                fs::remove_dir(&target)
                    .with_context(|| format!("replacing {}", target.display()))?;
            }
            fs::rename(&dir, &target).with_context(|| format!("moving {}", dir.display()))?;
            renames.push((dir.clone(), target));
        } else {
            // Merge what doesn't collide; anything that would is left where it is.
            for item in tree_entries(&dir) {
                let to = target.join(item.file_name().unwrap_or_default());
                if !to.exists() {
                    fs::rename(&item, &to).with_context(|| format!("moving {}", item.display()))?;
                    renames.push((item, to));
                }
            }
            if empty(&dir) {
                let _ = fs::remove_dir(&dir);
            }
        }
        done.push(format!("{} is its own section", area.title()));
    }

    // 0.3.0–0.3.6 renamed a label-less book's parts to pages without writing
    // the word down. Parts are the default again, so such a book says "Page"
    // in novel.toml to go on naming new ones the way its folders already read.
    // Nothing is renamed.
    if let Some(noted) = keep_page_label(root)? {
        done.push(noted);
    }

    let added = add_sections(root, false)?;
    if !added.is_empty() {
        done.push(format!("added {}", added.join(", ")));
    }
    if !renames.is_empty() {
        rewrite_links(root, &renames);
    }
    Ok(done)
}

/// For a book with no `part_label` whose manuscript is divided into pages
/// (`01-Page-One`, as 0.3.0–0.3.6 renamed them), write `part_label = "Page"`
/// so new ones match. `Some(what it did)`, for the status line.
fn keep_page_label(root: &Path) -> Result<Option<String>> {
    let toml_path = root.join("novel.toml");
    // A novel.toml that can't be read (an online-only file not downloaded
    // yet) is not an empty one: writing "part_label" into it would replace
    // the book's title and targets with a single line.
    let text = match fs::read_to_string(&toml_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Ok(None),
    };
    // A novel.toml that doesn't parse is left alone rather than appended to on
    // every launch.
    let Ok(table) = toml::from_str::<toml::Table>(&text) else {
        return Ok(None);
    };
    if table.contains_key("part_label") {
        return Ok(None);
    }
    let pages = tree_entries(&Area::Manuscript.path(root))
        .into_iter()
        .filter(|p| p.is_dir() && is_page_folder(p))
        .count();
    if pages == 0 {
        return Ok(None);
    }
    let mut text = text;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(
        "\n# What this book calls its largest division: Part, Act, Book…\npart_label = \"Page\"\n",
    );
    write_atomic(&toml_path, &text)?;
    Ok(Some("this book counts in pages, as its folders do".into()))
}

/// `02-Page-Two`, `01-page-one`, `PAGE III` — a folder named as a page.
fn is_page_folder(dir: &Path) -> bool {
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    let rest =
        name.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '-' | '_' | ' '));
    rest.get(..4)
        .is_some_and(|w| w.eq_ignore_ascii_case("page"))
        && !rest[4..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric())
}

/// What a new book starts with in its documents.
mod starter {
    // Written to be read in Grimoire's editor, which shows Markdown as typed:
    // plain prose, no emphasis marks or code quotes.
    pub const NOVEL_FORMAT: &str = "---\ntitle: \"Novel Format\"\n---\n\n\
How this book is laid out. Every section of the tree folds: Enter or Space \
opens and closes one. Esc opens the menu, from anywhere.\n\n\
MANUSCRIPT is the book itself: parts hold chapters, chapters hold scenes. In \
the tree, p makes a part, c a chapter and n a scene. Only what's in here counts \
toward your word count and goes into an export. If your book says Act or Book \
rather than Part, change part_label in novel.toml.\n\n\
CHARACTERS and PLACES are your notebook. Their names teach the spellchecker, \
and Ctrl-O on a name in a scene opens its note beside the scene.\n\n\
FRONT MATTER holds the pages before chapter one, one folder per edition: \
Manuscript Format for a Word submission, Ebook for the EPUB, Paperback for print. \
The starter pages stay out of the book until you want them: each begins with \
the line compile: false, and changing it to compile: true puts it in.\n\n\
NOTES and RESEARCH are for everything else you gather. Sample Output, in \
Research, is a place to keep exports you want to compare.\n\n\
TEMPLATE SHEETS are blank forms. Copy one into Characters or Places to fill in.\n\n\
TRASH keeps whatever you delete until you delete it from there too.\n";

    pub const CHARACTER_SKETCH: &str = "---\ntitle: \"Character Sketch\"\n---\n\n\
Name:\nAlso called:\nRole in the story:\nAge:\nAppearance:\n\n\
## Wants\n\n## Needs\n\n## Fears\n\n## Voice\nHow they talk, and what they never say.\n\n\
## Arc\nWho they are on the first page, and on the last.\n\n## Relationships\n";

    pub const SETTING_SKETCH: &str = "---\ntitle: \"Setting Sketch\"\n---\n\n\
Name:\nWhere it is:\nWhen:\n\n## First impression\nWhat someone notices walking in.\n\n\
## Senses\nSights, sounds, smells, weather, light.\n\n## History\n\n## Who lives or works here\n\n\
## What happens here\nScenes that use it, and why the story needs it.\n";

    pub const SUBMISSION_TITLE: &str = "---\ntitle: \"Title Page\"\ncompile: false\n---\n\n\
While this says compile: false, the Word export makes a standard title page \
from novel.toml: your name, the title and a rounded word count. Change it to \
compile: true and write your own here to use it instead.\n";

    pub const TITLE_PAGE: &str =
        "---\ntitle: \"Title Page\"\ncompile: false\n---\n\n# Title\n\nAuthor Name\n";

    pub const COPYRIGHT: &str = "---\ntitle: \"Copyright\"\ncompile: false\n---\n\n\
Copyright © Year Author Name\n\nAll rights reserved. No part of this book may be reproduced \
without permission, except for brief quotations in reviews.\n\n\
This is a work of fiction. Names, characters, places and incidents are products of the \
author's imagination.\n";

    pub const DEDICATION: &str = "---\ntitle: \"Dedication\"\ncompile: false\n---\n\nFor\n";

    pub const ACKNOWLEDGEMENTS: &str = "---\ntitle: \"Acknowledgements\"\ncompile: false\n---\n\n\
# Acknowledgements\n\nThank you to\n";

    pub const ABOUT_THE_AUTHOR: &str = "---\ntitle: \"About the Author\"\ncompile: false\n---\n\n\
# About the Author\n\nAuthor Name lives in\n";

    pub const ALSO_BY: &str = "---\ntitle: \"Also By\"\ncompile: false\n---\n\n\
# Also by Author Name\n\nTitle One\n";
}

/// `7` and "Chapter Seven" make `07-Chapter-Seven`.
fn numbered_dir(n: usize, name: &str) -> String {
    format!("{:02}-{}", n, crate::names::stem(name))
}

fn write_new(path: &Path, body: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    write_atomic(path, body)
}

/// Make a new scene (`.md`) or folder at the end of `dir`, numbered after
/// whatever is already there. Nothing existing is renamed, so links from
/// Obsidian or anywhere else keep working. Returns the new path.
pub fn create(dir: &Path, name: &str, folder: bool) -> Result<PathBuf> {
    let name = name.trim();
    let name = if name.is_empty() { "Untitled" } else { name };
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let prefix = format!("{:02}-", next_number(dir)?);
    let ext = if folder { "" } else { ".md" };
    let file = crate::names::fit(dir, &prefix, &crate::names::stem(name), ext);
    if let Some(other) = crate::names::clash(dir, &file, None) {
        anyhow::bail!("{}", clash_message(&file, &other));
    }
    let path = dir.join(&file);
    if folder {
        fs::create_dir(&path).with_context(|| format!("creating {}", path.display()))?;
        return Ok(path);
    }
    if path.exists() {
        anyhow::bail!("{} already exists", path.display());
    }
    // The title is kept exactly as typed; only the filename is tidied.
    let title = name.replace('"', "'");
    write_atomic(
        &path,
        &format!("---\ntitle: \"{title}\"\npov:\nstatus: draft\nsynopsis:\n---\n\n"),
    )?;
    Ok(path)
}

/// Rename a scene or folder where it stands, keeping its leading number so
/// nothing else in the folder shifts. A scene's frontmatter `title:` is
/// rewritten to match, because that is the name the tree shows.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("it needs a name");
    }
    let parent = path.parent().context("that has no folder to sit in")?;
    let prefix = leading_number(path)
        .map(|n| format!("{n:02}-"))
        .unwrap_or_default();
    let folder = path.is_dir();
    let ext = if folder { "" } else { ".md" };
    let file = crate::names::fit(parent, &prefix, &crate::names::stem(name), ext);
    let target = parent.join(&file);
    if target != path {
        // Capitals or accents apart from a neighbour is still a collision on
        // a Mac, on Windows and in Box; only this item's own name may change
        // case.
        if let Some(other) = crate::names::clash(parent, &file, Some(path)) {
            anyhow::bail!("{}", clash_message(&file, &other));
        }
        if target.exists() && !crate::names::same_file(&target, path) {
            anyhow::bail!("{} already exists", target.display());
        }
        let copies = if folder {
            Vec::new()
        } else {
            sync::copies_of(path)
        };
        fs::rename(path, &target).with_context(|| format!("renaming {}", path.display()))?;
        // Its conflict copies follow it under the new name.
        let new_stem = target
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        for (copy, c) in copies {
            let to = parent.join(format!("{new_stem}{}.md", c.suffix));
            if !to.exists() {
                let _ = fs::rename(&copy, &to);
            }
        }
    }
    // A file that isn't UTF-8 keeps its bytes: renamed, never rewritten.
    if !folder
        && let Ok(raw) = fs::read_to_string(&target)
        && let Some(updated) = retitle(&raw, name)
    {
        write_atomic(&target, &updated)?;
    }
    Ok(target)
}

/// Why a name can't be used: `file` would be the same file as `other` to a
/// client that ignores capitals and accents.
fn clash_message(file: &str, other: &Path) -> String {
    let theirs = other.file_name().unwrap_or_default().to_string_lossy();
    format!(
        "“{file}” would clash with “{theirs}” — Dropbox, Box, Macs and Windows treat names \
         that differ only in capitals or accents as one file"
    )
}

/// Swap the `title:` line inside a frontmatter block. `None` when there is no
/// block or no title in it — then the filename is already the name.
fn retitle(raw: &str, name: &str) -> Option<String> {
    let (front, body) = split_frontmatter(raw);
    let front = front?;
    let mut done = false;
    let mut out = String::new();
    for line in front.lines() {
        if !done
            && line
                .split_once(':')
                .is_some_and(|(k, _)| k.trim() == "title")
        {
            out.push_str(&format!("title: \"{}\"\n", name.replace('"', "'")));
            done = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    done.then(|| format!("---\n{out}---\n\n{body}"))
}

/// Move something out of the tree rather than destroying it. `.grimoire/` is
/// gitignored, so the trash never reaches a commit — and with no undo in the
/// editor yet, a delete key must not be the last word. Returns where it went.
pub fn trash(root: &Path, path: &Path) -> Result<PathBuf> {
    let dir = root.join(".grimoire/trash");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path
        .file_name()
        .context("there is nothing there to delete")?
        .to_string_lossy()
        .to_string();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Every chapter's first scene has the same file name, so two deletes in
    // one second would share a name — and a rename onto a file replaces it.
    let mut target = dir.join(format!("{stamp}-{name}"));
    let mut n = 2;
    while target.symlink_metadata().is_ok() {
        target = dir.join(format!("{stamp}-{n}-{name}"));
        n += 1;
    }
    fs::rename(path, &target).with_context(|| format!("moving {} to the trash", path.display()))?;
    Ok(target)
}

/// Remove something for good. Only ever reached for what is already in the
/// trash — everywhere else, deleting moves things there instead.
pub fn destroy(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| format!("deleting {}", path.display()))
}

/// What a move did on disk: every rename in order (old path → new path), where
/// the moved item ended up, and how many files had links rewritten.
#[derive(Debug, Clone, PartialEq)]
pub struct Moved {
    pub renames: Vec<(PathBuf, PathBuf)>,
    pub target: PathBuf,
    pub links: usize,
}

/// What the tree orders in a folder: subfolders and `.md` files, nothing
/// hidden — and no conflict copies, which aren't places in the book but
/// versions of one, and travel with their original (see [`with_copies`]).
fn tree_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| e.path())
                .filter(|p| p.is_dir() || p.extension().and_then(|s| s.to_str()) == Some("md"))
                .collect()
        })
        .unwrap_or_default();
    let stems = sync::md_stems(dir);
    let hint = sync::provider_of(dir);
    v.retain(|p| {
        p.is_dir()
            || p.file_stem().is_none_or(|s| {
                sync::copy_of(&s.to_string_lossy(), &|x| stems.contains(x), hint).is_none()
            })
    });
    v.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
    v
}

/// A scene that moves or is renamed takes its conflict copies with it,
/// renamed to follow ("01-Scene-One (Josh's conflicted copy).md" becomes
/// "02-Scene-One (Josh's conflicted copy).md"), so a copy is never left
/// behind under a number that now belongs to something else.
fn with_copies(renames: &[(PathBuf, PathBuf)]) -> Vec<(PathBuf, PathBuf)> {
    let mut out = renames.to_vec();
    for (from, to) in renames {
        if !from.is_file() {
            continue;
        }
        let (Some(dir), Some(stem)) = (to.parent(), to.file_stem()) else {
            continue;
        };
        let stem = stem.to_string_lossy().to_string();
        for (copy, c) in sync::copies_of(from) {
            let dest = dir.join(format!("{stem}{}.md", c.suffix));
            // Never onto something already there: a copy left where it was
            // is still a copy, but a rename onto a file replaces it.
            let taken = dest.exists() && !out.iter().any(|(f, _)| *f == dest);
            if taken || out.iter().any(|(f, _)| *f == copy) {
                continue;
            }
            out.push((copy, dest));
        }
    }
    out
}

/// `07-Low-Tide.md` → (7, 2, "Low-Tide.md").
fn split_number(path: &Path) -> Option<(usize, usize, String)> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = name[digits.len()..]
        .trim_start_matches(['-', '_', ' '])
        .to_string();
    Some((digits.parse().ok()?, digits.len(), rest))
}

fn numbered_name(n: usize, width: usize, rest: &str) -> String {
    format!("{n:0width$}-{rest}")
}

/// Move a scene or folder one place up or down. Within its folder it swaps
/// numbers with its neighbour; at the top or bottom it crosses into the
/// neighbouring chapter (or act, for a chapter) — onto the end going up, the
/// start going down. Only files whose number has to change are renamed, and
/// `[[links]]` to anything renamed are rewritten across the book.
pub fn move_item(root: &Path, path: &Path, up: bool) -> Result<Moved> {
    let parent = path.parent().context("that has no folder")?.to_path_buf();
    let top: Vec<PathBuf> = Area::ALL.iter().map(|a| a.path(root)).collect();
    if top.contains(&path.to_path_buf()) || path.starts_with(root.join(".grimoire")) {
        anyhow::bail!("that can't be moved");
    }
    // Order lives in the leading number. A folder where something has none
    // (a note made in another app, say) is numbered first, in the order it
    // shows, so the move has numbers to swap.
    let mut numbering: Vec<(PathBuf, PathBuf)> = Vec::new();
    let entries = tree_entries(&parent);
    let at = entries
        .iter()
        .position(|p| p == path)
        .context("it isn't in its folder any more")?;
    let beside = if up {
        at.checked_sub(1)
    } else {
        (at + 1 < entries.len()).then_some(at + 1)
    };
    if beside.is_some_and(|b| split_number(path).is_none() || split_number(&entries[b]).is_none()) {
        let width = entries.len().to_string().len().max(2);
        for (i, p) in entries.iter().enumerate() {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let rest = split_number(p).map_or(name, |(_, _, r)| r);
            let to = parent.join(numbered_name(i + 1, width, &rest));
            if to != *p {
                numbering.push((p.clone(), to));
            }
        }
        numbering = with_copies(&numbering);
        rename_all(&numbering)?;
        rewrite_links(root, &numbering);
    }
    let path = &numbering
        .iter()
        .find(|(f, _)| f == path)
        .map_or(path.to_path_buf(), |(_, t)| t.clone());
    let siblings = tree_entries(&parent);
    let pos = siblings
        .iter()
        .position(|p| p == path)
        .context("it isn't in its folder any more")?;
    let neighbour = if up {
        pos.checked_sub(1)
    } else {
        (pos + 1 < siblings.len()).then_some(pos + 1)
    };

    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    let target;
    if let Some(n) = neighbour.map(|i| siblings[i].clone()) {
        // Swap numbers with the neighbour.
        let (a_num, a_w, a_rest) = split_number(path)
            .context("only numbered items can be moved — give it a number first")?;
        let (b_num, b_w, b_rest) = split_number(&n)
            .context("its neighbour has no number, so there's nothing to swap with")?;
        let (a_num, b_num) = if a_num == b_num {
            // Same number (it happens): order by name, so just nudge.
            if up {
                (a_num, a_num + 1)
            } else {
                (a_num + 1, a_num)
            }
        } else {
            (a_num, b_num)
        };
        let a_to = parent.join(numbered_name(b_num, a_w.max(b_w), &a_rest));
        let b_to = parent.join(numbered_name(a_num, a_w.max(b_w), &b_rest));
        renames.push((path.to_path_buf(), a_to.clone()));
        renames.push((n, b_to));
        target = a_to;
    } else {
        // Cross into the neighbouring folder at the parent's level.
        if top.contains(&parent) || parent == root {
            anyhow::bail!(if up {
                "it's already first"
            } else {
                "it's already last"
            });
        }
        let grand = parent.parent().context("no folder above")?;
        let uncles: Vec<PathBuf> = tree_entries(grand)
            .into_iter()
            .filter(|p| p.is_dir())
            .collect();
        let at = uncles
            .iter()
            .position(|p| *p == parent)
            .context("its folder moved")?;
        let dest = if up {
            at.checked_sub(1)
        } else {
            (at + 1 < uncles.len()).then_some(at + 1)
        }
        .map(|i| uncles[i].clone())
        .or_else(|| cousin_folder(root, &parent, up))
        .with_context(|| {
            if up {
                "it's already first"
            } else {
                "it's already last"
            }
        })?;
        let (_, width, rest) = split_number(path).unwrap_or((
            0,
            2,
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        ));
        let there = tree_entries(&dest);
        let numbers: Vec<(usize, usize)> = there
            .iter()
            .filter_map(|p| split_number(p).map(|(n, w, _)| (n, w)))
            .collect();
        let width = numbers
            .iter()
            .map(|&(_, w)| w)
            .max()
            .unwrap_or(width)
            .max(width);
        let number = if up {
            numbers.iter().map(|&(n, _)| n).max().unwrap_or(0) + 1
        } else {
            let min = numbers.iter().map(|&(n, _)| n).min().unwrap_or(2);
            if min >= 2 {
                min - 1
            } else {
                // No room before the first: shift everything there along one.
                for p in there.iter().rev() {
                    if let Some((n, w, r)) = split_number(p) {
                        renames
                            .push((p.clone(), dest.join(numbered_name(n + 1, w.max(width), &r))));
                    }
                }
                1
            }
        };
        let to = dest.join(numbered_name(number, width, &rest));
        renames.push((path.to_path_buf(), to.clone()));
        target = to;
    }

    let renames = with_copies(&renames);
    for (_, to) in &renames {
        if to.exists() && !renames.iter().any(|(from, _)| from == to) {
            anyhow::bail!("{} already exists", to.display());
        }
    }
    rename_all(&renames)?;
    let links = rewrite_links(root, &renames);
    // Report it as one set of moves from where everything started.
    let renames = compose(&numbering, &renames);
    Ok(Moved {
        renames,
        target,
        links,
    })
}

/// `first` then `then`, as one list from the original paths to the final ones.
fn compose(first: &[(PathBuf, PathBuf)], then: &[(PathBuf, PathBuf)]) -> Vec<(PathBuf, PathBuf)> {
    if first.is_empty() {
        return then.to_vec();
    }
    let mut out: Vec<(PathBuf, PathBuf)> = first
        .iter()
        .map(|(a, b)| {
            (
                a.clone(),
                then.iter()
                    .find(|(f, _)| f == b)
                    .map_or(b.clone(), |(_, t)| t.clone()),
            )
        })
        .collect();
    for (f, t) in then {
        if !first.iter().any(|(_, b)| b == f) {
            out.push((f.clone(), t.clone()));
        }
    }
    out
}

/// At the edge of an act, the chapter before or after is in the neighbouring
/// act: the last folder of the previous one, or the first of the next.
fn cousin_folder(root: &Path, folder: &Path, up: bool) -> Option<PathBuf> {
    let grand = folder.parent()?;
    let great = grand.parent()?;
    if great == root {
        return None;
    }
    let aunts: Vec<PathBuf> = tree_entries(great)
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    let at = aunts.iter().position(|p| p == grand)?;
    let aunt = if up {
        at.checked_sub(1)?
    } else {
        (at + 1 < aunts.len()).then_some(at + 1)?
    };
    let kids: Vec<PathBuf> = tree_entries(&aunts[aunt])
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    if up {
        kids.last().cloned()
    } else {
        kids.first().cloned()
    }
}

/// Save the order of the tree's sections in novel.toml, as one
/// `sections = [...]` line; every other line of the file stays as it was.
pub fn save_section_order(root: &Path, order: &[Area]) -> Result<()> {
    let path = root.join("novel.toml");
    // Only a file that isn't there at all starts empty. One that can't be read
    // (online-only and offline, say) is refused: rewriting it from nothing
    // would leave the book with one line where its title and targets were.
    let old = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context("novel.toml can't be read right now, so the section order wasn't saved"));
        }
    };
    let keys: Vec<String> = order
        .iter()
        .filter(|&&a| a != Area::Trash)
        .map(|a| format!("\"{}\"", a.key()))
        .collect();
    let line = format!("sections = [{}]", keys.join(", "));
    let mut out = String::new();
    let mut done = false;
    for l in old.lines() {
        if l.trim_start().starts_with("sections") && l.contains('=') {
            if !done {
                out.push_str(&line);
                out.push('\n');
                done = true;
            }
            continue;
        }
        // Keys after a [table] header would belong to it, so it goes first.
        if !done && l.trim_start().starts_with('[') {
            out.push_str(&line);
            out.push_str("\n\n");
            done = true;
        }
        out.push_str(l);
        out.push('\n');
    }
    if !done {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&line);
        out.push('\n');
    }
    write_atomic(&path, &out)
}

// ── settling a conflict copy ────────────────────────────────────────

/// What to do with a conflict copy once both versions have been read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// The copy's version becomes the scene; the scene's words go to its
    /// history first, and the copy to the trash.
    TakeCopy,
    /// The scene stays as it is; the copy goes to the trash.
    KeepOriginal,
    /// Both stay: the copy becomes a scene of its own, right after the
    /// original, titled as the copy it was.
    KeepBoth,
}

/// What settling did on disk, in the order it did it, so it can be taken
/// back: files moved (the trash counts), and files whose text changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settled {
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// (file, text before, text after) — at the file's path after the moves.
    pub texts: Vec<(PathBuf, String, String)>,
    /// Whether `[[links]]` followed the moves.
    pub links: bool,
    /// Where the copy is now, if it's still in the book (`KeepBoth`).
    pub kept: Option<PathBuf>,
}

/// Settle the conflict copy at `copy`. `seen` is what the original looked
/// like when the writer read it; taking the copy only goes ahead while the
/// original still looks that way, so a third version arriving meanwhile is
/// never overwritten. Nothing is ever deleted: what's let go goes to the
/// trash, and the original's text to its history.
pub fn settle(root: &Path, copy: &Path, how: Settle, seen: Option<Fingerprint>) -> Result<Settled> {
    let of = sync::copy_of_path(copy).context("that isn't a conflict copy")?;
    let dir = copy.parent().context("it has no folder")?;
    let original = dir.join(format!("{}.md", of.original));
    let mut out = Settled::default();
    match how {
        Settle::KeepOriginal => {
            let to = trash(root, copy)?;
            out.moves.push((copy.to_path_buf(), to));
        }
        Settle::TakeCopy => {
            let theirs =
                fs::read_to_string(copy).with_context(|| format!("reading {}", copy.display()))?;
            let ours = fs::read_to_string(&original)
                .with_context(|| format!("reading {}", original.display()))?;
            crate::history::snapshot(root, &original, &ours, None)?;
            let seen = seen.or(Some(Fingerprint::of(&ours)));
            match sync::save_guarded(&original, &theirs, seen)? {
                SaveOutcome::Written => {}
                SaveOutcome::Conflict { .. } => anyhow::bail!(
                    "{} changed on disk just now — read it again first",
                    display_title(&original, None)
                ),
            }
            out.texts.push((original.clone(), ours, theirs));
            let to = trash(root, copy)?;
            out.moves.push((copy.to_path_buf(), to));
        }
        Settle::KeepBoth => {
            let label = format!("{} copy", of.source.name());
            let name = match split_number(&original) {
                Some((n, width, rest)) => {
                    let rest = rest.strip_suffix(".md").unwrap_or(&rest).to_string();
                    // Everything after the original moves down one, so the
                    // copy can stand right after it.
                    let mut renames: Vec<(PathBuf, PathBuf)> = tree_entries(dir)
                        .into_iter()
                        .filter_map(|p| {
                            let (m, w, r) = split_number(&p)?;
                            (m > n).then(|| (p.clone(), dir.join(numbered_name(m + 1, w, &r))))
                        })
                        .collect();
                    renames.sort_by_key(|(from, _)| std::cmp::Reverse(from.clone()));
                    let to = dir.join(numbered_name(
                        n + 1,
                        width,
                        &format!("{rest}-{}.md", crate::names::stem(&label)),
                    ));
                    renames.push((copy.to_path_buf(), to.clone()));
                    (renames, to)
                }
                None => {
                    let to = dir.join(format!("{}-{}.md", of.original, crate::names::stem(&label)));
                    (vec![(copy.to_path_buf(), to.clone())], to)
                }
            };
            let (renames, to) = name;
            let renames = with_copies(&renames);
            for (_, dest) in &renames {
                if dest.exists() && !renames.iter().any(|(from, _)| from == dest) {
                    anyhow::bail!("{} already exists", dest.display());
                }
            }
            rename_all(&renames)?;
            rewrite_links(root, &renames);
            out.links = true;
            out.moves = renames;
            // Titled as what it is, so the tree tells the two apart.
            if let Ok(before) = fs::read_to_string(&to) {
                let title = front_title(&before).unwrap_or_else(|| display_title(&original, None));
                if let Some(after) = retitle(&before, &format!("{title} — {label}")) {
                    write_atomic(&to, &after)?;
                    out.texts.push((to.clone(), before, after));
                }
            }
            out.kept = Some(to);
        }
    }
    Ok(out)
}

/// The `title:` in a file's frontmatter, if it has one.
fn front_title(raw: &str) -> Option<String> {
    let (front, _) = split_frontmatter(raw);
    front.as_deref().and_then(|f| front_get(f, "title"))
}

/// Carry out moves that were done once before, or take them back (pass them
/// reversed). Refuses before touching anything if a source has gone or a
/// destination is taken. `links` rewrites `[[links]]` to follow, as a move or
/// rename did the first time; a trip to or from the trash leaves them alone.
pub fn apply_moves(root: &Path, renames: &[(PathBuf, PathBuf)], links: bool) -> Result<()> {
    for (from, to) in renames {
        if !from.exists() {
            anyhow::bail!(
                "{} isn't there any more",
                from.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        if to.exists() && !renames.iter().any(|(f, _)| f == to) {
            anyhow::bail!(
                "{} is already taken",
                to.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        // A neighbour that differs only in capitals or accents, and isn't
        // itself moving out of the way, is the same file to most clients.
        if let (Some(dir), Some(name)) = (to.parent(), to.file_name())
            && let Some(other) = crate::names::clash(dir, &name.to_string_lossy(), Some(from))
            && !renames
                .iter()
                .any(|(f, _)| *f == other || crate::names::same_file(f, &other))
        {
            anyhow::bail!("{}", clash_message(&name.to_string_lossy(), &other));
        }
    }
    rename_all(renames)?;
    if links {
        rewrite_links(root, renames);
    }
    Ok(())
}

/// What a thing is called while a move is under way. The original name rides
/// along, so a move cut short (a crash, a power cut) can be put back by
/// [`restore_stranded`] under the name it had.
const MOVING: &str = ".grimoire-moving-";

/// Rename in two passes through temporary names, so a swap (or a shift along)
/// never collides with itself. All or nothing: if any rename fails, the ones
/// already made are undone, so nothing is left hidden under a temporary name.
fn rename_all(renames: &[(PathBuf, PathBuf)]) -> Result<()> {
    let renames = &with_copies(renames);
    let pid = std::process::id();
    // (where it was, where it waits, where it's going)
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    let unstage = |staged: &[(PathBuf, PathBuf, PathBuf)]| {
        for (from, tmp, _) in staged.iter().rev() {
            let _ = fs::rename(tmp, from);
        }
    };
    let host = moving_host();
    for (i, (from, to)) in renames.iter().enumerate() {
        let name = from.file_name().unwrap_or_default().to_string_lossy();
        let tmp = from.with_file_name(format!("{MOVING}{i}-{pid}~{host}-{name}"));
        if let Err(e) = fs::rename(from, &tmp) {
            unstage(&staged);
            return Err(e).with_context(|| format!("moving {}", from.display()));
        }
        staged.push((from.clone(), tmp, to.clone()));
    }
    let mut made: Vec<PathBuf> = Vec::new();
    for (k, (_, tmp, to)) in staged.iter().enumerate() {
        let landed = (|| -> Result<()> {
            if let Some(d) = to.parent() {
                // Note each folder this creates, so a rollback can take it away.
                let mut missing: Vec<PathBuf> = d
                    .ancestors()
                    .take_while(|a| !a.exists())
                    .map(Path::to_path_buf)
                    .collect();
                fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
                made.append(&mut missing);
            }
            fs::rename(tmp, to).with_context(|| format!("moving to {}", to.display()))
        })();
        if let Err(e) = landed {
            for (_, tmp, to) in staged[..k].iter().rev() {
                let _ = fs::rename(to, tmp);
            }
            unstage(&staged);
            made.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
            for d in &made {
                let _ = fs::remove_dir(d);
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Bring back anything a move left under its temporary name — a crash or a
/// power cut between the two passes of [`rename_all`] — so a scene can never
/// silently vanish from the tree. Each goes back beside where it was waiting,
/// under its old name (or that name with "recovered" if the old one has since
/// been taken). Returns where they went.
pub fn restore_stranded(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    find_stranded(root, &mut found);
    let mut out = Vec::new();
    for tmp in found {
        let raw = tmp
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let rest = &raw[MOVING.len()..];
        // "<i>-<pid>~<host>-<name>"; before the machine rode along it was
        // "<i>-<pid>-<name>", and before that only "<i>-<pid>".
        let mut parts = rest.splitn(3, '-');
        let (_, who, name) = (parts.next(), parts.next(), parts.next());
        let (pid, host) = match who.and_then(|w| w.split_once('~')) {
            Some((pid, host)) => (Some(pid), Some(host)),
            None => (who, None),
        };
        // Another machine's move, arrived through a sync app: that machine
        // puts it back itself if it was cut short. Restoring it here would
        // make a second copy of the scene the moment the move finishes there.
        if host.is_some_and(|h| h != moving_host()) {
            continue;
        }
        if pid
            .and_then(|p| p.parse::<u32>().ok())
            .is_some_and(|p| p != std::process::id() && process_alive(p))
        {
            continue; // another Grimoire is in the middle of this move
        }
        let name = match name {
            Some(n) if !n.is_empty() => n.to_string(),
            _ if tmp.is_dir() => format!("Recovered-{rest}"),
            _ => format!("Recovered-{rest}.md"),
        };
        let mut to = tmp.with_file_name(&name);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !tmp.is_dir() => (s.to_string(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let mut n = 1;
        while to.symlink_metadata().is_ok() {
            let tag = if n == 1 {
                "recovered".to_string()
            } else {
                format!("recovered-{n}")
            };
            to = tmp.with_file_name(format!("{stem}-{tag}{ext}"));
            n += 1;
        }
        if fs::rename(&tmp, &to).is_ok() {
            out.push(to);
        }
    }
    out
}

/// This machine, as it's written into a moving name: letters and digits only,
/// so the name still splits on its dashes.
fn moving_host() -> String {
    let host: String = crate::resume::machine_name()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if host.is_empty() { "here".into() } else { host }
}

fn find_stranded(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(MOVING) {
            out.push(path);
        } else if name != ".git" && e.file_type().is_ok_and(|t| t.is_dir()) {
            find_stranded(&path, out);
        }
    }
}

/// Is that process still running? Only answerable cheaply on Linux; elsewhere
/// assume not, since a move takes milliseconds.
fn process_alive(pid: u32) -> bool {
    cfg!(target_os = "linux") && Path::new(&format!("/proc/{pid}")).exists()
}

/// Point `[[links]]` at the new names. Both Obsidian forms are handled: a bare
/// file name (`[[02-Low-Tide]]`) and a path from the book's root
/// (`[[manuscript/01-Act-One/02-Low-Tide|Low Tide]]`), including paths through
/// a renamed folder. Returns how many files changed.
pub fn rewrite_links(root: &Path, renames: &[(PathBuf, PathBuf)]) -> usize {
    let rel = |p: &Path| -> String {
        let r = p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        r.strip_suffix(".md").map(str::to_string).unwrap_or(r)
    };
    let stem = |p: &Path| -> String {
        let n = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        n.strip_suffix(".md").map(str::to_string).unwrap_or(n)
    };
    let pairs: Vec<(String, String, String, String)> = renames
        .iter()
        .map(|(f, t)| (rel(f), rel(t), stem(f), stem(t)))
        .collect();

    let mut files = Vec::new();
    collect_md(root, &mut files);
    let mut changed = 0;
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        if !text.contains("[[") {
            continue;
        }
        let new = relink(&text, &pairs);
        if new != text && write_atomic(&file, &new).is_ok() {
            changed += 1;
        }
    }
    changed
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            collect_md(&p, out);
        } else if name.ends_with(".md") {
            out.push(p);
        }
    }
}

/// Rewrite the targets of every `[[…]]` in `text`. The target is the part
/// before `|` or `#`.
fn relink(text: &str, pairs: &[(String, String, String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("[[") {
        out.push_str(&rest[..open + 2]);
        rest = &rest[open + 2..];
        let Some(close) = rest.find("]]") else { break };
        let inner = &rest[..close];
        let cut = inner.find(['|', '#']).unwrap_or(inner.len());
        let (target, tail) = inner.split_at(cut);
        let mut new_target = target.to_string();
        for (old_rel, new_rel, old_stem, new_stem) in pairs {
            if new_target == *old_rel {
                new_target = new_rel.clone();
                break;
            } else if new_target == *old_stem {
                new_target = new_stem.clone();
                break;
            } else if let Some(below) = new_target.strip_prefix(&format!("{old_rel}/")) {
                new_target = format!("{new_rel}/{below}");
                break;
            }
        }
        out.push_str(&new_target);
        out.push_str(tail);
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

/// One more than the highest leading number among the entries in `dir`.
fn next_number(dir: &Path) -> Result<usize> {
    let mut top = 0;
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = e?.path();
        // A scene iCloud hasn't downloaded still has its number.
        let path = icloud_stub_for(&path).unwrap_or(path);
        if let Some(n) = leading_number(&path) {
            top = top.max(n);
        }
    }
    Ok(top + 1)
}

/// iCloud (before macOS 14) replaces a file it hasn't downloaded with a
/// hidden stub: `01-Scene.md` becomes `.01-Scene.md.icloud`. The file a stub
/// stands for, if `path` is one.
pub fn icloud_stub_for(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let real = name.strip_prefix('.')?.strip_suffix(".icloud")?;
    real.ends_with(".md").then(|| path.with_file_name(real))
}

/// Where iCloud's stub for `path` would be.
pub fn icloud_stub(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.icloud"))
}

/// The `1` in `01-the-archive.md`, which is what orders the tree.
pub fn leading_number(path: &Path) -> Option<usize> {
    let name = path.file_name()?.to_string_lossy();
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn new_items_are_numbered_after_what_is_there() {
        let d = temp_dir("number");
        fs::write(d.join("01-opening.md"), "x").unwrap();
        fs::write(d.join("02-gravel.md"), "x").unwrap();
        fs::write(d.join(".DS_Store"), "").unwrap();
        let scene = create(&d, "The Lamp", false).unwrap();
        assert_eq!(scene.file_name().unwrap(), "03-The-Lamp.md");
        let folder = create(&d, "Part Two", true).unwrap();
        assert!(folder.is_dir());
        assert_eq!(folder.file_name().unwrap(), "04-Part-Two");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_scene_keeps_its_title_exactly_as_typed() {
        let d = temp_dir("title");
        let p = create(&d, "  Wren's \"last\" letter: part 2 ", false).unwrap();
        let raw = fs::read_to_string(&p).unwrap();
        let (front, body) = split_frontmatter(&raw);
        assert_eq!(
            display_title(&p, front.as_deref()),
            "Wren's 'last' letter: part 2"
        );
        assert!(body.is_empty(), "a new scene starts blank");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_folder_reads_back_as_its_name() {
        let d = temp_dir("folder");
        let f = create(&d, "The Lamp's Light", true).unwrap();
        assert_eq!(display_title(&f, None), "The Lamp's Light");
        let blank = create(&d, "   ", true).unwrap();
        assert_eq!(blank.file_name().unwrap(), "02-Untitled");
        fs::remove_dir_all(&d).unwrap();
    }

    /// The shape a new book arrives in: three pages, twenty-seven chapters,
    /// three scenes in each, and no words written for you.
    #[test]
    fn the_template_is_three_pages_of_nine_chapters() {
        let d = temp_dir("template");
        scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        let containers = |depth: usize| {
            p.nodes
                .iter()
                .filter(|n| n.kind == Kind::Container && n.in_manuscript && n.depth == depth)
                .count()
        };
        assert_eq!(containers(1), PARTS, "parts");
        assert_eq!(containers(2), PARTS * CHAPTERS_PER_PART, "chapters");
        let scenes: Vec<&Node> = p
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Scene && n.in_manuscript)
            .collect();
        assert_eq!(scenes.len(), PARTS * CHAPTERS_PER_PART * SCENES_PER_CHAPTER);
        assert_eq!(p.total_words(), 0, "the scenes start empty");

        // Named straight through the book, and spelled the way they'd be read.
        let titles = |depth: usize| -> Vec<String> {
            p.nodes
                .iter()
                .filter(|n| n.kind == Kind::Container && n.in_manuscript && n.depth == depth)
                .map(|n| n.title.clone())
                .collect()
        };
        assert_eq!(titles(1), ["Part One", "Part Two", "Part Three"]);
        assert_eq!(titles(2)[0], "Chapter One");
        assert_eq!(titles(2)[9], "Chapter Ten");
        assert_eq!(titles(2)[26], "Chapter Twenty-Seven");
        assert_eq!(p.meta.part_word(), "Part", "a new book counts in parts");
        assert!(
            fs::read_to_string(d.join("novel.toml"))
                .unwrap()
                .contains("part_label = \"Part\""),
            "and says so, where it can be changed"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    /// Every section, in this order, each one a row that folds.
    #[test]
    fn a_new_book_has_every_section_in_order() {
        let d = temp_dir("sections");
        scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        let rows: Vec<(String, Kind)> = p
            .roots
            .iter()
            .map(|&r| (p.nodes[r].title.clone(), p.nodes[r].kind))
            .collect();
        let want: Vec<(String, Kind)> = Area::ALL
            .iter()
            .map(|a| {
                (
                    a.title().to_string(),
                    if *a == Area::Format {
                        Kind::Scene
                    } else {
                        Kind::Category
                    },
                )
            })
            .collect();
        assert_eq!(rows, want);

        let under = |area: Area| -> Vec<String> {
            let r = p
                .roots
                .iter()
                .find(|&&r| p.nodes[r].area == area)
                .copied()
                .unwrap();
            p.nodes[r]
                .children
                .iter()
                .map(|&c| p.nodes[c].title.clone())
                .collect()
        };
        assert_eq!(under(Area::FrontMatter), FRONT_MATTER_FORMATS);
        assert_eq!(under(Area::Research), ["Sample Output"]);
        assert_eq!(
            under(Area::Templates),
            ["Character Sketch", "Setting Sketch"]
        );
        assert!(under(Area::Characters).is_empty() && under(Area::Places).is_empty());

        // Starter front matter is there, but nothing of it goes into a book yet.
        let fm: Vec<&Node> = p
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Scene && n.front_matter)
            .collect();
        assert_eq!(fm.len(), 7);
        assert!(fm.iter().all(|n| !n.compile));
        assert!(
            p.nodes
                .iter()
                .filter(|n| n.kind == Kind::Scene && n.area == Area::Templates)
                .all(|n| !n.area.is_notebook())
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn every_section_folds_and_the_trash_starts_folded() {
        let d = temp_dir("fold");
        scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        for &r in &p.roots {
            let n = &p.nodes[r];
            if n.kind == Kind::Category {
                assert_eq!(n.expanded, n.area != Area::Trash, "{}", n.title);
                assert!(n.children.iter().all(|&c| p.nodes[c].depth == 1));
            }
        }
        let visible = p.visible();
        let fm = p
            .roots
            .iter()
            .copied()
            .find(|&r| p.nodes[r].area == Area::FrontMatter)
            .unwrap();
        let editions = &p.nodes[fm].children;
        assert!(
            editions.iter().all(|&e| !p.nodes[e].expanded),
            "each edition starts folded"
        );
        assert!(!visible.iter().any(|&v| p.nodes[v].title == "Copyright"));
        fs::remove_dir_all(&d).unwrap();
    }

    /// A book from before sections: parts, and a notebook holding everything.
    fn old_book(tag: &str) -> PathBuf {
        let d = temp_dir(tag);
        let put = |rel: &str, text: &str| {
            let path = d.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        fs::write(d.join("novel.toml"), "title = \"Old\"\n").unwrap();
        put(
            "manuscript/01-part-one/01-chapter-one/01-opening.md",
            "---\ntitle: Opening\n---\n\nWren waits. See [[notes/01-Characters/01-Wren]].\n",
        );
        put(
            "manuscript/02-Part-Two/01-Chapter-Two/01-later.md",
            "Later.\n",
        );
        put(
            "notes/01-Characters/01-Wren.md",
            "---\ntitle: Wren\n---\n\nSee [[manuscript/02-Part-Two/01-Chapter-Two/01-later]].\n",
        );
        put("notes/places/harbour.md", "Salt.\n");
        put("notes/03-Magic-System/01-rules.md", "Rules.\n");
        put("notes/07-Notes/01-stray.md", "A thought.\n");
        put("notes/08-Research/01-tides.md", "Tides.\n");
        d
    }

    #[test]
    fn opening_an_old_book_brings_it_into_sections_without_losing_anything() {
        let d = old_book("upgrade");
        let done = upgrade(&d).unwrap();
        assert!(!done.is_empty());

        assert!(d.join("characters/01-Wren.md").exists());
        assert!(d.join("places/harbour.md").exists());
        assert!(d.join("research/01-tides.md").exists());
        assert!(
            d.join("notes/01-stray.md").exists(),
            "a Notes folder inside notes empties into it"
        );
        assert!(!d.join("notes/07-Notes").exists());
        assert!(
            d.join("notes/03-Magic-System/01-rules.md").exists(),
            "other notebook folders stay"
        );
        assert!(
            d.join("manuscript/01-part-one/01-chapter-one/01-opening.md")
                .exists(),
            "parts keep their names"
        );
        assert!(
            d.join("manuscript/02-Part-Two/01-Chapter-Two/01-later.md")
                .exists()
        );
        assert!(
            d.join("Novel-Format.md").exists()
                && d.join("template-sheets/01-Character-Sketch.md").exists()
        );
        assert!(d.join("front-matter/01-Manuscript-Format").is_dir());
        let fm_files = fs::read_dir(d.join("front-matter/02-Paperback"))
            .unwrap()
            .count();
        assert_eq!(
            fm_files, 0,
            "an existing book gets no starter pages in its exports"
        );

        // Links follow the notebook's move, and the ones into the
        // manuscript still point where they did.
        let opening =
            fs::read_to_string(d.join("manuscript/01-part-one/01-chapter-one/01-opening.md"))
                .unwrap();
        assert!(opening.contains("[[characters/01-Wren]]"), "{opening}");
        let wren = fs::read_to_string(d.join("characters/01-Wren.md")).unwrap();
        assert!(
            wren.contains("[[manuscript/02-Part-Two/01-Chapter-Two/01-later]]"),
            "{wren}"
        );

        let p = Project::load(&d).unwrap();
        assert_eq!(p.meta.part_word(), "Part");
        assert!(
            !fs::read_to_string(d.join("novel.toml"))
                .unwrap()
                .contains("part_label"),
            "a book of parts needs no label written"
        );
        assert_eq!(p.total_words(), 5);
        assert!(upgrade(&d).unwrap().is_empty(), "it happens once");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_book_that_counts_in_acts_keeps_them() {
        let d = old_book("upgrade-acts");
        fs::write(
            d.join("novel.toml"),
            "title = \"Old\"\npart_label = \"Act\"\n",
        )
        .unwrap();
        upgrade(&d).unwrap();
        assert!(
            d.join("manuscript/01-part-one").exists(),
            "its own word was chosen; parts aren't touched"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    /// 0.3.0–0.3.6 renamed a label-less book's parts to pages. It keeps
    /// them, and writes the word down so new ones are pages too.
    #[test]
    fn a_book_whose_parts_became_pages_keeps_counting_in_pages() {
        let d = temp_dir("upgrade-pages");
        let put = |rel: &str, text: &str| {
            let path = d.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        fs::write(d.join("novel.toml"), "title = \"Paged\"").unwrap();
        put("manuscript/01-Page-One/01-Chapter-One/01-a.md", "A.\n");
        put("manuscript/02-Page-Two/01-Chapter-Two/01-b.md", "B.\n");
        let done = upgrade(&d).unwrap();
        assert!(done.iter().any(|l| l.contains("pages")), "{done:?}");
        assert!(d.join("manuscript/01-Page-One").is_dir(), "nothing renamed");
        let toml = fs::read_to_string(d.join("novel.toml")).unwrap();
        assert!(toml.starts_with("title = \"Paged\"\n"), "{toml}");
        let p = Project::load(&d).unwrap();
        assert_eq!(p.meta.title, "Paged");
        assert_eq!(p.meta.part_word(), "Page");
        assert!(upgrade(&d).unwrap().is_empty(), "written once");
        fs::remove_dir_all(&d).unwrap();
    }

    /// The editor shows Markdown as typed, so the guide a new book opens on
    /// has no emphasis marks or code quotes in it.
    #[test]
    fn the_novel_format_guide_reads_as_plain_prose() {
        let body = starter::NOVEL_FORMAT.split("---\n").nth(2).unwrap();
        assert!(!body.contains("**") && !body.contains('`'), "{body}");
        assert!(body.contains("parts hold chapters"), "{body}");
    }

    #[test]
    fn a_page_folder_is_named_as_one() {
        let is = |n: &str| is_page_folder(Path::new(n));
        assert!(is("02-Page-Two") && is("01-page-one") && is("PAGE III"));
        assert!(!is("03-Pageant") && !is("01-Part-One") && !is("04-Chapter-One"));
    }

    #[test]
    fn the_trash_is_in_the_tree_from_the_start_and_counts_for_nothing() {
        let d = temp_dir("trash-shown");
        scaffold(&d).unwrap();
        let scene = Project::load(&d)
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.kind == Kind::Scene)
            .map(|n| n.path.clone())
            .unwrap();
        fs::write(&scene, "---\ntitle: \"Scene One\"\n---\n\nOne two three.\n").unwrap();
        trash(&d, &scene).unwrap();

        let p = Project::load(&d).unwrap();
        assert!(
            p.nodes
                .iter()
                .any(|n| n.kind == Kind::Category && n.title == "Trash"),
            "the section shows"
        );
        let gone = p
            .nodes
            .iter()
            .position(|n| n.title.contains("Scene One") && n.path.starts_with(trash_dir(&d)))
            .expect("the deleted scene is listed under it");
        assert!(p.in_trash(gone));
        assert!(!p.nodes[gone].in_manuscript);
        assert_eq!(
            p.total_words(),
            0,
            "trashed words don't count toward the draft"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_spelled_out_chapter_keeps_its_hyphen_but_a_title_keeps_its_spaces() {
        assert_eq!(
            display_title(Path::new("07-Chapter-Twenty-Seven"), None),
            "Chapter Twenty-Seven"
        );
        assert_eq!(
            display_title(Path::new("01-the-archive.md"), None),
            "The Archive"
        );
        assert_eq!(display_title(Path::new("03-Act-Three"), None), "Act Three");
    }

    #[test]
    fn renaming_keeps_the_number_and_rewrites_the_title() {
        let d = temp_dir("rename");
        let scene = create(&d, "Opening", false).unwrap();
        let to = rename(&scene, "The Gravel Road").unwrap();
        assert_eq!(to.file_name().unwrap(), "01-The-Gravel-Road.md");
        assert!(!scene.exists());
        let raw = fs::read_to_string(&to).unwrap();
        let (front, body) = split_frontmatter(&raw);
        assert_eq!(display_title(&to, front.as_deref()), "The Gravel Road");
        assert!(
            front.unwrap().contains("status: draft"),
            "the rest of the frontmatter stays"
        );
        assert!(body.is_empty());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn renaming_a_folder_carries_everything_inside_it() {
        let d = temp_dir("rename-folder");
        let part = create(&d, "Part One", true).unwrap();
        create(&part, "Opening", false).unwrap();
        let to = rename(&part, "Act One").unwrap();
        assert_eq!(to.file_name().unwrap(), "01-Act-One");
        assert!(to.join("01-Opening.md").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn renaming_to_the_name_it_already_has_is_not_an_error() {
        let d = temp_dir("rename-same");
        let scene = create(&d, "Opening", false).unwrap();
        assert_eq!(rename(&scene, "Opening").unwrap(), scene);
        assert!(rename(&scene, "   ").is_err(), "a blank name is refused");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn deleting_moves_it_to_the_trash_rather_than_destroying_it() {
        let d = temp_dir("trash");
        let scene = create(&d, "Cut This", false).unwrap();
        let gone = trash(&d, &scene).unwrap();
        assert!(!scene.exists());
        assert!(gone.exists(), "it's still on disk");
        assert!(gone.starts_with(d.join(".grimoire/trash")));
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_move_that_fails_halfway_puts_everything_back() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp_dir("rollback");
        let (src, locked) = (d.join("src"), d.join("locked"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&locked).unwrap();
        fs::write(src.join("01-A.md"), "scene a").unwrap();
        fs::write(src.join("02-B.md"), "scene b").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let renames = vec![
            (src.join("01-A.md"), src.join("03-A.md")),
            (src.join("02-B.md"), locked.join("new").join("01-B.md")),
        ];
        let result = rename_all(&renames);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err(), "the locked folder refuses");
        assert_eq!(fs::read_to_string(src.join("01-A.md")).unwrap(), "scene a");
        assert_eq!(fs::read_to_string(src.join("02-B.md")).unwrap(), "scene b");
        assert!(
            !src.join("03-A.md").exists(),
            "the half that landed went back"
        );
        let left: Vec<String> = fs::read_dir(&src)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(MOVING))
            .collect();
        assert!(
            left.is_empty(),
            "nothing hidden under a temporary name: {left:?}"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn what_a_cut_short_move_left_hidden_comes_back() {
        let d = temp_dir("stranded");
        let ch = d.join("manuscript/01-Act-One/01-Chapter-One");
        fs::create_dir_all(&ch).unwrap();
        // A crash between the passes: one scene still under its moving name.
        fs::write(
            ch.join(".grimoire-moving-0-4000000-02-Low-Tide.md"),
            "the tide",
        )
        .unwrap();
        // Its old name has since been taken by something else.
        fs::write(
            ch.join(".grimoire-moving-1-4000000-01-Gravel.md"),
            "gravel, hidden",
        )
        .unwrap();
        fs::write(ch.join("01-Gravel.md"), "gravel, visible").unwrap();
        // The old form, from before names rode along.
        fs::write(ch.join(".grimoire-moving-2-4000000"), "nameless").unwrap();
        let back = restore_stranded(&d);
        assert_eq!(back.len(), 3, "{back:?}");
        assert_eq!(
            fs::read_to_string(ch.join("02-Low-Tide.md")).unwrap(),
            "the tide"
        );
        assert_eq!(
            fs::read_to_string(ch.join("01-Gravel.md")).unwrap(),
            "gravel, visible"
        );
        assert_eq!(
            fs::read_to_string(ch.join("01-Gravel-recovered.md")).unwrap(),
            "gravel, hidden"
        );
        assert_eq!(
            fs::read_to_string(ch.join("Recovered-2-4000000.md")).unwrap(),
            "nameless"
        );
        let p = Project::load(&d).unwrap();
        assert!(
            p.nodes.iter().any(|n| n.path == ch.join("02-Low-Tide.md")),
            "it's in the tree"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_move_another_grimoire_is_making_right_now_is_left_alone() {
        let d = temp_dir("stranded-live");
        // pid 1 is always running.
        fs::write(d.join(".grimoire-moving-0-1-01-Busy.md"), "mid-move").unwrap();
        assert!(restore_stranded(&d).is_empty());
        assert!(d.join(".grimoire-moving-0-1-01-Busy.md").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn two_deletes_of_the_same_name_in_one_second_both_stay_in_the_trash() {
        let d = temp_dir("trash-same-name");
        let (a, b) = (d.join("one"), d.join("two"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("01-Scene-One.md"), "the first chapter's words").unwrap();
        fs::write(b.join("01-Scene-One.md"), "the second chapter's words").unwrap();
        let first = trash(&d, &a.join("01-Scene-One.md")).unwrap();
        let second = trash(&d, &b.join("01-Scene-One.md")).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            fs::read_to_string(&first).unwrap(),
            "the first chapter's words"
        );
        assert_eq!(
            fs::read_to_string(&second).unwrap(),
            "the second chapter's words"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    fn names_in(dir: &Path) -> Vec<String> {
        tree_entries(dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect()
    }

    fn move_book(tag: &str) -> PathBuf {
        let d = temp_dir(tag);
        let ch = |a: &str, c: &str| d.join("manuscript").join(a).join(c);
        for (a, c, scenes) in [
            (
                "01-Act-One",
                "01-Chapter-One",
                vec!["01-Gravel.md", "02-Low-Tide.md"],
            ),
            (
                "01-Act-One",
                "02-Chapter-Two",
                vec!["01-Ashfall.md", "02-The-Crossing.md"],
            ),
            ("02-Act-Two", "03-Chapter-Three", vec!["01-Lantern.md"]),
        ] {
            fs::create_dir_all(ch(a, c)).unwrap();
            for s in scenes {
                fs::write(
                    ch(a, c).join(s),
                    format!("---\ntitle: {s}\n---\n\nwords of {s}\n"),
                )
                .unwrap();
            }
        }
        fs::create_dir_all(d.join("notes/01-Characters")).unwrap();
        fs::write(
            d.join("notes/01-Characters/01-Wren.md"),
            "See [[02-Low-Tide]], [[manuscript/01-Act-One/01-Chapter-One/01-Gravel|the lot]] and [[01-Act-One/02-Chapter-Two/01-Ashfall#Bells]].\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn moving_within_a_chapter_swaps_numbers_and_follows_links() {
        let d = move_book("move-swap");
        let ch1 = d.join("manuscript/01-Act-One/01-Chapter-One");
        let m = move_item(&d, &ch1.join("02-Low-Tide.md"), true).unwrap();
        assert_eq!(names_in(&ch1), ["01-Low-Tide.md", "02-Gravel.md"]);
        assert_eq!(m.target, ch1.join("01-Low-Tide.md"));
        assert_eq!(m.renames.len(), 2, "only the two that swapped");
        assert!(
            fs::read_to_string(ch1.join("01-Low-Tide.md"))
                .unwrap()
                .contains("words of 02-Low-Tide.md"),
            "content travels with the name"
        );
        let note = fs::read_to_string(d.join("notes/01-Characters/01-Wren.md")).unwrap();
        assert!(note.contains("[[01-Low-Tide]]"));
        assert!(note.contains("[[manuscript/01-Act-One/01-Chapter-One/02-Gravel|the lot]]"));
        assert_eq!(m.links, 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_top_scene_moving_up_goes_to_the_end_of_the_previous_chapter() {
        let d = move_book("move-up-cross");
        let ch1 = d.join("manuscript/01-Act-One/01-Chapter-One");
        let ch2 = d.join("manuscript/01-Act-One/02-Chapter-Two");
        move_item(&d, &ch2.join("01-Ashfall.md"), true).unwrap();
        assert_eq!(
            names_in(&ch1),
            ["01-Gravel.md", "02-Low-Tide.md", "03-Ashfall.md"]
        );
        assert_eq!(
            names_in(&ch2),
            ["02-The-Crossing.md"],
            "the gap is left; nothing else renamed"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_bottom_scene_moving_down_goes_to_the_start_of_the_next_chapter_across_acts() {
        let d = move_book("move-down-cross");
        let ch2 = d.join("manuscript/01-Act-One/02-Chapter-Two");
        let ch3 = d.join("manuscript/02-Act-Two/03-Chapter-Three");
        let m = move_item(&d, &ch2.join("02-The-Crossing.md"), false).unwrap();
        assert_eq!(
            names_in(&ch3),
            ["01-The-Crossing.md", "02-Lantern.md"],
            "the first there shifts along to make room"
        );
        assert_eq!(m.target, ch3.join("01-The-Crossing.md"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_chapter_moves_between_acts_and_links_through_it_follow() {
        let d = move_book("move-chapter");
        let act1 = d.join("manuscript/01-Act-One");
        let act2 = d.join("manuscript/02-Act-Two");
        move_item(&d, &act1.join("02-Chapter-Two"), false).unwrap();
        assert_eq!(names_in(&act1), ["01-Chapter-One"]);
        assert_eq!(names_in(&act2), ["02-Chapter-Two", "03-Chapter-Three"]);
        assert!(
            act2.join("02-Chapter-Two/01-Ashfall.md").exists(),
            "scenes travel inside"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn nothing_moves_past_the_ends() {
        let d = move_book("move-ends");
        assert!(move_item(&d, &d.join("manuscript/01-Act-One"), true).is_err());
        assert!(
            move_item(
                &d,
                &d.join("manuscript/02-Act-Two/03-Chapter-Three/01-Lantern.md"),
                false
            )
            .is_err()
        );
        assert!(move_item(&d, &d.join("manuscript"), true).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_move_can_be_taken_back_and_done_again_with_its_links() {
        let d = move_book("move-undo");
        let ch1 = d.join("manuscript/01-Act-One/01-Chapter-One");
        let m = move_item(&d, &ch1.join("02-Low-Tide.md"), true).unwrap();
        let back: Vec<(PathBuf, PathBuf)> = m
            .renames
            .iter()
            .map(|(f, t)| (t.clone(), f.clone()))
            .collect();
        apply_moves(&d, &back, true).unwrap();
        assert_eq!(names_in(&ch1), ["01-Gravel.md", "02-Low-Tide.md"]);
        let note = fs::read_to_string(d.join("notes/01-Characters/01-Wren.md")).unwrap();
        assert!(
            note.contains("[[02-Low-Tide]]"),
            "links follow it back: {note}"
        );
        apply_moves(&d, &m.renames, true).unwrap();
        assert_eq!(names_in(&ch1), ["01-Low-Tide.md", "02-Gravel.md"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_delete_comes_back_out_of_the_trash_unless_its_place_is_taken() {
        let d = temp_dir("untrash");
        let scene = create(&d, "Cut This", false).unwrap();
        let gone = trash(&d, &scene).unwrap();
        apply_moves(&d, &[(gone.clone(), scene.clone())], false).unwrap();
        assert!(scene.exists() && !gone.exists());

        let gone = trash(&d, &scene).unwrap();
        fs::write(&scene, "something new in its place").unwrap();
        assert!(
            apply_moves(&d, &[(gone.clone(), scene.clone())], false).is_err(),
            "never overwrites"
        );
        assert!(gone.exists(), "and touches nothing when it refuses");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_note_without_a_number_gets_one_so_it_can_move() {
        let d = temp_dir("move-unnumbered");
        let chars = d.join("characters");
        fs::create_dir_all(&chars).unwrap();
        fs::write(chars.join("ann.md"), "Ann.\n").unwrap();
        fs::write(chars.join("bo.md"), "Bo, see [[ann]].\n").unwrap();
        let m = move_item(&d, &chars.join("bo.md"), true).unwrap();
        assert_eq!(names_in(&chars), ["01-bo.md", "02-ann.md"]);
        assert_eq!(
            fs::read_to_string(chars.join("01-bo.md")).unwrap(),
            "Bo, see [[02-ann]].\n"
        );
        // Undo is one set of moves back to the original names.
        let back: Vec<(PathBuf, PathBuf)> = m
            .renames
            .iter()
            .map(|(f, t)| (t.clone(), f.clone()))
            .collect();
        apply_moves(&d, &back, true).unwrap();
        assert_eq!(names_in(&chars), ["ann.md", "bo.md"]);
        assert_eq!(
            fs::read_to_string(chars.join("bo.md")).unwrap(),
            "Bo, see [[ann]].\n"
        );
        // A lone note with nowhere to go is left exactly as it was.
        fs::create_dir_all(d.join("places")).unwrap();
        fs::write(d.join("places/harbour.md"), "Salt.\n").unwrap();
        assert!(move_item(&d, &d.join("places/harbour.md"), true).is_err());
        assert_eq!(names_in(&d.join("places")), ["harbour.md"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sections_keep_the_order_they_are_given_with_the_trash_last() {
        let d = temp_dir("section-order");
        scaffold(&d).unwrap();
        let order = [Area::Characters, Area::Trash, Area::Manuscript];
        save_section_order(&d, &order).unwrap();
        let toml = fs::read_to_string(d.join("novel.toml")).unwrap();
        assert!(
            toml.contains("part_label = \"Part\""),
            "the rest of novel.toml stays"
        );
        let p = Project::load(&d).unwrap();
        let areas: Vec<Area> = p.roots.iter().map(|&r| p.nodes[r].area).collect();
        assert_eq!(&areas[..2], [Area::Characters, Area::Manuscript]);
        assert_eq!(areas.last(), Some(&Area::Trash));
        assert_eq!(areas.len(), Area::ALL.len());
        save_section_order(&d, &Area::ALL).unwrap();
        assert_eq!(
            fs::read_to_string(d.join("novel.toml"))
                .unwrap()
                .matches("sections =")
                .count(),
            1
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn swapping_two_scenes_with_the_same_title_does_not_collide() {
        let d = temp_dir("move-same");
        fs::write(d.join("01-Scene.md"), "first").unwrap();
        fs::write(d.join("02-Scene.md"), "second").unwrap();
        let m = move_item(&d, &d.join("02-Scene.md"), true).unwrap();
        assert_eq!(fs::read_to_string(d.join("01-Scene.md")).unwrap(), "second");
        assert_eq!(fs::read_to_string(d.join("02-Scene.md")).unwrap(), "first");
        assert_eq!(m.target, d.join("01-Scene.md"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn setting_a_frontmatter_value_touches_only_that_line() {
        let front = "title: \"Gravel\"\n# a comment\npov:\nmood: grey\ntags:\n  - lot\n";
        let out = set_front(Some(front), "pov", "Wren");
        assert_eq!(
            out,
            "title: \"Gravel\"\n# a comment\npov: Wren\nmood: grey\ntags:\n  - lot\n"
        );
        let out = set_front(Some(&out), "synopsis", "Confronts the caretaker: again");
        assert!(
            out.ends_with("synopsis: \"Confronts the caretaker: again\"\n"),
            "added at the end, quoted for the colon"
        );
        assert_eq!(
            front_get(&out, "synopsis").as_deref(),
            Some("Confronts the caretaker: again")
        );
        let out = set_front(Some(&out), "pov", "");
        assert!(out.contains("\npov:\n"), "clearing leaves the key");
        assert_eq!(set_front(None, "status", "draft"), "status: draft\n");
    }

    #[test]
    fn the_first_item_in_a_new_folder_is_number_one() {
        let d = temp_dir("empty");
        let p = create(&d.join("new-chapter"), "Opening", false).unwrap();
        assert_eq!(p.file_name().unwrap(), "01-Opening.md");
        fs::remove_dir_all(&d).unwrap();
    }

    // ---- the book changing under us ------------------------------------

    /// A one-scene book: manuscript/01-Act-One/01-Chapter-One/01-Gravel.md.
    fn sync_book(tag: &str) -> (PathBuf, PathBuf) {
        let d = temp_dir(&format!("sync-{tag}"));
        let ch = d.join("manuscript/01-Act-One/01-Chapter-One");
        fs::create_dir_all(&ch).unwrap();
        fs::write(d.join("novel.toml"), "title = \"Sync\"\n").unwrap();
        let scene = ch.join("01-Gravel.md");
        fs::write(
            &scene,
            "---\ntitle: \"Gravel\"\n---\n\nThe lot was empty.\n",
        )
        .unwrap();
        (d, scene)
    }

    fn scene_idx(p: &Project, path: &Path) -> usize {
        p.nodes.iter().position(|n| n.path == path).unwrap()
    }

    #[test]
    fn a_save_never_overwrites_a_change_made_elsewhere() {
        let (d, scene) = sync_book("clash");
        let mut p = Project::load(&d).unwrap();
        let i = scene_idx(&p, &scene);

        // the phone writes the scene; this session is still typing in it
        fs::write(&scene, "---\ntitle: \"Gravel\"\n---\n\nPHONE EDIT.\n").unwrap();
        p.nodes[i].body = "The lot was empty. Then a van.\n".into();
        p.nodes[i].dirty = true;

        let report = p.save_dirty();
        assert!(
            report.saved.is_empty() && report.failed.is_empty(),
            "{report:?}"
        );
        let (at, copy) = &report.parked[0];
        assert_eq!(*at, i);
        // the phone's words are the scene, on disk and in memory
        assert!(fs::read_to_string(&scene).unwrap().contains("PHONE EDIT."));
        assert_eq!(p.nodes[i].body, "PHONE EDIT.\n");
        assert!(!p.nodes[i].dirty);
        // and this session's words are parked beside it, not lost
        assert!(copy.starts_with(scene.parent().unwrap()));
        assert!(fs::read_to_string(copy).unwrap().contains("Then a van."));
        assert!(sync::is_conflict_copy(copy));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_scene_deleted_elsewhere_is_not_brought_back_and_its_words_go_to_the_trash() {
        let (d, scene) = sync_book("gone");
        let mut p = Project::load(&d).unwrap();
        let i = scene_idx(&p, &scene);
        fs::remove_file(&scene).unwrap();
        p.nodes[i].body = "Words typed after it went.\n".into();
        p.nodes[i].dirty = true;

        // Missing once is a sync client replacing it: the words wait.
        let report = p.save_dirty();
        assert_eq!(report.waiting, vec![i]);
        assert!(p.nodes[i].dirty, "still unsaved, not dropped");
        assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Same);
        // Missing twice is gone.
        let DiskChange::GoneParked(parked) = p.check_disk(i, false).unwrap() else {
            panic!("a second miss should park the words in the trash");
        };
        assert!(!scene.exists(), "resurrected at its old path");
        assert!(parked.starts_with(trash_dir(&d)));
        assert!(
            fs::read_to_string(parked)
                .unwrap()
                .contains("Words typed after it went.")
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn an_unreadable_novel_toml_is_never_rewritten_by_the_section_order() {
        use std::os::unix::fs::PermissionsExt;
        let (d, _) = sync_book("sections-unreadable");
        let toml = d.join("novel.toml");
        let before = fs::read(&toml).unwrap();
        fs::set_permissions(&toml, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&toml).is_ok() {
            fs::set_permissions(&toml, fs::Permissions::from_mode(0o644)).unwrap();
            fs::remove_dir_all(&d).unwrap();
            return; // root: permissions don't stop reads
        }
        assert!(save_section_order(&d, &[Area::Notes, Area::Manuscript]).is_err());
        fs::set_permissions(&toml, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(fs::read(&toml).unwrap(), before, "untouched");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_scene_that_turns_unreadable_is_parked_once_not_every_check() {
        use std::os::unix::fs::PermissionsExt;
        let (d, scene) = sync_book("unreadable-park");
        let mut p = Project::load(&d).unwrap();
        let i = scene_idx(&p, &scene);
        p.nodes[i].body = "Mine, unsaved.\n".into();
        p.nodes[i].dirty = true;
        // Changed elsewhere and then unreadable: an online-only file gone
        // offline, say.
        fs::write(&scene, "---\ntitle: \"Gravel\"\n---\n\nTheirs.\n").unwrap();
        fs::set_permissions(&scene, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&scene).is_ok() {
            // Running as root: permissions don't stop reads, nothing to test.
            fs::set_permissions(&scene, fs::Permissions::from_mode(0o644)).unwrap();
            fs::remove_dir_all(&d).unwrap();
            return;
        }
        // Unreadable: the words wait (nothing can be compared yet), and no
        // copy is made however many checks go by.
        assert!(matches!(
            p.check_disk(i, true).unwrap(),
            DiskChange::Unavailable(_)
        ));
        for _ in 0..3 {
            // Said once; after that, nothing new to report.
            assert!(!matches!(
                p.check_disk(i, true).unwrap(),
                DiskChange::Parked(_)
            ));
        }
        assert!(p.nodes[i].dirty, "the words are still here, waiting");
        let copies = |d: &Path| {
            fs::read_dir(d)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().contains(" (from "))
                .count()
        };
        let dir = scene.parent().unwrap().to_path_buf();
        assert_eq!(copies(&dir), 0, "no copy every two seconds");
        // Readable again, and changed elsewhere: parked beside it, once.
        fs::set_permissions(&scene, fs::Permissions::from_mode(0o644)).unwrap();
        let mut parked = 0;
        for _ in 0..4 {
            if matches!(p.check_disk(i, true).unwrap(), DiskChange::Parked(_)) {
                parked += 1;
            }
        }
        assert_eq!(parked, 1);
        assert!(!p.nodes[i].dirty, "the words are in the copy now");
        assert_eq!(copies(&dir), 1, "one copy, not one every check");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_change_on_disk_is_taken_in_or_parked_depending_on_what_is_unsaved() {
        let (d, scene) = sync_book("check");
        let mut p = Project::load(&d).unwrap();
        let i = scene_idx(&p, &scene);
        assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Same);

        // nothing unsaved here: the new version is simply taken in
        fs::write(&scene, "---\ntitle: \"Gravel\"\n---\n\nSynced in.\n").unwrap();
        assert_eq!(p.check_disk(i, true).unwrap(), DiskChange::Adopted);
        assert_eq!(p.nodes[i].body, "Synced in.\n");
        assert_eq!(p.check_disk(i, true).unwrap(), DiskChange::Same);

        // unsaved words here, and it changes again: both are kept
        p.nodes[i].body = "Mine, unsaved.\n".into();
        p.nodes[i].dirty = true;
        fs::write(&scene, "---\ntitle: \"Gravel\"\n---\n\nTheirs again.\n").unwrap();
        let DiskChange::Parked(copy) = p.check_disk(i, true).unwrap() else {
            panic!("should have parked");
        };
        assert_eq!(p.nodes[i].body, "Theirs again.\n");
        assert!(
            fs::read_to_string(&copy)
                .unwrap()
                .contains("Mine, unsaved.")
        );

        // and a scene that vanishes with nothing unsaved is gone — on the
        // second look, not the first
        fs::remove_file(&scene).unwrap();
        assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Same);
        assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Gone);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_file_added_or_removed_elsewhere_makes_the_tree_stale() {
        let (d, scene) = sync_book("stale");
        let p = Project::load(&d).unwrap();
        assert!(!p.tree_is_stale());
        let new = scene.with_file_name("02-From-The-Phone.md");
        fs::write(&new, "New.\n").unwrap();
        assert!(p.tree_is_stale());
        fs::remove_file(&new).unwrap();
        assert!(!p.tree_is_stale());
        fs::remove_file(&scene).unwrap();
        assert!(p.tree_is_stale());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_book_is_not_stale_the_moment_it_is_read() {
        // Otherwise every tick would re-read the whole book.
        let d = temp_dir("sync-scaffold");
        let root = d.join("Book");
        fs::create_dir_all(&root).unwrap();
        scaffold(&root).unwrap();
        let p = Project::load(&root).unwrap();
        assert!(p.nodes.len() > 80);
        assert!(!p.tree_is_stale());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_parked_copy_is_shown_but_never_counted_or_compiled() {
        let (d, scene) = sync_book("parkedcount");
        let before = Project::load(&d).unwrap().total_words();
        let copy = sync::write_conflict_copy(&scene, "Four more words here.\n").unwrap();
        let p = Project::load(&d).unwrap();
        let j = scene_idx(&p, &copy);
        assert!(p.nodes[j].parked && !p.nodes[j].compile);
        assert_eq!(p.total_words(), before);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn one_file_that_is_not_utf8_does_not_lock_the_book() {
        let (d, scene) = sync_book("latin1");
        let odd = scene.with_file_name("02-Cafe.md");
        let bytes = b"Le caf\xe9 \xe9tait ferm\xe9.\n".to_vec();
        fs::write(&odd, &bytes).unwrap();

        let mut p = Project::load(&d).expect("the book opens");
        let i = scene_idx(&p, &odd);
        assert!(p.nodes[i].read_only);
        assert!(p.nodes[i].body.contains("caf"));
        assert!(!p.nodes[scene_idx(&p, &scene)].read_only);

        // even if something marks it changed, it is never written
        p.nodes[i].body = "overwritten".into();
        p.nodes[i].dirty = true;
        let report = p.save_dirty();
        assert!(report.failed.is_empty());
        assert_eq!(fs::read(&odd).unwrap(), bytes);
        assert_eq!(p.check_disk(i, true).unwrap(), DiskChange::Same);

        // and renaming it keeps its bytes
        let renamed = rename(&odd, "Bistro").unwrap();
        assert_eq!(fs::read(&renamed).unwrap(), bytes);
        fs::remove_dir_all(&d).unwrap();
    }
}
