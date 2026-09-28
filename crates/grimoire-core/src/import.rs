//! Bringing in a draft written somewhere else — a Word file, Markdown or plain
//! text — as a new book, split into parts, chapters and scenes at its
//! headings and scene breaks. Nothing is dropped: every word of the draft
//! lands in a scene, and the scene files are ordinary Markdown.

use crate::project::{Shape, ShapeChapter, ShapePart, ShapeScene};
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::Path;

/// The kinds of file a draft can be, by extension.
pub const KINDS: &[&str] = &["docx", "md", "markdown", "txt", "text"];

/// Whether `path` looks like a draft Grimoire can bring in.
pub fn is_draft(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| KINDS.contains(&e.to_lowercase().as_str()))
}

/// A draft, read and split up, ready to become a book.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// From the file itself (a Word title, Markdown frontmatter, a lone top
    /// heading), or the file's name.
    pub title: String,
    pub shape: Shape,
    /// What came before the first chapter in a draft that has chapters: a
    /// title page, a contents list, the author's address. Kept, in Notes,
    /// rather than made into a chapter nobody wrote.
    pub before: String,
}

impl Draft {
    pub fn chapters(&self) -> usize {
        self.shape.parts.iter().map(|p| p.chapters.len()).sum()
    }

    pub fn scenes(&self) -> usize {
        self.shape
            .parts
            .iter()
            .flat_map(|p| &p.chapters)
            .map(|c| c.scenes.len())
            .sum()
    }

    pub fn words(&self) -> usize {
        self.before.split_whitespace().count() + self.manuscript_words()
    }

    /// The words that went into the manuscript itself.
    pub fn manuscript_words(&self) -> usize {
        self.shape
            .parts
            .iter()
            .flat_map(|p| &p.chapters)
            .flat_map(|c| &c.scenes)
            .map(|s| s.body.split_whitespace().count())
            .sum()
    }
}

/// One paragraph-level piece of the draft.
#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// A heading the file marks as one (`#`, a Word heading style), by level.
    Heading(u8, String),
    /// The book's own title (Word's Title style).
    Title(String),
    /// `***`, `* * *`, `#`, `~`, `§`, `⁂`: the end of a scene.
    Break,
    Para(String),
}

/// Read the draft at `path`.
pub fn read(path: &Path) -> Result<Draft> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let (blocks, front_title) = match ext.as_str() {
        "docx" => (docx_blocks(path)?, None),
        "md" | "markdown" | "txt" | "text" => {
            let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let text = String::from_utf8_lossy(&raw).replace("\r\n", "\n");
            let (front, body) = split_frontmatter(&text);
            (text_blocks(body, ext.starts_with('m')), front)
        }
        _ => {
            bail!("Grimoire can bring in a Word file (.docx), Markdown (.md) or plain text (.txt)")
        }
    };
    // "wolf_winter.docx" is "Wolf Winter".
    let fallback = path
        .file_stem()
        .map(|s| {
            s.to_string_lossy()
                .replace(['_', '-'], " ")
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.chars().any(char::is_uppercase) {
                s
            } else {
                s.split(' ')
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        })
        .unwrap_or_else(|| "Untitled".into());
    let draft = build(blocks, front_title, fallback);
    if draft.words() == 0 {
        bail!("there are no words in {} to bring in", path.display());
    }
    Ok(draft)
}

/// Make the book at `root` from `draft`: its manuscript, every section any
/// book has, and (if there was any) what came before the first chapter as a
/// note in Notes. `root` must be new or empty.
pub fn make_book(root: &Path, draft: &Draft) -> Result<()> {
    std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    crate::project::scaffold_with(root, Some(&draft.title), &draft.shape)?;
    if !draft.before.trim().is_empty() {
        let notes = crate::project::Area::Notes.path(root);
        let note = crate::project::create(&notes, "Before the first chapter", false)?;
        let mut text = std::fs::read_to_string(&note)?;
        text.push_str("From the start of the draft, before its first chapter.\n\n");
        text.push_str(draft.before.trim());
        text.push('\n');
        crate::atomic::write_text(&note, &text)?;
    }
    Ok(())
}

/// Markdown frontmatter's `title:`, and the text after the frontmatter.
fn split_frontmatter(text: &str) -> (Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (None, text);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, text);
    };
    let title = rest[..end]
        .lines()
        .find_map(|l| l.strip_prefix("title:"))
        .map(|t| t.trim().trim_matches(['"', '\'']).to_string())
        .filter(|t| !t.is_empty());
    let after = &rest[end + 4..];
    (title, after.strip_prefix('\n').unwrap_or(after))
}

fn is_break(line: &str) -> bool {
    // Markdown exports escape a lone `#` or `*` as `\#`, `\*`.
    let unescaped = line.replace('\\', "");
    let t = unescaped.trim();
    !t.is_empty()
        && t.chars().count() <= 12
        && t.chars()
            .all(|c| matches!(c, '*' | '#' | '~' | '§' | '⁂' | '•' | '·' | '-' | '—' | '=' | ' '))
        // A lone dash could be the start of a list; three of anything is a break.
        && (t.chars().filter(|c| *c != ' ').count() >= 3 || matches!(t, "#" | "§" | "⁂" | "~" | "*"))
}

/// Paragraphs of Markdown or plain text: blank lines between them, headings
/// marked with `#` (Markdown only; in plain text a `#` line is a break).
fn text_blocks(text: &str, markdown: bool) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join("\n")));
            para.clear();
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        if is_break(t) {
            flush(&mut para, &mut out);
            out.push(Block::Break);
            continue;
        }
        if markdown {
            let hashes = t.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
                flush(&mut para, &mut out);
                let text = t[hashes..].trim().trim_end_matches('#').trim();
                out.push(Block::Heading(hashes as u8, text.to_string()));
                continue;
            }
        }
        para.push(line.trim_end());
    }
    flush(&mut para, &mut out);
    out
}

/// "Part Two", "BOOK ONE", "Act 3", "Part IV: The Sea": a heading for a
/// part. "Book of Days" is a chapter.
fn is_part(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    let mut words = lower.split(|c: char| c.is_whitespace() || c == ':' || c == '.');
    let (Some(first), Some(second)) = (words.next(), words.find(|w| !w.is_empty())) else {
        return false;
    };
    const NUMBERS: &[&str] = &[
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve", "first", "second", "third", "fourth", "fifth", "sixth", "last", "final",
    ];
    matches!(first, "part" | "book" | "act" | "volume")
        && (second.chars().all(|c| c.is_ascii_digit())
            || second.chars().all(|c| matches!(c, 'i' | 'v' | 'x' | 'l'))
            || NUMBERS.contains(&second))
}

/// A paragraph standing alone that reads as a chapter heading in a file with
/// no headings marked: "Chapter 3", "CHAPTER THREE: The Door", "Prologue".
fn is_plain_chapter(text: &str) -> bool {
    let t = text.trim();
    if t.contains('\n') || t.chars().count() > 70 {
        return false;
    }
    let lower = t.to_lowercase();
    let first = lower
        .split(|c: char| c.is_whitespace() || c == ':' || c == '.')
        .next()
        .unwrap_or("");
    matches!(first, "chapter" | "prologue" | "epilogue" | "interlude")
}

/// "PART ONE" as "Part One": a heading in capitals is a style, not a name.
fn tidy(heading: &str) -> String {
    let t = heading.trim();
    if t.chars().any(|c| c.is_lowercase()) || !t.chars().any(|c| c.is_uppercase()) {
        return t.to_string();
    }
    t.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_string() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Turn the draft's blocks into parts, chapters and scenes.
fn build(mut blocks: Vec<Block>, front_title: Option<String>, fallback: String) -> Draft {
    let marked = blocks.iter().any(|b| matches!(b, Block::Heading(..)));
    // With no headings marked, lines that read as headings count as them.
    if !marked {
        for b in &mut blocks {
            if let Block::Para(t) = b
                && (is_plain_chapter(t)
                    || (is_part(t) && t.chars().count() <= 40 && !t.contains('\n')))
            {
                *b = Block::Heading(if is_part(t) { 1 } else { 2 }, t.trim().to_string());
            }
        }
    }
    let mut title = front_title;
    if title.is_none() {
        title = blocks.iter().find_map(|b| match b {
            Block::Title(t) => Some(t.clone()),
            _ => None,
        });
    }
    // One top-level heading above deeper ones is the book's title, unless
    // it reads as a chapter ("Prologue").
    let levels: Vec<u8> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Heading(l, t) if !is_part(t) => Some(*l),
            _ => None,
        })
        .collect();
    if let Some(&top) = levels.iter().min() {
        let at_top = levels.iter().filter(|&&l| l == top).count();
        let first_heading = blocks.iter().position(|b| matches!(b, Block::Heading(..)));
        if at_top == 1
            && levels.iter().any(|&l| l > top)
            && let Some(i) = first_heading
            && matches!(&blocks[i], Block::Heading(l, t) if *l == top && !is_part(t) && !is_plain_chapter(t))
            && let Block::Heading(_, t) = blocks.remove(i)
        {
            title.get_or_insert(t);
        }
    }
    // The chapters are the level most "Chapter …" headings sit at; with
    // none named that way, the highest level that isn't a part.
    let named: Vec<u8> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Heading(l, t)
                if is_plain_chapter(t) && t.to_lowercase().starts_with("chapter") =>
            {
                Some(*l)
            }
            _ => None,
        })
        .collect();
    let chapter_level = if named.is_empty() {
        blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading(l, t) if !is_part(t) => Some(*l),
                _ => None,
            })
            .min()
    } else {
        (1..=9u8).max_by_key(|l| named.iter().filter(|&&n| n == *l).count())
    };
    // "Prologue" above the chapters' level is a chapter too.
    let heads_chapter = |l: u8, t: &str| {
        Some(l) == chapter_level || (is_plain_chapter(t) && chapter_level.is_some_and(|c| l <= c))
    };
    for b in &mut blocks {
        if let Block::Heading(_, t) = b {
            *t = tidy(t);
        }
    }
    // A draft with chapters: anything before the first one isn't a chapter.
    let structured = blocks.iter().any(|b| matches!(b, Block::Heading(..)));
    let mut started = !structured;
    let mut before: Vec<String> = Vec::new();

    let mut parts: Vec<ShapePart> = Vec::new();
    let mut part: Option<ShapePart> = None;
    let mut chapters: Vec<ShapeChapter> = Vec::new();
    let mut chapter: Option<ShapeChapter> = None;
    let mut scene_title: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    fn close_scene(
        chapter: &mut Option<ShapeChapter>,
        title: &mut Option<String>,
        body: &mut Vec<String>,
    ) {
        let text = body.join("\n\n");
        body.clear();
        let titled = title.take();
        if text.trim().is_empty() && titled.is_none() {
            return;
        }
        let ch = chapter.get_or_insert_with(|| ShapeChapter {
            title: "Opening".into(),
            scenes: Vec::new(),
        });
        let n = ch.scenes.len() + 1;
        ch.scenes.push(ShapeScene {
            title: titled.unwrap_or_else(|| crate::manuscript::numbered("Scene", n)),
            status: "draft",
            target: None,
            body: text,
        });
    }
    fn close_chapter(chapters: &mut Vec<ShapeChapter>, chapter: &mut Option<ShapeChapter>) {
        if let Some(c) = chapter.take() {
            chapters.push(c);
        }
    }

    for b in blocks {
        let heads_a_chapter = match &b {
            Block::Heading(l, t) => is_part(t) || heads_chapter(*l, t),
            _ => false,
        };
        if !started && !heads_a_chapter {
            match b {
                Block::Title(t) if Some(&t) == title.as_ref() => {}
                Block::Title(t) | Block::Heading(_, t) | Block::Para(t) => before.push(t),
                Block::Break => {}
            }
            continue;
        }
        started = true;
        match b {
            // The title is the book's name; a second one is just words.
            Block::Title(t) if Some(&t) == title.as_ref() => {}
            Block::Title(t) => body.push(t),
            Block::Heading(_, t) if is_part(&t) => {
                close_scene(&mut chapter, &mut scene_title, &mut body);
                close_chapter(&mut chapters, &mut chapter);
                let earlier = std::mem::take(&mut chapters);
                match part.take() {
                    Some(mut p) => {
                        p.chapters = earlier;
                        parts.push(p);
                    }
                    // Chapters before the first part (a prologue) stand on
                    // their own, ahead of it.
                    None if !earlier.is_empty() => parts.push(ShapePart {
                        title: None,
                        chapters: earlier,
                    }),
                    None => {}
                }
                part = Some(ShapePart {
                    title: Some(t),
                    chapters: Vec::new(),
                });
            }
            Block::Heading(l, t) if heads_chapter(l, &t) => {
                close_scene(&mut chapter, &mut scene_title, &mut body);
                close_chapter(&mut chapters, &mut chapter);
                // "1" alone isn't a name anyone could find in the outline.
                let title = if t.chars().all(|c| c.is_ascii_digit() || c == '.') && !t.is_empty() {
                    format!("Chapter {}", t.trim_end_matches('.'))
                } else {
                    t
                };
                chapter = Some(ShapeChapter {
                    title,
                    scenes: Vec::new(),
                });
            }
            Block::Heading(_, t) => {
                close_scene(&mut chapter, &mut scene_title, &mut body);
                scene_title = Some(t);
            }
            Block::Break => close_scene(&mut chapter, &mut scene_title, &mut body),
            Block::Para(t) => body.push(t),
        }
    }
    // "END" or "THE END" closing a manuscript is a marker, and the export
    // adds its own; kept, it would print twice.
    if body.last().is_some_and(|t| {
        matches!(
            t.trim().to_lowercase().trim_matches(['.', '*']),
            "end" | "the end"
        )
    }) {
        body.pop();
    }
    close_scene(&mut chapter, &mut scene_title, &mut body);
    close_chapter(&mut chapters, &mut chapter);
    match part.take() {
        Some(mut p) => {
            p.chapters = chapters;
            parts.push(p);
        }
        None => parts.push(ShapePart {
            title: None,
            chapters,
        }),
    }
    // A draft with no chapter headings at all is one chapter.
    for p in &mut parts {
        for c in &mut p.chapters {
            if c.title == "Opening" && chapter_level.is_none() {
                c.title = "Chapter One".into();
            }
        }
    }
    parts.retain(|p| !p.chapters.is_empty());
    Draft {
        title: title.unwrap_or(fallback),
        shape: Shape {
            // A draft already past 80,000 aims for the next ten thousand.
            target_words: {
                let words: usize = parts
                    .iter()
                    .flat_map(|p| &p.chapters)
                    .flat_map(|c| &c.scenes)
                    .map(|s| s.body.split_whitespace().count())
                    .sum();
                80_000.max((words / 10_000 + 1) * 10_000)
            },
            daily_target: 1_000,
            parts,
        },
        before: before.join("\n\n"),
    }
}

// ---- Word ----------------------------------------------------------------

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn docx_part(path: &Path, name: &str) -> Result<Option<String>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("{} isn't a Word file Grimoire can open", path.display()))?;
    let Ok(mut entry) = zip.by_name(name) else {
        return Ok(None);
    };
    let mut xml = String::new();
    entry.read_to_string(&mut xml)?;
    Ok(Some(xml))
}

/// What the draft's styles mean, from styles.xml.
#[derive(Default)]
struct Styles {
    /// Paragraph styles that are headings: style id → level (0 for Title).
    /// Word stores the id; the name ("heading 1") is the same in every
    /// language, the id isn't.
    headings: std::collections::HashMap<String, u8>,
    /// Character styles that are italic or bold ("Emphasis", "Strong").
    chars: std::collections::HashMap<String, (bool, bool)>,
}

fn read_styles(path: &Path) -> Result<Styles> {
    let mut out = Styles::default();
    let Some(xml) = docx_part(path, "word/styles.xml")? else {
        return Ok(out);
    };
    let doc = roxmltree::Document::parse(&xml).context("reading the Word file's styles")?;
    for style in doc.descendants().filter(|n| n.has_tag_name((W, "style"))) {
        let Some(id) = style.attribute((W, "styleId")) else {
            continue;
        };
        let name = style
            .children()
            .find(|n| n.has_tag_name((W, "name")))
            .and_then(|n| n.attribute((W, "val")))
            .unwrap_or("")
            .to_lowercase();
        if style.attribute((W, "type")) == Some("character") {
            let props = style.children().find(|n| n.has_tag_name((W, "rPr")));
            let italic = on(props, "i") || name == "emphasis";
            let bold = on(props, "b") || name == "strong";
            if italic || bold {
                out.chars.insert(id.to_string(), (italic, bold));
            }
        } else if name == "title" {
            out.headings.insert(id.to_string(), 0);
        } else if let Some(n) = name.strip_prefix("heading ")
            && let Ok(level) = n.trim().parse::<u8>()
        {
            out.headings.insert(id.to_string(), level);
        }
    }
    Ok(out)
}

fn on(run_props: Option<roxmltree::Node>, tag: &str) -> bool {
    run_props
        .and_then(|p| p.children().find(|n| n.has_tag_name((W, tag))))
        .is_some_and(|n| !matches!(n.attribute((W, "val")), Some("0" | "false" | "none")))
}

/// A Word paragraph as Markdown: italic and bold kept, everything else plain.
fn docx_paragraph(p: roxmltree::Node, styles: &Styles) -> String {
    let mut out = String::new();
    let mut open: (bool, bool) = (false, false);
    let mut pending_space = String::new();
    let mark = |out: &mut String, from: (bool, bool), to: (bool, bool)| {
        if from.0 && !to.0 {
            out.push('*');
        }
        if from.1 && !to.1 {
            out.push_str("**");
        }
        if !from.1 && to.1 {
            out.push_str("**");
        }
        if !from.0 && to.0 {
            out.push('*');
        }
    };
    for r in p.descendants().filter(|n| n.has_tag_name((W, "r"))) {
        // Deleted text in tracked changes isn't part of the draft.
        if r.ancestors().any(|a| a.has_tag_name((W, "del"))) {
            continue;
        }
        let props = r.children().find(|n| n.has_tag_name((W, "rPr")));
        let (styled_i, styled_b) = props
            .and_then(|p| p.children().find(|n| n.has_tag_name((W, "rStyle"))))
            .and_then(|s| s.attribute((W, "val")))
            .and_then(|id| styles.chars.get(id).copied())
            .unwrap_or((false, false));
        let style = (on(props, "i") || styled_i, on(props, "b") || styled_b);
        for c in r.children() {
            let text = if c.has_tag_name((W, "t")) {
                c.text().unwrap_or("").to_string()
            } else if c.has_tag_name((W, "tab")) {
                " ".into()
            } else if c.has_tag_name((W, "br")) && c.attribute((W, "type")).is_none() {
                "\n".into()
            } else {
                continue;
            };
            // Spaces sit outside the * marks, or Markdown won't see them.
            let lead: String = text.chars().take_while(|c| c.is_whitespace()).collect();
            let core = text.trim();
            if core.is_empty() {
                pending_space.push_str(&text);
                continue;
            }
            let trail: String = text
                .chars()
                .rev()
                .take_while(|c| c.is_whitespace())
                .collect();
            if style != open {
                mark(&mut out, open, (false, false));
                out.push_str(&pending_space);
                out.push_str(&lead);
                mark(&mut out, (false, false), style);
                open = style;
            } else {
                out.push_str(&pending_space);
                out.push_str(&lead);
            }
            pending_space.clear();
            out.push_str(core);
            pending_space.push_str(&trail);
        }
    }
    mark(&mut out, open, (false, false));
    out.trim().to_string()
}

fn docx_blocks(path: &Path) -> Result<Vec<Block>> {
    let styles = read_styles(path)?;
    let Some(xml) = docx_part(path, "word/document.xml")? else {
        bail!("{} has no document inside it", path.display());
    };
    let doc = roxmltree::Document::parse(&xml).context("reading the Word file")?;
    let mut out = Vec::new();
    for p in doc.descendants().filter(|n| n.has_tag_name((W, "p"))) {
        let style = p
            .children()
            .find(|n| n.has_tag_name((W, "pPr")))
            .and_then(|pr| pr.children().find(|n| n.has_tag_name((W, "pStyle"))))
            .and_then(|s| s.attribute((W, "val")))
            .and_then(|id| {
                styles
                    .headings
                    .get(id)
                    .copied()
                    .or_else(|| builtin_heading(id))
            });
        let text = docx_paragraph(p, &styles);
        if text.is_empty() {
            continue;
        }
        let plain = || text.replace(['*'], "").trim().to_string();
        out.push(match style {
            Some(0) => Block::Title(plain()),
            Some(level) => Block::Heading(level, plain()),
            None if is_break(&text) => Block::Break,
            None => Block::Para(text),
        });
    }
    Ok(out)
}

/// Heading style ids Word and Google Docs use when styles.xml is missing.
fn builtin_heading(id: &str) -> Option<u8> {
    if id.eq_ignore_ascii_case("title") {
        return Some(0);
    }
    id.strip_prefix("Heading")
        .and_then(|n| n.parse::<u8>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> usize {
        s.split_whitespace().count()
    }

    /// Each part's title, and its chapters' titles with their scenes'.
    type Outline = Vec<(Option<String>, Vec<(String, Vec<String>)>)>;

    fn shape_of(d: &Draft) -> Outline {
        d.shape
            .parts
            .iter()
            .map(|p| {
                (
                    p.title.clone(),
                    p.chapters
                        .iter()
                        .map(|c| {
                            (
                                c.title.clone(),
                                c.scenes.iter().map(|s| s.title.clone()).collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn markdown_splits_at_chapters_and_scene_breaks_and_keeps_every_word() {
        let md = "---\ntitle: The Salt Archive\n---\n\nA note before it all.\n\n## Chapter One\n\nThe rain had stopped.\n\n\\#\n\nOren had promised.\n\n## CHAPTER TWO\n\nShe crossed the lot.\n";
        let (front, body) = split_frontmatter(md);
        let d = build(text_blocks(body, true), front, "x".into());
        assert_eq!(d.title, "The Salt Archive");
        assert_eq!(d.before, "A note before it all.");
        assert_eq!(
            shape_of(&d),
            vec![(
                None,
                vec![
                    (
                        "Chapter One".into(),
                        vec!["Scene One".into(), "Scene Two".into()]
                    ),
                    ("Chapter Two".into(), vec!["Scene One".into()]),
                ]
            )]
        );
        let body_words = words(
            "A note before it all. The rain had stopped. Oren had promised. She crossed the lot.",
        );
        assert_eq!(d.words(), body_words);
    }

    #[test]
    fn a_lone_top_heading_is_the_title_and_parts_hold_chapters() {
        let md = "# Wolf Winter\n\n## Prologue\n\nCold.\n\n# Part One\n\n## 1\n\nSnow.\n\n## 2\n\nMore snow.\n\n# Part Two\n\n## 3\n\n### The Thaw\n\nWater.\n";
        let d = build(text_blocks(md, true), None, "x".into());
        assert_eq!(d.title, "Wolf Winter");
        assert_eq!(
            shape_of(&d),
            vec![
                (None, vec![("Prologue".into(), vec!["Scene One".into()])]),
                (
                    Some("Part One".into()),
                    vec![
                        ("Chapter 1".into(), vec!["Scene One".into()]),
                        ("Chapter 2".into(), vec!["Scene One".into()]),
                    ]
                ),
                (
                    Some("Part Two".into()),
                    vec![("Chapter 3".into(), vec!["The Thaw".into()])]
                ),
            ]
        );
    }

    #[test]
    fn a_prologue_above_the_chapters_level_is_a_chapter_not_the_title() {
        let md = "# Prologue\n\nBefore the war.\n\n# Part One\n\n## Chapter 1\n\nBoats.\n\n## Chapter 2\n\n### The Pier\n\nGone.\n";
        let d = build(text_blocks(md, true), None, "Harbour".into());
        assert_eq!(d.title, "Harbour");
        assert_eq!(
            shape_of(&d),
            vec![
                (None, vec![("Prologue".into(), vec!["Scene One".into()])]),
                (
                    Some("Part One".into()),
                    vec![
                        ("Chapter 1".into(), vec!["Scene One".into()]),
                        ("Chapter 2".into(), vec!["The Pier".into()]),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn plain_text_finds_chapter_lines_and_a_file_with_none_is_one_chapter() {
        let txt = "CHAPTER ONE\n\nIt began\nwith a letter.\n\n#\n\nThen another.\n\nChapter 2: The Reply\n\nNo one wrote back.\n\nTHE END\n";
        let d = build(text_blocks(txt, false), None, "Letters".into());
        assert_eq!(d.title, "Letters");
        assert_eq!(d.words(), 11, "THE END is left for the export to add");
        assert_eq!(
            shape_of(&d),
            vec![(
                None,
                vec![
                    (
                        "Chapter One".into(),
                        vec!["Scene One".into(), "Scene Two".into()]
                    ),
                    ("Chapter 2: The Reply".into(), vec!["Scene One".into()]),
                ]
            )]
        );
        let none = build(
            text_blocks("Just a story.\n\n***\n\nThe last line.", false),
            None,
            "Tiny".into(),
        );
        assert_eq!(
            shape_of(&none),
            vec![(
                None,
                vec![(
                    "Chapter One".into(),
                    vec!["Scene One".into(), "Scene Two".into()]
                )]
            )]
        );
    }

    #[test]
    fn word_keeps_italics_and_reads_heading_styles_by_name() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Titel"/></w:pPr><w:r><w:t>The Long Night</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="berschrift1"/></w:pPr><w:r><w:t>Chapter One</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">It was </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">very </w:t></w:r><w:r><w:t>late.</w:t></w:r></w:p>
            <w:p><w:r><w:t>* * *</w:t></w:r></w:p>
            <w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Morning.</w:t></w:r><w:del><w:r><w:t>gone</w:t></w:r></w:del></w:p>
            </w:body></w:document>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let paras: Vec<String> = doc
            .descendants()
            .filter(|n| n.has_tag_name((W, "p")))
            .map(|p| docx_paragraph(p, &Styles::default()))
            .collect();
        assert_eq!(paras[2], "It was *very* late.");
        assert_eq!(paras[4], "**Morning.**");
        assert_eq!(builtin_heading("Heading2"), Some(2));
    }
}
