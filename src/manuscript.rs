//! The project file, and compiling a manuscript.
//!
//! Two jobs, both centred on one idea: **`project.md` is the map.** A single
//! Markdown file at the project root describes the whole book — every part,
//! chapter and scene, with word counts, status and synopsis — and links to each
//! scene with an Obsidian wikilink, so the same file is a working index inside
//! a vault and a plain readable document outside one.
//!
//! Structure borrows from Scrivener, which is what most novelists actually use:
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

use anyhow::{Context, Result};
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

/// A container holding scenes is a chapter; one holding only containers is a
/// part. Depth alone lies as soon as someone skips the part level.
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
    if holds(idx, Kind::Scene) {
        return Section::Chapter;
    }
    if holds(idx, Kind::Container) {
        return Section::Part;
    }
    // A book that calls its parts Acts says "Act One" here, so the label from
    // novel.toml counts alongside the default word.
    match n.title.to_lowercase().split_whitespace().next() {
        Some("chapter") => return Section::Chapter,
        Some("part") => return Section::Part,
        Some(w) if w == p.meta.part_noun() => return Section::Part,
        _ => {}
    }
    let parent = n.path.parent();
    if p.nodes.iter().any(|m| m.kind == Kind::Container && Some(m.path.as_path()) == parent) {
        return Section::Chapter;
    }
    let neighbours: Vec<usize> = (0..p.nodes.len())
        .filter(|&i| i != idx && p.nodes[i].kind == Kind::Container && p.nodes[i].path.parent() == parent)
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
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .trim_end_matches(".md")
        .to_string()
}

fn commas(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Shunn wants an approximate count, not an exact one.
pub fn rounded_words(n: usize) -> usize {
    let step = if n < 2_000 {
        100
    } else if n < 10_000 {
        500
    } else {
        1_000
    };
    ((n + step / 2) / step) * step
}

const ONES: [&str; 20] = [
    "ZERO", "ONE", "TWO", "THREE", "FOUR", "FIVE", "SIX", "SEVEN", "EIGHT", "NINE", "TEN",
    "ELEVEN", "TWELVE", "THIRTEEN", "FOURTEEN", "FIFTEEN", "SIXTEEN", "SEVENTEEN", "EIGHTEEN",
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
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = structure_block(p);

    let out = if existing.is_empty() {
        format!("{}\n{body}\n", scaffold_head(p))
    } else if let (Some(a), Some(b)) = (existing.find(BEGIN), existing.find(END)) {
        // Splice: keep the author's prose above and below untouched.
        format!("{}{body}{}", &existing[..a], &existing[b + END.len()..])
    } else {
        format!("{}\n\n{body}\n", existing.trim_end())
    };

    std::fs::write(&path, out).with_context(|| format!("writing {}", path.display()))?;
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
        .filter(|n| n.kind == Kind::Scene && n.in_manuscript)
        .count();
    let excluded = p
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.in_manuscript && !n.compile)
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
        let _ = writeln!(s, "\n> {excluded} scene(s) marked `compile: false` — in the tree, out of the manuscript.");
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
    for &r in &p.roots {
        if p.nodes[r].in_manuscript && p.nodes[r].kind != Kind::Divider {
            emit(p, r, &mut s);
        }
    }

    let notes: Vec<usize> = p
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == Kind::Scene && !n.in_manuscript && !n.front_matter)
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

fn front_value(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        let (k, v) = line.split_once(':')?;
        if k.trim() == key {
            let v = v.trim().trim_matches('"').trim();
            return (!v.is_empty()).then(|| v.to_string());
        }
    }
    None
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
pub fn compile(p: &Project) -> Result<Compiled> {
    let m = &p.meta;
    let mut out = String::new();
    let mut words = 0usize;
    let mut chapters = 0usize;
    let mut scenes = 0usize;
    let mut skipped = 0usize;

    let total: usize = p
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.in_manuscript && n.compile)
        .map(|n| n.words())
        .sum();

    let front: Vec<&crate::project::Node> = p
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.front_matter && !n.body.trim().is_empty())
        .collect();

    if front.is_empty() {
        // No front matter, so generate a Shunn title page.
        if !m.author.is_empty() {
            let _ = writeln!(out, "{}  ", m.author);
        }
        let _ = writeln!(out, "\nabout {} words\n", commas(rounded_words(total)));
        let _ = writeln!(out, "# {}\n", m.title.to_uppercase());
        if !m.author.is_empty() {
            let _ = writeln!(out, "by {}\n", m.author);
        }
    } else {
        // Front matter owns the title page — same rule Scrivener uses. Emit it
        // verbatim rather than duplicating a generated one on top of it.
        for n in front {
            let _ = writeln!(out, "{}\n", n.body.trim());
        }
        let _ = writeln!(out, "\nabout {} words\n", commas(rounded_words(total)));
    }

    for &r in &p.roots {
        if p.nodes[r].in_manuscript && p.nodes[r].kind != Kind::Divider {
            walk(p, r, &mut out, &mut words, &mut chapters, &mut scenes, &mut skipped);
        }
    }

    let _ = writeln!(out, "\nTHE END");

    let slug: String = m
        .title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').replace("--", "-");
    let path = p.root.join(format!("{slug}-manuscript.md"));
    std::fs::write(&path, out).with_context(|| format!("writing {}", path.display()))?;

    Ok(Compiled {
        path,
        words,
        chapters,
        scenes,
        skipped,
    })
}

fn walk(
    p: &Project,
    idx: usize,
    out: &mut String,
    words: &mut usize,
    chapters: &mut usize,
    scenes: &mut usize,
    skipped: &mut usize,
) {
    let n = &p.nodes[idx];
    match section_of(p, idx) {
        Section::Part => {
            let _ = writeln!(out, "\n# {}\n", n.title.to_uppercase());
            for &c in &n.children {
                walk(p, c, out, words, chapters, scenes, skipped);
            }
        }
        Section::Chapter => {
            let any = n
                .children
                .iter()
                .any(|&c| p.nodes[c].kind == Kind::Scene && p.nodes[c].compile);
            if !any {
                return;
            }
            *chapters += 1;
            let _ = writeln!(out, "\n## CHAPTER {}\n", spell(*chapters));

            let mut first = true;
            for &c in &n.children {
                let s = &p.nodes[c];
                if s.kind != Kind::Scene {
                    continue;
                }
                if !s.compile {
                    *skipped += 1;
                    continue;
                }
                // Shunn: a scene break is `#` alone on a line. Escaped so
                // Markdown renders a literal hash instead of an empty heading.
                if !first {
                    let _ = writeln!(out, "\n\\#\n");
                }
                first = false;
                let _ = writeln!(out, "{}", s.body.trim());
                *words += s.words();
                *scenes += 1;
            }
        }
        Section::Scene => {
            let s = &p.nodes[idx];
            if !s.compile {
                *skipped += 1;
                return;
            }
            let _ = writeln!(out, "{}", s.body.trim());
            *words += s.words();
            *scenes += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_counts_round_the_way_shunn_asks() {
        assert_eq!(rounded_words(1_240), 1_200);
        assert_eq!(rounded_words(1_260), 1_300);
        assert_eq!(rounded_words(6_400), 6_500);
        assert_eq!(rounded_words(82_400), 82_000);
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
