//! What a submission needs that the book's files don't say: who an agent
//! writes back to, and how the manuscript should look.
//!
//! Both live in `novel.toml`, in tables of their own:
//!
//! ```toml
//! author = "Avery Marlowe"          # the byline, as before
//!
//! [contact]                         # the title page's top-left block
//! legal_name = "Jane Q. Smith"
//! address = ["12 Harbour Road", "Portland, ME 04101"]
//! phone = "555-0100"
//! email = "jane@example.com"
//! agent = ["Represented by Sam Lee", "Lee Literary"]
//!
//! [manuscript]                      # how the manuscript looks
//! format = "modern"                 # or "classic" (Courier, underlines)
//! paper = "letter"                  # or "a4"
//! spacing = "double"                # or "1.5"
//! chapter_heading = "words"         # Chapter One / caps / digits / number / title
//! first_paragraph = "indented"      # or "flush"
//! ending = "END"                    # or "THE END", "none"
//! header_keyword = ""               # empty: the title's first real word
//! spaced_hyphen = "em"              # or "en"
//! ```
//!
//! The byline stays the top-level `author` string rather than moving into a
//! table: older copies of Grimoire, and the phone, read `author` as a string,
//! and a book synced between them must keep opening everywhere.
//!
//! Reading is forgiving — a value it doesn't know is the default, never an
//! error that would stop the book opening. Writing rewrites only these two
//! tables, leaves every other line (and comment) as it was, and refuses when
//! novel.toml can't be read, so nothing is ever written over settings it
//! couldn't see.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};
use std::path::Path;

use crate::typeset::SpacedHyphen;

/// The contact block on a novel's title page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contact {
    /// The name on the contract. Falls back to the byline.
    pub legal_name: String,
    pub address: Vec<String>,
    pub phone: String,
    pub email: String,
    /// Lines for an agent's block, if there is one.
    pub agent: Vec<String>,
}

impl Contact {
    pub fn is_empty(&self) -> bool {
        self.legal_name.trim().is_empty()
            && self.address.iter().all(|l| l.trim().is_empty())
            && self.phone.trim().is_empty()
            && self.email.trim().is_empty()
            && self.agent.iter().all(|l| l.trim().is_empty())
    }
}

/// Modern (Shunn): Times New Roman, italics, smart quotes. Classic ("Shunn
/// Classic"): Courier New, underlines for italics, straight quotes and `--`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Modern,
    Classic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Paper {
    /// US Letter, one-inch margins.
    #[default]
    Letter,
    /// A4, 2.54 cm margins — UK and European agents.
    A4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Spacing {
    #[default]
    Double,
    /// Some UK agents accept it; Shunn never does.
    OneAndHalf,
}

/// How a numbered chapter is headed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChapterHeading {
    /// Chapter One
    #[default]
    Words,
    /// CHAPTER ONE
    Caps,
    /// Chapter 1
    Digits,
    /// 1
    Number,
    /// The folder's own title alone.
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FirstParagraph {
    /// Shunn: indent every paragraph alike.
    #[default]
    Indented,
    /// CMOS and published books: flush after a heading or a break.
    Flush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ending {
    #[default]
    End,
    TheEnd,
    None,
}

impl Ending {
    pub fn text(self) -> Option<&'static str> {
        match self {
            Ending::End => Some("END"),
            Ending::TheEnd => Some("THE END"),
            Ending::None => None,
        }
    }
}

/// The look of the submission manuscript. The defaults are Shunn's modern
/// novel format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manuscript {
    pub format: Format,
    pub paper: Paper,
    pub spacing: Spacing,
    pub chapter_heading: ChapterHeading,
    pub first_paragraph: FirstParagraph,
    pub ending: Ending,
    /// The running header's title word. Empty: the title's first real word.
    pub header_keyword: String,
    pub spaced_hyphen: SpacedHyphen,
}

impl Manuscript {
    /// Smart quotes and real dashes: yes, unless it's the classic format.
    pub fn typography(&self) -> bool {
        self.format == Format::Modern
    }

    /// One line for a dialog: "Modern · Letter · double · Chapter One".
    pub fn summary(&self) -> String {
        format!(
            "{} · {} · {} · {}",
            self.format.label(),
            self.paper.label(),
            self.spacing.label(),
            self.chapter_heading.label()
        )
    }
}

// ── labels and values, for the dialog and for novel.toml ──────────────

/// A setting with a fixed set of choices: how it reads, how novel.toml spells
/// it, and the choice after it (for ←/→ in a dialog).
pub trait Choice: Sized + Copy + PartialEq + 'static {
    const ALL: &'static [Self];
    fn label(self) -> &'static str;
    fn key(self) -> &'static str;
    /// By its key or its label: exactly first ("CHAPTER ONE" is the capitals
    /// style, not "Chapter One"), then ignoring case.
    fn from_key(s: &str) -> Option<Self> {
        let s = s.trim();
        let all = Self::ALL.iter().copied();
        all.clone()
            .find(|c| c.key() == s || c.label() == s)
            .or_else(|| {
                all.clone()
                    .find(|c| c.key().eq_ignore_ascii_case(s) || c.label().eq_ignore_ascii_case(s))
            })
    }
    fn step(self, forward: bool) -> Self {
        let all = Self::ALL;
        let i = all.iter().position(|&c| c == self).unwrap_or(0);
        let n = all.len();
        all[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

impl Choice for Format {
    const ALL: &'static [Self] = &[Format::Modern, Format::Classic];
    fn label(self) -> &'static str {
        match self {
            Format::Modern => "Modern (Times New Roman)",
            Format::Classic => "Classic (Courier)",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Format::Modern => "modern",
            Format::Classic => "classic",
        }
    }
}

impl Choice for Paper {
    const ALL: &'static [Self] = &[Paper::Letter, Paper::A4];
    fn label(self) -> &'static str {
        match self {
            Paper::Letter => "US Letter",
            Paper::A4 => "A4",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Paper::Letter => "letter",
            Paper::A4 => "a4",
        }
    }
}

impl Choice for Spacing {
    const ALL: &'static [Self] = &[Spacing::Double, Spacing::OneAndHalf];
    fn label(self) -> &'static str {
        match self {
            Spacing::Double => "double-spaced",
            Spacing::OneAndHalf => "1.5-spaced",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Spacing::Double => "double",
            Spacing::OneAndHalf => "1.5",
        }
    }
}

impl Choice for ChapterHeading {
    const ALL: &'static [Self] = &[
        ChapterHeading::Words,
        ChapterHeading::Caps,
        ChapterHeading::Digits,
        ChapterHeading::Number,
        ChapterHeading::Title,
    ];
    fn label(self) -> &'static str {
        match self {
            ChapterHeading::Words => "Chapter One",
            ChapterHeading::Caps => "CHAPTER ONE",
            ChapterHeading::Digits => "Chapter 1",
            ChapterHeading::Number => "1",
            ChapterHeading::Title => "the chapter's title",
        }
    }
    fn key(self) -> &'static str {
        match self {
            ChapterHeading::Words => "words",
            ChapterHeading::Caps => "caps",
            ChapterHeading::Digits => "digits",
            ChapterHeading::Number => "number",
            ChapterHeading::Title => "title",
        }
    }
}

impl Choice for FirstParagraph {
    const ALL: &'static [Self] = &[FirstParagraph::Indented, FirstParagraph::Flush];
    fn label(self) -> &'static str {
        match self {
            FirstParagraph::Indented => "indented, like the rest",
            FirstParagraph::Flush => "flush after a heading or break",
        }
    }
    fn key(self) -> &'static str {
        match self {
            FirstParagraph::Indented => "indented",
            FirstParagraph::Flush => "flush",
        }
    }
}

impl Choice for Ending {
    const ALL: &'static [Self] = &[Ending::End, Ending::TheEnd, Ending::None];
    fn label(self) -> &'static str {
        match self {
            Ending::End => "END",
            Ending::TheEnd => "THE END",
            Ending::None => "nothing",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Ending::End => "END",
            Ending::TheEnd => "THE END",
            Ending::None => "none",
        }
    }
}

impl Choice for SpacedHyphen {
    const ALL: &'static [Self] = &[SpacedHyphen::Em, SpacedHyphen::En];
    fn label(self) -> &'static str {
        match self {
            SpacedHyphen::Em => "em dash, closed up (US)",
            SpacedHyphen::En => "en dash, spaced (UK)",
        }
    }
    fn key(self) -> &'static str {
        match self {
            SpacedHyphen::Em => "em",
            SpacedHyphen::En => "en",
        }
    }
}

// ── reading ──────────────────────────────────────────────────────────

fn string(v: &toml::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .unwrap_or_default()
}

/// A list of lines, or one string with its lines in it.
fn lines(v: &toml::Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(toml::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .map(str::to_string)
            .collect(),
        Some(toml::Value::String(s)) => s.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn choice<C: Choice + Default>(v: &toml::Value, key: &str) -> C {
    v.get(key)
        .and_then(|x| match x {
            toml::Value::String(s) => C::from_key(s),
            toml::Value::Float(f) => C::from_key(&f.to_string()),
            toml::Value::Integer(i) => C::from_key(&i.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

impl Contact {
    pub fn from_value(v: &toml::Value) -> Contact {
        Contact {
            legal_name: string(v, "legal_name"),
            address: lines(v, "address"),
            phone: string(v, "phone"),
            email: string(v, "email"),
            agent: lines(v, "agent"),
        }
    }
}

impl Manuscript {
    pub fn from_value(v: &toml::Value) -> Manuscript {
        Manuscript {
            format: choice(v, "format"),
            paper: choice(v, "paper"),
            spacing: choice(v, "spacing"),
            chapter_heading: choice(v, "chapter_heading"),
            first_paragraph: choice(v, "first_paragraph"),
            ending: choice(v, "ending"),
            header_keyword: string(v, "header_keyword"),
            spaced_hyphen: choice(v, "spaced_hyphen"),
        }
    }
}

/// For `ProjectMeta`: read the table however it's written, never failing.
pub fn lenient_contact<'de, D: Deserializer<'de>>(d: D) -> Result<Contact, D::Error> {
    Ok(toml::Value::deserialize(d)
        .map(|v| Contact::from_value(&v))
        .unwrap_or_default())
}

pub fn lenient_manuscript<'de, D: Deserializer<'de>>(d: D) -> Result<Manuscript, D::Error> {
    Ok(toml::Value::deserialize(d)
        .map(|v| Manuscript::from_value(&v))
        .unwrap_or_default())
}

// ── writing ──────────────────────────────────────────────────────────

fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn list(items: &[String]) -> String {
    let kept: Vec<String> = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(quoted)
        .collect();
    format!("[{}]", kept.join(", "))
}

fn contact_table(c: &Contact) -> String {
    let mut s = String::from("[contact]\n");
    s.push_str(&format!("legal_name = {}\n", quoted(c.legal_name.trim())));
    s.push_str(&format!("address = {}\n", list(&c.address)));
    s.push_str(&format!("phone = {}\n", quoted(c.phone.trim())));
    s.push_str(&format!("email = {}\n", quoted(c.email.trim())));
    s.push_str(&format!("agent = {}\n", list(&c.agent)));
    s
}

fn manuscript_table(m: &Manuscript) -> String {
    let mut s = String::from("[manuscript]\n");
    s.push_str(&format!("format = {}\n", quoted(m.format.key())));
    s.push_str(&format!("paper = {}\n", quoted(m.paper.key())));
    s.push_str(&format!("spacing = {}\n", quoted(m.spacing.key())));
    s.push_str(&format!(
        "chapter_heading = {}\n",
        quoted(m.chapter_heading.key())
    ));
    s.push_str(&format!(
        "first_paragraph = {}\n",
        quoted(m.first_paragraph.key())
    ));
    s.push_str(&format!("ending = {}\n", quoted(m.ending.key())));
    s.push_str(&format!(
        "header_keyword = {}\n",
        quoted(m.header_keyword.trim())
    ));
    s.push_str(&format!(
        "spaced_hyphen = {}\n",
        quoted(m.spaced_hyphen.key())
    ));
    s
}

/// Replace `[name]` (its header and every line up to the next table) with
/// `table`, or add it at the end. Other lines stay exactly as they were.
fn put_table(old: &str, name: &str, table: &str) -> String {
    let header = format!("[{name}]");
    let is_header = |l: &str| {
        let t = l.trim();
        t.starts_with('[') && !t.starts_with("[[")
    };
    let mut out = String::new();
    let mut lines = old.lines().peekable();
    let mut placed = false;
    while let Some(l) = lines.next() {
        if l.trim() == header {
            if !placed {
                out.push_str(table);
                placed = true;
            }
            // Skip the old table's body.
            while let Some(next) = lines.peek() {
                if is_header(next) {
                    break;
                }
                lines.next();
            }
            if lines.peek().is_some() {
                out.push('\n');
            }
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    if !placed {
        while out.ends_with("\n\n") {
            out.pop();
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(table);
    }
    out
}

/// Replace one top-level `key = …` line (before the first table), or add it
/// there.
fn put_key(old: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut out = String::new();
    let mut done = false;
    let mut in_table = false;
    for l in old.lines() {
        let t = l.trim_start();
        if t.starts_with('[') {
            if !done {
                out.push_str(&line);
                out.push_str("\n\n");
                done = true;
            }
            in_table = true;
        }
        if !in_table && t.starts_with(key) && t[key.len()..].trim_start().starts_with('=') {
            if !done {
                out.push_str(&line);
                out.push('\n');
                done = true;
            }
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    if !done {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Save the byline, the contact block and the manuscript look to novel.toml.
/// Refuses when novel.toml exists but can't be read, or doesn't read as TOML:
/// rewriting settings it couldn't see would lose them.
pub fn save(root: &Path, byline: &str, contact: &Contact, look: &Manuscript) -> Result<()> {
    let path = root.join("novel.toml");
    let old = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "novel.toml can't be read right now, so it wasn't changed ({})",
                    path.display()
                )
            });
        }
    };
    if !old.trim().is_empty() && toml::from_str::<toml::Table>(&old).is_err() {
        bail!("novel.toml doesn't read as settings right now, so it wasn't changed");
    }
    let text = put_key(&old, "author", &quoted(byline.trim()));
    let text = put_table(&text, "contact", &contact_table(contact));
    let text = put_table(&text, "manuscript", &manuscript_table(look));
    // Never write a file that wouldn't read back.
    toml::from_str::<toml::Table>(&text).context("the new novel.toml didn't read back")?;
    crate::atomic::write_text(&path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(s: &str) -> toml::Value {
        toml::from_str::<toml::Table>(s)
            .map(toml::Value::Table)
            .unwrap()
    }

    #[test]
    fn a_missing_or_strange_value_is_the_default_never_an_error() {
        let m = Manuscript::from_value(&value("format = \"typewriter\"\npaper = 3\n"));
        assert_eq!(m, Manuscript::default());
        let m = Manuscript::from_value(&value(
            "format = \"classic\"\npaper = \"A4\"\nspacing = 1.5\nchapter_heading = \"CHAPTER ONE\"\nending = \"the end\"\n",
        ));
        assert_eq!(m.format, Format::Classic);
        assert_eq!(m.paper, Paper::A4);
        assert_eq!(m.spacing, Spacing::OneAndHalf);
        assert_eq!(m.chapter_heading, ChapterHeading::Caps);
        assert_eq!(m.ending, Ending::TheEnd);
    }

    #[test]
    fn contact_lines_may_be_a_list_or_one_string() {
        let c = Contact::from_value(&value(
            "legal_name = \"Jane Smith\"\naddress = \"12 Road\\nTown\"\nagent = [\"Sam\"]\n",
        ));
        assert_eq!(c.address, ["12 Road", "Town"]);
        assert_eq!(c.agent, ["Sam"]);
        assert!(!c.is_empty());
        assert!(Contact::default().is_empty());
    }

    #[test]
    fn saving_keeps_every_other_line_and_reads_back() {
        let d =
            std::env::temp_dir().join(format!("grimoire-submission-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let before = "title = \"The Archive\"\nauthor = \"J. King\"\ndraft = \"2\"\n\n\
                      # What this book calls its largest division\npart_label = \"Act\"\n\n\
                      [manuscript]\nformat = \"modern\"\nold_key = 1\n\n[other]\nkeep = true\n";
        std::fs::write(d.join("novel.toml"), before).unwrap();
        let contact = Contact {
            legal_name: "Josh \"J\" King".into(),
            address: vec!["1 Road".into(), "Town".into()],
            email: "j@example.com".into(),
            ..Contact::default()
        };
        let look = Manuscript {
            paper: Paper::A4,
            ..Manuscript::default()
        };
        save(&d, "Joshua King", &contact, &look).unwrap();
        let after = std::fs::read_to_string(d.join("novel.toml")).unwrap();
        assert!(after.contains("# What this book calls its largest division"));
        assert!(after.contains("part_label = \"Act\""));
        assert!(after.contains("[other]\nkeep = true"), "{after}");
        assert!(
            !after.contains("old_key"),
            "the old table is replaced whole"
        );
        assert_eq!(after.matches("[manuscript]").count(), 1);
        let meta: toml::Table = toml::from_str(&after).unwrap();
        assert_eq!(meta["author"].as_str(), Some("Joshua King"));
        assert_eq!(meta["title"].as_str(), Some("The Archive"));
        let c = Contact::from_value(&meta["contact"]);
        assert_eq!(c, contact);
        assert_eq!(Manuscript::from_value(&meta["manuscript"]).paper, Paper::A4);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn an_unreadable_novel_toml_is_never_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let d =
            std::env::temp_dir().join(format!("grimoire-submission-locked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("novel.toml");
        std::fs::write(&toml_path, "title = \"Kept\"\n").unwrap();
        std::fs::set_permissions(&toml_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&toml_path).is_ok() {
            std::fs::set_permissions(&toml_path, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::remove_dir_all(&d).unwrap();
            return; // running as root
        }
        assert!(save(&d, "X", &Contact::default(), &Manuscript::default()).is_err());
        std::fs::set_permissions(&toml_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&toml_path).unwrap(),
            "title = \"Kept\"\n"
        );
        // Half-synced garbage is refused the same way.
        std::fs::write(&toml_path, "title = \"Kept\n[[[").unwrap();
        assert!(save(&d, "X", &Contact::default(), &Manuscript::default()).is_err());
        assert_eq!(
            std::fs::read_to_string(&toml_path).unwrap(),
            "title = \"Kept\n[[["
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn choices_step_round() {
        assert_eq!(Paper::Letter.step(true), Paper::A4);
        assert_eq!(Paper::A4.step(true), Paper::Letter);
        assert_eq!(ChapterHeading::Words.step(false), ChapterHeading::Title);
    }
}
