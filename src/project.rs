//! Manuscript model: a project is a directory tree of plain Markdown files.
//!
//! Structure on disk is the source of truth — no central index to conflict in git:
//!
//!   novel.toml                     project metadata
//!   manuscript/01-part-one/01-chapter-one/01-the-archive.md
//!   notes/characters/wren.md
//!
//! Directories are containers (parts, chapters). `.md` files are scenes.
//! Ordering comes from the filename; a leading `01-` is stripped for display.
//! Scene metadata lives in YAML frontmatter, which we preserve verbatim so
//! Obsidian and anything else can read and write it without us mangling it.

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
    Divider,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
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
}

impl Node {
    pub fn words(&self) -> usize {
        self.body.split_whitespace().count()
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
}

pub struct Project {
    pub root: PathBuf,
    pub meta: ProjectMeta,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

impl Project {
    pub fn load(root: &Path) -> Result<Project> {
        let meta_path = root.join("novel.toml");
        let meta: ProjectMeta = if meta_path.exists() {
            let s = fs::read_to_string(&meta_path)
                .with_context(|| format!("reading {}", meta_path.display()))?;
            toml::from_str(&s).with_context(|| format!("parsing {}", meta_path.display()))?
        } else {
            ProjectMeta::default()
        };

        let mut p = Project {
            root: root.to_path_buf(),
            meta,
            nodes: Vec::new(),
            roots: Vec::new(),
        };

        // Front matter sits beside the draft, not inside it — Scrivener's
        // arrangement, and the reason it compiles without inflating wordcount.
        let fm = root.join("front-matter");
        if fm.is_dir() {
            let div = p.push(Node {
                kind: Kind::Divider,
                title: "FRONT MATTER".into(),
                path: fm.clone(),
                depth: 0,
                expanded: true,
                children: Vec::new(),
                in_manuscript: false,
                compile: true,
                front_matter: true,
                front: None,
                body: String::new(),
                dirty: false,
                pov: None,
                status: None,
            });
            p.roots.push(div);
            let idxs = p.scan(&fm, 0, false, true)?;
            p.roots.extend(idxs);
        }

        let manuscript = root.join("manuscript");
        if manuscript.is_dir() {
            if fm.is_dir() {
                let div = p.push(Node {
                    kind: Kind::Divider,
                    title: "MANUSCRIPT".into(),
                    path: manuscript.clone(),
                    depth: 0,
                    expanded: true,
                    children: Vec::new(),
                    in_manuscript: true,
                    compile: true,
                    front_matter: false,
                    front: None,
                    body: String::new(),
                    dirty: false,
                    pov: None,
                    status: None,
                });
                p.roots.push(div);
            }
            let idxs = p.scan(&manuscript, 0, true, false)?;
            p.roots.extend(idxs);
        }

        let notes = root.join("notes");
        if notes.is_dir() {
            let div = p.push(Node {
                kind: Kind::Divider,
                title: "NOTES".into(),
                path: notes.clone(),
                depth: 0,
                expanded: true,
                children: Vec::new(),
                in_manuscript: false,
                compile: false,
                front_matter: false,
                front: None,
                body: String::new(),
                dirty: false,
                pov: None,
                status: None,
            });
            p.roots.push(div);
            let idxs = p.scan(&notes, 0, false, false)?;
            p.roots.extend(idxs);
        }

        // Last, and last for a reason: what's been deleted is still on screen,
        // so nothing ever simply disappears. It counts for nothing and
        // compiles into nothing.
        let bin = trash_dir(root);
        if bin.is_dir() {
            let div = p.push(Node {
                kind: Kind::Divider,
                title: "TRASH".into(),
                path: bin.clone(),
                depth: 0,
                expanded: true,
                children: Vec::new(),
                in_manuscript: false,
                compile: false,
                front_matter: false,
                front: None,
                body: String::new(),
                dirty: false,
                pov: None,
                status: None,
            });
            p.roots.push(div);
            let idxs = p.scan(&bin, 0, false, false)?;
            p.roots.extend(idxs);
        }

        Ok(p)
    }

    /// Is this row already in the trash? Then deleting it means for good.
    pub fn in_trash(&self, idx: usize) -> bool {
        self.nodes[idx].path.starts_with(trash_dir(&self.root))
    }

    fn push(&mut self, n: Node) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    fn scan(
        &mut self,
        dir: &Path,
        depth: usize,
        in_manuscript: bool,
        front_matter: bool,
    ) -> Result<Vec<usize>> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                !name.starts_with('.')
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        let mut out = Vec::new();
        for e in entries {
            let path = e.path();
            if path.is_dir() {
                let idx = self.push(Node {
                    kind: Kind::Container,
                    title: display_title(&path, None),
                    path: path.clone(),
                    depth,
                    // A novel tree is small. Start it open; collapsing is a
                    // deliberate act, not something to make the user undo.
                    expanded: true,
                    children: Vec::new(),
                    in_manuscript,
                    compile: true,
                    front_matter,
                    front: None,
                    body: String::new(),
                    dirty: false,
                    pov: None,
                    status: None,
                });
                let kids = self.scan(&path, depth + 1, in_manuscript, front_matter)?;
                self.nodes[idx].children = kids;
                out.push(idx);
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let raw = fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let (front, body) = split_frontmatter(&raw);
                let pov = front.as_deref().and_then(|f| front_get(f, "pov"));
                let status = front.as_deref().and_then(|f| front_get(f, "status"));
                let compile = front
                    .as_deref()
                    .and_then(|f| front_get(f, "compile"))
                    .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "no" | "0"))
                    .unwrap_or(true);
                let title = display_title(&path, front.as_deref());
                let idx = self.push(Node {
                    kind: Kind::Scene,
                    title,
                    path: path.clone(),
                    depth,
                    expanded: false,
                    children: Vec::new(),
                    in_manuscript,
                    compile,
                    front_matter,
                    front,
                    body,
                    dirty: false,
                    pov,
                    status,
                });
                out.push(idx);
            }
        }
        Ok(out)
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
            Kind::Scene => n.words(),
            _ => n.children.iter().map(|&c| self.subtree_words(c)).sum(),
        }
    }

    pub fn total_words(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.kind == Kind::Scene && n.in_manuscript)
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
    /// mid-save can never leave half a scene behind.
    pub fn save_dirty(&mut self) -> SaveReport {
        let mut report = SaveReport::default();
        for i in 0..self.nodes.len() {
            if !(self.nodes[i].dirty && self.nodes[i].kind == Kind::Scene) {
                continue;
            }
            let n = &self.nodes[i];
            match write_atomic(&n.path, &n.file_text()) {
                Ok(()) => {
                    self.nodes[i].dirty = false;
                    report.saved.push(i);
                }
                // The reason, not the path: "Permission denied", "No space left".
                Err(e) => report.failed.push((i, short_reason(&e))),
            }
        }
        report
    }



    pub fn dirty_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.dirty).count()
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
/// never a torn mix: write a hidden sibling, flush it to disk, then rename it
/// over. The sibling starts with a dot, so a crash that leaves one behind never
/// shows up in the tree.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "scene".into());
    let tmp = path.with_file_name(format!(".{name}.saving"));
    let written = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", path.display()));
    }
    if fs::rename(&tmp, path).is_ok() {
        return Ok(());
    }
    // Windows refuses to replace a file another program has open (a sync
    // client, an editor). Writing in place is second best, but it saves.
    let _ = fs::remove_file(&tmp);
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
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
            let body = rest[offset + line.len()..].trim_start_matches('\n').to_string();
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
    let needs_quotes = value.contains(':') || value.contains('#') || value.starts_with(['"', '\'', '[', '{', '-', '&', '*', '!', '|', '>', '%', '@', '`']);
    let line = if value.is_empty() {
        format!("{key}:")
    } else if needs_quotes {
        format!("{key}: \"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("{key}: {value}")
    };
    let mut out = String::new();
    let mut done = false;
    for l in front.unwrap_or("").lines() {
        let is_key = !l.starts_with([' ', '\t']) && l.split_once(':').is_some_and(|(k, _)| k.trim() == key);
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
    if let Some(f) = front {
        if let Some(t) = front_get(f, "title") {
            return t;
        }
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

/// Three acts, nine chapters each, three scenes in every chapter.
pub const ACTS: usize = 3;
pub const CHAPTERS_PER_ACT: usize = 9;
pub const SCENES_PER_CHAPTER: usize = 3;

/// The notebook a secondary world needs, in the order one gets built rather
/// than in alphabetical order — which is why they're numbered on disk.
pub const NOTE_SECTIONS: [&str; 8] = [
    "Characters",
    "Races",
    "Regions",
    "Magic System",
    "Politics",
    "Religion",
    "Notes",
    "Research",
];

/// Where deleted things go. Inside `.grimoire/`, which is gitignored, but the
/// tree shows it so nothing ever just vanishes.
pub fn trash_dir(root: &Path) -> PathBuf {
    root.join(".grimoire/trash")
}

/// Create a new manuscript skeleton: the three-act shape, with every chapter
/// and scene already standing so the book is a thing to fill in rather than a
/// blank page. The scenes are empty — the structure is a suggestion, the
/// writing isn't presumed. Refuses to touch a directory that already holds a
/// project.
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
            "title = \"{title}\"\nauthor = \"\"\ndraft = \"1\"\ntarget_words = 80000\ndaily_target = 1000\n\n# What this book calls its largest division: Act, Part, Book…\npart_label = \"Act\"\n"
        ),
    )?;

    let mut chapter = 0usize;
    for act in 1..=ACTS {
        let act_dir = root
            .join("manuscript")
            .join(numbered_dir(act, &crate::manuscript::numbered("Act", act)));
        for c in 1..=CHAPTERS_PER_ACT {
            chapter += 1;
            // Chapters are numbered straight through the book, the way the
            // finished manuscript numbers them, not restarted in each act.
            let ch_dir = act_dir.join(numbered_dir(c, &crate::manuscript::numbered("Chapter", chapter)));
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

    for (i, section) in NOTE_SECTIONS.iter().enumerate() {
        let dir = root.join("notes").join(numbered_dir(i + 1, section));
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    // The trash exists from the first launch, so "where did that chapter go?"
    // is answered on screen before anything has been deleted.
    let bin = trash_dir(root);
    fs::create_dir_all(&bin).with_context(|| format!("creating {}", bin.display()))?;

    write_new(&root.join(".gitignore"), ".grimoire/\n")?;
    Ok(())
}

/// `7` and "Chapter Seven" make `07-Chapter-Seven`.
fn numbered_dir(n: usize, name: &str) -> String {
    format!("{:02}-{}", n, file_safe(name))
}

fn write_new(path: &Path, body: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

/// Make a new scene (`.md`) or folder at the end of `dir`, numbered after
/// whatever is already there. Nothing existing is renamed, so links from
/// Obsidian or anywhere else keep working. Returns the new path.
pub fn create(dir: &Path, name: &str, folder: bool) -> Result<PathBuf> {
    let name = name.trim();
    let name = if name.is_empty() { "Untitled" } else { name };
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let stem = format!("{:02}-{}", next_number(dir)?, file_safe(name));
    if folder {
        let path = dir.join(stem);
        fs::create_dir(&path).with_context(|| format!("creating {}", path.display()))?;
        return Ok(path);
    }
    let path = dir.join(format!("{stem}.md"));
    if path.exists() {
        anyhow::bail!("{} already exists", path.display());
    }
    // The title is kept exactly as typed; only the filename is tidied.
    let title = name.replace('"', "'");
    fs::write(
        &path,
        format!("---\ntitle: \"{title}\"\npov:\nstatus: draft\nsynopsis:\n---\n\n"),
    )
    .with_context(|| format!("writing {}", path.display()))?;
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
    let stem = match leading_number(path) {
        Some(n) => format!("{:02}-{}", n, file_safe(name)),
        None => file_safe(name),
    };
    let folder = path.is_dir();
    let target = if folder {
        parent.join(stem)
    } else {
        parent.join(format!("{stem}.md"))
    };
    if target != path {
        if target.exists() {
            anyhow::bail!("{} already exists", target.display());
        }
        fs::rename(path, &target)
            .with_context(|| format!("renaming {}", path.display()))?;
    }
    if !folder {
        let raw = fs::read_to_string(&target)
            .with_context(|| format!("reading {}", target.display()))?;
        if let Some(updated) = retitle(&raw, name) {
            fs::write(&target, updated)
                .with_context(|| format!("writing {}", target.display()))?;
        }
    }
    Ok(target)
}

/// Swap the `title:` line inside a frontmatter block. `None` when there is no
/// block or no title in it — then the filename is already the name.
fn retitle(raw: &str, name: &str) -> Option<String> {
    let (front, body) = split_frontmatter(raw);
    let front = front?;
    let mut done = false;
    let mut out = String::new();
    for line in front.lines() {
        if !done && line.split_once(':').is_some_and(|(k, _)| k.trim() == "title") {
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
    let target = dir.join(format!("{stamp}-{name}"));
    fs::rename(path, &target)
        .with_context(|| format!("moving {} to the trash", path.display()))?;
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

/// What the tree shows in a folder, in its order: subfolders and `.md` files,
/// nothing hidden.
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
    v.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
    v
}

/// `07-Low-Tide.md` → (7, 2, "Low-Tide.md").
fn split_number(path: &Path) -> Option<(usize, usize, String)> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = name[digits.len()..].trim_start_matches(['-', '_', ' ']).to_string();
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
    let top = [root.join("manuscript"), root.join("notes"), root.join("front-matter")];
    if top.contains(&path.to_path_buf()) || path.starts_with(root.join(".grimoire")) {
        anyhow::bail!("that can't be moved");
    }
    let siblings = tree_entries(&parent);
    let pos = siblings.iter().position(|p| p == path).context("it isn't in its folder any more")?;
    let neighbour = if up { pos.checked_sub(1) } else { (pos + 1 < siblings.len()).then_some(pos + 1) };

    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    let target;
    if let Some(n) = neighbour.map(|i| siblings[i].clone()) {
        // Swap numbers with the neighbour.
        let (a_num, a_w, a_rest) = split_number(path).context("only numbered items can be moved — give it a number first")?;
        let (b_num, b_w, b_rest) = split_number(&n).context("its neighbour has no number, so there's nothing to swap with")?;
        let (a_num, b_num) = if a_num == b_num {
            // Same number (it happens): order by name, so just nudge.
            if up { (a_num, a_num + 1) } else { (a_num + 1, a_num) }
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
            anyhow::bail!(if up { "it's already first" } else { "it's already last" });
        }
        let grand = parent.parent().context("no folder above")?;
        let uncles: Vec<PathBuf> = tree_entries(grand).into_iter().filter(|p| p.is_dir()).collect();
        let at = uncles.iter().position(|p| *p == parent).context("its folder moved")?;
        let dest = if up { at.checked_sub(1) } else { (at + 1 < uncles.len()).then_some(at + 1) }
            .map(|i| uncles[i].clone())
            .or_else(|| cousin_folder(root, &parent, up))
            .with_context(|| if up { "it's already first" } else { "it's already last" })?;
        let (_, width, rest) = split_number(path).unwrap_or((0, 2, path.file_name().unwrap_or_default().to_string_lossy().to_string()));
        let there = tree_entries(&dest);
        let numbers: Vec<(usize, usize)> = there.iter().filter_map(|p| split_number(p).map(|(n, w, _)| (n, w))).collect();
        let width = numbers.iter().map(|&(_, w)| w).max().unwrap_or(width).max(width);
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
                        renames.push((p.clone(), dest.join(numbered_name(n + 1, w.max(width), &r))));
                    }
                }
                1
            }
        };
        let to = dest.join(numbered_name(number, width, &rest));
        renames.push((path.to_path_buf(), to.clone()));
        target = to;
    }

    for (_, to) in &renames {
        if to.exists() && !renames.iter().any(|(from, _)| from == to) {
            anyhow::bail!("{} already exists", to.display());
        }
    }
    rename_all(&renames)?;
    let links = rewrite_links(root, &renames);
    Ok(Moved { renames, target, links })
}

/// At the edge of an act, the chapter before or after is in the neighbouring
/// act: the last folder of the previous one, or the first of the next.
fn cousin_folder(root: &Path, folder: &Path, up: bool) -> Option<PathBuf> {
    let grand = folder.parent()?;
    let great = grand.parent()?;
    if great == root {
        return None;
    }
    let aunts: Vec<PathBuf> = tree_entries(great).into_iter().filter(|p| p.is_dir()).collect();
    let at = aunts.iter().position(|p| p == grand)?;
    let aunt = if up { at.checked_sub(1)? } else { (at + 1 < aunts.len()).then_some(at + 1)? };
    let kids: Vec<PathBuf> = tree_entries(&aunts[aunt]).into_iter().filter(|p| p.is_dir()).collect();
    if up { kids.last().cloned() } else { kids.first().cloned() }
}

/// Rename in two passes through temporary names, so a swap (or a shift along)
/// never collides with itself.
fn rename_all(renames: &[(PathBuf, PathBuf)]) -> Result<()> {
    let mut staged = Vec::new();
    for (i, (from, to)) in renames.iter().enumerate() {
        let tmp = from.with_file_name(format!(".grimoire-moving-{i}-{}", std::process::id()));
        fs::rename(from, &tmp).with_context(|| format!("moving {}", from.display()))?;
        staged.push((tmp, to));
    }
    for (tmp, to) in staged {
        if let Some(d) = to.parent() {
            fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        fs::rename(&tmp, to).with_context(|| format!("moving to {}", to.display()))?;
    }
    Ok(())
}

/// Point `[[links]]` at the new names. Both Obsidian forms are handled: a bare
/// file name (`[[02-Low-Tide]]`) and a path from the book's root
/// (`[[manuscript/01-Act-One/02-Low-Tide|Low Tide]]`), including paths through
/// a renamed folder. Returns how many files changed.
pub fn rewrite_links(root: &Path, renames: &[(PathBuf, PathBuf)]) -> usize {
    let rel = |p: &Path| -> String {
        let r = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
        r.strip_suffix(".md").map(str::to_string).unwrap_or(r)
    };
    let stem = |p: &Path| -> String {
        let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
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
        let Ok(text) = fs::read_to_string(&file) else { continue };
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
        if let Some(n) = leading_number(&e?.path()) {
            top = top.max(n);
        }
    }
    Ok(top + 1)
}

/// The `1` in `01-the-archive.md`, which is what orders the tree.
pub fn leading_number(path: &Path) -> Option<usize> {
    let name = path.file_name()?.to_string_lossy();
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// A name as it can live on disk: words joined by dashes, with capitals and
/// apostrophes kept so the tree shows a folder back the way it was typed.
fn file_safe(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        "untitled".into()
    } else {
        words.join("-")
    }
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
        assert_eq!(display_title(&p, front.as_deref()), "Wren's 'last' letter: part 2");
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

    /// The shape a new book arrives in: three acts, twenty-seven chapters,
    /// three scenes in each, and no words written for you.
    #[test]
    fn the_template_is_three_acts_of_nine_chapters() {
        let d = temp_dir("template");
        scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        let containers = |depth: usize| {
            p.nodes
                .iter()
                .filter(|n| n.kind == Kind::Container && n.in_manuscript && n.depth == depth)
                .count()
        };
        assert_eq!(containers(0), ACTS, "acts");
        assert_eq!(containers(1), ACTS * CHAPTERS_PER_ACT, "chapters");
        let scenes: Vec<&Node> = p
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Scene && n.in_manuscript)
            .collect();
        assert_eq!(scenes.len(), ACTS * CHAPTERS_PER_ACT * SCENES_PER_CHAPTER);
        assert_eq!(p.total_words(), 0, "the scenes start empty");

        // Named straight through the book, and spelled the way they'd be read.
        let titles: Vec<&str> = p
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Container && n.in_manuscript && n.depth == 1)
            .map(|n| n.title.as_str())
            .collect();
        assert_eq!(titles[0], "Chapter One");
        assert_eq!(titles[9], "Chapter Ten");
        assert_eq!(titles[26], "Chapter Twenty-Seven");
        assert_eq!(p.meta.part_word(), "Act", "a new book counts in acts");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_notebook_reads_in_the_order_a_world_gets_built() {
        let d = temp_dir("notebook");
        scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        let sections: Vec<String> = p
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Container && !n.in_manuscript && n.depth == 0)
            .filter(|n| !n.path.starts_with(trash_dir(&d)))
            .map(|n| n.title.clone())
            .collect();
        assert_eq!(sections, NOTE_SECTIONS.to_vec(), "not alphabetical");
        fs::remove_dir_all(&d).unwrap();
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
        assert!(p.nodes.iter().any(|n| n.title == "TRASH"), "the heading shows");
        let gone = p
            .nodes
            .iter()
            .position(|n| n.title.contains("Scene One") && n.path.starts_with(trash_dir(&d)))
            .expect("the deleted scene is listed under it");
        assert!(p.in_trash(gone));
        assert!(!p.nodes[gone].in_manuscript);
        assert_eq!(p.total_words(), 0, "trashed words don't count toward the draft");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_spelled_out_chapter_keeps_its_hyphen_but_a_title_keeps_its_spaces() {
        assert_eq!(display_title(Path::new("07-Chapter-Twenty-Seven"), None), "Chapter Twenty-Seven");
        assert_eq!(display_title(Path::new("01-the-archive.md"), None), "The Archive");
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
        assert!(front.unwrap().contains("status: draft"), "the rest of the frontmatter stays");
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

    fn names_in(dir: &Path) -> Vec<String> {
        tree_entries(dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect()
    }

    fn move_book(tag: &str) -> PathBuf {
        let d = temp_dir(tag);
        let ch = |a: &str, c: &str| d.join("manuscript").join(a).join(c);
        for (a, c, scenes) in [
            ("01-Act-One", "01-Chapter-One", vec!["01-Gravel.md", "02-Low-Tide.md"]),
            ("01-Act-One", "02-Chapter-Two", vec!["01-Ashfall.md", "02-The-Crossing.md"]),
            ("02-Act-Two", "03-Chapter-Three", vec!["01-Lantern.md"]),
        ] {
            fs::create_dir_all(ch(a, c)).unwrap();
            for s in scenes {
                fs::write(ch(a, c).join(s), format!("---\ntitle: {s}\n---\n\nwords of {s}\n")).unwrap();
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
        assert!(fs::read_to_string(ch1.join("01-Low-Tide.md")).unwrap().contains("words of 02-Low-Tide.md"), "content travels with the name");
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
        assert_eq!(names_in(&ch1), ["01-Gravel.md", "02-Low-Tide.md", "03-Ashfall.md"]);
        assert_eq!(names_in(&ch2), ["02-The-Crossing.md"], "the gap is left; nothing else renamed");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_bottom_scene_moving_down_goes_to_the_start_of_the_next_chapter_across_acts() {
        let d = move_book("move-down-cross");
        let ch2 = d.join("manuscript/01-Act-One/02-Chapter-Two");
        let ch3 = d.join("manuscript/02-Act-Two/03-Chapter-Three");
        let m = move_item(&d, &ch2.join("02-The-Crossing.md"), false).unwrap();
        assert_eq!(names_in(&ch3), ["01-The-Crossing.md", "02-Lantern.md"], "the first there shifts along to make room");
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
        assert!(act2.join("02-Chapter-Two/01-Ashfall.md").exists(), "scenes travel inside");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn nothing_moves_past_the_ends() {
        let d = move_book("move-ends");
        assert!(move_item(&d, &d.join("manuscript/01-Act-One"), true).is_err());
        assert!(move_item(&d, &d.join("manuscript/02-Act-Two/03-Chapter-Three/01-Lantern.md"), false).is_err());
        assert!(move_item(&d, &d.join("manuscript"), true).is_err());
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
        assert_eq!(out, "title: \"Gravel\"\n# a comment\npov: Wren\nmood: grey\ntags:\n  - lot\n");
        let out = set_front(Some(&out), "synopsis", "Confronts the caretaker: again");
        assert!(out.ends_with("synopsis: \"Confronts the caretaker: again\"\n"), "added at the end, quoted for the colon");
        assert_eq!(front_get(&out, "synopsis").as_deref(), Some("Confronts the caretaker: again"));
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
}
