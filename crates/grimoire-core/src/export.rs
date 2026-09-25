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
//! All three are drawn from one [`Book`], built by the same walk `compile`
//! uses: a folder holding scenes is a chapter, one holding chapters is a part,
//! `compile: false` scenes stay out, and front matter replaces the generated
//! title page. Choosing only some parts never renumbers the chapters — Act Two
//! still opens on its own chapter number.

use anyhow::{Context, Result, bail};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use crate::manuscript::{Section, commas, numbered, rounded_words, section_of, spell};
use crate::notes;
use crate::project::{Kind, Node, Project};

/// Roughly how much of a novel fits on one double-spaced manuscript page.
const WORDS_PER_PAGE: usize = 250;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub docx: bool,
    pub epub: bool,
    pub markdown: bool,
    /// Node indices of the top-level parts to include, as [`parts`] lists
    /// them. `None` is the whole book.
    pub parts: Option<Vec<usize>>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            docx: true,
            epub: true,
            markdown: false,
            parts: None,
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
}

/// The book's top-level parts (acts), in order: node index and title.
pub fn parts(p: &Project) -> Vec<(usize, String)> {
    manuscript_roots(p)
        .filter(|&i| p.nodes[i].kind == Kind::Container && section_of(p, i) == Section::Part)
        .map(|i| (i, p.nodes[i].title.clone()))
        .collect()
}

/// Write the chosen formats to `<book>/exports/`.
pub fn export(p: &Project, opts: &ExportOptions) -> Result<Exported> {
    p.ensure_whole()?;
    if !(opts.docx || opts.epub || opts.markdown) {
        bail!("nothing to export — choose DOCX, EPUB or Markdown");
    }
    let mut book = book(p, opts.parts.as_deref())?;
    let dir = p.root.join("exports");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let stem = slug(book.title);

    let mut files = Vec::new();
    let mut put = |ext: &str, bytes: &[u8]| -> Result<()> {
        let path = dir.join(format!("{stem}.{ext}"));
        crate::atomic::write(&path, bytes)?;
        files.push(path);
        Ok(())
    };
    if opts.docx {
        put("docx", &docx(&book)?)?;
    }
    if opts.epub {
        let submission = std::mem::replace(&mut book.front, front_pages(p, Edition::Ebook));
        put("epub", &epub(&book)?)?;
        book.front = submission;
    }
    if opts.markdown {
        put("md", markdown(&book).as_bytes())?;
    }

    Ok(Exported {
        files,
        words: book.words,
        chapters: book.chapters,
        pages: pages(&book),
    })
}

// ── the book ─────────────────────────────────────────────────────────

/// The manuscript as it will be read: what's in, in order, numbered.
pub(crate) struct Book<'a> {
    pub title: &'a str,
    pub author: &'a str,
    /// Front-matter scenes, which stand in for the generated title page.
    /// Notes and TKs are already out of every piece of text in here.
    pub front: Vec<Cow<'a, str>>,
    /// Words in every compiled scene of the chosen parts — the title page count.
    pub total: usize,
    pub pieces: Vec<Piece<'a>>,
    pub words: usize,
    pub chapters: usize,
    pub scenes: usize,
    pub skipped: usize,
}

pub(crate) enum Piece<'a> {
    Part {
        title: &'a str,
        depth: usize,
    },
    /// `number` is the chapter's place in the whole book, not in this export.
    Chapter {
        number: usize,
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
fn front_pages(p: &Project, edition: Edition) -> Vec<Cow<'_, str>> {
    p.nodes
        .iter()
        .filter(|n| n.kind == Kind::Scene && n.front_matter && n.compile)
        .filter(|n| edition_of(p, n).is_none_or(|e| e == edition))
        .map(|n| prose(&n.body))
        .filter(|body| !body.is_empty())
        .collect()
}

/// A scene's text as it goes into the book: `%% notes %%` and TKs taken out,
/// then trimmed.
fn prose(body: &str) -> Cow<'_, str> {
    match notes::strip(body) {
        Cow::Borrowed(b) => Cow::Borrowed(b.trim()),
        Cow::Owned(o) => Cow::Owned(o.trim().to_string()),
    }
}

/// Walk the manuscript. With `parts`, only those top-level parts are kept, but
/// every chapter is still counted so the kept ones keep their numbers.
pub(crate) fn book<'a>(p: &'a Project, parts: Option<&[usize]>) -> Result<Book<'a>> {
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

    let m = &p.meta;
    let title = m.title.trim();
    let mut b = Book {
        title: if title.is_empty() { "Untitled" } else { title },
        author: m.author.trim(),
        front: front_pages(p, Edition::Manuscript),
        total: 0,
        pieces: Vec::new(),
        words: 0,
        chapters: 0,
        scenes: 0,
        skipped: 0,
    };

    let mut number = 0usize;
    for r in manuscript_roots(p) {
        let keep = parts.is_none_or(|chosen| chosen.contains(&r));
        if keep {
            b.total += compiled_words(p, r);
        }
        gather(p, r, keep, &mut number, &mut b);
    }
    Ok(b)
}

fn compiled_words(p: &Project, idx: usize) -> usize {
    let n = &p.nodes[idx];
    match n.kind {
        Kind::Scene if n.compile => n.words(),
        Kind::Scene => 0,
        _ => n.children.iter().map(|&c| compiled_words(p, c)).sum(),
    }
}

fn gather<'a>(p: &'a Project, idx: usize, keep: bool, number: &mut usize, b: &mut Book<'a>) {
    let n = &p.nodes[idx];
    match section_of(p, idx) {
        Section::Part => {
            if keep {
                b.pieces.push(Piece::Part {
                    title: &n.title,
                    depth: n.depth.saturating_sub(1),
                });
            }
            for &c in &n.children {
                gather(p, c, keep, number, b);
            }
        }
        Section::Chapter => {
            let scenes: Vec<&Node> = n
                .children
                .iter()
                .map(|&c| &p.nodes[c])
                .filter(|s| s.kind == Kind::Scene)
                .collect();
            // A chapter with nothing compiled in it isn't in the book, and
            // doesn't take a number.
            if !scenes.iter().any(|s| s.compile) {
                return;
            }
            *number += 1;
            if !keep {
                return;
            }
            b.chapters += 1;
            let mut bodies = Vec::new();
            for s in scenes {
                if !s.compile {
                    b.skipped += 1;
                    continue;
                }
                bodies.push(prose(&s.body));
                b.words += s.words();
                b.scenes += 1;
            }
            b.pieces.push(Piece::Chapter {
                number: *number,
                title: &n.title,
                depth: n.depth.saturating_sub(1),
                scenes: bodies,
            });
        }
        Section::Scene => {
            if !keep {
                return;
            }
            if !n.compile {
                b.skipped += 1;
                return;
            }
            b.pieces.push(Piece::Scene(prose(&n.body)));
            b.words += n.words();
            b.scenes += 1;
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

/// `King / ARCHIVE / ` — the page number follows.
fn header_text(b: &Book) -> String {
    let name = surname(b.author);
    let key = keyword(b.title);
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
        let _ = writeln!(out, "# {}\n", b.title.to_uppercase());
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
            Piece::Chapter { number, scenes, .. } => {
                let _ = writeln!(out, "\n## CHAPTER {}\n", spell(*number));
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
    let _ = writeln!(out, "\nTHE END");
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

/// Blank double-spaced lines above a part or chapter heading: seven lines of
/// 12pt (~27.6pt each) under a one-inch margin put it a third of the way down
/// a US Letter page.
const OPENER_BLANK_LINES: usize = 7;

/// US Letter, one-inch margins, header half an inch from the top.
const PAGE: &str = "<w:pgSz w:w=\"12240\" w:h=\"15840\"/>\
    <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" \
    w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>";

struct Para {
    style: &'static str,
    page_break: bool,
    /// A blank double-spaced line after it, as under a chapter heading.
    gap_after: bool,
    runs: String,
}

impl Para {
    fn new(style: &'static str, runs: String) -> Self {
        Self {
            style,
            page_break: false,
            gap_after: false,
            runs,
        }
    }

    /// `sect` closes a section on this paragraph. Children of `w:pPr` go in
    /// schema order; Word is strict about it.
    fn xml(&self, sect: Option<&str>) -> String {
        let mut s = format!("<w:p><w:pPr><w:pStyle w:val=\"{}\"/>", self.style);
        if self.page_break {
            s.push_str("<w:pageBreakBefore/>");
        }
        if self.gap_after {
            s.push_str("<w:spacing w:after=\"480\"/>");
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

fn docx_runs(text: &str) -> String {
    let mut out = String::new();
    for r in inline(text) {
        out.push_str("<w:r>");
        if r.bold || r.italic {
            out.push_str("<w:rPr>");
            if r.bold {
                out.push_str("<w:b/>");
            }
            if r.italic {
                out.push_str("<w:i/>");
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

fn docx_blocks(body: &str, front: bool, out: &mut Vec<Para>) {
    for block in blocks(body) {
        out.push(match block {
            Block::Para(t) => Para::new(if front { "Unindented" } else { "Normal" }, docx_runs(t)),
            Block::Heading(t) => Para::new("Centered", docx_runs(t)),
            Block::Break => Para::new("SceneBreak", docx_text("#")),
            Block::Quote(t) => Para::new("Quote", docx_runs(t)),
        });
    }
}

fn docx(b: &Book) -> Result<Vec<u8>> {
    // Section one: the title page, or the author's own front matter. No
    // running header here.
    let count = format!("about {} words", commas(rounded_words(b.total)));
    let tab = "<w:r><w:tab/></w:r>";
    let mut opening: Vec<Para> = Vec::new();
    if b.front.is_empty() {
        opening.push(Para::new(
            "TitlePageLine",
            format!("{}{tab}{}", docx_text(b.author), docx_text(&count)),
        ));
        opening.push(Para::new("Title", docx_text(&b.title.to_uppercase())));
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
            docx_blocks(f, true, &mut opening);
            if i > 0 && start < opening.len() {
                opening[start].page_break = true;
            }
        }
    }

    // Section two: the book, numbered from 1, header on every page.
    let mut body: Vec<Para> = Vec::new();
    for piece in &b.pieces {
        let start = body.len();
        // Parts and chapters open a third of the way down a new page. Blank
        // double-spaced lines put them there, as Shunn says to: space-before
        // at the top of a page is dropped by LibreOffice, and blank lines
        // hold in every word processor.
        if !matches!(piece, Piece::Scene(_)) {
            for _ in 0..OPENER_BLANK_LINES {
                body.push(Para::new("Blank", String::new()));
            }
        }
        match piece {
            Piece::Part { title, .. } => {
                let mut h = Para::new("Heading1", docx_runs(title));
                h.gap_after = true;
                body.push(h);
            }
            Piece::Chapter {
                number,
                title,
                scenes,
                ..
            } => {
                body.push(Para::new(
                    "Heading2",
                    docx_text(&numbered("Chapter", *number)),
                ));
                if let Some(name) = chapter_name(title) {
                    body.push(Para::new("ChapterTitle", docx_runs(name)));
                }
                if let Some(last) = body.last_mut() {
                    last.gap_after = true;
                }
                for (i, s) in scenes.iter().enumerate() {
                    if i > 0 {
                        body.push(Para::new("SceneBreak", docx_text("#")));
                    }
                    docx_blocks(s, false, &mut body);
                }
            }
            Piece::Scene(s) => docx_blocks(s, false, &mut body),
        }
        // The first thing in the section is already on a new page.
        if start > 0 && start < body.len() && !matches!(piece, Piece::Scene(_)) {
            body[start].page_break = true;
        }
    }
    body.push(Para::new("Centered", docx_text("END")));

    let mut doc = format!("{XML_HEAD}<w:document xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:body>\n");
    let title_sect = format!("<w:sectPr>{PAGE}</w:sectPr>");
    let last = opening.len() - 1;
    for (i, para) in opening.iter().enumerate() {
        doc.push_str(&para.xml((i == last).then_some(title_sect.as_str())));
    }
    for para in &body {
        doc.push_str(&para.xml(None));
    }
    let _ = write!(
        doc,
        "<w:sectPr><w:headerReference w:type=\"default\" r:id=\"rIdHeader\"/>{PAGE}\
         <w:pgNumType w:start=\"1\"/></w:sectPr>\n</w:body></w:document>\n"
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
        ("word/styles.xml", docx_styles()),
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
fn docx_styles() -> String {
    let font = "<w:rFonts w:ascii=\"Times New Roman\" w:hAnsi=\"Times New Roman\" \
                w:eastAsia=\"Times New Roman\" w:cs=\"Times New Roman\"/>";
    let plain = format!(
        "<w:rPr>{font}<w:b w:val=\"0\"/><w:bCs w:val=\"0\"/><w:i w:val=\"0\"/><w:iCs w:val=\"0\"/>\
         <w:color w:val=\"000000\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr>"
    );
    // (id, name, pPr contents)
    let styles: [(&str, &str, &str); 11] = [
        (
            "Heading1",
            "heading 1",
            "<w:keepNext/><w:keepLines/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/><w:outlineLvl w:val=\"0\"/>",
        ),
        (
            "Heading2",
            "heading 2",
            "<w:keepNext/><w:keepLines/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/><w:outlineLvl w:val=\"1\"/>",
        ),
        // The empty lines that bring a chapter heading down the page.
        (
            "Blank",
            "Blank Line",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/>",
        ),
        // The title sits about halfway down its page.
        (
            "Title",
            "Title",
            "<w:keepNext/><w:spacing w:before=\"5760\"/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>",
        ),
        (
            "ChapterTitle",
            "Chapter Title",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>",
        ),
        (
            "SceneBreak",
            "Scene Break",
            "<w:keepNext/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>",
        ),
        (
            "Centered",
            "Centered",
            "<w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/>",
        ),
        ("Unindented", "Unindented", "<w:ind w:firstLine=\"0\"/>"),
        (
            "Quote",
            "Quote",
            "<w:ind w:left=\"720\" w:right=\"720\" w:firstLine=\"0\"/>",
        ),
        // Name top left, word count top right, single-spaced.
        (
            "TitlePageLine",
            "Title Page Line",
            "<w:tabs><w:tab w:val=\"right\" w:pos=\"9360\"/></w:tabs>\
             <w:spacing w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:firstLine=\"0\"/>",
        ),
        (
            "Header",
            "header",
            "<w:tabs><w:tab w:val=\"right\" w:pos=\"9360\"/></w:tabs>\
             <w:spacing w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"right\"/>",
        ),
    ];

    let mut s = format!(
        "{XML_HEAD}<w:styles xmlns:w=\"{W_NS}\">\
         <w:docDefaults><w:rPrDefault><w:rPr>{font}<w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/>\
         <w:lang w:val=\"en-US\" w:eastAsia=\"en-US\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"480\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
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

    let (mut part_no, mut text_no) = (0, 0);
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
                let heading = numbered("Chapter", *number);
                let name = chapter_name(title);
                let mut body = String::from("<div class=\"chapter\" epub:type=\"chapter\">\n");
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
                let id = format!("chapter-{number:02}");
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
            epub: true,
            markdown: true,
            parts: None,
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
        assert!(docx_runs("<w:p> & *it*").contains("&lt;w:p&gt; &amp; </w:t>"));
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
        let docx_path = d.join("exports/the-archive.docx");
        assert!(out.files.contains(&docx_path));

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
        assert!(has("Title", "THE ARCHIVE"));
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

        // Parts and chapters open a page (Act One opens the book's section, so
        // it needs no break); the title page is a section of its own, and the
        // book's section carries the header and numbers from 1.
        assert_eq!(doc.matches("<w:pageBreakBefore/>").count(), 4);
        let ch1 = paras
            .iter()
            .position(|(s, t)| s == "Heading2" && t == "Chapter One")
            .unwrap();
        assert!(
            paras[ch1 - OPENER_BLANK_LINES..ch1]
                .iter()
                .all(|(s, t)| s == "Blank" && t.is_empty()),
            "a chapter heading sits a third of the way down its page"
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
        let b = book(&p, None).unwrap();
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
        let b = book(&p, None).unwrap();
        assert_eq!(b.chapters, 2);
        assert!(markdown(&b).contains("## CHAPTER TWO\n\nThe road ran"));
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

        let b = book(&p, None).unwrap();
        let md = markdown(&b);
        for secret in ["SECRETNOTE", "BLOCKNOTE", "FRONTNOTE", "%%", " TK"] {
            assert!(!md.contains(secret), "{secret} reached the Markdown:\n{md}");
        }
        assert!(
            md.contains("Gravel under her boots.\n\nShe waited."),
            "{md}"
        );

        export(&p, &all_options()).unwrap();
        let docx = unzip(&fs::read(d.join("exports/the-archive.docx")).unwrap());
        let doc = entry(&docx, "word/document.xml");
        let paras = docx_paragraphs(doc);
        let text: String = paras
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for secret in ["SECRETNOTE", "BLOCKNOTE", "FRONTNOTE", "%%", "TK"] {
            assert!(!text.contains(secret), "{secret} reached the DOCX");
        }
        assert!(paras.iter().any(|(_, t)| t == "Gravel under her boots."));
        // No empty paragraph where the block note was.
        let at = paras
            .iter()
            .position(|(_, t)| t == "Gravel under her boots.")
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
        assert!(entry(&epub, "OEBPS/chapter-01.xhtml").contains("Gravel under her boots."));
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
        assert!(md.contains("# ACT TWO\n\n\n## CHAPTER THREE\n"), "{md}");
        assert!(!md.contains("ACT ONE") && !md.contains("CHAPTER ONE") && !md.contains("Gravel"));
        // The title page counts only what's exported.
        assert_eq!(book(&p, None).unwrap().total, 33);
        assert_eq!(book(&p, Some(&[acts[1].0])).unwrap().total, 6);

        let docx_file = unzip(&fs::read(d.join("exports/the-archive.docx")).unwrap());
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
        let b = book(&p, None).unwrap();
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
    fn a_new_template_book_exports() {
        let d =
            std::env::temp_dir().join(format!("grimoire-export-template-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        crate::project::scaffold(&d).unwrap();
        let p = Project::load(&d).unwrap();
        assert_eq!(parts(&p).len(), 3);
        let out = export(&p, &all_options()).unwrap();
        assert_eq!(out.chapters, 27);
        assert_eq!(out.words, 0);
        let files = unzip(&fs::read(&out.files[1]).unwrap());
        assert!(entry(&files, "OEBPS/nav.xhtml").contains("Chapter Twenty-Seven"));
        fs::remove_dir_all(&d).unwrap();
    }

    /// `grimoire compile` and a Markdown export write the same manuscript.
    #[test]
    fn compile_writes_the_shunn_markdown() {
        let d = book_dir("compile");
        let p = Project::load(&d).unwrap();
        let expected = concat!(
            "Josh King  \n\nabout 0 words\n\n# THE ARCHIVE\n\nby Josh King\n\n",
            "\n# ACT ONE\n\n",
            "\n## CHAPTER ONE\n\n",
            "The building had *no* windows on the north face.\n\n“Wait — **now**,” she said, & meant <it>.\n",
            "\n\\#\n\n",
            "Gravel under her boots.\n",
            "\n## CHAPTER TWO\n\nA lamp at the far end.\n",
            "\n# ACT TWO\n\n",
            "\n## CHAPTER THREE\n\nThe road ran _on_ and on.\n",
            "\nTHE END\n",
        );
        let c = crate::manuscript::compile(&p).unwrap();
        assert_eq!(c.path, d.join("the-archive-manuscript.md"));
        assert_eq!(fs::read_to_string(&c.path).unwrap(), expected);
        assert_eq!((c.words, c.chapters, c.scenes, c.skipped), (33, 3, 4, 1));
        assert_eq!(markdown(&book(&p, None).unwrap()), expected);
        fs::remove_dir_all(&d).unwrap();
    }
}
