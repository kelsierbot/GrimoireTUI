//! The print edition: the book laid out for paper, as Scrivener's
//! "Paperback Novel" compile format does it, and as KDP and IngramSpark want
//! a print-ready interior.
//!
//! - **Trim size** is the page: 5 × 8, 5.25 × 8, 5.5 × 8.5 or 6 × 9 inches.
//! - **Mirrored margins**, the inside one widened by KDP's gutter for the
//!   book's page count, so no line runs into the spine.
//! - **Running heads**: the author on the left-hand page, the title on the
//!   right, in small capitals — none on a chapter's first page, where only
//!   the folio shows, at the foot. Page numbers centred at the foot.
//! - **Chapters open on a right-hand page**, their heading sunk a third of the
//!   way down, the first paragraph flush left with its first words in small
//!   capitals (or a drop cap). Text justified and hyphenated, widows and
//!   orphans kept off page edges, scene breaks as a centred ornament.
//! - **Front matter** from the book's Paperback front-matter folder (title,
//!   copyright, dedication), or a title page and copyright page made from
//!   novel.toml; **back matter** after the story.
//!
//! One page model ([`Section`]s of [`Para`]s) is written two ways: a DOCX to
//! open in Word or LibreOffice for last touches, and — when LibreOffice is
//! installed — a PDF made from the same model as a LibreOffice document,
//! whose right-hand-only page styles put every chapter on a recto and leave
//! the pages it has to skip truly blank ([`to_pdf`]). (LibreOffice reads a
//! DOCX's odd-page section breaks by giving the skipped page the chapter's
//! first-page style, which puts a running head on the chapter's opening
//! page; Word gets them right.)

use anyhow::{Context, Result, bail};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::export::{Block, Book, Piece, blocks, chapter_name, inline, pack, xml_escape};
use crate::manuscript::numbered;
use crate::project::{EbookMeta, PaperbackMeta};

/// A paperback's page size, in inches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trim {
    T5x8,
    T525x8,
    T55x85,
    T6x9,
}

impl Trim {
    pub const ALL: [Trim; 4] = [Trim::T5x8, Trim::T525x8, Trim::T55x85, Trim::T6x9];

    /// `6x9`, `6 x 9`, `5.5x8.5`, `5.25×8`, `6x9in`…
    pub fn parse(s: &str) -> Option<Trim> {
        let t: String = s
            .to_lowercase()
            .replace("in", "")
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '"')
            .map(|c| if c == '×' { 'x' } else { c })
            .collect();
        match t.as_str() {
            "5x8" => Some(Trim::T5x8),
            "5.25x8" => Some(Trim::T525x8),
            "5.5x8.5" => Some(Trim::T55x85),
            "6x9" => Some(Trim::T6x9),
            _ => None,
        }
    }

    pub fn inches(self) -> (f64, f64) {
        match self {
            Trim::T5x8 => (5.0, 8.0),
            Trim::T525x8 => (5.25, 8.0),
            Trim::T55x85 => (5.5, 8.5),
            Trim::T6x9 => (6.0, 9.0),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Trim::T5x8 => "5 × 8 in",
            Trim::T525x8 => "5.25 × 8 in",
            Trim::T55x85 => "5.5 × 8.5 in",
            Trim::T6x9 => "6 × 9 in",
        }
    }

    pub fn next(self) -> Trim {
        let i = Trim::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Trim::ALL[(i + 1) % Trim::ALL.len()]
    }

    pub fn prev(self) -> Trim {
        let i = Trim::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Trim::ALL[(i + Trim::ALL.len() - 1) % Trim::ALL.len()]
    }
}

/// How the pages look: `[paperback]` in novel.toml, or its defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub trim: Trim,
    pub font: String,
    /// Text size in points.
    pub size: f64,
    pub ornament: String,
    pub drop_cap: bool,
}

impl Layout {
    pub fn from_meta(m: &PaperbackMeta) -> Layout {
        let font = m.font.trim();
        let ornament = m.ornament.trim();
        Layout {
            trim: Trim::parse(&m.trim).unwrap_or(Trim::T6x9),
            font: if font.is_empty() {
                "Garamond".into()
            } else {
                font.into()
            },
            size: if (8.0..=16.0).contains(&m.size) {
                m.size
            } else {
                11.0
            },
            ornament: if ornament.is_empty() {
                "* * *".into()
            } else {
                ornament.into()
            },
            drop_cap: m.drop_cap,
        }
    }

    /// Line to line, in points: a book's comfortable 1.3.
    fn leading(&self) -> f64 {
        self.size * 1.3
    }
}

/// What the print edition carries beyond the manuscript.
pub struct Print<'a> {
    pub layout: Layout,
    pub back: Vec<Cow<'a, str>>,
    /// Language, publisher, ISBN and rights: for hyphenation and the
    /// generated copyright page.
    pub meta: &'a EbookMeta,
}

/// KDP's minimum inside (gutter) margin for a page count, in inches.
pub fn gutter(pages: usize) -> f64 {
    match pages {
        0..=150 => 0.375,
        151..=300 => 0.5,
        301..=500 => 0.625,
        501..=700 => 0.75,
        _ => 0.875,
    }
}

/// Margins: (top, bottom, inside, outside), in inches. A little more than
/// KDP's minimum on the inside, so no line hugs the spine.
fn margins(pages: usize) -> (f64, f64, f64, f64) {
    (0.75, 0.75, gutter(pages) + 0.125, 0.5)
}

/// About how many pages the book will print to: words over what fits a page
/// at this size and trim, plus openers (and the blank versos half of them
/// need), plus front and back matter.
pub(crate) fn estimate_pages(b: &Book, layout: &Layout, back: usize) -> usize {
    let (w, h) = layout.trim.inches();
    let (top, bottom, inside, outside) = margins(300);
    let area = (w - inside - outside) * (h - top - bottom);
    // ~300 words fill a 6 × 9 page of 11pt type; scale by text area and size.
    let base_area = (6.0 - 0.625 - 0.5) * (9.0 - 1.5);
    let per_page = 300.0 * (area / base_area) * (11.0 / layout.size).powi(2);
    let openers = b
        .pieces
        .iter()
        .filter(|p| !matches!(p, Piece::Scene(_)))
        .count();
    let front = b.front.len().max(2);
    front + (b.words as f64 / per_page).ceil() as usize + openers + openers / 2 + back
}

// ── the page model ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct Span {
    text: String,
    italic: bool,
    bold: bool,
    small: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Blank,
    Body,
    BodyFirst,
    PartTitle,
    ChapterNumber,
    ChapterTitle,
    SubHeading,
    Ornament,
    Quote,
    BookTitle,
    BookAuthor,
    Copyright,
    MatterCentered,
    MatterText,
    MatterHeading,
}

impl Style {
    const ALL: [Style; 15] = [
        Style::Blank,
        Style::Body,
        Style::BodyFirst,
        Style::PartTitle,
        Style::ChapterNumber,
        Style::ChapterTitle,
        Style::SubHeading,
        Style::Ornament,
        Style::Quote,
        Style::BookTitle,
        Style::BookAuthor,
        Style::Copyright,
        Style::MatterCentered,
        Style::MatterText,
        Style::MatterHeading,
    ];

    fn id(self) -> &'static str {
        match self {
            Style::Blank => "Blank",
            Style::Body => "Body",
            Style::BodyFirst => "BodyFirst",
            Style::PartTitle => "PartTitle",
            Style::ChapterNumber => "ChapterNumber",
            Style::ChapterTitle => "ChapterTitle",
            Style::SubHeading => "SubHeading",
            Style::Ornament => "Ornament",
            Style::Quote => "BookQuote",
            Style::BookTitle => "BookTitle",
            Style::BookAuthor => "BookAuthor",
            Style::Copyright => "Copyright",
            Style::MatterCentered => "MatterCentered",
            Style::MatterText => "MatterText",
            Style::MatterHeading => "MatterHeading",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Style::Blank => "Blank Line",
            Style::Body => "Body Text",
            Style::BodyFirst => "Body First",
            Style::PartTitle => "Part Title",
            Style::ChapterNumber => "Chapter Number",
            Style::ChapterTitle => "Chapter Title",
            Style::SubHeading => "Scene Heading",
            Style::Ornament => "Scene Break",
            Style::Quote => "Book Quote",
            Style::BookTitle => "Book Title",
            Style::BookAuthor => "Book Author",
            Style::Copyright => "Copyright",
            Style::MatterCentered => "Matter Centred",
            Style::MatterText => "Matter Text",
            Style::MatterHeading => "Matter Heading",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Para {
    style: Style,
    spans: Vec<Span>,
    /// The first letter (with any quotation mark before it) as a two-line
    /// drop cap.
    drop: bool,
}

impl Para {
    fn new(style: Style, spans: Vec<Span>) -> Para {
        Para {
            style,
            spans,
            drop: false,
        }
    }

    fn blank() -> Para {
        Para::new(Style::Blank, Vec::new())
    }

    fn text(style: Style, text: &str) -> Para {
        Para::new(style, plain_span(text))
    }
}

/// Where a section starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// On a right-hand (odd) page, a blank left-hand page before it if needed.
    Right,
    /// On the next page, whichever side.
    Next,
}

/// Which heads and folios a section's pages show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Heads {
    /// None at all: title, copyright, part pages, back matter.
    None,
    /// A chapter: running heads after its first page, folios on every page.
    Chapter,
}

#[derive(Debug, Clone, PartialEq)]
struct Section {
    start: Start,
    heads: Heads,
    /// The first page of the story restarts the numbering at 1.
    restart: bool,
    paras: Vec<Para>,
}

fn plain_span(text: &str) -> Vec<Span> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![Span {
        text: text.to_string(),
        italic: false,
        bold: false,
        small: false,
    }]
}

/// Inline Markdown to spans; the first `caps` words in small capitals.
fn spans(text: &str, caps: usize) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut words_left = caps;
    let mut in_word = false;
    for r in inline(text) {
        let (mut lead, mut rest) = (String::new(), String::new());
        for c in r.text.chars() {
            if words_left > 0 {
                if c.is_whitespace() {
                    if in_word {
                        words_left -= 1;
                        in_word = false;
                    }
                } else {
                    in_word = true;
                }
            }
            if words_left > 0 {
                lead.push(c);
            } else {
                rest.push(c);
            }
        }
        for (text, small) in [(lead, true), (rest, false)] {
            if !text.is_empty() {
                out.push(Span {
                    text,
                    italic: r.italic,
                    bold: r.bold,
                    small,
                });
            }
        }
    }
    out
}

/// Blank lines that sink an opener's heading about a third of the way down.
/// Blank lines rather than space-before: word processors drop space above
/// the first line of a page, but a blank line always holds.
fn sink(layout: &Layout, out: &mut Vec<Para>) {
    let (_, h) = layout.trim.inches();
    let lines = ((h - 1.5) * 72.0 * 0.28 / layout.leading()).round() as usize;
    out.extend(std::iter::repeat_with(Para::blank).take(lines.max(3)));
}

/// A scene's text as book paragraphs. `opening`: the chapter's first scene,
/// whose first paragraph gets the small-capital lead-in or the drop cap.
fn scene_paras(body: &str, layout: &Layout, opening: bool, out: &mut Vec<Para>) {
    let mut first = true;
    let mut lead = opening;
    for block in blocks(body) {
        match block {
            Block::Para(t) => {
                if lead {
                    let mut p = Para::new(
                        Style::BodyFirst,
                        spans(t, if layout.drop_cap { 0 } else { 3 }),
                    );
                    p.drop = layout.drop_cap;
                    out.push(p);
                    lead = false;
                } else {
                    let style = if first { Style::BodyFirst } else { Style::Body };
                    out.push(Para::new(style, spans(t, 0)));
                }
                first = false;
            }
            Block::Heading(t) => {
                out.push(Para::new(Style::SubHeading, spans(t, 0)));
                first = true;
            }
            Block::Break => {
                out.push(Para::text(Style::Ornament, &layout.ornament));
                first = true;
            }
            Block::Quote(t) => {
                out.push(Para::new(Style::Quote, spans(t, 0)));
                first = true;
            }
        }
    }
}

/// A page of front or back matter, as the author wrote it.
fn matter_paras(body: &str, heading: Style, para: Style, out: &mut Vec<Para>) {
    for block in blocks(body) {
        out.push(match block {
            Block::Para(t) => Para::new(para, spans(t, 0)),
            Block::Heading(t) => Para::new(heading, spans(t, 0)),
            Block::Break => Para::text(Style::Ornament, "*"),
            Block::Quote(t) => Para::new(Style::Quote, spans(t, 0)),
        });
    }
}

/// The whole book as sections of paragraphs.
fn build(b: &Book, pr: &Print) -> Vec<Section> {
    let layout = &pr.layout;
    let mut out: Vec<Section> = Vec::new();
    let section = |start, heads, paras| Section {
        start,
        heads,
        restart: false,
        paras,
    };

    // ── front matter ──
    if b.front.is_empty() {
        let mut title = Vec::new();
        sink(layout, &mut title);
        title.push(Para::text(Style::BookTitle, b.title));
        if !b.author.is_empty() {
            title.push(Para::text(Style::BookAuthor, b.author));
        }
        out.push(section(Start::Right, Heads::None, title));

        // The copyright page, on the back of the title page.
        let mut copy = Vec::new();
        copy.extend(std::iter::repeat_with(Para::blank).take(12));
        let year = chrono::Local::now().format("%Y");
        let who = if b.author.is_empty() {
            "the author"
        } else {
            b.author
        };
        copy.push(Para::text(
            Style::Copyright,
            &format!("Copyright © {year} {who}"),
        ));
        let rights = pr.meta.rights.trim();
        copy.push(Para::text(
            Style::Copyright,
            if rights.is_empty() {
                "All rights reserved."
            } else {
                rights
            },
        ));
        let isbn = pr.meta.isbn.trim();
        if !isbn.is_empty() {
            copy.push(Para::text(Style::Copyright, &format!("ISBN {isbn}")));
        }
        let publisher = pr.meta.publisher.trim();
        if !publisher.is_empty() {
            copy.push(Para::text(
                Style::Copyright,
                &format!("Published by {publisher}"),
            ));
        }
        out.push(section(Start::Next, Heads::None, copy));
    } else {
        // The author's own pages: the first is the title page (on a
        // right-hand page), the second the copyright page on its back, the
        // rest (dedication, epigraph) each on a right-hand page.
        for (i, f) in b.front.iter().enumerate() {
            let mut paras = Vec::new();
            let start = match i {
                0 => {
                    sink(layout, &mut paras);
                    matter_paras(f, Style::BookTitle, Style::BookAuthor, &mut paras);
                    Start::Right
                }
                1 => {
                    matter_paras(f, Style::MatterHeading, Style::Copyright, &mut paras);
                    Start::Next
                }
                _ => {
                    sink(layout, &mut paras);
                    matter_paras(f, Style::MatterHeading, Style::MatterCentered, &mut paras);
                    Start::Right
                }
            };
            out.push(section(start, Heads::None, paras));
        }
    }

    // ── the story ──
    let story = out.len();
    for piece in &b.pieces {
        match piece {
            Piece::Part { title, .. } => {
                let mut paras = Vec::new();
                sink(layout, &mut paras);
                paras.push(Para::new(Style::PartTitle, spans(title, 0)));
                out.push(section(Start::Right, Heads::None, paras));
            }
            Piece::Chapter {
                number,
                title,
                scenes,
                ..
            } => {
                let mut paras = Vec::new();
                sink(layout, &mut paras);
                paras.push(Para::text(
                    Style::ChapterNumber,
                    &numbered("Chapter", *number),
                ));
                if let Some(name) = chapter_name(title) {
                    paras.push(Para::new(Style::ChapterTitle, spans(name, 0)));
                }
                paras.push(Para::blank());
                for (i, s) in scenes.iter().enumerate() {
                    if i > 0 {
                        paras.push(Para::text(Style::Ornament, &layout.ornament));
                    }
                    scene_paras(s, layout, i == 0, &mut paras);
                }
                out.push(section(Start::Right, Heads::Chapter, paras));
            }
            Piece::Scene(s) => {
                let mut paras = Vec::new();
                scene_paras(s, layout, true, &mut paras);
                out.push(section(Start::Right, Heads::Chapter, paras));
            }
        }
    }
    if let Some(first) = out.get_mut(story) {
        first.restart = true;
    }

    // ── back matter ──
    for (i, text) in pr.back.iter().enumerate() {
        let mut paras = Vec::new();
        paras.extend(std::iter::repeat_with(Para::blank).take(4));
        matter_paras(text, Style::MatterHeading, Style::MatterText, &mut paras);
        let start = if i == 0 { Start::Right } else { Start::Next };
        out.push(section(start, Heads::None, paras));
    }
    out.retain(|s| s.paras.iter().any(|p| p.style != Style::Blank));
    out
}

/// A style's look, shared by the two writers so the DOCX and the PDF are the
/// same book. Sizes in points.
struct Look {
    align: &'static str,
    indent: f64,
    margin_lr: f64,
    before: f64,
    after: f64,
    size: f64,
    /// Exact line height.
    line: f64,
    italic: bool,
    small_caps: bool,
    caps: bool,
    tracking: f64,
    keep_next: bool,
    heading: Option<u8>,
}

fn look(style: Style, l: &Layout) -> Look {
    let size = l.size;
    let lead = l.leading();
    let base = Look {
        align: "justify",
        indent: 0.0,
        margin_lr: 0.0,
        before: 0.0,
        after: 0.0,
        size,
        line: lead,
        italic: false,
        small_caps: false,
        caps: false,
        tracking: 0.0,
        keep_next: false,
        heading: None,
    };
    let centred = |extra: Look| Look {
        align: "center",
        ..extra
    };
    match style {
        Style::Body => Look {
            indent: size * 1.6,
            ..base
        },
        Style::BodyFirst => base,
        Style::Blank => Look {
            align: "left",
            keep_next: true,
            ..base
        },
        Style::PartTitle => centred(Look {
            size: size * 1.6,
            line: size * 1.6 * 1.3,
            caps: true,
            tracking: 3.0,
            keep_next: true,
            heading: Some(0),
            ..base
        }),
        Style::ChapterNumber => centred(Look {
            size: size * 1.25,
            line: size * 1.25 * 1.3,
            after: 12.0,
            small_caps: true,
            tracking: 2.0,
            keep_next: true,
            heading: Some(1),
            ..base
        }),
        Style::ChapterTitle => centred(Look {
            size: size * 1.5,
            line: size * 1.5 * 1.3,
            after: 12.0,
            italic: true,
            keep_next: true,
            ..base
        }),
        Style::SubHeading => centred(Look {
            small_caps: true,
            keep_next: true,
            ..base
        }),
        Style::Ornament => centred(Look {
            before: lead,
            after: lead,
            keep_next: true,
            ..base
        }),
        // Block quotes and epigraphs: indented both sides, half a line
        // clear of the text around them.
        Style::Quote => Look {
            margin_lr: size * 2.0,
            size: size * 0.95,
            line: size * 0.95 * 1.3,
            before: lead / 2.0,
            after: lead / 2.0,
            ..base
        },
        Style::BookTitle => centred(Look {
            size: size * 2.2,
            line: size * 2.2 * 1.25,
            after: 24.0,
            caps: true,
            tracking: 2.0,
            ..base
        }),
        Style::BookAuthor => centred(Look {
            size: size * 1.3,
            line: size * 1.3 * 1.3,
            small_caps: true,
            tracking: 1.5,
            ..base
        }),
        Style::Copyright => Look {
            align: "left",
            size: size * 0.85,
            line: size * 0.85 * 1.3,
            after: 6.0,
            ..base
        },
        Style::MatterCentered => centred(Look {
            after: lead / 2.0,
            ..base
        }),
        Style::MatterText => Look {
            after: lead / 2.0,
            ..base
        },
        Style::MatterHeading => centred(Look {
            size: size * 1.25,
            line: size * 1.25 * 1.3,
            after: lead,
            small_caps: true,
            tracking: 2.0,
            keep_next: true,
            heading: Some(1),
            ..base
        }),
    }
}

/// `en-GB` → (`en`, `GB`).
fn lang_country(lang: &str) -> (String, String) {
    let mut parts = lang.split(['-', '_']);
    let l = parts.next().unwrap_or("en").to_lowercase();
    let c = parts.next().map(str::to_uppercase).unwrap_or_else(|| {
        match l.as_str() {
            "en" => "US",
            "fr" => "FR",
            "de" => "DE",
            "es" => "ES",
            "it" => "IT",
            _ => "",
        }
        .to_string()
    });
    (l, c)
}

/// A drop cap splits off the first letter (and a quotation mark before it).
fn split_cap(spans: &[Span]) -> Option<(String, Vec<Span>)> {
    let first = spans.first()?;
    let mut cap = String::new();
    let mut taken = 0;
    for c in first.text.chars() {
        cap.push(c);
        taken += c.len_utf8();
        if c.is_alphanumeric() {
            break;
        }
    }
    if !cap.chars().any(char::is_alphanumeric) {
        return None;
    }
    let mut rest = spans.to_vec();
    rest[0].text = first.text[taken..].to_string();
    if rest[0].text.is_empty() {
        rest.remove(0);
    }
    Some((cap, rest))
}

// ── DOCX ─────────────────────────────────────────────────────────────

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn twips(inches: f64) -> i64 {
    (inches * 1440.0).round() as i64
}

fn tw(pt: f64) -> i64 {
    (pt * 20.0).round() as i64
}

fn docx_runs(spans: &[Span]) -> String {
    let mut out = String::new();
    for s in spans {
        out.push_str("<w:r>");
        if s.bold || s.italic || s.small {
            out.push_str("<w:rPr>");
            if s.bold {
                out.push_str("<w:b/>");
            }
            if s.italic {
                out.push_str("<w:i/>");
            }
            if s.small {
                out.push_str("<w:smallCaps/>");
            }
            out.push_str("</w:rPr>");
        }
        let _ = write!(
            out,
            "<w:t xml:space=\"preserve\">{}</w:t></w:r>",
            xml_escape(&s.text)
        );
    }
    out
}

fn docx_para(p: &Para, layout: &Layout, sect: Option<&str>) -> String {
    let mut out = String::new();
    let mut spans = p.spans.clone();
    if p.drop
        && let Some((cap, rest)) = split_cap(&p.spans)
    {
        let size = (layout.size * 2.0 * 2.6).round() as i64;
        let _ = writeln!(
            out,
            "<w:p><w:pPr><w:pStyle w:val=\"BodyFirst\"/><w:keepNext/>\
             <w:framePr w:dropCap=\"drop\" w:lines=\"2\" w:wrap=\"around\" w:vAnchor=\"text\" w:hAnchor=\"text\"/>\
             <w:spacing w:line=\"{}\" w:lineRule=\"exact\"/></w:pPr>\
             <w:r><w:rPr><w:position w:val=\"-4\"/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr>\
             <w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            tw(layout.leading() * 2.0),
            xml_escape(&cap)
        );
        spans = rest;
    }
    let _ = writeln!(
        out,
        "<w:p><w:pPr><w:pStyle w:val=\"{}\"/>{}</w:pPr>{}</w:p>",
        p.style.id(),
        sect.unwrap_or(""),
        docx_runs(&spans)
    );
    out
}

fn docx_styles(layout: &Layout, lang: &str) -> String {
    let face = xml_escape(&layout.font);
    let lang = xml_escape(lang);
    let font = format!(
        "<w:rFonts w:ascii=\"{face}\" w:hAnsi=\"{face}\" w:eastAsia=\"{face}\" w:cs=\"{face}\"/>"
    );
    let half = |pt: f64| (pt * 2.0).round() as i64;
    let mut s = format!(
        "{XML_HEAD}<w:styles xmlns:w=\"{W_NS}\">\
         <w:docDefaults><w:rPrDefault><w:rPr>{font}<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/>\
         <w:lang w:val=\"{lang}\" w:eastAsia=\"{lang}\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"{1}\" w:lineRule=\"exact\"/></w:pPr></w:pPrDefault>\
         </w:docDefaults>\n\
         <w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\">\
         <w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/></w:style>\n",
        half(layout.size),
        tw(layout.leading()),
    );
    for style in Style::ALL {
        let k = look(style, layout);
        let mut ppr = String::new();
        if k.keep_next {
            ppr.push_str("<w:keepNext/><w:keepLines/>");
        }
        ppr.push_str("<w:widowControl/>");
        let _ = write!(
            ppr,
            "<w:spacing w:before=\"{}\" w:after=\"{}\" w:line=\"{}\" w:lineRule=\"exact\"/>\
             <w:ind w:left=\"{3}\" w:right=\"{3}\" w:firstLine=\"{4}\"/><w:jc w:val=\"{5}\"/>",
            tw(k.before),
            tw(k.after),
            tw(k.line),
            tw(k.margin_lr),
            tw(k.indent),
            match k.align {
                "center" => "center",
                "left" => "left",
                _ => "both",
            }
        );
        if let Some(level) = k.heading {
            let _ = write!(ppr, "<w:outlineLvl w:val=\"{level}\"/>");
        }
        let mut rpr = font.clone();
        if k.italic {
            rpr.push_str("<w:i/>");
        }
        if k.caps {
            rpr.push_str("<w:caps/>");
        }
        if k.small_caps {
            rpr.push_str("<w:smallCaps/>");
        }
        let _ = write!(
            rpr,
            "<w:color w:val=\"000000\"/><w:spacing w:val=\"{}\"/><w:sz w:val=\"{1}\"/><w:szCs w:val=\"{1}\"/>",
            tw(k.tracking),
            half(k.size)
        );
        let default = if style == Style::Body {
            " w:default=\"1\""
        } else {
            ""
        };
        let _ = writeln!(
            s,
            "<w:style w:type=\"paragraph\"{default} w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:qFormat/>\
             <w:pPr>{ppr}</w:pPr><w:rPr>{rpr}</w:rPr></w:style>",
            style.id(),
            style.name()
        );
    }
    // Running heads and folios.
    let _ = writeln!(
        s,
        "<w:style w:type=\"paragraph\" w:styleId=\"RunningHead\"><w:name w:val=\"Running Head\"/>\
         <w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/></w:pPr>\
         <w:rPr>{font}<w:smallCaps/><w:spacing w:val=\"30\"/><w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/></w:rPr></w:style>\n\
         <w:style w:type=\"paragraph\" w:styleId=\"Folio\"><w:name w:val=\"Folio\"/>\
         <w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/><w:ind w:firstLine=\"0\"/><w:jc w:val=\"center\"/></w:pPr>\
         <w:rPr>{font}<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/></w:rPr></w:style>",
        half(layout.size * 0.85)
    );
    s.push_str("</w:styles>\n");
    s
}

fn docx(b: &Book, pr: &Print, sections: &[Section], pages: usize) -> Result<Vec<u8>> {
    let layout = &pr.layout;
    let (w, h) = layout.trim.inches();
    let (top, bottom, inside, outside) = margins(pages);
    // With mirrored margins, "left" is the inside and "right" the outside.
    let page = format!(
        "<w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" \
         w:header=\"{}\" w:footer=\"{}\" w:gutter=\"0\"/>",
        twips(w),
        twips(h),
        twips(top),
        twips(outside),
        twips(bottom),
        twips(inside),
        twips(0.4),
        twips(0.35)
    );
    let sect_pr = |s: &Section| {
        let refs = match s.heads {
            Heads::None => {
                "<w:headerReference w:type=\"default\" r:id=\"rIdHBlank\"/>\
                 <w:headerReference w:type=\"even\" r:id=\"rIdHBlank\"/>\
                 <w:headerReference w:type=\"first\" r:id=\"rIdHBlank\"/>\
                 <w:footerReference w:type=\"default\" r:id=\"rIdFBlank\"/>\
                 <w:footerReference w:type=\"even\" r:id=\"rIdFBlank\"/>\
                 <w:footerReference w:type=\"first\" r:id=\"rIdFBlank\"/>"
            }
            Heads::Chapter => {
                "<w:headerReference w:type=\"default\" r:id=\"rIdHTitle\"/>\
                 <w:headerReference w:type=\"even\" r:id=\"rIdHAuthor\"/>\
                 <w:headerReference w:type=\"first\" r:id=\"rIdHBlank\"/>\
                 <w:footerReference w:type=\"default\" r:id=\"rIdFFolio\"/>\
                 <w:footerReference w:type=\"even\" r:id=\"rIdFFolio\"/>\
                 <w:footerReference w:type=\"first\" r:id=\"rIdFFolio\"/>"
            }
        };
        let start = match s.start {
            Start::Right => "oddPage",
            Start::Next => "nextPage",
        };
        let numbering = if s.restart {
            "<w:pgNumType w:fmt=\"decimal\" w:start=\"1\"/>"
        } else {
            ""
        };
        format!(
            "<w:sectPr>{refs}<w:type w:val=\"{start}\"/>{page}{numbering}<w:titlePg/></w:sectPr>"
        )
    };

    let mut doc = format!("{XML_HEAD}<w:document xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:body>\n");
    let last = sections.len().saturating_sub(1);
    for (si, s) in sections.iter().enumerate() {
        let lastp = s.paras.len().saturating_sub(1);
        for (pi, p) in s.paras.iter().enumerate() {
            // A section's properties ride on its last paragraph — except the
            // book's last, which close the body.
            let sect = (si < last && pi == lastp).then(|| sect_pr(s));
            doc.push_str(&docx_para(p, layout, sect.as_deref()));
        }
    }
    if let Some(s) = sections.last() {
        doc.push_str(&sect_pr(s));
    }
    doc.push_str("\n</w:body></w:document>\n");

    let header = |text: &str| {
        format!(
            "{XML_HEAD}<w:hdr xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:p><w:pPr><w:pStyle w:val=\"RunningHead\"/></w:pPr>{}</w:p></w:hdr>\n",
            docx_runs(&plain_span(text))
        )
    };
    let folio = format!(
        "{XML_HEAD}<w:ftr xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:p><w:pPr><w:pStyle w:val=\"Folio\"/></w:pPr>\
         <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>\
         <w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r>\
         <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>1</w:t></w:r>\
         <w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p></w:ftr>\n"
    );
    let blank_footer = format!(
        "{XML_HEAD}<w:ftr xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:p><w:pPr><w:pStyle w:val=\"Folio\"/></w:pPr></w:p></w:ftr>\n"
    );
    let author_head = if b.author.is_empty() {
        b.title
    } else {
        b.author
    };
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
    let files: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), CONTENT_TYPES.into()),
        ("_rels/.rels".into(), RELS.into()),
        ("docProps/core.xml".into(), core),
        (
            "docProps/app.xml".into(),
            format!(
                "{XML_HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Grimoire</Application></Properties>\n"
            ),
        ),
        ("word/document.xml".into(), doc),
        ("word/_rels/document.xml.rels".into(), DOCUMENT_RELS.into()),
        (
            "word/styles.xml".into(),
            docx_styles(layout, pr.meta.language()),
        ),
        ("word/settings.xml".into(), SETTINGS.into()),
        ("word/header-title.xml".into(), header(b.title)),
        ("word/header-author.xml".into(), header(author_head)),
        ("word/header-blank.xml".into(), header("")),
        ("word/footer-folio.xml".into(), folio),
        ("word/footer-blank.xml".into(), blank_footer),
    ];
    pack(&files)
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
<Override PartName=\"/word/header-title.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
<Override PartName=\"/word/header-author.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
<Override PartName=\"/word/header-blank.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
<Override PartName=\"/word/footer-folio.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>\
<Override PartName=\"/word/footer-blank.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\
</Types>
";

const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\
</Relationships>
";

const DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\
<Relationship Id=\"rIdHTitle\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header-title.xml\"/>\
<Relationship Id=\"rIdHAuthor\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header-author.xml\"/>\
<Relationship Id=\"rIdHBlank\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header-blank.xml\"/>\
<Relationship Id=\"rIdFFolio\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer-folio.xml\"/>\
<Relationship Id=\"rIdFBlank\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer-blank.xml\"/>\
</Relationships>
";

/// Mirrored margins, left and right pages with their own heads, and
/// automatic hyphenation (no more than two hyphens in a row).
const SETTINGS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:mirrorMargins/>\
<w:defaultTabStop w:val=\"360\"/>\
<w:autoHyphenation/>\
<w:consecutiveHyphenLimit w:val=\"2\"/>\
<w:hyphenationZone w:val=\"357\"/>\
<w:evenAndOddHeaders/>\
<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>\
</w:settings>
";

// ── the LibreOffice document behind the PDF ─────────────────────────

const ODF_NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
     xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
     xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
     xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
     xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
     xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
     xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\"";

fn inch(v: f64) -> String {
    format!("{v:.4}in")
}

fn pt(v: f64) -> String {
    format!("{v:.2}pt")
}

/// The master page a section opens on: right-hand only, or either side.
fn master(s: &Section) -> &'static str {
    match (s.start, s.heads) {
        (Start::Right, Heads::Chapter) => "Opener",
        (Start::Right, Heads::None) => "Plain-Right",
        (Start::Next, Heads::Chapter) => "Opener-Any",
        (Start::Next, Heads::None) => "Plain",
    }
}

fn opener_style(s: &Section, first: Style) -> String {
    format!(
        "open-{}-{}{}",
        master(s),
        first.id(),
        if s.restart { "-1" } else { "" }
    )
}

/// The book as a flat LibreOffice document (FODT): the same paragraphs and
/// styles as the DOCX, with page styles that do what DOCX section breaks
/// can't in LibreOffice — a right-hand-only opener page whose skipped left
/// page stays blank, and a body page style with left and right heads.
fn fodt(b: &Book, pr: &Print, sections: &[Section], pages: usize) -> String {
    let layout = &pr.layout;
    let (w, h) = layout.trim.inches();
    let (top, bottom, inside, outside) = margins(pages);
    let (lang, country) = lang_country(pr.meta.language());
    let face = xml_escape(&layout.font);

    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document {ODF_NS} office:version=\"1.3\" \
         office:mimetype=\"application/vnd.oasis.opendocument.text\">\n\
         <office:meta><dc:title>{}</dc:title><meta:initial-creator>{}</meta:initial-creator></office:meta>\n\
         <office:font-face-decls><style:font-face style:name=\"{face}\" svg:font-family=\"'{face}'\" \
         style:font-family-generic=\"roman\"/></office:font-face-decls>\n\
         <office:styles>\n\
         <style:default-style style:family=\"paragraph\"><style:paragraph-properties fo:hyphenation-ladder-count=\"2\"/>\
         <style:text-properties style:font-name=\"{face}\" fo:font-size=\"{}\" fo:language=\"{lang}\" fo:country=\"{country}\" \
         fo:hyphenate=\"true\" fo:hyphenation-remain-char-count=\"3\" fo:hyphenation-push-char-count=\"3\"/></style:default-style>\n",
        xml_escape(b.title),
        xml_escape(b.author),
        pt(layout.size)
    );
    for style in Style::ALL {
        let k = look(style, layout);
        let mut para = format!(
            "fo:text-align=\"{align}\" fo:text-indent=\"{indent}\" fo:margin-left=\"{side}\" fo:margin-right=\"{side}\" \
             fo:margin-top=\"{before}\" fo:margin-bottom=\"{after}\" fo:line-height=\"{line}\" fo:widows=\"2\" fo:orphans=\"2\"",
            align = k.align,
            indent = pt(k.indent),
            side = pt(k.margin_lr),
            before = pt(k.before),
            after = pt(k.after),
            line = pt(k.line)
        );
        if k.keep_next {
            para.push_str(" fo:keep-with-next=\"always\"");
        }
        if k.align == "justify" {
            para.push_str(" fo:text-align-last=\"start\"");
        }
        let mut text = format!("fo:font-size=\"{}\"", pt(k.size));
        if k.italic {
            text.push_str(" fo:font-style=\"italic\"");
        }
        if k.small_caps {
            text.push_str(" fo:font-variant=\"small-caps\"");
        }
        if k.caps {
            text.push_str(" fo:text-transform=\"uppercase\"");
        }
        if k.tracking > 0.0 {
            let _ = write!(text, " fo:letter-spacing=\"{}\"", pt(k.tracking));
        }
        let outline = k
            .heading
            .map(|l| format!(" style:default-outline-level=\"{}\"", l + 1))
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "<style:style style:name=\"{}\" style:display-name=\"{}\" style:family=\"paragraph\"{outline}>\
             <style:paragraph-properties {para}/><style:text-properties {text}/></style:style>",
            style.id(),
            style.name()
        );
    }
    let small = pt(layout.size * 0.85);
    let _ = writeln!(
        s,
        "<style:style style:name=\"RunningHead\" style:family=\"paragraph\">\
         <style:paragraph-properties fo:text-align=\"center\"/>\
         <style:text-properties fo:font-size=\"{small}\" fo:font-variant=\"small-caps\" fo:letter-spacing=\"1.5pt\"/></style:style>\n\
         <style:style style:name=\"Folio\" style:family=\"paragraph\">\
         <style:paragraph-properties fo:text-align=\"center\"/><style:text-properties fo:font-size=\"{small}\"/></style:style>\n\
         </office:styles>"
    );

    // Automatic styles: the page layouts, a paragraph style per (master,
    // style) that opens a section, drop caps, and the text spans.
    s.push_str("<office:automatic-styles>\n");
    // The header and footer sit in the top and bottom margins.
    let page_props = format!(
        "fo:page-width=\"{}\" fo:page-height=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" \
         fo:margin-left=\"{}\" fo:margin-right=\"{}\" style:print-orientation=\"portrait\"",
        inch(w),
        inch(h),
        inch(top - 0.35),
        inch(bottom - 0.35),
        inch(inside),
        inch(outside)
    );
    let hf = "<style:header-style><style:header-footer-properties fo:min-height=\"0.2in\" fo:margin-bottom=\"0.15in\"/></style:header-style>\
              <style:footer-style><style:header-footer-properties fo:min-height=\"0.2in\" fo:margin-top=\"0.15in\"/></style:footer-style>";
    for (name, usage) in [("pl-right", "right"), ("pl-mirrored", "mirrored")] {
        let _ = writeln!(
            s,
            "<style:page-layout style:name=\"{name}\" style:page-usage=\"{usage}\">\
             <style:page-layout-properties {page_props}/>{hf}</style:page-layout>"
        );
    }
    let mut openers: Vec<String> = Vec::new();
    for sec in sections {
        let first = sec.paras.first().map(|p| p.style).unwrap_or(Style::Blank);
        let name = opener_style(sec, first);
        if openers.contains(&name) {
            continue;
        }
        let number = if sec.restart {
            " style:page-number=\"1\""
        } else {
            ""
        };
        let _ = writeln!(
            s,
            "<style:style style:name=\"{name}\" style:family=\"paragraph\" style:parent-style-name=\"{}\" \
             style:master-page-name=\"{}\"><style:paragraph-properties{number}/></style:style>",
            first.id(),
            master(sec)
        );
        openers.push(name);
    }
    for len in 1..=3 {
        let _ = writeln!(
            s,
            "<style:style style:name=\"drop-{len}\" style:family=\"paragraph\" style:parent-style-name=\"BodyFirst\">\
             <style:paragraph-properties><style:drop-cap style:lines=\"2\" style:length=\"{len}\" style:distance=\"0.04in\"/>\
             </style:paragraph-properties></style:style>"
        );
    }
    for bits in 1..8u8 {
        let mut t = String::new();
        if bits & 1 != 0 {
            t.push_str(" fo:font-style=\"italic\"");
        }
        if bits & 2 != 0 {
            t.push_str(" fo:font-weight=\"bold\"");
        }
        if bits & 4 != 0 {
            t.push_str(" fo:font-variant=\"small-caps\"");
        }
        let _ = writeln!(
            s,
            "<style:style style:name=\"t{bits}\" style:family=\"text\"><style:text-properties{t}/></style:style>"
        );
    }
    s.push_str("</office:automatic-styles>\n");

    // Master pages. An opener is right-hand only; the page after it is the
    // body page, with the author on the left and the title on the right.
    let author_head = if b.author.is_empty() {
        b.title
    } else {
        b.author
    };
    let folio = "<text:p text:style-name=\"Folio\"><text:page-number text:select-page=\"current\">1</text:page-number></text:p>";
    let _ = writeln!(
        s,
        "<office:master-styles>\n\
         <style:master-page style:name=\"Plain-Right\" style:page-layout-name=\"pl-right\" style:next-style-name=\"Plain\"/>\n\
         <style:master-page style:name=\"Plain\" style:page-layout-name=\"pl-mirrored\"/>\n\
         <style:master-page style:name=\"Opener\" style:page-layout-name=\"pl-right\" style:next-style-name=\"Body\">\
         <style:footer>{folio}</style:footer></style:master-page>\n\
         <style:master-page style:name=\"Opener-Any\" style:page-layout-name=\"pl-mirrored\" style:next-style-name=\"Body\">\
         <style:footer>{folio}</style:footer><style:footer-left>{folio}</style:footer-left></style:master-page>\n\
         <style:master-page style:name=\"Body\" style:page-layout-name=\"pl-mirrored\">\
         <style:header><text:p text:style-name=\"RunningHead\">{}</text:p></style:header>\
         <style:header-left><text:p text:style-name=\"RunningHead\">{}</text:p></style:header-left>\
         <style:footer>{folio}</style:footer><style:footer-left>{folio}</style:footer-left></style:master-page>\n\
         </office:master-styles>",
        xml_escape(b.title),
        xml_escape(author_head)
    );

    s.push_str("<office:body><office:text>\n");
    let span_xml = |spans: &[Span]| {
        let mut out = String::new();
        for sp in spans {
            let bits = u8::from(sp.italic) | (u8::from(sp.bold) << 1) | (u8::from(sp.small) << 2);
            let text = xml_escape(&sp.text);
            if bits == 0 {
                out.push_str(&text);
            } else {
                let _ = write!(
                    out,
                    "<text:span text:style-name=\"t{bits}\">{text}</text:span>"
                );
            }
        }
        out
    };
    for sec in sections {
        for (i, p) in sec.paras.iter().enumerate() {
            let style = if i == 0 {
                opener_style(sec, p.style)
            } else if p.drop
                && let Some((cap, _)) = split_cap(&p.spans)
            {
                format!("drop-{}", cap.chars().count().clamp(1, 3))
            } else {
                p.style.id().to_string()
            };
            let _ = writeln!(
                s,
                "<text:p text:style-name=\"{style}\">{}</text:p>",
                span_xml(&p.spans)
            );
        }
    }
    s.push_str("</office:text></office:body></office:document>\n");
    s
}

/// The print edition: the DOCX, and the LibreOffice document for the PDF.
pub(crate) fn paperback(b: &Book, pr: &Print) -> Result<(Vec<u8>, String)> {
    let sections = build(b, pr);
    if sections.is_empty() {
        bail!("there's nothing in the book to print yet");
    }
    let pages = estimate_pages(b, &pr.layout, pr.back.len());
    Ok((
        docx(b, pr, &sections, pages)?,
        fodt(b, pr, &sections, pages),
    ))
}

// ── PDF, through LibreOffice ─────────────────────────────────────────

/// How to run LibreOffice here, if it's installed: on the PATH, in the usual
/// macOS and Windows places, or as the Flathub app (`true`: sandboxed, so it
/// can't see /tmp).
fn soffice() -> Option<(Vec<String>, bool)> {
    let on_path = |exe: &str| {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths).find_map(|d| {
                let p = d.join(exe);
                p.is_file().then(|| p.to_string_lossy().to_string())
            })
        })
    };
    for exe in ["soffice", "libreoffice", "soffice.exe"] {
        if let Some(p) = on_path(exe) {
            return Some((vec![p], false));
        }
    }
    for p in [
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
        "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
    ] {
        if Path::new(p).is_file() {
            return Some((vec![p.to_string()], false));
        }
    }
    let flatpak = on_path("flatpak")?;
    let installed = std::process::Command::new(&flatpak)
        .args(["info", "org.libreoffice.LibreOffice"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    installed.then(|| {
        (
            vec![flatpak, "run".into(), "org.libreoffice.LibreOffice".into()],
            true,
        )
    })
}

/// Make `pdf` from the paperback's LibreOffice document, fonts embedded and
/// the blank pages a printer needs kept. Says how to get LibreOffice when it
/// isn't there.
pub fn to_pdf(document: &str, pdf: &Path) -> Result<PathBuf> {
    let Some((cmd, sandboxed)) = soffice() else {
        bail!(
            "no PDF: LibreOffice isn't installed — get it from libreoffice.org (or `flatpak install flathub org.libreoffice.LibreOffice`) and export again, or open {} in Word and save it as PDF",
            pdf.with_extension("docx")
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
    };
    // Work in a folder of our own — in the cache for the Flathub app, which
    // sees the home folder but not /tmp.
    let stage = if sandboxed {
        crate::paths::home().join(".cache").join("grimoire")
    } else {
        std::env::temp_dir()
    }
    .join(format!("paperback-{}", std::process::id()));
    std::fs::create_dir_all(&stage).with_context(|| format!("creating {}", stage.display()))?;
    let src = stage.join("paperback.fodt");
    std::fs::write(&src, document).with_context(|| format!("writing {}", src.display()))?;
    let result = convert(&cmd, &stage, &src, pdf);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn convert(cmd: &[String], stage: &Path, src: &Path, pdf: &Path) -> Result<PathBuf> {
    // A profile of its own, so a LibreOffice already open doesn't swallow
    // the conversion.
    let profile = stage.join("profile");
    let profile_url = format!("file://{}", profile.to_string_lossy().replace('\\', "/"));
    let mut child = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .arg("--headless")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--convert-to")
        .arg("pdf:writer_pdf_Export:{\"IsSkipEmptyPages\":{\"type\":\"boolean\",\"value\":\"false\"}}")
        .arg("--outdir")
        .arg(stage)
        .arg(src)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting LibreOffice")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("no PDF: LibreOffice took more than four minutes");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let made = src.with_extension("pdf");
    if !status.success() || !made.is_file() {
        bail!("no PDF: LibreOffice couldn't lay out the paperback");
    }
    let bytes = std::fs::read(&made).context("reading the PDF")?;
    crate::atomic::write(pdf, &bytes)?;
    Ok(pdf.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_parse_the_ways_people_write_them() {
        assert_eq!(Trim::parse("6x9"), Some(Trim::T6x9));
        assert_eq!(Trim::parse("6 × 9 in"), Some(Trim::T6x9));
        assert_eq!(Trim::parse("5.5x8.5"), Some(Trim::T55x85));
        assert_eq!(Trim::parse("5.25 x 8"), Some(Trim::T525x8));
        assert_eq!(Trim::parse("A4"), None);
        assert_eq!(Trim::T6x9.next(), Trim::T5x8);
        assert_eq!(Trim::T5x8.prev(), Trim::T6x9);
    }

    #[test]
    fn the_gutter_follows_kdp_page_counts() {
        assert_eq!(gutter(120), 0.375);
        assert_eq!(gutter(151), 0.5);
        assert_eq!(gutter(420), 0.625);
        assert_eq!(gutter(650), 0.75);
        assert_eq!(gutter(800), 0.875);
    }

    #[test]
    fn a_lead_in_puts_the_first_words_in_small_capitals() {
        let s = spans("The rain had *stopped* an hour", 3);
        let small: String = s
            .iter()
            .filter(|s| s.small)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(small, "The rain had");
        assert!(
            s.iter()
                .any(|s| s.italic && !s.small && s.text == "stopped")
        );
    }

    #[test]
    fn a_drop_cap_takes_a_quotation_mark_with_its_letter() {
        let (cap, rest) = split_cap(&plain_span("“You’re early,” she said.")).unwrap();
        assert_eq!(cap, "“Y");
        assert_eq!(rest[0].text, "ou’re early,” she said.");
    }
}
