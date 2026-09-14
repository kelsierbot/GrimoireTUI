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
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            author: String::new(),
            draft: String::new(),
            target_words: 80_000,
            daily_target: 1_000,
        }
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

        Ok(p)
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

    pub fn save_all(&mut self) -> Result<usize> {
        let mut count = 0;
        for i in 0..self.nodes.len() {
            if self.nodes[i].dirty && self.nodes[i].kind == Kind::Scene {
                let n = &self.nodes[i];
                let mut out = String::new();
                if let Some(f) = &n.front {
                    out.push_str("---\n");
                    out.push_str(f);
                    if !f.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("---\n\n");
                }
                out.push_str(&n.body);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                fs::write(&n.path, out)
                    .with_context(|| format!("writing {}", n.path.display()))?;
                self.nodes[i].dirty = false;
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn dirty_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.dirty).count()
    }
}

/// Split a `---` fenced YAML frontmatter block off the front of a file.
fn split_frontmatter(raw: &str) -> (Option<String>, String) {
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

    stripped
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Create a new manuscript skeleton. Refuses to touch a directory that
/// already holds a project.
pub fn scaffold(root: &Path) -> Result<()> {
    if root.join("manuscript").is_dir() {
        anyhow::bail!("{} already contains a manuscript/", root.display());
    }

    let title = root
        .file_name()
        .map(|s| display_title(Path::new(s), None))
        .unwrap_or_else(|| "Untitled".into());

    let scene_dir = root.join("manuscript/01-part-one/01-chapter-one");
    fs::create_dir_all(&scene_dir)
        .with_context(|| format!("creating {}", scene_dir.display()))?;
    fs::create_dir_all(root.join("notes/characters"))?;
    fs::create_dir_all(root.join("notes/places"))?;

    write_new(
        &root.join("novel.toml"),
        &format!(
            "title = \"{title}\"\nauthor = \"\"\ndraft = \"1\"\ntarget_words = 80000\ndaily_target = 1000\n"
        ),
    )?;

    write_new(
        &scene_dir.join("01-opening.md"),
        "---\ntitle: Opening\npov:\nstatus: outline\nsynopsis:\ntarget: 1500\n---\n\n",
    )?;

    write_new(
        &root.join("notes/characters/example.md"),
        "---\ntitle: Example\n---\n\nDelete this and write someone real.\n",
    )?;

    write_new(&root.join(".gitignore"), ".grimoire/\n")?;
    Ok(())
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

    #[test]
    fn the_first_item_in_a_new_folder_is_number_one() {
        let d = temp_dir("empty");
        let p = create(&d.join("new-chapter"), "Opening", false).unwrap();
        assert_eq!(p.file_name().unwrap(), "01-Opening.md");
        fs::remove_dir_all(&d).unwrap();
    }
}
