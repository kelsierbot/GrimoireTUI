//! Export: the manuscript as files a reader can open, with no other tools.
//!
//! - **DOCX in standard manuscript format** (Shunn's modern novel format) —
//!   what agents and editors ask for: Times New Roman 12pt, double-spaced,
//!   half-inch first-line indents, one-inch margins, a running
//!   `Surname / KEYWORD / page` header, chapters opening a third of the way
//!   down a fresh page, `#` for a scene break and `END` at the end.
//! - **EPUB 3** with an EPUB 2 table of contents alongside, so the same file
//!   opens in current readers and in the older ones still on people's shelves.
//! - **Shunn Markdown**, the same text `grimoire compile` writes.
//!
//! All of them are drawn from one [`Book`], built by the same walk `compile`
//! uses — Scrivener's Compile, in short:
//!
//! - a folder holding scenes is a chapter, one holding chapters is a part;
//! - `compile: false` scenes stay out, and so do empty scenes, chapters with
//!   nothing written in them, and parts with no chapters — a template's
//!   twenty-four untouched chapters never become twenty-four blank pages;
//! - a scene break goes only between two scenes that have words in them;
//! - `%% notes %%` come out; a TK stays in, since a hole silently closed up
//!   is worse than one left showing — and [`tks`] lists them before export;
//! - book typography — curly quotes, real dashes, ellipses ([`crate::typeset`])
//!   — is done once, here, for every format (the classic Courier manuscript
//!   turns it back into typewriter marks);
//! - a Prologue, Epilogue or Interlude keeps its own heading and takes no
//!   chapter number;
//! - front matter replaces the generated title page;
//! - choosing some parts, or a submission sample ([`Scope`]), never
//!   renumbers the chapters, and the title page still gives the whole book's
//!   word count.

use anyhow::{Context, Result, bail};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use crate::manuscript::{Section, commas, numbered, rounded_words, section_of, spell};
use crate::notes;
use crate::project::{Kind, Node, Project};
use crate::submission::{
    ChapterHeading, Contact, FirstParagraph, Format, Manuscript, Paper, Spacing,
};
use crate::typeset;

/// Roughly how much of a novel fits on one double-spaced manuscript page.
const WORDS_PER_PAGE: usize = 250;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub docx: bool,
    /// The manuscript DOCX converted to PDF by LibreOffice ([`crate::pdf`]).
    /// Implies the DOCX.
    pub pdf: bool,
    pub epub: bool,
    pub markdown: bool,
    /// Node indices of the top-level parts to include, as [`parts`] lists
    /// them. `None` is the whole book.
    pub parts: Option<Vec<usize>>,
    /// All of it, or the sample an agent asked for.
    pub scope: Scope,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            docx: true,
            pdf: false,
            epub: true,
            markdown: false,
            parts: None,
            scope: Scope::Whole,
        }
    }
}

/// How much of the book: agents ask for "the first three chapters", or "the
/// first fifty pages" (about 12,500 words).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    Whole,
    /// Chapters `from` to `to` by their numbers, counting from 1, with any
    /// unnumbered chapter — a prologue, an interlude — that falls among them.
    /// A prologue comes with chapter 1; an epilogue with the last chapter.
    Chapters { from: usize, to: usize },
    /// From the start, a whole scene at a time, until at least this many words.
    Words(usize),
}

impl Scope {
    /// The first `n` chapters.
    pub fn first(n: usize) -> Scope {
        Scope::Chapters {
            from: 1,
            to: n.max(1),
        }
    }

    /// "the whole book", "chapters 1–3", "the first 10,000 words".
    pub fn describe(&self) -> String {
        match *self {
            Scope::Whole => "the whole book".into(),
            Scope::Chapters { from: 1, to: 1 } => "the first chapter".into(),
            Scope::Chapters { from: 1, to } => format!("the first {to} chapters"),
            Scope::Chapters { from, to } if from == to => format!("chapter {from}"),
            Scope::Chapters { from, to } => format!("chapters {from}–{to}"),
            Scope::Words(n) => format!("the first {} words", commas(n)),
        }
    }

    /// What a sample adds to its file name.
    fn file_suffix(&self) -> String {
        match *self {
            Scope::Whole => String::new(),
            Scope::Chapters { from, to } if from == to => format!("_Chapter-{from}"),
            Scope::Chapters { from, to } => format!("_Chapters-{from}-{to}"),
            Scope::Words(n) => format!("_First-{n}-Words"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Exported {
    pub files: Vec<PathBuf>,
    pub words: usize,
    pub chapters: usize,
    /// About how many manuscript pages: ~250 words a page, plus the title
    /// page, and a page for each chapter opener and part heading.
    pub pages: usize,
    /// Where the TKs still in the exported text are, one line each.
    pub tks: Vec<String>,
    /// Scenes, chapters and parts left out because nothing was written in them.
    pub empty: usize,
    /// The PDF couldn't be made, and why. The other files were still written.
    pub pdf_error: Option<String>,
}

/// The book's top-level parts (acts), in order: node index and title.
pub fn parts(p: &Project) -> Vec<(usize, String)> {
    manuscript_roots(p)
        .filter(|&i| p.nodes[i].kind == Kind::Container && section_of(p, i) == Section::Part)
        .map(|i| (i, p.nodes[i].title.clone()))
        .collect()
}

/// The TKs the export would carry, where they are — to warn before it runs.
pub fn tks(p: &Project, opts: &ExportOptions) -> Result<Vec<String>> {
    Ok(book(p, opts.parts.as_deref(), opts.scope)?.tks)
}

/// Write the chosen formats to `<book>/exports/`.
pub fn export(p: &Project, opts: &ExportOptions) -> Result<Exported> {
    p.ensure_whole()?;
    if !(opts.docx || opts.pdf || opts.epub || opts.markdown) {
        bail!("nothing to export — choose Word, PDF, EPUB or Markdown");
    }
    let mut book = book(p, opts.parts.as_deref(), opts.scope)?;
    if book.pieces.is_empty() {
        bail!("there's nothing written to export yet");
    }
    let dir = p.root.join("exports");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let stem = slug(book.title);
    let manuscript = manuscript_stem(&book, opts.parts.is_some(), opts.scope);

    let mut files = Vec::new();
    let mut put = |name: &str, bytes: &[u8]| -> Result<PathBuf> {
        let path = dir.join(name);
        crate::atomic::write(&path, bytes)?;
        files.push(path.clone());
        Ok(path)
    };
    let mut pdf_error = None;
    if opts.docx || opts.pdf {
        let docx_path = put(&format!("{manuscript}.docx"), &docx(&book)?)?;
        if opts.pdf {
            match crate::pdf::find() {
                None => pdf_error = Some(crate::pdf::HOW_TO_GET.to_string()),
                Some(office) => match crate::pdf::convert(&office, &docx_path) {
                    Ok(pdf) => files.push(pdf),
                    Err(e) => pdf_error = Some(format!("{e:#}")),
                },
            }
        }
    }
    let mut put = |name: &str, bytes: &[u8]| -> Result<()> {
        let path = dir.join(name);
        crate::atomic::write(&path, bytes)?;
        files.push(path);
        Ok(())
    };
    if opts.epub {
        let submission = std::mem::replace(&mut book.front, front_pages(p, Edition::Ebook));
        put(&format!("{stem}.epub"), &epub(&book)?)?;
        book.front = submission;
    }
    if opts.markdown {
        put(&format!("{stem}.md"), markdown(&book).as_bytes())?;
    }

    Ok(Exported {
        files,
        words: book.words,
        chapters: book.chapters,
        pages: pages(&book),
        tks: book.tks.clone(),
        empty: book.empty,
        pdf_error,
    })
}

/// `Marlowe_The-Salt-Archive_Manuscript` — "Lastname_Title", as agents ask,
/// with what a sample holds after it. Safe on every system (names rules).
fn manuscript_stem(b: &Book, some_parts: bool, scope: Scope) -> String {
    let name = crate::names::stem(surname(b.author));
    let title = crate::names::stem(b.title);
    let mut stem = if surname(b.author).is_empty() {
        format!("{title}_Manuscript")
    } else {
        format!("{name}_{title}_Manuscript")
    };
    if some_parts {
        let kept: Vec<String> = b
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Part { title, .. } => Some(crate::names::stem(title)),
                _ => None,
            })
            .collect();
        if !kept.is_empty() {
            stem.push('_');
            stem.push_str(&kept.join("_"));
        }
    }
    stem.push_str(&scope.file_suffix());
    stem
}

// ── the book ─────────────────────────────────────────────────────────

/// The manuscript as it will be read: what's in, in order, numbered.
pub(crate) struct Book<'a> {
    pub title: &'a str,
    /// The byline.
    pub author: &'a str,
    /// The title page's contact block.
    pub contact: &'a Contact,
    /// How the submission manuscript looks.
    pub look: &'a Manuscript,
    /// Front-matter scenes, which stand in for the generated title page.
    /// Notes are already out of every piece of text in here, and the
    /// typography done.
    pub front: Vec<Cow<'a, str>>,
    /// Words in every compiled scene of the whole book — the title page's
    /// count, whatever part of it this export holds.
    pub total: usize,
    pub pieces: Vec<Piece<'a>>,
    /// Words, chapters and scenes in this export.
    pub words: usize,
    pub chapters: usize,
    pub scenes: usize,
    /// Scenes marked `compile: false`.
    pub skipped: usize,
    /// Scenes, chapters and parts left out for having nothing in them.
    pub empty: usize,
    /// Every TK still in the exported text: "Chapter Two, Gravel: …took the TK…".
    pub tks: Vec<String>,
    /// This export runs to the book's end, so END goes after it. A sample
    /// doesn't say END.
    pub to_the_end: bool,
}

pub(crate) enum Piece<'a> {
    Part {
        title: &'a str,
        depth: usize,
    },
    /// `number` is the chapter's place in the whole book, not in this export;
    /// `None` for a chapter that takes no number (a Prologue, an Epilogue).
    Chapter {
        number: Option<usize>,
        title: &'a str,
        depth: usize,
        scenes: Vec<Cow<'a, str>>,
    },
    /// A scene sitting outside any chapter.
    Scene(Cow<'a, str>),
}

fn manuscript_roots(p: &Project) -> impl Iterator<Item = usize> + '_ {
    p.manuscript().into_iter()
}

/// The edition an export is for. Front matter in a folder named for one goes
/// only into that one; front matter loose in the section goes into all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edition {
    Manuscript,
    Paperback,
    Ebook,
}

fn edition_of(p: &Project, n: &Node) -> Option<Edition> {
    let fm = crate::project::Area::FrontMatter.path(&p.root);
    let first = n.path.strip_prefix(&fm).ok()?.components().next()?;
    let folder = fm.join(first);
    if folder == n.path {
        return None;
    }
    let name = p
        .nodes
        .iter()
        .find(|m| m.path == folder)
        .map(|m| m.title.to_lowercase())?;
    match name.as_str() {
        "manuscript format" | "manuscript" => Some(Edition::Manuscript),
        "paperback" | "print" => Some(Edition::Paperback),
        "ebook" | "e-book" | "epub" => Some(Edition::Ebook),
        _ => None,
    }
}

/// The front-matter pages that go into `edition`, in order.
pub(crate) fn front_pages(p: &Project, edition: Edition) -> Vec<Cow<'_, str>> {
    let hyphen = p.meta.manuscript.spaced_hyphen;
    p.nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.front_matter && n.compile)
        .filter(|n| edition_of(p, n).is_none_or(|e| e == edition))
        .map(|n| prose(&n.body, hyphen))
        .filter(|body| !body.is_empty())
        .collect()
}

/// A scene's text as it goes into the book: `%% notes %%` out, TKs kept
/// (see [`Book::tks`]), trimmed, and book typography done.
pub(crate) fn prose(body: &str, hyphen: typeset::SpacedHyphen) -> Cow<'_, str> {
    let noted = notes::strip_notes(body);
    let trimmed: Cow<'_, str> = match noted {
        Cow::Borrowed(b) => Cow::Borrowed(b.trim()),
        Cow::Owned(o) => Cow::Owned(o.trim().to_string()),
    };
    let typeset = match typeset::body(&trimmed, hyphen, is_break) {
        Cow::Owned(o) => Some(o),
        Cow::Borrowed(_) => None,
    };
    match typeset {
        Some(o) => Cow::Owned(o),
        None => trimmed,
    }
}

/// Chapter folders named for what they are take no number and keep their own
/// heading: "Prologue", "Epilogue: After", "Interlude".
const UNNUMBERED: [&str; 12] = [
    "prologue",
    "epilogue",
    "interlude",
    "prelude",
    "foreword",
    "preface",
    "introduction",
    "afterword",
    "coda",
    "postscript",
    "intermission",
    "entr'acte",
];

pub(crate) fn is_unnumbered(title: &str) -> bool {
    let first = title
        .trim()
        .split(|c: char| c.is_whitespace() || matches!(c, ':' | '.' | '—' | '–' | '-'))
        .next()
        .unwrap_or("")
        .to_lowercase();
    UNNUMBERED.contains(&first.as_str())
}

/// A chapter's heading, and the line under it if there is one, in `style`.
/// "Chapter Two" over "The Lamp"; a Prologue is headed by its own title.
pub(crate) fn chapter_heading(
    number: Option<usize>,
    title: &str,
    style: ChapterHeading,
) -> (String, Option<String>) {
    let name = chapter_name(title).map(str::to_string);
    let Some(n) = number else {
        return (title.trim().to_string(), None);
    };
    match style {
        ChapterHeading::Words => (numbered("Chapter", n), name),
        ChapterHeading::Caps => (format!("CHAPTER {}", spell(n)), name),
        ChapterHeading::Digits => (format!("Chapter {n}"), name),
        ChapterHeading::Number => (n.to_string(), name),
        ChapterHeading::Title => match name {
            Some(name) => (name, None),
            None => (numbered("Chapter", n), None),
        },
    }
}

/// One chapter's scenes as the walk finds them, with their words.
struct Found<'a> {
    number: Option<usize>,
    title: &'a str,
    depth: usize,
    /// Text, words, and the scene's node.
    scenes: Vec<(Cow<'a, str>, usize, usize)>,
}

enum Item<'a> {
    Part { title: &'a str, depth: usize },
    Chapter(Found<'a>),
    Scene(Cow<'a, str>, usize, usize),
}

/// Walk the manuscript. With `parts`, only those top-level parts are kept,
/// and with a `scope`, only that sample of them — but every chapter is still
/// counted, so the kept ones keep their numbers, and the title page's count
/// is still the whole book's.
pub(crate) fn book<'a>(p: &'a Project, parts: Option<&[usize]>, scope: Scope) -> Result<Book<'a>> {
    let noun = p.meta.part_noun();
    if let Some(chosen) = parts {
        if chosen.is_empty() {
            bail!("choose at least one {noun} to export");
        }
        let known = self::parts(p);
        for &i in chosen {
            if !known.iter().any(|&(k, _)| k == i) {
                bail!("that is not one of the book's {noun}s");
            }
        }
    }
    match scope {
        Scope::Chapters { from, to } if from == 0 || to < from => {
            bail!("chapters run from 1 — give a range like 1–3")
        }
        Scope::Words(0) => bail!("give a number of words for the sample"),
        _ => {}
    }

    let m = &p.meta;
    let title = m.title.trim();
    let mut b = Book {
        title: if title.is_empty() { "Untitled" } else { title },
        author: m.author.trim(),
        contact: &m.contact,
        look: &m.manuscript,
        front: front_pages(p, Edition::Manuscript),
        total: 0,
        pieces: Vec::new(),
        words: 0,
        chapters: 0,
        scenes: 0,
        skipped: 0,
        empty: 0,
        tks: Vec::new(),
        to_the_end: scope == Scope::Whole
            && parts.is_none_or(|chosen| {
                self::parts(p)
                    .last()
                    .is_none_or(|(last, _)| chosen.contains(last))
            }),
    };

    let mut items = Vec::new();
    let mut number = 0usize;
    for r in manuscript_roots(p) {
        let keep = parts.is_none_or(|chosen| chosen.contains(&r));
        gather(p, r, keep, &mut number, &mut b, &mut items);
    }
    let items = sample(items, scope);
    finish(p, &mut b, items);
    Ok(b)
}

fn gather<'a>(
    p: &'a Project,
    idx: usize,
    keep: bool,
    number: &mut usize,
    b: &mut Book<'a>,
    out: &mut Vec<Item<'a>>,
) {
    let n = &p.nodes[idx];
    let hyphen = p.meta.manuscript.spaced_hyphen;
    match section_of(p, idx) {
        Section::Part => {
            if keep {
                out.push(Item::Part {
                    title: &n.title,
                    depth: n.depth.saturating_sub(1),
                });
            }
            for &c in &n.children {
                gather(p, c, keep, number, b, out);
            }
        }
        Section::Chapter => {
            let mut scenes = Vec::new();
            for &c in &n.children {
                let s = &p.nodes[c];
                if s.kind != Kind::Scene {
                    continue;
                }
                if !s.compile {
                    if keep {
                        b.skipped += 1;
                    }
                    continue;
                }
                let text = prose(&s.body, hyphen);
                if text.is_empty() {
                    if keep {
                        b.empty += 1;
                    }
                    continue;
                }
                b.total += s.words();
                scenes.push((text, s.words(), c));
            }
            // A chapter with nothing written in it isn't in the book, and
            // doesn't take a number.
            if scenes.is_empty() {
                if keep {
                    b.empty += 1;
                }
                return;
            }
            let chapter_no = if is_unnumbered(&n.title) {
                None
            } else {
                *number += 1;
                Some(*number)
            };
            if !keep {
                return;
            }
            out.push(Item::Chapter(Found {
                number: chapter_no,
                title: &n.title,
                depth: n.depth.saturating_sub(1),
                scenes,
            }));
        }
        Section::Scene => {
            if !n.compile {
                if keep {
                    b.skipped += 1;
                }
                return;
            }
            let text = prose(&n.body, hyphen);
            if text.is_empty() {
                if keep {
                    b.empty += 1;
                }
                return;
            }
            b.total += n.words();
            if keep {
                out.push(Item::Scene(text, n.words(), idx));
            }
        }
    }
}

/// Where each TK in scene `idx` is, for the warning before export.
fn note_tks(p: &Project, idx: usize, number: Option<usize>, chapter: &str, b: &mut Book) {
    let s = &p.nodes[idx];
    let lines: Vec<&str> = s.body.lines().collect();
    // Which TK this is on its line: the first, the second…
    let mut on_line = (usize::MAX, 0usize);
    for m in notes::marks(&s.body) {
        if m.kind != notes::MarkKind::Tk {
            continue;
        }
        on_line = if on_line.0 == m.line {
            (m.line, on_line.1 + 1)
        } else {
            (m.line, 0)
        };
        let place = match (number, chapter.is_empty()) {
            (Some(n), _) => format!("{}, {}", numbered("Chapter", n), s.title),
            (None, false) => format!("{chapter}, {}", s.title),
            (None, true) => s.title.clone(),
        };
        let seen = lines
            .get(m.line)
            .map(|l| around_tk(l, on_line.1))
            .unwrap_or(m.snippet);
        b.tks.push(format!("{place}: {seen}"));
    }
}

/// The words either side of the `nth` TK on `line`, notes left out: "…a
/// drawer slid shut. She took the TK from her coat."
fn around_tk(line: &str, nth: usize) -> String {
    const SIDE: usize = 5;
    let clean = notes::strip_notes(line);
    let words: Vec<&str> = clean.split_whitespace().collect();
    let is_tk = |w: &str| {
        let core = w.trim_matches(|c: char| !c.is_alphanumeric());
        !core.is_empty() && core.len() % 2 == 0 && core.as_bytes().chunks(2).all(|p| p == b"TK")
    };
    let at = words
        .iter()
        .enumerate()
        .filter(|(_, w)| is_tk(w))
        .nth(nth)
        .map_or(0, |(i, _)| i);
    let (from, to) = (at.saturating_sub(SIDE), (at + SIDE + 1).min(words.len()));
    format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        words[from..to].join(" "),
        if to < words.len() { "…" } else { "" }
    )
}

/// Only the sample `scope` asks for.
fn sample(items: Vec<Item<'_>>, scope: Scope) -> Vec<Item<'_>> {
    match scope {
        Scope::Whole => items,
        Scope::Chapters { from, to } => {
            // An unnumbered chapter goes with the numbered one after it, or,
            // at the very end, with the last.
            let next_numbers: Vec<Option<usize>> = (0..items.len())
                .map(|i| {
                    items[i..].iter().find_map(|it| match it {
                        Item::Chapter(c) => c.number,
                        _ => None,
                    })
                })
                .collect();
            let last = items
                .iter()
                .filter_map(|it| match it {
                    Item::Chapter(c) => c.number,
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            items
                .into_iter()
                .zip(next_numbers)
                .filter(|(it, next)| match it {
                    Item::Part { .. } => true,
                    Item::Chapter(c) => match c.number {
                        Some(n) => (from..=to).contains(&n),
                        None => match next {
                            Some(n) => (from..=to).contains(n),
                            None => to >= last,
                        },
                    },
                    Item::Scene(..) => match next {
                        Some(n) => (from..=to).contains(n),
                        None => to >= last,
                    },
                })
                .map(|(it, _)| it)
                .collect()
        }
        Scope::Words(limit) => {
            let mut words = 0;
            let mut out = Vec::new();
            for it in items {
                if words >= limit {
                    break;
                }
                match it {
                    Item::Part { .. } => out.push(it),
                    Item::Scene(text, n, idx) => {
                        words += n;
                        out.push(Item::Scene(text, n, idx));
                    }
                    Item::Chapter(mut c) => {
                        let mut kept = Vec::new();
                        for (text, n, idx) in c.scenes.drain(..) {
                            if words >= limit {
                                break;
                            }
                            words += n;
                            kept.push((text, n, idx));
                        }
                        c.scenes = kept;
                        out.push(Item::Chapter(c));
                    }
                }
            }
            out
        }
    }
}

/// The kept items as the book's pieces, counted; a part left with nothing in
/// it goes too.
fn finish<'a>(p: &'a Project, b: &mut Book<'a>, items: Vec<Item<'a>>) {
    let has_content_after = |i: usize| {
        items[i + 1..]
            .iter()
            .take_while(|it| !matches!(it, Item::Part { .. }))
            .any(|it| matches!(it, Item::Chapter(_) | Item::Scene(..)))
    };
    let keep: Vec<bool> = (0..items.len())
        .map(|i| !matches!(items[i], Item::Part { .. }) || has_content_after(i))
        .collect();
    for (it, keep) in items.into_iter().zip(keep) {
        if !keep {
            b.empty += 1;
            continue;
        }
        match it {
            Item::Part { title, depth } => b.pieces.push(Piece::Part { title, depth }),
            Item::Chapter(c) => {
                b.chapters += 1;
                b.scenes += c.scenes.len();
                b.words += c.scenes.iter().map(|(_, n, _)| n).sum::<usize>();
                for &(_, _, idx) in &c.scenes {
                    note_tks(p, idx, c.number, c.title, b);
                }
                b.pieces.push(Piece::Chapter {
                    number: c.number,
                    title: c.title,
                    depth: c.depth,
                    scenes: c.scenes.into_iter().map(|(t, _, _)| t).collect(),
                });
            }
            Item::Scene(text, n, idx) => {
                note_tks(p, idx, None, "", b);
                b.scenes += 1;
                b.words += n;
                b.pieces.push(Piece::Scene(text));
            }
        }
    }
}

fn pages(b: &Book) -> usize {
    let opening = b.front.len().max(1);
    let parts = b
        .pieces
        .iter()
        .filter(|p| matches!(p, Piece::Part { .. }))
        .count();
    opening + b.words.div_ceil(WORDS_PER_PAGE) + b.chapters + parts
}

/// "The Archive" makes `the-archive` — the name every export file shares.
pub(crate) fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "book".into()
    } else {
        slug.to_string()
    }
}

/// What a chapter heading adds under "Chapter One": the folder's own title —
/// unless that title is only the chapter's number again. "Chapter Two: The
/// Lamp" gives "The Lamp"; "Chapter Two", "Chapter 2" and "Chapter II" give
/// nothing.
fn chapter_name(title: &str) -> Option<&str> {
    let t = title.trim();
    let word = "chapter";
    let is_chapter = t.len() >= word.len()
        && t.as_bytes()[..word.len()].eq_ignore_ascii_case(word.as_bytes())
        && t[word.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace);
    if !is_chapter {
        return (!t.is_empty()).then_some(t);
    }
    let rest = t[word.len()..].trim_start();
    let n = number_len(rest);
    if n == 0 && !rest.is_empty() {
        return Some(t);
    }
    let name = rest[n..].trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, ':' | '.' | '-' | '–' | '—')
    });
    (!name.is_empty()).then_some(name)
}

/// Length in bytes of the number `s` starts with — digits, a spelled-out
/// number, or a Roman numeral — or 0 if it doesn't start with one.
fn number_len(s: &str) -> usize {
    let ends_word = |n: usize| !s[n..].chars().next().is_some_and(char::is_alphanumeric);

    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && ends_word(digits) {
        return digits;
    }

    let mut best = 0;
    for k in 0..100 {
        let spelled = spell(k);
        for form in [spelled.clone(), spelled.replace('-', " ")] {
            let n = form.len();
            if n > best
                && s.len() >= n
                && s.as_bytes()[..n].eq_ignore_ascii_case(form.as_bytes())
                && ends_word(n)
            {
                best = n;
            }
        }
    }
    if best > 0 {
        return best;
    }

    let roman = s.bytes().take_while(|b| b"ivxlcIVXLC".contains(b)).count();
    if roman > 0
        && ends_word(roman)
        && (1..400).any(|k| to_roman(k).eq_ignore_ascii_case(&s[..roman]))
    {
        return roman;
    }
    0
}

fn to_roman(mut n: usize) -> String {
    const NUMERALS: [(usize, &str); 9] = [
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

/// The running header's name: the author's surname, passing over "Jr." and
/// the like.
fn surname(author: &str) -> &str {
    const SUFFIXES: [&str; 7] = ["jr", "sr", "ii", "iii", "iv", "phd", "md"];
    let words: Vec<&str> = author
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == ',' || c == '.'))
        .filter(|w| !w.is_empty())
        .collect();
    words
        .iter()
        .rev()
        .find(|w| !SUFFIXES.contains(&w.to_lowercase().as_str()))
        .or(words.last())
        .copied()
        .unwrap_or("")
}

/// The running header's title word: the first that isn't an article or a
/// little joining word, in capitals.
fn keyword(title: &str) -> String {
    const SMALL: [&str; 13] = [
        "a", "an", "the", "of", "and", "or", "in", "on", "at", "to", "for", "by", "with",
    ];
    let words: Vec<&str> = title
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect();
    words
        .iter()
        .find(|w| !SMALL.contains(&w.to_lowercase().as_str()))
        .or(words.first())
        .map(|w| w.to_uppercase())
        .unwrap_or_else(|| "UNTITLED".into())
}

/// `King / ARCHIVE / ` — the page number follows. The keyword is the
/// manuscript setting's, or the title's first real word.
fn header_text(b: &Book) -> String {
    let name = surname(b.author);
    let set = b.look.header_keyword.trim();
    let key = if set.is_empty() {
        keyword(b.title)
    } else {
        set.to_string()
    };
    if name.is_empty() {
        format!("{key} / ")
    } else {
        format!("{name} / {key} / ")
    }
}

// ── Markdown, the text inside every format ───────────────────────────

/// One styled stretch of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub text: String,
    pub italic: bool,
    pub bold: bool,
}

/// Inline Markdown to runs: `*x*` and `_x_` italic, `**x**` and `__x__` bold,
/// backslash escapes. A marker with nothing to pair with stays as typed, and
/// an underscore inside a word is just an underscore.
pub(crate) fn inline(s: &str) -> Vec<Run> {
    let chars: Vec<char> = s.chars().collect();
    let mut runs: Vec<Run> = Vec::new();
    let mut buf = String::new();
    let (mut italic, mut bold) = (None::<char>, None::<char>);

    let flush = |buf: &mut String, runs: &mut Vec<Run>, italic: bool, bold: bool| {
        if buf.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some(r) if r.italic == italic && r.bold == bold => r.text.push_str(buf),
            _ => runs.push(Run {
                text: buf.clone(),
                italic,
                bold,
            }),
        }
        buf.clear();
    };

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c != '*' && c != '_' {
            buf.push(c);
            i += 1;
            continue;
        }

        let end = i + chars[i..].iter().take_while(|&&d| d == c).count();
        let before = i.checked_sub(1).map(|k| chars[k]);
        let after = chars.get(end).copied();
        let mut left = end - i;

        if can_close(c, before, after) {
            if left >= 2 && bold == Some(c) {
                flush(&mut buf, &mut runs, italic.is_some(), true);
                bold = None;
                left -= 2;
            }
            if left >= 1 && italic == Some(c) {
                flush(&mut buf, &mut runs, true, bold.is_some());
                italic = None;
                left -= 1;
            }
        }
        if left > 0 && can_open(c, before, after) {
            if left >= 2 && bold.is_none() && has_closer(&chars, end, c, 2) {
                flush(&mut buf, &mut runs, italic.is_some(), false);
                bold = Some(c);
                left -= 2;
            }
            if left >= 1 && italic.is_none() && has_closer(&chars, end, c, 1) {
                flush(&mut buf, &mut runs, false, bold.is_some());
                italic = Some(c);
                left -= 1;
            }
        }
        for _ in 0..left {
            buf.push(c);
        }
        i = end;
    }
    flush(&mut buf, &mut runs, italic.is_some(), bold.is_some());
    runs
}

fn can_open(c: char, before: Option<char>, after: Option<char>) -> bool {
    after.is_some_and(|a| !a.is_whitespace())
        && (c == '*' || !before.is_some_and(char::is_alphanumeric))
}

fn can_close(c: char, before: Option<char>, after: Option<char>) -> bool {
    before.is_some_and(|b| !b.is_whitespace())
        && (c == '*' || !after.is_some_and(char::is_alphanumeric))
}

/// Is there a run of at least `need` markers later in the line that could close?
fn has_closer(chars: &[char], from: usize, c: char, need: usize) -> bool {
    let mut k = from;
    while k < chars.len() {
        if chars[k] == '\\' {
            k += 2;
            continue;
        }
        if chars[k] != c {
            k += 1;
            continue;
        }
        let end = k + chars[k..].iter().take_while(|&&d| d == c).count();
        let before = k.checked_sub(1).map(|j| chars[j]);
        if end - k >= need && can_close(c, before, chars.get(end).copied()) {
            return true;
        }
        k = end;
    }
    false
}

fn plain(s: &str) -> String {
    inline(s).into_iter().map(|r| r.text).collect()
}

/// A scene body, line by line. The editor keeps one paragraph to a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block<'a> {
    Para(&'a str),
    Heading(&'a str),
    Break,
    Quote(&'a str),
}

pub(crate) fn blocks(body: &str) -> Vec<Block<'_>> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            if is_break(l) {
                return Some(Block::Break);
            }
            let hashes = l.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&hashes) && l[hashes..].starts_with(' ') {
                let text = l[hashes..].trim().trim_end_matches('#').trim_end();
                return (!text.is_empty()).then_some(Block::Heading(text));
            }
            if let Some(q) = l.strip_prefix('>') {
                let q = q.trim_start();
                return (!q.is_empty()).then_some(Block::Quote(q));
            }
            Some(Block::Para(l))
        })
        .collect()
}

/// `#` alone (Shunn's scene break), or a Markdown rule: `***`, `* * *`, `---`.
fn is_break(line: &str) -> bool {
    if line == "#" || line == "\\#" {
        return true;
    }
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3
        && ['*', '-', '_']
            .iter()
            .any(|&m| marks.iter().all(|&c| c == m))
}

/// Escape text for XML. Also drops the control characters XML 1.0 forbids
/// outright, so one stray byte in a scene can't make a file unreadable.
pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

// ── Shunn Markdown ───────────────────────────────────────────────────

/// The manuscript as one Markdown file — the text `grimoire compile` writes.
pub(crate) fn markdown(b: &Book) -> String {
    let mut out = String::new();
    let count = commas(rounded_words(b.total));
    if b.front.is_empty() {
        if !b.author.is_empty() {
            let _ = writeln!(out, "{}  ", b.author);
        }
        let _ = writeln!(out, "\nabout {count} words\n");
        let _ = writeln!(out, "# {}\n", b.title);
        if !b.author.is_empty() {
            let _ = writeln!(out, "by {}\n", b.author);
        }
    } else {
        // Front matter owns the title page — Scrivener's rule too.
        for f in &b.front {
            let _ = writeln!(out, "{f}\n");
        }
        let _ = writeln!(out, "\nabout {count} words\n");
    }

    for piece in &b.pieces {
        match piece {
            Piece::Part { title, .. } => {
                let _ = writeln!(out, "\n# {}\n", title.to_uppercase());
            }
            Piece::Chapter {
                number,
                title,
                scenes,
                ..
            } => {
                let (heading, name) = chapter_heading(*number, title, b.look.chapter_heading);
                let _ = writeln!(out, "\n## {heading}\n");
                if let Some(name) = name {
                    let _ = writeln!(out, "*{name}*\n");
                }
                for (i, s) in scenes.iter().enumerate() {
                    // Shunn: a scene break is `#` alone on a line. Escaped so
                    // Markdown renders a literal hash, not an empty heading.
                    if i > 0 {
                        let _ = writeln!(out, "\n\\#\n");
                    }
                    let _ = writeln!(out, "{s}");
                }
            }
            Piece::Scene(s) => {
                let _ = writeln!(out, "{s}");
            }
        }
    }
    if let Some(end) = b.look.ending.text().filter(|_| b.to_the_end) {
        let _ = writeln!(out, "\n{end}");
    }
    out
}

// ── zip ──────────────────────────────────────────────────────────────

/// Pack text files into a zip, in order. `mimetype` is stored uncompressed,
/// which EPUB requires so a reader can recognise the file by its first bytes.
fn pack(entries: &[(String, String)]) -> Result<Vec<u8>> {
    use chrono::{Datelike, Timelike};
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    let now = chrono::Local::now();
    let stamp = zip::DateTime::from_date_and_time(
        now.year().clamp(1980, 2107) as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )
    .unwrap_or_default();

    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in entries {
        let method = if name == "mimetype" {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let opts = SimpleFileOptions::default()
            .compression_method(method)
            .last_modified_time(stamp);
        z.start_file(name.as_str(), opts)?;
        z.write_all(text.as_bytes())?;
    }
    Ok(z.finish()?.into_inner())
}

// ── DOCX ─────────────────────────────────────────────────────────────

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The page and type the manuscript settings add up to. All lengths in
/// twentieths of a point (twips): 1440 to the inch.
struct Look {
    font: &'static str,
    /// Classic: italics become underlines, typography back to typewriter.
    classic: bool,
    width: u32,
    height: u32,
    margin: u32,
    /// `w:line` for the body: 480 double, 360 one-and-a-half.
    line: u32,
    /// Blank lines above a chapter heading to put it a third of the way down.
    third: usize,
    /// Blank lines more, under a part heading, to bring the first chapter's
    /// heading to halfway.
    half_more: usize,
    flush_first: bool,
    ending: Option<&'static str>,
    heading: ChapterHeading,
}

impl Look {
    fn of(m: &Manuscript) -> Look {
        let (width, height) = match m.paper {
            Paper::Letter => (12240, 15840),
            Paper::A4 => (11906, 16838),
        };
        let line = match m.spacing {
            Spacing::Double => 480,
            Spacing::OneAndHalf => 360,
        };
        // One body line in points: 12pt type, set at `line`/240 of its
        // leading (~1.15 × the size for these fonts).
        let line_pt = 12.0 * 1.15 * line as f32 / 240.0;
        let page = height as f32 / 20.0;
        // A third of the way down the page, and halfway, measured from the
        // top margin.
        let third_pt = page / 3.0 - 72.0;
        let half_pt = page / 2.0 - 72.0;
        let third = (third_pt / line_pt).round().max(1.0) as usize;
        let half = ((half_pt / line_pt).round() as usize).max(third + 2);
        Look {
            font: match m.format {
                Format::Modern => "Times New Roman",
                Format::Classic => "Courier New",
            },
            classic: m.format == Format::Classic,
            width,
            height,
            margin: 1440,
            line,
            third,
            half_more: half - third - 1,
            flush_first: m.first_paragraph == FirstParagraph::Flush,
            ending: m.ending.text(),
            heading: m.chapter_heading,
        }
    }

    /// Width of the text between the margins: where a right tab sits.
    fn text_width(&self) -> u32 {
        self.width - 2 * self.margin
    }

    fn page(&self) -> String {
        format!(
            "<w:pgSz w:w=\"{}\" w:h=\"{}\"/>\
             <w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" \
             w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>",
            self.width,
            self.height,
            m = self.margin
        )
    }

    /// Text as this manuscript sets it: typewriter marks for the classic.
    fn text<'t>(&self, s: &'t str) -> Cow<'t, str> {
        if self.classic {
            typeset::plain(s)
        } else {
            Cow::Borrowed(s)
        }
    }
}

struct Para {
    style: &'static str,
    page_break: bool,
    /// A blank line after it, as under a chapter heading.
    gap_after: bool,
    /// Space before it, in twips, overriding the style.
    before: Option<u32>,
    runs: String,
}

impl Para {
    fn new(style: &'static str, runs: String) -> Self {
        Self {
            style,
            page_break: false,
            gap_after: false,
            before: None,
            runs,
        }
    }

    /// `sect` closes a section on this paragraph. Children of `w:pPr` go in
    /// schema order; Word is strict about it.
    fn xml(&self, sect: Option<&str>, line: u32) -> String {
        let mut s = format!("<w:p><w:pPr><w:pStyle w:val=\"{}\"/>", self.style);
        if self.page_break {
            s.push_str("<w:pageBreakBefore/>");
        }
        match (self.before, self.gap_after) {
            (Some(b), true) => {
                let _ = write!(s, "<w:spacing w:before=\"{b}\" w:after=\"{line}\"/>");
            }
            (Some(b), false) => {
                let _ = write!(s, "<w:spacing w:before=\"{b}\"/>");
            }
            (None, true) => {
                let _ = write!(s, "<w:spacing w:after=\"{line}\"/>");
            }
            (None, false) => {}
        }
        if let Some(sect) = sect {
            s.push_str(sect);
        }
        s.push_str("</w:pPr>");
        s.push_str(&self.runs);
        s.push_str("</w:p>\n");
        s
    }
}

fn docx_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
        xml_escape(text)
    )
}

fn docx_runs(text: &str, look: &Look) -> String {
    let text = look.text(text);
    let mut out = String::new();
    for r in inline(&text) {
        out.push_str("<w:r>");
        if r.bold || r.italic {
            out.push_str("<w:rPr>");
            if r.bold {
                out.push_str("<w:b/>");
            }
            if r.italic {
                // Shunn Classic: underline what would be italic.
                out.push_str(if look.classic {
                    "<w:u w:val=\"single\"/>"
                } else {
                    "<w:i/>"
                });
            }
            out.push_str("</w:rPr>");
        }
        let _ = write!(
            out,
            "<w:t xml:space=\"preserve\">{}</w:t></w:r>",
            xml_escape(&r.text)
        );
    }
    out
}

/// A body's blocks. `first` is whether the next paragraph opens a chapter or
/// follows a break, for the flush-first-paragraph setting.
fn docx_blocks(body: &str, front: bool, look: &Look, first: &mut bool, out: &mut Vec<Para>) {
    for block in blocks(body) {
        let para = match block {
            Block::Para(t) => {
                let style = if front || (*first && look.flush_first) {
                    "Unindented"
                } else {
                    "Normal"
                };
                *first = false;
                Para::new(style, docx_runs(t, look))
            }
            Block::Heading(t) => {
                *first = true;
                Para::new("Centered", docx_runs(t, look))
            }
            Block::Break => {
                *first = true;
                Para::new("SceneBreak", docx_text("#"))
            }
            Block::Quote(t) => {
                *first = true;
                Para::new("Quote", docx_runs(t, look))
            }
        };
        out.push(para);
    }
}

/// Title case as typed for the modern format; capitals for the classic
/// typewriter page.
fn title_line(b: &Book, look: &Look) -> String {
    if look.classic {
        b.title.to_uppercase()
    } else {
        b.title.to_string()
    }
}

fn docx(b: &Book) -> Result<Vec<u8>> {
    let look = Look::of(b.look);
    // Section one: the title page, or the author's own front matter. No
    // running header here.
    let count = format!("about {} words", commas(rounded_words(b.total)));
    let tab = "<w:r><w:tab/></w:r>";
    let mut opening: Vec<Para> = Vec::new();
    if b.front.is_empty() {
        // Shunn's novel title page: the contact block top left, single-
        // spaced, the word count top right; title and byline centred halfway
        // down.
        let c = b.contact;
        let legal = if c.legal_name.trim().is_empty() {
            b.author
        } else {
            c.legal_name.trim()
        };
        let mut block: Vec<String> = vec![legal.to_string()];
        block.extend(c.address.iter().map(|l| l.trim().to_string()));
        block.push(c.phone.trim().to_string());
        block.push(c.email.trim().to_string());
        block.retain(|l| !l.is_empty());
        let agent: Vec<&str> = c
            .agent
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        let first = block.first().cloned().unwrap_or_default();
        opening.push(Para::new(
            "TitlePageLine",
            format!("{}{tab}{}", docx_text(&first), docx_text(&count)),
        ));
        for l in block.iter().skip(1) {
            opening.push(Para::new("TitlePageLine", docx_text(l)));
        }
        if !agent.is_empty() {
            opening.push(Para::new("TitlePageLine", String::new()));
            for l in &agent {
                opening.push(Para::new("TitlePageLine", docx_text(l)));
            }
        }
        // Halfway down the page, whatever the block above took.
        let lines_above = opening.len() as u32;
        let single = 276; // one 12pt line, single-spaced
        let half = look.height / 2 - look.margin;
        let mut t = Para::new("Title", docx_runs(&title_line(b, &look), &look));
        t.before = Some(half.saturating_sub(lines_above * single).max(single));
        opening.push(t);
        if !b.author.is_empty() {
            opening.push(Para::new(
                "Centered",
                docx_text(&format!("by {}", b.author)),
            ));
        }
    } else {
        opening.push(Para::new(
            "TitlePageLine",
            format!("{tab}{}", docx_text(&count)),
        ));
        for (i, f) in b.front.iter().enumerate() {
            let start = opening.len();
            let mut first = false;
            docx_blocks(f, true, &look, &mut first, &mut opening);
            if i > 0 && start < opening.len() {
                opening[start].page_break = true;
            }
        }
    }

    // Section two: the book, numbered from 1, header on every page.
    let mut body: Vec<Para> = Vec::new();
    let blanks = |n: usize, body: &mut Vec<Para>| {
        for _ in 0..n {
            body.push(Para::new("Blank", String::new()));
        }
    };
    // A part heading shares its first chapter's page (Shunn): the part a
    // third of the way down, the chapter heading halfway.
    let mut under_part = false;
    for piece in &b.pieces {
        let start = body.len();
        match piece {
            Piece::Part { title, .. } => {
                blanks(look.third, &mut body);
                body.push(Para::new("Heading1", docx_runs(title, &look)));
                under_part = true;
            }
            Piece::Chapter {
                number,
                title,
                scenes,
                ..
            } => {
                let new_page = !under_part;
                if under_part {
                    blanks(look.half_more, &mut body);
                } else {
                    blanks(look.third, &mut body);
                }
                under_part = false;
                let (heading, name) = chapter_heading(*number, title, look.heading);
                body.push(Para::new("Heading2", docx_runs(&heading, &look)));
                if let Some(name) = name {
                    body.push(Para::new("ChapterTitle", docx_runs(&name, &look)));
                }
                if let Some(last) = body.last_mut() {
                    last.gap_after = true;
                }
                let mut first = true;
                for (i, s) in scenes.iter().enumerate() {
                    if i > 0 {
                        body.push(Para::new("SceneBreak", docx_text("#")));
                        first = true;
                    }
                    docx_blocks(s, false, &look, &mut first, &mut body);
                }
                if start > 0 && new_page {
                    body[start].page_break = true;
                }
                continue;
            }
            Piece::Scene(s) => {
                under_part = false;
                let mut first = false;
                docx_blocks(s, false, &look, &mut first, &mut body);
            }
        }
        // A part opens a new page; the first thing in the section already is.
        if start > 0 && start < body.len() && matches!(piece, Piece::Part { .. }) {
            body[start].page_break = true;
        }
    }
    if let Some(end) = look.ending.filter(|_| b.to_the_end) {
        body.push(Para::new("Centered", docx_text(end)));
    }

    let mut doc = format!("{XML_HEAD}<w:document xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:body>\n");
    let title_sect = format!("<w:sectPr>{}</w:sectPr>", look.page());
    let last = opening.len() - 1;
    for (i, para) in opening.iter().enumerate() {
        doc.push_str(&para.xml((i == last).then_some(title_sect.as_str()), look.line));
    }
    for para in &body {
        doc.push_str(&para.xml(None, look.line));
    }
    let _ = write!(
        doc,
        "<w:sectPr><w:headerReference w:type=\"default\" r:id=\"rIdHeader\"/>{}\
         <w:pgNumType w:start=\"1\"/></w:sectPr>\n</w:body></w:document>\n",
        look.page()
    );

    let header = format!(
        "{XML_HEAD}<w:hdr xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:p><w:pPr><w:pStyle w:val=\"Header\"/></w:pPr>\
         {}<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>\
         <w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r>\
         <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>1</w:t></w:r>\
         <w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p></w:hdr>\n",
        docx_text(&header_text(b))
    );

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let core = format!(
        "{XML_HEAD}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <dc:title>{}</dc:title><dc:creator>{}</dc:creator>\
         <dcterms:created xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:created>\
         <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:modified>\
         </cp:coreProperties>\n",
        xml_escape(b.title),
        xml_escape(b.author)
    );

    let files = [
        ("[Content_Types].xml", DOCX_CONTENT_TYPES.to_string()),
        ("_rels/.rels", DOCX_RELS.to_string()),
        ("docProps/core.xml", core),
        ("docProps/app.xml", DOCX_APP.to_string()),
        ("word/document.xml", doc),
        (
            "word/_rels/document.xml.rels",
            DOCX_DOCUMENT_RELS.to_string(),
        ),
        ("word/styles.xml", docx_styles(&look)),
        ("word/settings.xml", DOCX_SETTINGS.to_string()),
        ("word/header1.xml", header),
    ];
    let entries: Vec<(String, String)> =
        files.into_iter().map(|(n, t)| (n.to_string(), t)).collect();
    pack(&entries)
}

const DOCX_CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
<Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\
</Types>
";

const DOCX_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\
</Relationships>
";

const DOCX_DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\
<Relationship Id=\"rIdHeader\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>\
</Relationships>
";

const DOCX_APP: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Grimoire</Application></Properties>
";

/// Compatibility mode 15 keeps Word from opening the file in compatibility view.
const DOCX_SETTINGS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:defaultTabStop w:val=\"720\"/>\
<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>\
</w:settings>
";

/// Every style names its font, size and weight outright. Word would inherit
/// them, but LibreOffice lays its own defaults under headings and titles.
fn docx_styles(look: &Look) -> String {
    let face = look.font;
    let font = format!(
        "<w:rFonts w:ascii=\"{face}\" w:hAnsi=\"{face}\" w:eastAsia=\"{face}\" w:cs=\"{face}\"/>"
    );
    let plain = format!(
        "<w:rPr>{font}<w:b w:val=\"0\"/><w:bCs w:val=\"0\"/><w:i w:val=\"0\"/><w:iCs w:val=\"0\"/>\
         <w:color w:val=\"000000\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr>"
    );
    let right = look.text_width();
    let single = "<w:spacing w:line=\"240\" w:lineRule=\"auto\"/>";
    let title_page = format!(
        "<w:tabs><w:tab w:val=\"right\" w:pos=\"{right}\"/></w:tabs>{single}<w:ind w:firstLine=\"0\"/>"
    );
    let header = format!(
        "<w:tabs><w:tab w:val=\"right\" w:pos=\"{right}\"/></w:tabs>{single}<w:ind w:firstLine=\"0\"/><w:jc w:val=\"right\"/>"
    );
    // (id, name, pPr contents)
    let styles: [(&str, &str, String); 11] = [
        (
            "Heading1",
            "heading 1",
            "<w:keepNext/><w:keepLines/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/><w:outlineLvl w:val=\"0\"/>".into(),
        ),
        (
            "Heading2",
            "heading 2",
            "<w:keepNext/><w:keepLines/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/><w:outlineLvl w:val=\"1\"/>".into(),
        ),
        // The empty lines that bring a chapter heading down the page.
        (
            "Blank",
            "Blank Line",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/>".into(),
        ),
        // The title sits about halfway down its page (space set per title page).
        (
            "Title",
            "Title",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>".into(),
        ),
        (
            "ChapterTitle",
            "Chapter Title",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>".into(),
        ),
        (
            "SceneBreak",
            "Scene Break",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>".into(),
        ),
        (
            "Centered",
            "Centered",
            "<w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>".into(),
        ),
        (
            "Unindented",
            "Unindented",
            "<w:ind w:firstLine=\"0\"/>".into(),
        ),
        (
            "Quote",
            "Quote",
            "<w:ind w:left=\"720\" w:right=\"720\" w:firstLine=\"0\"/>".into(),
        ),
        // Contact block top left, word count top right, single-spaced.
        ("TitlePageLine", "Title Page Line", title_page),
        ("Header", "header", header),
    ];

    let line = look.line;
    let mut s = format!(
        "{XML_HEAD}<w:styles xmlns:w=\"{W_NS}\">\
         <w:docDefaults><w:rPrDefault><w:rPr>{font}<w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/>\
         <w:lang w:val=\"en-US\" w:eastAsia=\"en-US\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"{line}\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
         </w:docDefaults>\n\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:widowControl/><w:ind w:firstLine=\"720\"/></w:pPr>{plain}</w:style>\n\
         <w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\">\
         <w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/></w:style>\n"
    );
    for (id, name, ppr) in styles {
        let _ = writeln!(
            s,
            "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>\
             <w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr>{ppr}</w:pPr>{plain}</w:style>"
        );
    }
    s.push_str("</w:styles>\n");
    s
}

// ── EPUB ─────────────────────────────────────────────────────────────

struct Toc {
    label: String,
    href: String,
    kids: Vec<Toc>,
}

/// File a contents entry `level` deep, under the latest entry at each level
/// above it.
fn toc_insert(list: &mut Vec<Toc>, level: usize, entry: Toc) {
    match list.last_mut() {
        Some(last) if level > 0 => toc_insert(&mut last.kids, level - 1, entry),
        _ => list.push(entry),
    }
}

fn toc_depth(list: &[Toc]) -> usize {
    list.iter()
        .map(|t| 1 + toc_depth(&t.kids))
        .max()
        .unwrap_or(0)
}

fn nav_list(list: &[Toc], out: &mut String) {
    out.push_str("<ol>\n");
    for t in list {
        let _ = write!(
            out,
            "<li><a href=\"{}\">{}</a>",
            xml_escape(&t.href),
            xml_escape(&t.label)
        );
        if !t.kids.is_empty() {
            out.push('\n');
            nav_list(&t.kids, out);
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ol>\n");
}

fn ncx_points(list: &[Toc], order: &mut usize, out: &mut String) {
    for t in list {
        *order += 1;
        let _ = writeln!(
            out,
            "<navPoint id=\"nav-{o}\" playOrder=\"{o}\"><navLabel><text>{}</text></navLabel><content src=\"{}\"/>",
            xml_escape(&t.label),
            xml_escape(&t.href),
            o = *order
        );
        ncx_points(&t.kids, order, out);
        out.push_str("</navPoint>\n");
    }
}

fn html_inline(text: &str) -> String {
    let mut out = String::new();
    for r in inline(text) {
        let t = xml_escape(&r.text);
        match (r.bold, r.italic) {
            (true, true) => {
                let _ = write!(out, "<strong><em>{t}</em></strong>");
            }
            (true, false) => {
                let _ = write!(out, "<strong>{t}</strong>");
            }
            (false, true) => {
                let _ = write!(out, "<em>{t}</em>");
            }
            (false, false) => out.push_str(&t),
        }
    }
    out
}

/// Paragraphs in book style: the first after a heading or break isn't indented.
fn html_blocks(body: &str) -> String {
    let mut out = String::new();
    let mut first = true;
    for block in blocks(body) {
        match block {
            Block::Para(t) => {
                let class = if first { " class=\"first\"" } else { "" };
                let _ = writeln!(out, "<p{class}>{}</p>", html_inline(t));
                first = false;
            }
            Block::Heading(t) => {
                let _ = writeln!(out, "<h3>{}</h3>", html_inline(t));
                first = true;
            }
            Block::Break => {
                out.push_str("<p class=\"break\">* * *</p>\n");
                first = true;
            }
            Block::Quote(t) => {
                let _ = writeln!(out, "<blockquote><p>{}</p></blockquote>", html_inline(t));
                first = true;
            }
        }
    }
    out
}

fn xhtml(title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"en\" lang=\"en\">\n\
         <head>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n\
         <body>\n{body}\n</body>\n</html>\n",
        xml_escape(title)
    )
}

/// The same book always gets the same identifier, so a reader's library
/// updates it rather than shelving a second copy. FNV-1a, spelled out here
/// because std's hasher may change between Rust releases.
fn book_id(title: &str, author: &str) -> String {
    let fnv = |seed: u64| {
        let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
        for &byte in title
            .as_bytes()
            .iter()
            .chain(b"\x00")
            .chain(author.as_bytes())
        {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    };
    let (a, b) = (fnv(0), fnv(0x9e37_79b9_7f4a_7c15));
    // Laid out as a UUID: version 8 (custom), RFC 4122 variant.
    let a = (a & !0xf000) | 0x8000;
    let b = (b & !(0xc0u64 << 56)) | (0x80u64 << 56);
    format!(
        "urn:uuid:{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xffff,
        b >> 48,
        b & 0xffff_ffff_ffff
    )
}

const EPUB_CSS: &str = "body { margin: 0 5%; }
h1, h2, h3 { font-weight: normal; text-align: center; }
p { margin: 0; text-indent: 1.5em; }
p.first { text-indent: 0; }
p.break { margin: 1em 0; text-align: center; text-indent: 0; }
blockquote { margin: 1em 2em; }
blockquote p { text-indent: 0; }
.titlepage { text-align: center; }
.titlepage h1 { margin: 6em 0 1em; font-size: 2em; }
.titlepage p.byline { font-size: 1.2em; text-indent: 0; }
.front p { text-indent: 0; }
.front h3 { font-size: 1.5em; margin: 2em 0 1em; }
.part h1 { margin-top: 8em; font-size: 1.8em; }
.chapter h2 { margin: 4em 0 2em; font-size: 1.4em; }
.chapter h2 .name { font-style: italic; }
h3 { font-size: 1em; margin: 1em 0; }
";

fn epub(b: &Book) -> Result<Vec<u8>> {
    // (file, id, xhtml)
    let mut docs: Vec<(String, String, String)> = Vec::new();
    let mut toc: Vec<Toc> = Vec::new();

    if b.front.is_empty() {
        let mut body = String::from("<div class=\"titlepage\" epub:type=\"titlepage\">\n");
        let _ = writeln!(body, "<h1 class=\"title\">{}</h1>", xml_escape(b.title));
        if !b.author.is_empty() {
            let _ = writeln!(body, "<p class=\"byline\">{}</p>", xml_escape(b.author));
        }
        body.push_str("</div>");
        docs.push(("title.xhtml".into(), "title".into(), xhtml(b.title, &body)));
    } else {
        for (i, f) in b.front.iter().enumerate() {
            let body = format!("<div class=\"front\">\n{}</div>", html_blocks(f));
            docs.push((
                format!("front-{}.xhtml", i + 1),
                format!("front-{}", i + 1),
                xhtml(b.title, &body),
            ));
        }
    }
    let opening = docs.len();
    toc.push(Toc {
        label: "Title Page".into(),
        href: docs[0].0.clone(),
        kids: Vec::new(),
    });

    let (mut part_no, mut text_no, mut chapter_no) = (0, 0, 0);
    for piece in &b.pieces {
        match piece {
            Piece::Part { title, depth } => {
                part_no += 1;
                let id = format!("part-{part_no:02}");
                let body = format!(
                    "<div class=\"part\" epub:type=\"part\">\n<h1>{}</h1>\n</div>",
                    html_inline(title)
                );
                let file = format!("{id}.xhtml");
                toc_insert(
                    &mut toc,
                    *depth,
                    Toc {
                        label: plain(title),
                        href: file.clone(),
                        kids: Vec::new(),
                    },
                );
                docs.push((file, id, xhtml(&plain(title), &body)));
            }
            Piece::Chapter {
                number,
                title,
                depth,
                scenes,
            } => {
                chapter_no += 1;
                let (heading, name) = chapter_heading(*number, title, b.look.chapter_heading);
                let name = name.as_deref();
                // EPUB's own words for the chapters that take no number.
                let first = title.split_whitespace().next().unwrap_or("");
                let kind = match first.trim_end_matches(':').to_lowercase().as_str() {
                    "prologue" if number.is_none() => "prologue",
                    "epilogue" if number.is_none() => "epilogue",
                    "foreword" if number.is_none() => "foreword",
                    "preface" if number.is_none() => "preface",
                    "introduction" if number.is_none() => "introduction",
                    "afterword" if number.is_none() => "afterword",
                    _ => "chapter",
                };
                let mut body = format!("<div class=\"chapter\" epub:type=\"{kind}\">\n");
                let _ = write!(
                    body,
                    "<h2><span class=\"number\">{}</span>",
                    xml_escape(&heading)
                );
                if let Some(name) = name {
                    let _ = write!(
                        body,
                        "<br/><span class=\"name\">{}</span>",
                        html_inline(name)
                    );
                }
                body.push_str("</h2>\n");
                for (i, s) in scenes.iter().enumerate() {
                    if i > 0 {
                        body.push_str("<p class=\"break\">* * *</p>\n");
                    }
                    body.push_str(&html_blocks(s));
                }
                body.push_str("</div>");

                let label = match name {
                    Some(name) => format!("{heading}: {}", plain(name)),
                    None => heading,
                };
                // Numbered chapters keep the book's number in their file name,
                // even in an export of one part.
                let id = match number {
                    Some(n) => format!("chapter-{n:02}"),
                    None => format!("{kind}-{chapter_no:02}"),
                };
                let file = format!("{id}.xhtml");
                toc_insert(
                    &mut toc,
                    *depth,
                    Toc {
                        label: label.clone(),
                        href: file.clone(),
                        kids: Vec::new(),
                    },
                );
                docs.push((file, id, xhtml(&label, &body)));
            }
            Piece::Scene(s) => {
                text_no += 1;
                let id = format!("text-{text_no:02}");
                let body = format!("<div class=\"text\">\n{}</div>", html_blocks(s));
                docs.push((format!("{id}.xhtml"), id, xhtml(b.title, &body)));
            }
        }
    }

    let id = book_id(b.title, b.author);
    let title = xml_escape(b.title);
    let modified = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    let mut opf = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\" xml:lang=\"en\">\n\
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n",
    );
    let _ = writeln!(opf, "<dc:identifier id=\"book-id\">{id}</dc:identifier>");
    let _ = writeln!(opf, "<dc:title>{title}</dc:title>");
    if !b.author.is_empty() {
        let _ = writeln!(
            opf,
            "<dc:creator id=\"author\">{}</dc:creator>",
            xml_escape(b.author)
        );
        opf.push_str(
            "<meta refines=\"#author\" property=\"role\" scheme=\"marc:relators\">aut</meta>\n",
        );
    }
    opf.push_str("<dc:language>en</dc:language>\n");
    let _ = writeln!(opf, "<meta property=\"dcterms:modified\">{modified}</meta>");
    opf.push_str("</metadata>\n<manifest>\n");
    opf.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    opf.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    opf.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for (file, id, _) in &docs {
        let _ = writeln!(
            opf,
            "<item id=\"{id}\" href=\"{file}\" media-type=\"application/xhtml+xml\"/>"
        );
    }
    opf.push_str("</manifest>\n<spine toc=\"ncx\">\n");
    for (_, id, _) in &docs {
        let _ = writeln!(opf, "<itemref idref=\"{id}\"/>");
    }
    opf.push_str("</spine>\n</package>\n");

    let mut nav = String::from("<nav epub:type=\"toc\" id=\"toc\">\n<h1>Contents</h1>\n");
    nav_list(&toc, &mut nav);
    nav.push_str("</nav>\n<nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"hidden\">\n<h2>Landmarks</h2>\n<ol>\n");
    let _ = writeln!(
        nav,
        "<li><a epub:type=\"titlepage\" href=\"{}\">Title Page</a></li>",
        docs[0].0
    );
    if let Some((file, _, _)) = docs.get(opening) {
        let _ = writeln!(
            nav,
            "<li><a epub:type=\"bodymatter\" href=\"{file}\">Start of the book</a></li>"
        );
    }
    nav.push_str("</ol>\n</nav>");

    let mut ncx = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\" xml:lang=\"en\">\n<head>\n\
         <meta name=\"dtb:uid\" content=\"{id}\"/>\n<meta name=\"dtb:depth\" content=\"{}\"/>\n\
         <meta name=\"dtb:totalPageCount\" content=\"0\"/>\n<meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n\
         </head>\n<docTitle><text>{title}</text></docTitle>\n",
        toc_depth(&toc)
    );
    if !b.author.is_empty() {
        let _ = writeln!(
            ncx,
            "<docAuthor><text>{}</text></docAuthor>",
            xml_escape(b.author)
        );
    }
    ncx.push_str("<navMap>\n");
    ncx_points(&toc, &mut 0, &mut ncx);
    ncx.push_str("</navMap>\n</ncx>\n");

    let mut entries: Vec<(String, String)> = vec![
        ("mimetype".into(), "application/epub+zip".into()),
        (
            "META-INF/container.xml".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n\
             <rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n\
             </rootfiles>\n</container>\n"
                .into(),
        ),
        ("OEBPS/content.opf".into(), opf),
        ("OEBPS/nav.xhtml".into(), xhtml("Contents", &nav)),
        ("OEBPS/toc.ncx".into(), ncx),
        ("OEBPS/style.css".into(), EPUB_CSS.into()),
    ];
    entries.extend(
        docs.into_iter()
            .map(|(file, _, text)| (format!("OEBPS/{file}"), text)),
    );
    pack(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read as _;
    use std::path::Path;

    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Two acts. Act One: Chapter One (two scenes, prose with italics, smart
    /// quotes, an ampersand), Chapter Two (one scene kept, one cut). Act Two:
    /// a chapter with a title of its own.
    fn book_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-export-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        put(
            &d,
            "novel.toml",
            "title = \"The Archive\"\nauthor = \"Josh King\"\npart_label = \"Act\"\n",
        );
        put(
            &d,
            "manuscript/01-Act-One/01-Chapter-One/01-Scene-One.md",
            "---\ntitle: \"Scene One\"\nstatus: draft\n---\n\n\
             The building had *no* windows on the north face.\n\n\
             “Wait — **now**,” she said, & meant <it>.\n",
        );
        put(
            &d,
            "manuscript/01-Act-One/01-Chapter-One/02-Scene-Two.md",
            "Gravel under her boots.\n",
        );
        put(
            &d,
            "manuscript/01-Act-One/02-Chapter-Two/01-Scene-One.md",
            "A lamp at the far end.\n",
        );
        put(
            &d,
            "manuscript/01-Act-One/02-Chapter-Two/02-Cut.md",
            "---\ncompile: false\n---\n\nCUT MATERIAL that must not ship.\n",
        );
        put(
            &d,
            "manuscript/02-Act-Two/01-The-Long-Road/01-Road.md",
            "The road ran _on_ and on.\n",
        );
        put(&d, "notes/01-Characters/wren.md", "NOTES never ship.\n");
        d
    }

    fn unzip(bytes: &[u8]) -> Vec<(String, zip::CompressionMethod, String)> {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).unwrap();
                let mut text = String::new();
                f.read_to_string(&mut text).unwrap();
                (f.name().to_string(), f.compression(), text)
            })
            .collect()
    }

    fn entry<'a>(files: &'a [(String, zip::CompressionMethod, String)], name: &str) -> &'a str {
        &files
            .iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("{name} is missing"))
            .2
    }

    /// Parse as XML. XHTML files carry `<!DOCTYPE html>`, which roxmltree
    /// refuses unless told.
    fn xml_doc(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
        let opts = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        roxmltree::Document::parse_with_options(text, opts)
    }

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    /// Each paragraph of a DOCX body as (style, text).
    fn docx_paragraphs(xml: &str) -> Vec<(String, String)> {
        let doc = xml_doc(xml).expect("document.xml is well-formed");
        doc.descendants()
            .filter(|n| n.has_tag_name((W, "p")))
            .map(|p| {
                let style = p
                    .descendants()
                    .find(|n| n.has_tag_name((W, "pStyle")))
                    .and_then(|n| n.attribute((W, "val")))
                    .unwrap_or("")
                    .to_string();
                let text = p
                    .descendants()
                    .filter(|n| n.has_tag_name((W, "t")))
                    .filter_map(|n| n.text())
                    .collect();
                (style, text)
            })
            .collect()
    }

    fn all_options() -> ExportOptions {
        ExportOptions {
            docx: true,
            pdf: false,
            epub: true,
            markdown: true,
            parts: None,
            scope: Scope::Whole,
        }
    }

    #[test]
    fn inline_markdown_becomes_runs() {
        let run = |text: &str, italic, bold| Run {
            text: text.into(),
            italic,
            bold,
        };
        assert_eq!(inline("plain"), vec![run("plain", false, false)]);
        assert_eq!(
            inline("a *b* _c_ **d** __e__"),
            vec![
                run("a ", false, false),
                run("b", true, false),
                run(" ", false, false),
                run("c", true, false),
                run(" ", false, false),
                run("d", false, true),
                run(" ", false, false),
                run("e", false, true),
            ]
        );
        assert_eq!(inline("***both***"), vec![run("both", true, true)]);
        assert_eq!(
            inline("**bold *and italic* too**"),
            vec![
                run("bold ", false, true),
                run("and italic", true, true),
                run(" too", false, true)
            ]
        );
        // Nothing to pair with, a lone multiplication sign, a snake_case
        // name, and escapes all stay as typed.
        assert_eq!(inline("5 * 3 = 15"), vec![run("5 * 3 = 15", false, false)]);
        assert_eq!(
            inline("an *unclosed star"),
            vec![run("an *unclosed star", false, false)]
        );
        assert_eq!(
            inline("snake_case_name"),
            vec![run("snake_case_name", false, false)]
        );
        assert_eq!(
            inline(r"\*not\* italic"),
            vec![run("*not* italic", false, false)]
        );
        // Smart quotes and dashes are just text.
        assert_eq!(
            inline("“Wait — *now*,” she said."),
            vec![
                run("“Wait — ", false, false),
                run("now", true, false),
                run(",” she said.", false, false)
            ]
        );
    }

    #[test]
    fn body_lines_become_blocks() {
        let body = "First.\n\n# A Heading\n#\n* * *\n> quoted\nLast.";
        assert_eq!(
            blocks(body),
            vec![
                Block::Para("First."),
                Block::Heading("A Heading"),
                Block::Break,
                Block::Break,
                Block::Quote("quoted"),
                Block::Para("Last."),
            ]
        );
        assert_eq!(blocks("#hashtag"), vec![Block::Para("#hashtag")]);
    }

    #[test]
    fn xml_is_escaped() {
        assert_eq!(
            xml_escape("Tom & \"Jerry\" <b>’s</b>"),
            "Tom &amp; &quot;Jerry&quot; &lt;b&gt;’s&lt;/b&gt;"
        );
        assert_eq!(
            xml_escape("bell\u{7} form\u{c}feed"),
            "bell formfeed",
            "XML 1.0 forbids these"
        );
        let look = Look::of(&Manuscript::default());
        assert!(docx_runs("<w:p> & *it*", &look).contains("&lt;w:p&gt; &amp; </w:t>"));
        assert_eq!(html_inline("a < b & *c*"), "a &lt; b &amp; <em>c</em>");
    }

    #[test]
    fn a_chapter_heading_only_adds_a_title_that_is_more_than_its_number() {
        assert_eq!(chapter_name("Chapter One"), None);
        assert_eq!(chapter_name("chapter twenty-seven"), None);
        assert_eq!(chapter_name("Chapter Twenty Seven"), None);
        assert_eq!(chapter_name("Chapter 12"), None);
        assert_eq!(chapter_name("Chapter XIV"), None);
        assert_eq!(chapter_name("Chapter"), None);
        assert_eq!(chapter_name("Chapter Seventeen"), None);
        assert_eq!(chapter_name("Chapter Two: The Lamp"), Some("The Lamp"));
        assert_eq!(chapter_name("Chapter 3 — Gravel"), Some("Gravel"));
        assert_eq!(chapter_name("The Long Road"), Some("The Long Road"));
        assert_eq!(chapter_name("Chapterhouse"), Some("Chapterhouse"));
        assert_eq!(chapter_name("Chapter Civil War"), Some("Chapter Civil War"));
    }

    #[test]
    fn the_running_header_is_surname_and_keyword() {
        assert_eq!(surname("Josh King"), "King");
        assert_eq!(surname("Martin Luther King Jr."), "King");
        assert_eq!(surname(""), "");
        assert_eq!(keyword("The Archive"), "ARCHIVE");
        assert_eq!(keyword("Of Mice and Men"), "MICE");
        assert_eq!(keyword("The"), "THE");
        assert_eq!(keyword(""), "UNTITLED");
    }

    #[test]
    fn file_names_come_from_the_title() {
        assert_eq!(slug("The Archive"), "the-archive");
        assert_eq!(slug("The Archive: Book One"), "the-archive-book-one");
        assert_eq!(slug("Of Mice & <Men>"), "of-mice-men");
        assert_eq!(slug("?!"), "book");
    }

    #[test]
    fn book_identifiers_are_stable_and_distinct() {
        let a = book_id("The Archive", "Josh King");
        assert_eq!(a, book_id("The Archive", "Josh King"));
        assert_ne!(a, book_id("The Archive", "Someone Else"));
        assert_ne!(book_id("ab", "c"), book_id("a", "bc"));
        assert!(
            a.starts_with("urn:uuid:") && a.len() == "urn:uuid:".len() + 36,
            "{a}"
        );
    }

    #[test]
    fn docx_has_its_parts_and_a_well_formed_document() {
        let d = book_dir("docx");
        let p = Project::load(&d).unwrap();
        let out = export(&p, &all_options()).unwrap();
        assert_eq!(out.files.len(), 3);
        let docx_path = d.join("exports/King_The-Archive_Manuscript.docx");
        assert!(
            out.files.contains(&docx_path),
            "Lastname_Title: {:?}",
            out.files
        );

        let files = unzip(&fs::read(&docx_path).unwrap());
        for part in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
            "word/settings.xml",
            "word/header1.xml",
            "docProps/core.xml",
            "docProps/app.xml",
        ] {
            xml_doc(entry(&files, part))
                .unwrap_or_else(|e| panic!("{part} is not well-formed: {e}"));
        }

        let doc = entry(&files, "word/document.xml");
        let paras = docx_paragraphs(doc);
        let has = |style: &str, text: &str| paras.iter().any(|(s, t)| s == style && t == text);
        assert!(
            paras[0].1.starts_with("Josh King") && paras[0].1.ends_with("about 0 words"),
            "{:?}",
            paras[0]
        );
        assert!(has("Title", "The Archive"), "title case, as typed");
        assert!(has("Centered", "by Josh King"));
        assert!(has("Heading1", "Act One") && has("Heading1", "Act Two"));
        assert!(
            has("Heading2", "Chapter One")
                && has("Heading2", "Chapter Two")
                && has("Heading2", "Chapter Three")
        );
        assert!(
            has("ChapterTitle", "The Long Road"),
            "a chapter's own title shows"
        );
        assert_eq!(
            paras.iter().filter(|(s, _)| s == "ChapterTitle").count(),
            1,
            "\"Chapter One\" isn't said twice"
        );
        assert!(has("SceneBreak", "#"));
        assert_eq!(
            paras.last().unwrap(),
            &("Centered".to_string(), "END".to_string())
        );
        assert!(has("Normal", "“Wait — now,” she said, & meant <it>."));
        assert!(doc.contains("<w:rPr><w:i/></w:rPr><w:t xml:space=\"preserve\">no</w:t>"));
        assert!(doc.contains("<w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">now</w:t>"));
        assert!(!doc.contains("CUT MATERIAL") && !doc.contains("NOTES"));

        // A part shares its first chapter's page (Shunn): Act One a third of
        // the way down, Chapter One halfway. Chapter Two and Act Two open new
        // pages; Act One opens the book's section, so needs no break. The
        // title page is a section of its own, and the book's section carries
        // the header and numbers from 1.
        assert_eq!(doc.matches("<w:pageBreakBefore/>").count(), 2);
        let look = Look::of(&Manuscript::default());
        let at = |style: &str, text: &str| {
            paras
                .iter()
                .position(|(s, t)| s == style && t == text)
                .unwrap()
        };
        let blank = |range: std::ops::Range<usize>| {
            paras[range]
                .iter()
                .all(|(s, t)| s == "Blank" && t.is_empty())
        };
        let (act1, ch1, ch2) = (
            at("Heading1", "Act One"),
            at("Heading2", "Chapter One"),
            at("Heading2", "Chapter Two"),
        );
        assert!(
            blank(act1 - look.third..act1),
            "a part a third of the way down"
        );
        assert_eq!(ch1, act1 + look.half_more + 1, "its first chapter halfway");
        assert!(blank(act1 + 1..ch1));
        assert!(
            blank(ch2 - look.third..ch2),
            "a chapter a third of the way down its page"
        );
        assert_eq!(doc.matches("<w:sectPr>").count(), 2);
        assert!(doc.contains("<w:headerReference w:type=\"default\" r:id=\"rIdHeader\"/>"));
        assert!(doc.contains("<w:pgNumType w:start=\"1\"/>"));
        let header = entry(&files, "word/header1.xml");
        assert!(header.contains("King / ARCHIVE / "));
        assert!(header.contains(" PAGE "));
        assert!(entry(&files, "word/styles.xml").contains("Times New Roman"));

        assert_eq!(out.chapters, 3);
        assert_eq!(out.words, 33);
        // One page of words, the title page, three chapter openers, two acts.
        assert_eq!(out.pages, 1 + 1 + 3 + 2);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn epub_mimetype_comes_first_and_uncompressed() {
        let d = book_dir("epub");
        let p = Project::load(&d).unwrap();
        export(
            &p,
            &ExportOptions {
                docx: false,
                markdown: false,
                ..all_options()
            },
        )
        .unwrap();
        let raw = fs::read(d.join("exports/the-archive.epub")).unwrap();
        // A reader sniffs the type at a fixed offset: "mimetype" at byte 30,
        // its content straight after, no extra field in between.
        assert_eq!(&raw[30..38], b"mimetype");
        assert_eq!(&raw[38..58], b"application/epub+zip");

        let files = unzip(&raw);
        assert_eq!(files[0].0, "mimetype");
        assert_eq!(files[0].1, zip::CompressionMethod::Stored);
        assert_eq!(files[0].2, "application/epub+zip");
        assert!(
            files[1..]
                .iter()
                .all(|(_, m, _)| *m == zip::CompressionMethod::Deflated)
        );

        for (name, _, text) in &files {
            if [".xml", ".opf", ".ncx", ".xhtml"]
                .iter()
                .any(|ext| name.ends_with(ext))
            {
                xml_doc(text).unwrap_or_else(|e| panic!("{name} is not well-formed: {e}"));
            }
        }
        let opf = entry(&files, "OEBPS/content.opf");
        let id = book_id("The Archive", "Josh King");
        assert!(opf.contains(&format!(
            "<dc:identifier id=\"book-id\">{id}</dc:identifier>"
        )));
        assert!(opf.contains("<dc:language>en</dc:language>"));
        assert!(opf.contains("property=\"dcterms:modified\""));
        assert!(entry(&files, "OEBPS/toc.ncx").contains(&format!("content=\"{id}\"")));

        let ch1 = entry(&files, "OEBPS/chapter-01.xhtml");
        assert!(ch1.contains("<span class=\"number\">Chapter One</span></h2>"));
        assert!(ch1.contains("<p class=\"break\">* * *</p>"));
        assert!(ch1.contains("<em>no</em>"));
        assert!(ch1.contains("“Wait — <strong>now</strong>,” she said, &amp; meant &lt;it&gt;."));
        let ch3 = entry(&files, "OEBPS/chapter-03.xhtml");
        assert!(ch3.contains("<br/><span class=\"name\">The Long Road</span>"));
        assert!(ch3.contains("The road ran <em>on</em> and on."));
        assert!(entry(&files, "OEBPS/part-02.xhtml").contains("<h1>Act Two</h1>"));

        // The contents nest chapters under their acts.
        let nav = entry(&files, "OEBPS/nav.xhtml");
        let act_two = nav.find(">Act Two<").unwrap();
        assert!(
            nav[act_two..].contains(
                "<ol>\n<li><a href=\"chapter-03.xhtml\">Chapter Three: The Long Road</a>"
            )
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn scenes_marked_compile_false_are_left_out() {
        let d = book_dir("skip");
        let p = Project::load(&d).unwrap();
        let b = book(&p, None, Scope::Whole).unwrap();
        assert_eq!(b.skipped, 1);
        assert_eq!(b.scenes, 4);
        assert!(!markdown(&b).contains("CUT MATERIAL"));
        let files = unzip(&epub(&b).unwrap());
        assert!(files.iter().all(|(_, _, t)| !t.contains("CUT MATERIAL")));

        // A chapter whose every scene is cut isn't in the book and takes no
        // number: Chapter Three becomes Chapter Two.
        put(
            &d,
            "manuscript/01-Act-One/02-Chapter-Two/01-Scene-One.md",
            "---\ncompile: false\n---\n\nGone.\n",
        );
        let p = Project::load(&d).unwrap();
        let b = book(&p, None, Scope::Whole).unwrap();
        assert_eq!(b.chapters, 2);
        assert!(
            markdown(&b).contains("## Chapter Two\n\n*The Long Road*\n\nThe road ran"),
            "{}",
            markdown(&b)
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn notes_and_tks_never_reach_any_export() {
        let d = book_dir("notes");
        put(
            &d,
            "manuscript/01-Act-One/01-Chapter-One/02-Scene-Two.md",
            "Gravel %% SECRETNOTE check the weather %% under her boots TK.\n\n\
             %%\nBLOCKNOTE across\nthree lines\n%%\n\n\
             She waited.\n",
        );
        put(
            &d,
            "front-matter/01-Title.md",
            "THE ARCHIVE %% FRONTNOTE %%\n",
        );
        let p = Project::load(&d).unwrap();

        // Counted as words: only the prose. Scene Two is 6 words, not 17.
        let two = p
            .nodes
            .iter()
            .find(|n| n.path.ends_with("02-Scene-Two.md"))
            .unwrap();
        assert_eq!(two.words(), 6);

        let b = book(&p, None, Scope::Whole).unwrap();
        let md = markdown(&b);
        for secret in ["SECRETNOTE", "BLOCKNOTE", "FRONTNOTE", "%%"] {
            assert!(!md.contains(secret), "{secret} reached the Markdown:\n{md}");
        }
        // A TK stays: a gap silently closed up is worse than one left showing.
        assert!(
            md.contains("Gravel under her boots TK.\n\nShe waited."),
            "{md}"
        );
        assert_eq!(b.tks.len(), 1);
        assert!(
            b.tks[0].starts_with("Chapter One, Scene Two: "),
            "{:?}",
            b.tks
        );

        let out = export(&p, &all_options()).unwrap();
        assert_eq!(out.tks.len(), 1, "the export says so too");
        let docx = unzip(&fs::read(d.join("exports/King_The-Archive_Manuscript.docx")).unwrap());
        let doc = entry(&docx, "word/document.xml");
        let paras = docx_paragraphs(doc);
        let text: String = paras
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for secret in ["SECRETNOTE", "BLOCKNOTE", "FRONTNOTE", "%%"] {
            assert!(!text.contains(secret), "{secret} reached the DOCX");
        }
        assert!(paras.iter().any(|(_, t)| t == "Gravel under her boots TK."));
        // No empty paragraph where the block note was.
        let at = paras
            .iter()
            .position(|(_, t)| t == "Gravel under her boots TK.")
            .unwrap();
        assert_eq!(paras[at + 1].1, "She waited.");

        let epub = unzip(&fs::read(d.join("exports/the-archive.epub")).unwrap());
        for (name, _, text) in &epub {
            if name.ends_with(".xhtml") {
                xml_doc(text).unwrap_or_else(|e| panic!("{name} is not well-formed: {e}"));
            }
            for secret in ["SECRETNOTE", "BLOCKNOTE", "FRONTNOTE", "%%"] {
                assert!(!text.contains(secret), "{secret} reached {name}");
            }
        }
        assert!(entry(&epub, "OEBPS/chapter-01.xhtml").contains("Gravel under her boots TK."));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn choosing_parts_keeps_the_books_own_chapter_numbers() {
        let d = book_dir("parts");
        let p = Project::load(&d).unwrap();
        let acts = parts(&p);
        assert_eq!(
            acts.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(),
            ["Act One", "Act Two"]
        );

        let only_two = ExportOptions {
            parts: Some(vec![acts[1].0]),
            ..all_options()
        };
        let out = export(&p, &only_two).unwrap();
        assert_eq!(out.chapters, 1);
        assert_eq!(out.words, 6);

        let md = fs::read_to_string(d.join("exports/the-archive.md")).unwrap();
        assert!(md.contains("# ACT TWO\n\n\n## Chapter Three\n"), "{md}");
        assert!(!md.contains("ACT ONE") && !md.contains("Chapter One") && !md.contains("Gravel"));
        // The title page always gives the whole book's count.
        assert_eq!(book(&p, None, Scope::Whole).unwrap().total, 33);
        assert_eq!(
            book(&p, Some(&[acts[1].0]), Scope::Whole).unwrap().total,
            33
        );

        let docx_file =
            unzip(&fs::read(d.join("exports/King_The-Archive_Manuscript_Act-Two.docx")).unwrap());
        let paras = docx_paragraphs(entry(&docx_file, "word/document.xml"));
        let headings: Vec<&str> = paras
            .iter()
            .filter(|(s, _)| s.starts_with("Heading"))
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(headings, ["Act Two", "Chapter Three"]);

        let files = unzip(&fs::read(d.join("exports/the-archive.epub")).unwrap());
        assert!(files.iter().any(|(n, _, _)| n == "OEBPS/chapter-03.xhtml"));
        assert!(!files.iter().any(|(n, _, _)| n == "OEBPS/chapter-01.xhtml"));

        // Something that isn't a part, or nothing at all, is refused.
        let chapter = p
            .nodes
            .iter()
            .position(|n| n.title == "Chapter One")
            .unwrap();
        assert!(
            export(
                &p,
                &ExportOptions {
                    parts: Some(vec![chapter]),
                    ..all_options()
                }
            )
            .is_err()
        );
        assert!(
            export(
                &p,
                &ExportOptions {
                    parts: Some(vec![]),
                    ..all_options()
                }
            )
            .is_err()
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn front_matter_replaces_the_generated_title_page() {
        let d = book_dir("front");
        put(
            &d,
            "front-matter/01-title.md",
            "Josh King\n\n# The Archive\n\nA *novel*\n",
        );
        put(&d, "front-matter/02-dedication.md", "For Wren.\n");
        let p = Project::load(&d).unwrap();
        let b = book(&p, None, Scope::Whole).unwrap();
        assert_eq!(b.front.len(), 2);

        let files = unzip(&docx(&b).unwrap());
        let paras = docx_paragraphs(entry(&files, "word/document.xml"));
        assert!(
            !paras.iter().any(|(s, _)| s == "Title"),
            "no generated title"
        );
        assert_eq!(paras[0].1, "about 0 words");
        assert!(
            paras
                .iter()
                .any(|(s, t)| s == "Centered" && t == "The Archive")
        );
        assert!(
            paras
                .iter()
                .any(|(s, t)| s == "Unindented" && t == "For Wren.")
        );

        let files = unzip(&epub(&b).unwrap());
        assert!(entry(&files, "OEBPS/front-1.xhtml").contains("<h3>The Archive</h3>"));
        assert!(files.iter().all(|(n, _, _)| n != "OEBPS/title.xhtml"));
        assert_eq!(pages(&b), 2 + 1 + 3 + 2);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_template_book_exports_only_what_is_written() {
        let d =
            std::env::temp_dir().join(format!("grimoire-export-template-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        crate::project::scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(parts(&p).len(), 3);
        // Nothing written: nothing to export, said plainly.
        let err = export(&p, &all_options()).unwrap_err().to_string();
        assert!(err.contains("nothing written"), "{err}");

        // One scene written in Chapter Two: one chapter, no blank pages for
        // the other twenty-six, no stray scene breaks, and it's Chapter One.
        let two = d.join("manuscript/01-Part-One/02-Chapter-Two/02-Scene-Two.md");
        let text = fs::read_to_string(&two).unwrap();
        fs::write(&two, format!("{text}\nThe only words so far.\n")).unwrap();
        let p = Project::load(&d).unwrap();
        let out = export(&p, &all_options()).unwrap();
        assert_eq!(out.chapters, 1);
        assert_eq!(
            out.empty,
            80 + 26 + 2,
            "80 empty scenes, 26 chapters, 2 parts"
        );
        let docx = unzip(&fs::read(&out.files[0]).unwrap());
        let paras = docx_paragraphs(entry(&docx, "word/document.xml"));
        let headings: Vec<&str> = paras
            .iter()
            .filter(|(s, _)| s.starts_with("Heading"))
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(headings, ["Part One", "Chapter One"]);
        assert!(!paras.iter().any(|(s, _)| s == "SceneBreak"));
        fs::remove_dir_all(&d).unwrap();
    }

    /// `grimoire compile` and a Markdown export write the same manuscript.
    #[test]
    fn compile_writes_the_shunn_markdown() {
        let d = book_dir("compile");
        let p = Project::load(&d).unwrap();
        let expected = concat!(
            "Josh King  \n\nabout 0 words\n\n# The Archive\n\nby Josh King\n\n",
            "\n# ACT ONE\n\n",
            "\n## Chapter One\n\n",
            "The building had *no* windows on the north face.\n\n“Wait — **now**,” she said, & meant <it>.\n",
            "\n\\#\n\n",
            "Gravel under her boots.\n",
            "\n## Chapter Two\n\nA lamp at the far end.\n",
            "\n# ACT TWO\n\n",
            "\n## Chapter Three\n\n*The Long Road*\n\nThe road ran _on_ and on.\n",
            "\nEND\n",
        );
        let c = crate::manuscript::compile(&p).unwrap();
        assert_eq!(c.path, d.join("the-archive-manuscript.md"));
        assert_eq!(fs::read_to_string(&c.path).unwrap(), expected);
        assert_eq!((c.words, c.chapters, c.scenes, c.skipped), (33, 3, 4, 1));
        assert_eq!(markdown(&book(&p, None, Scope::Whole).unwrap()), expected);
        fs::remove_dir_all(&d).unwrap();
    }

    // ── the submission manuscript (Shunn, Scrivener's Manuscript format) ──

    fn set_look(d: &Path, extra: &str) {
        let toml = d.join("novel.toml");
        let mut text = fs::read_to_string(&toml).unwrap();
        text.push_str(extra);
        fs::write(toml, text).unwrap();
    }

    fn docx_of(d: &Path, opts: &ExportOptions) -> (Vec<(String, String)>, String, String, String) {
        let p = Project::load(d).unwrap();
        let out = export(&p, opts).unwrap();
        let path = out
            .files
            .iter()
            .find(|f| f.extension().is_some_and(|e| e == "docx"))
            .unwrap();
        let files = unzip(&fs::read(path).unwrap());
        let doc = entry(&files, "word/document.xml").to_string();
        (
            docx_paragraphs(&doc),
            doc,
            entry(&files, "word/styles.xml").to_string(),
            entry(&files, "word/header1.xml").to_string(),
        )
    }

    #[test]
    fn the_title_page_has_the_contact_block_and_the_title_as_typed() {
        let d = book_dir("titlepage");
        set_look(
            &d,
            "\n[contact]\nlegal_name = \"Joshua King\"\naddress = [\"12 Harbour Road\", \"Kokomo\"]\n\
             phone = \"555-0100\"\nemail = \"josh@example.com\"\nagent = [\"Sam Lee, Lee Literary\"]\n",
        );
        let (paras, doc, styles, header) = docx_of(&d, &all_options());
        let lines: Vec<&str> = paras
            .iter()
            .take_while(|(s, _)| s == "TitlePageLine")
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(
            lines,
            [
                "Joshua Kingabout 0 words",
                "12 Harbour Road",
                "Kokomo",
                "555-0100",
                "josh@example.com",
                "",
                "Sam Lee, Lee Literary"
            ],
            "legal name and word count on one line (a right tab between), then the rest"
        );
        let title = paras.iter().position(|(s, _)| s == "Title").unwrap();
        assert_eq!(paras[title].1, "The Archive", "title case, as typed");
        assert_eq!(paras[title + 1], ("Centered".into(), "by Josh King".into()));
        // Halfway down: the space before the title makes up what the block
        // above didn't take (4.5 inches from the top margin in all).
        assert!(
            doc.contains(&format!("<w:spacing w:before=\"{}\"/>", 6480 - 7 * 276)),
            "{doc}"
        );
        // Shunn's modern defaults.
        assert!(styles.contains("w:ascii=\"Times New Roman\""));
        assert!(styles.contains("w:line=\"480\""));
        assert!(styles.contains("<w:ind w:firstLine=\"720\"/>"));
        assert!(doc.contains("<w:pgSz w:w=\"12240\" w:h=\"15840\"/>"));
        assert!(header.contains("King / ARCHIVE / "), "the byline's surname");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_classic_format_is_courier_on_a4_with_underlines_and_typewriter_marks() {
        let d = book_dir("classic");
        set_look(
            &d,
            "\n[manuscript]\nformat = \"classic\"\npaper = \"a4\"\nspacing = \"1.5\"\n\
             chapter_heading = \"caps\"\nfirst_paragraph = \"flush\"\nending = \"THE END\"\n\
             header_keyword = \"Archive Book\"\n",
        );
        let (paras, doc, styles, header) = docx_of(&d, &all_options());
        assert!(styles.contains("w:ascii=\"Courier New\""));
        assert!(!styles.contains("Times New Roman"));
        assert!(styles.contains("w:line=\"360\""));
        assert!(doc.contains("<w:pgSz w:w=\"11906\" w:h=\"16838\"/>"));
        assert!(doc.contains("<w:u w:val=\"single\"/>") && !doc.contains("<w:i/>"));
        assert!(
            paras
                .iter()
                .any(|(_, t)| t == "\"Wait -- now,\" she said, & meant <it>."),
            "straight quotes and a double hyphen: {paras:?}"
        );
        let has = |style: &str, text: &str| paras.iter().any(|(s, t)| s == style && t == text);
        assert!(
            has("Title", "THE ARCHIVE"),
            "capitals on the typewriter page"
        );
        assert!(has("Heading2", "CHAPTER ONE"));
        let ch1 = paras.iter().position(|(_, t)| t == "CHAPTER ONE").unwrap();
        assert_eq!(paras[ch1 + 1].0, "Unindented", "flush after the heading");
        assert_eq!(paras.last().unwrap().1, "THE END");
        assert!(header.contains("King / Archive Book / "));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn book_typography_reaches_every_format() {
        let d = book_dir("typography");
        put(
            &d,
            "manuscript/01-Act-One/01-Chapter-One/02-Scene-Two.md",
            "\"Hi,\" she said -- 'tis the '90s... and 'yes,' he said - finally.\n",
        );
        let want = "“Hi,” she said—’tis the ’90s… and ‘yes,’ he said—finally.";
        let (paras, ..) = docx_of(&d, &all_options());
        assert!(paras.iter().any(|(_, t)| t == want), "{paras:?}");
        let p = Project::load(&d).unwrap();
        let b = book(&p, None, Scope::Whole).unwrap();
        assert!(markdown(&b).contains(want));
        let files = unzip(&epub(&b).unwrap());
        assert!(entry(&files, "OEBPS/chapter-01.xhtml").contains(want));
        // The words on disk are never touched.
        assert!(
            fs::read_to_string(d.join("manuscript/01-Act-One/01-Chapter-One/02-Scene-Two.md"))
                .unwrap()
                .contains("\"Hi,\" she said --")
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_prologue_keeps_its_own_heading_and_takes_no_number() {
        let d = book_dir("prologue");
        put(
            &d,
            "manuscript/01-Act-One/00-Prologue/01-Fire.md",
            "The archive burned.\n",
        );
        let (paras, ..) = docx_of(&d, &all_options());
        let headings: Vec<&str> = paras
            .iter()
            .filter(|(s, _)| s == "Heading2")
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(
            headings,
            ["Prologue", "Chapter One", "Chapter Two", "Chapter Three"]
        );
        let p = Project::load(&d).unwrap();
        let files = unzip(&epub(&book(&p, None, Scope::Whole).unwrap()).unwrap());
        let (_, _, prologue) = files
            .iter()
            .find(|(n, _, _)| n.contains("prologue-"))
            .expect("a prologue file");
        assert!(prologue.contains("epub:type=\"prologue\""));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_scene_break_only_sits_between_two_written_scenes() {
        let d = book_dir("breaks");
        put(
            &d,
            "manuscript/01-Act-One/01-Chapter-One/02-Scene-Two.md",
            "%% only a note %%\n",
        );
        put(
            &d,
            "manuscript/01-Act-One/01-Chapter-One/03-Scene-Three.md",
            "Rain again.\n",
        );
        let (paras, ..) = docx_of(&d, &all_options());
        let one = paras.iter().position(|(_, t)| t == "Chapter One").unwrap();
        let two = paras.iter().position(|(_, t)| t == "Chapter Two").unwrap();
        let breaks = paras[one..two]
            .iter()
            .filter(|(s, _)| s == "SceneBreak")
            .count();
        assert_eq!(breaks, 1, "one break, between the two scenes with words");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_sample_is_the_chapters_or_words_asked_for_with_the_whole_books_count() {
        let d = book_dir("sample");
        put(
            &d,
            "manuscript/01-Act-One/00-Prologue/01-Fire.md",
            "The archive burned the year she was born.\n",
        );
        let p = Project::load(&d).unwrap();
        let headings = |b: &Book| -> Vec<String> {
            b.pieces
                .iter()
                .filter_map(|p| match p {
                    Piece::Chapter { number, title, .. } => {
                        Some(chapter_heading(*number, title, ChapterHeading::Words).0)
                    }
                    _ => None,
                })
                .collect()
        };
        let whole = book(&p, None, Scope::Whole).unwrap();
        assert!(whole.to_the_end);

        let first = book(&p, None, Scope::first(1)).unwrap();
        assert_eq!(headings(&first), ["Prologue", "Chapter One"]);
        assert_eq!(
            first.total, whole.total,
            "the title page counts the whole book"
        );
        assert!(!first.to_the_end, "a sample doesn't say END");

        let middle = book(&p, None, Scope::Chapters { from: 2, to: 3 }).unwrap();
        assert_eq!(headings(&middle), ["Chapter Two", "Chapter Three"]);

        // Words: whole scenes until the count is reached.
        let words = book(&p, None, Scope::Words(12)).unwrap();
        assert_eq!(headings(&words), ["Prologue", "Chapter One"]);
        assert!(words.words >= 12 && words.words < whole.words);

        let out = export(
            &p,
            &ExportOptions {
                scope: Scope::first(1),
                ..all_options()
            },
        )
        .unwrap();
        assert!(
            out.files.iter().any(
                |f| f.ends_with("King_The-Archive_Manuscript_Chapters-1-1.docx")
                    || f.ends_with("King_The-Archive_Manuscript_Chapter-1.docx")
            ),
            "{:?}",
            out.files
        );
        let md = markdown(&first);
        assert!(!md.contains("\nEND"), "{md}");
        assert!(book(&p, None, Scope::Chapters { from: 3, to: 1 }).is_err());
        assert!(book(&p, None, Scope::Words(0)).is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}
