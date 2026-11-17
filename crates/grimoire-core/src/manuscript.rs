//! The project file, and compiling a manuscript.
//!
//! Two jobs, both centred on one idea: **`project.md` is the map.** A single
//! Markdown file at the project root describes the whole book — every part,
//! chapter and scene, with word counts, status and synopsis — and links to each
//! scene with an Obsidian wikilink, so the same file is a working index inside
//! a vault and a plain readable document outside one.
//!
//! How the book is structured for compiling:
//!
//! - **Front matter is a sibling of the draft**, not inside it, so a title page
//!   compiles into the manuscript without inflating the wordcount.
//! - **Section type comes from depth** — containers holding scenes are
//!   chapters, containers holding chapters are parts.
//! - **Include-in-compile is per scene** (`compile: false` in frontmatter), so
//!   cut material can stay in the tree without reaching the finished draft.
//!
//! Compile targets Shunn's standard manuscript format, which is what agents and
//! magazines expect: rounded word count on the title page, spelled-out chapter
//! headings, and `#` alone on a line for a scene break.

use anyhow::Result;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::project::{Kind, Project};

const BEGIN: &str = "<!-- grimoire:structure -->";
const END: &str = "<!-- /grimoire:structure -->";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Part,
    Chapter,
    Scene,
}

/// A container holding only scenes is a chapter; one holding containers is a
/// part, and any scenes beside them are loose scenes of that part. Depth alone lies as soon as someone skips the part level.
///
/// An empty folder has nothing inside to judge by, and a chapter is always
/// empty the moment it's made. So it goes by its name first ("Chapter Two",
/// "Part Two"), then by where it sits: inside another folder it's a chapter,
/// at the top it matches its neighbours.
pub fn section_of(p: &Project, idx: usize) -> Section {
    let n = &p.nodes[idx];
    if n.kind == Kind::Scene {
        return Section::Scene;
    }
    let holds = |i: usize, kind: Kind| p.nodes[i].children.iter().any(|&c| p.nodes[c].kind == kind);
    // A folder holding folders is a part, even with a loose scene or two
    // beside them: as a chapter, its own chapters would drop out of the book.
    if holds(idx, Kind::Container) {
        return Section::Part;
    }
    if holds(idx, Kind::Scene) {
        return Section::Chapter;
    }
    // A book that calls its parts Acts says "Act One" here, so the label from
    // novel.toml counts alongside the default word.
    match n.title.to_lowercase().split_whitespace().next() {
        Some("chapter") => return Section::Chapter,
        Some("page") | Some("part") => return Section::Part,
        Some(w) if w == p.meta.part_noun() => return Section::Part,
        _ => {}
    }
    let parent = n.path.parent();
    if p.nodes
        .iter()
        .any(|m| m.kind == Kind::Container && Some(m.path.as_path()) == parent)
    {
        return Section::Chapter;
    }
    let neighbours: Vec<usize> = (0..p.nodes.len())
        .filter(|&i| {
            i != idx && p.nodes[i].kind == Kind::Container && p.nodes[i].path.parent() == parent
        })
        .collect();
    if neighbours.iter().any(|&i| holds(i, Kind::Container)) {
        Section::Part
    } else if neighbours.iter().any(|&i| holds(i, Kind::Scene)) {
        Section::Chapter
    } else {
        Section::Part
    }
}

/// "Chapter" and 2 make "Chapter Two" — how the book names its own divisions,
/// both when one is created and when the template lays them out.
pub fn numbered(what: &str, n: usize) -> String {
    let word: Vec<String> = spell(n)
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_string() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect();
    format!("{what} {}", word.join("-"))
}

pub fn project_file(root: &Path) -> PathBuf {
    root.join("project.md")
}

fn rel(root: &Path, p: &Path) -> String {
    link_target(&p.strip_prefix(root).unwrap_or(p).to_string_lossy())
}

/// A path from the book's root as a `[[link]]` target: always `/`, as
/// Obsidian writes it on every system, and no `.md`.
fn link_target(rel: &str) -> String {
    let r = rel.replace('\\', "/");
    r.strip_suffix(".md").map(str::to_string).unwrap_or(r)
}

pub fn commas(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The approximate count a title page gives, the way Shunn and the SFWA's
/// lengths have it: to the nearest 100 for a story (under 17,500 words), 500
/// for a novella (under 40,000), 1,000 for a novel.
pub fn rounded_words(n: usize) -> usize {
    let step = if n < 17_500 {
        100
    } else if n < 40_000 {
        500
    } else {
        1_000
    };
    // A few lines of a first scene are "about 100 words", never "about 0".
    (((n + step / 2) / step) * step).max(if n > 0 { step } else { 0 })
}

const ONES: [&str; 20] = [
    "ZERO",
    "ONE",
    "TWO",
    "THREE",
    "FOUR",
    "FIVE",
    "SIX",
    "SEVEN",
    "EIGHT",
    "NINE",
    "TEN",
    "ELEVEN",
    "TWELVE",
    "THIRTEEN",
    "FOURTEEN",
    "FIFTEEN",
    "SIXTEEN",
    "SEVENTEEN",
    "EIGHTEEN",
    "NINETEEN",
];
const TENS: [&str; 10] = [
    "", "", "TWENTY", "THIRTY", "FORTY", "FIFTY", "SIXTY", "SEVENTY", "EIGHTY", "NINETY",
];

/// "CHAPTER TWENTY-THREE" reads better than "CHAPTER 23" in a manuscript.
pub fn spell(n: usize) -> String {
    match n {
        0..=19 => ONES[n].to_string(),
        20..=99 => {
            let (t, o) = (n / 10, n % 10);
            if o == 0 {
                TENS[t].to_string()
            } else {
                format!("{}-{}", TENS[t], ONES[o])
            }
        }
        _ => n.to_string(),
    }
}

// ── the project file ─────────────────────────────────────────────────

/// Regenerate the managed block, leaving everything the author wrote alone.
pub fn write_project_file(p: &Project) -> Result<PathBuf> {
    let path = project_file(&p.root);
    // Only a file that isn't there starts fresh. One that can't be read (not
    // UTF-8, or online-only and offline) is the author's, so nothing is written.
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(
                "project.md can't be read right now, so the map wasn't updated (your notes in it are safe)",
            ));
        }
    };
    let body = structure_block(p);

    let out = if existing.is_empty() {
        format!("{}\n{body}\n", scaffold_head(p))
    } else if let (Some(a), Some(b)) = (existing.find(BEGIN), existing.find(END)) {
        // Splice: keep the author's prose above and below untouched.
        format!("{}{body}{}", &existing[..a], &existing[b + END.len()..])
    } else {
        format!("{}\n\n{body}\n", existing.trim_end())
    };

    crate::atomic::write_text(&path, &out)?;
    Ok(path)
}

fn scaffold_head(p: &Project) -> String {
    let m = &p.meta;
    format!(
        "---\ngrimoire: project\ntitle: {}\nauthor: {}\ndraft: {}\ntarget_words: {}\ndaily_target: {}\n---\n\n\
         # {}\n\n\
         *Notes, outline, anything you like. Grimoire only ever rewrites the block below.*\n",
        m.title, m.author, m.draft, m.target_words, m.daily_target, m.title
    )
}

fn structure_block(p: &Project) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{BEGIN}");
    let _ = writeln!(
        s,
        "*Map of the project — regenerated by Grimoire. Edits inside this block are overwritten.*\n"
    );

    let total = p.total_words();
    let scenes = p
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.in_manuscript && !n.parked)
        .count();
    let excluded = p
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.in_manuscript && !n.compile && !n.parked)
        .count();

    let _ = writeln!(
        s,
        "**{} of {} words** · {scenes} scene{} · draft {}",
        commas(total),
        commas(p.meta.target_words),
        if scenes == 1 { "" } else { "s" },
        p.meta.draft
    );
    if excluded > 0 {
        let _ = writeln!(
            s,
            "\n> {excluded} scene(s) marked `compile: false` — in the tree, out of the manuscript."
        );
    }

    let fm: Vec<usize> = p
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == Kind::Scene && n.front_matter)
        .map(|(i, _)| i)
        .collect();
    if !fm.is_empty() {
        let _ = writeln!(s, "\n## Front matter\n");
        for i in fm {
            let n = &p.nodes[i];
            let _ = writeln!(s, "- [[{}|{}]]", rel(&p.root, &n.path), n.title);
        }
    }

    let _ = writeln!(s, "\n## Manuscript\n");
    for r in p.manuscript() {
        emit(p, r, &mut s);
    }

    let notes: Vec<usize> = p
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| n.kind == Kind::Scene && n.area.is_notebook() && !p.in_trash(*i))
        .map(|(i, _)| i)
        .collect();
    if !notes.is_empty() {
        let _ = writeln!(s, "\n## Notes\n");
        for i in notes {
            let n = &p.nodes[i];
            let _ = writeln!(s, "- [[{}|{}]]", rel(&p.root, &n.path), n.title);
        }
    }

    let _ = write!(s, "\n{END}");
    s
}

fn emit(p: &Project, idx: usize, s: &mut String) {
    let n = &p.nodes[idx];
    match section_of(p, idx) {
        Section::Part => {
            let _ = writeln!(s, "### {}\n", n.title);
            for &c in &n.children {
                emit(p, c, s);
            }
        }
        Section::Chapter => {
            let w = p.subtree_words(idx);
            let _ = writeln!(s, "#### {} — {} words\n", n.title, commas(w));
            for &c in &n.children {
                emit(p, c, s);
            }
            s.push('\n');
        }
        // A conflict copy is a version of a scene, not a place in the book.
        Section::Scene if n.parked => {}
        Section::Scene => {
            let mut bits: Vec<String> = Vec::new();
            if let Some(st) = &n.status {
                bits.push(st.clone());
            }
            if let Some(pov) = &n.pov {
                bits.push(format!("POV {pov}"));
            }
            bits.push(format!("{} words", commas(n.words())));
            if !n.compile {
                bits.push("**not compiled**".into());
            }
            let _ = writeln!(
                s,
                "- **[[{}|{}]]** · {}",
                rel(&p.root, &n.path),
                n.title,
                bits.join(" · ")
            );
            if let Some(syn) = n.front.as_deref().and_then(|f| front_value(f, "synopsis")) {
                let _ = writeln!(s, "  > {syn}");
            }
        }
    }
}

/// Read the way the tree reads it, so a quoted synopsis shows as written. A
/// list item (`  - The Lot`) has no colon and never ends the search.
fn front_value(front: &str, key: &str) -> Option<String> {
    crate::project::front_scalar(front, key)
}

// ── compile ──────────────────────────────────────────────────────────

pub struct Compiled {
    pub path: PathBuf,
    pub words: usize,
    pub chapters: usize,
    pub scenes: usize,
    pub skipped: usize,
}

/// Assemble the draft into one Markdown file in standard manuscript shape.
///
/// The walk lives in `export`, which draws DOCX, EPUB and this same Markdown
/// from one model of the book — so compile and export can't drift apart.
/// Front matter owns the title page: it's emitted verbatim
/// rather than having a generated one stacked on top of it.
pub fn compile(p: &Project) -> Result<Compiled> {
    p.ensure_whole()?;
    let book = crate::export::book(p, None, crate::export::Scope::Whole)?;
    let path = p
        .root
        .join(format!("{}-manuscript.md", crate::export::slug(book.name)));
    crate::atomic::write_text(&path, &crate::export::markdown(&book))?;

    Ok(Compiled {
        path,
        words: book.words,
        chapters: book.chapters,
        scenes: book.scenes,
        skipped: book.skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_counts_round_the_way_shunn_asks() {
        assert_eq!(rounded_words(0), 0);
        assert_eq!(rounded_words(20), 100, "something written is never 0");
        assert_eq!(rounded_words(1_240), 1_200);
        assert_eq!(rounded_words(1_260), 1_300);
        assert_eq!(rounded_words(6_440), 6_400, "a story: nearest 100");
        assert_eq!(rounded_words(22_700), 22_500, "a novella: nearest 500");
        assert_eq!(rounded_words(82_400), 82_000, "a novel: nearest 1,000");
        assert_eq!(rounded_words(82_600), 83_000);
    }

    #[test]
    fn chapters_are_spelled_out() {
        assert_eq!(spell(1), "ONE");
        assert_eq!(spell(13), "THIRTEEN");
        assert_eq!(spell(20), "TWENTY");
        assert_eq!(spell(23), "TWENTY-THREE");
        assert_eq!(spell(40), "FORTY");
        assert_eq!(spell(99), "NINETY-NINE");
    }

    #[test]
    fn an_unreadable_project_file_is_left_alone() {
        let d = book("unreadable-map");
        let map = project_file(&d);
        // Not UTF-8: the author's notes must not become the scaffold.
        let bytes = b"# My notes\n\xff\xfe caf\xe9\n".to_vec();
        fs::write(&map, &bytes).unwrap();
        let p = Project::load(&d).unwrap();
        assert!(write_project_file(&p).is_err());
        assert_eq!(fs::read(&map).unwrap(), bytes, "nothing written");
        // Missing is fine: it starts fresh.
        fs::remove_file(&map).unwrap();
        write_project_file(&p).unwrap();
        assert!(fs::read_to_string(&map).unwrap().contains(BEGIN));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_list_before_the_synopsis_does_not_hide_it() {
        let front = "title: Gravel\naliases:\n  - The Lot\nsynopsis: Ada waits.\n";
        assert_eq!(
            front_value(front, "synopsis").as_deref(),
            Some("Ada waits.")
        );
        let quoted = "synopsis: \"\\\"Run,\\\" she said.\"\n";
        assert_eq!(
            front_value(quoted, "synopsis").as_deref(),
            Some("\"Run,\" she said.")
        );
    }

    #[test]
    fn map_links_use_forward_slashes_everywhere() {
        assert_eq!(
            link_target("manuscript\\01-Act-One\\01-Gravel.md"),
            "manuscript/01-Act-One/01-Gravel"
        );
        assert_eq!(link_target("notes/01-Ada.md"), "notes/01-Ada");
        assert_eq!(link_target("notes/01-Ada.md.md"), "notes/01-Ada.md");
    }

    use crate::project;
    use std::fs;

    /// A small book built by hand: Part One / Chapter One / Opening. Not the
    /// template, so these tests stay about levels rather than about its size.
    fn book(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-ms-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ch = d.join("manuscript/01-part-one/01-chapter-one");
        fs::create_dir_all(&ch).unwrap();
        fs::write(ch.join("01-opening.md"), "Words.\n").unwrap();
        fs::create_dir_all(d.join("notes/01-characters")).unwrap();
        d
    }

    /// A book with no parts: Chapter One / Opening straight under manuscript/.
    fn flat_book(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-ms-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ch = d.join("manuscript/01-chapter-one");
        fs::create_dir_all(&ch).unwrap();
        fs::write(ch.join("01-opening.md"), "Words.\n").unwrap();
        d
    }

    fn level(p: &Project, title: &str) -> Section {
        let i = p.nodes.iter().position(|n| n.title == title);
        section_of(p, i.unwrap_or_else(|| panic!("no {title} in the tree")))
    }

    #[test]
    fn an_empty_folder_inside_a_part_is_a_chapter() {
        let d = book("nested");
        project::create(&d.join("manuscript/01-part-one"), "The Lamp", true).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(level(&p, "The Lamp"), Section::Chapter);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_empty_folder_takes_the_level_of_its_neighbours() {
        let d = book("beside-parts");
        project::create(&d.join("manuscript"), "Book Two", true).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(level(&p, "Book Two"), Section::Part);
        fs::remove_dir_all(&d).unwrap();

        let d = flat_book("beside-chapters");
        project::create(&d.join("manuscript"), "Interlude", true).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(level(&p, "Interlude"), Section::Chapter);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_empty_folder_named_for_its_level_is_that_level() {
        let d = book("named-chapter");
        project::create(&d.join("manuscript"), "Chapter Two", true).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(level(&p, "Chapter Two"), Section::Chapter);
        fs::remove_dir_all(&d).unwrap();

        let d = flat_book("named-part");
        project::create(&d.join("manuscript"), "Part One", true).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(level(&p, "Part One"), Section::Part);
        fs::remove_dir_all(&d).unwrap();
    }
}
