//! The help: one Markdown article per topic, compiled into the binary from
//! `assets/help/`, and a small reader for the Markdown they use — `#`
//! headings, paragraphs, `- ` bullets, two-column `|` tables, and `**bold**`,
//! `` `code` `` and `*italic*` inside them. Drawing lives with the overlay
//! (`app::overlays::help`); this is only the words and their shape.

/// One article. `id` is how the rest of the app asks for it ("the topic for
/// the outline"); `name` is its row in the topic list.
pub struct Topic {
    pub id: &'static str,
    pub name: &'static str,
    pub body: &'static str,
}

macro_rules! topic {
    ($id:literal, $name:literal, $file:literal) => {
        Topic {
            id: $id,
            name: $name,
            body: include_str!(concat!("../assets/help/", $file)),
        }
    };
}

/// Every topic, in the order the list shows them.
pub const TOPICS: &[Topic] = &[
    topic!(
        "getting-started",
        "Getting started",
        "01-getting-started.md"
    ),
    topic!("outline", "The outline", "02-the-outline.md"),
    topic!("writing", "Writing", "03-writing.md"),
    topic!("focus", "Focus mode & typewriter", "04-focus-mode.md"),
    topic!("notes", "Notes & TKs", "05-notes-and-tks.md"),
    topic!("spelling", "Spelling", "06-spelling.md"),
    topic!("find", "Find & replace", "07-find-and-replace.md"),
    topic!("notebook", "The notebook & names", "08-the-notebook.md"),
    topic!("beside", "A scene beside", "09-a-scene-beside.md"),
    topic!("corkboard", "The corkboard", "10-the-corkboard.md"),
    topic!(
        "history",
        "History & recovery",
        "11-history-and-recovery.md"
    ),
    topic!("revision", "Revision", "12-revision.md"),
    topic!("sprints", "Sprints & targets", "13-sprints-and-targets.md"),
    topic!("pomodoro", "The Pomodoro", "14-the-pomodoro.md"),
    topic!("visualizer", "The Visualizer", "15-the-visualizer.md"),
    topic!("themes", "Themes", "16-themes.md"),
    topic!("music", "Music", "17-music.md"),
    topic!("compile", "Compile: the manuscript", "18-compile.md"),
    topic!("paperback", "Paperback & EPUB", "19-paperback-and-epub.md"),
    topic!("sync", "Sync & conflicts", "20-sync-and-conflicts.md"),
    topic!("sessions", "Writing sessions", "21-writing-sessions.md"),
    topic!("on-disk", "On disk", "22-on-disk.md"),
    topic!("settings", "Settings", "23-settings.md"),
    topic!("keys", "Every key", "24-every-key.md"),
    topic!("license", "License", "25-license.md"),
];

/// Where a topic is in [`TOPICS`]; the first one for an id that isn't there.
pub fn index(id: &str) -> usize {
    TOPICS.iter().position(|t| t.id == id).unwrap_or(0)
}

/// The topics a search finds, in order: those whose name matches first, then
/// those that only mention it. Every word of the query must be there, in any
/// order, ignoring case. An empty query finds everything.
pub fn search(query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return (0..TOPICS.len()).collect();
    }
    let has_all = |text: &str| {
        let text = text.to_lowercase();
        words.iter().all(|w| text.contains(w.as_str()))
    };
    let mut by_name = Vec::new();
    let mut by_body = Vec::new();
    for (i, t) in TOPICS.iter().enumerate() {
        if has_all(t.name) {
            by_name.push(i);
        } else if has_all(&plain(t.body)) {
            by_body.push(i);
        }
    }
    by_name.extend(by_body);
    by_name
}

/// The words of an article without its Markdown marks, for searching.
fn plain(body: &str) -> String {
    body.chars()
        .filter(|c| !matches!(c, '`' | '*' | '|' | '#'))
        .collect()
}

/// How a run of words is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Plain,
    Bold,
    Italic,
    /// A key, a command or a file name.
    Code,
}

/// A run of words in one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub mark: Mark,
}

/// One block of an article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `#` is 1 (the article's title), `##` is 2.
    Heading(u8, Vec<Run>),
    Paragraph(Vec<Run>),
    Bullet(Vec<Run>),
    /// A two-column table row; `head` for the row above a `|---|` line.
    Row {
        key: Vec<Run>,
        text: Vec<Run>,
        head: bool,
    },
}

/// Read an article into blocks. Consecutive lines make one paragraph; a blank
/// line ends it.
pub fn parse(body: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Paragraph(runs(&para.join(" "))));
            para.clear();
        }
    };
    let lines: Vec<&str> = body.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(h) = line.strip_prefix("## ") {
            flush(&mut para, &mut out);
            out.push(Block::Heading(2, runs(h)));
        } else if let Some(h) = line.strip_prefix("# ") {
            flush(&mut para, &mut out);
            out.push(Block::Heading(1, runs(h)));
        } else if let Some(b) = line.strip_prefix("- ") {
            flush(&mut para, &mut out);
            out.push(Block::Bullet(runs(b)));
        } else if line.starts_with('|') {
            flush(&mut para, &mut out);
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            if cells
                .iter()
                .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
            {
                continue; // the |---|---| under a table's head
            }
            let head = lines
                .get(i + 1)
                .is_some_and(|next| next.trim().starts_with("|-") || next.trim().starts_with("|:"));
            out.push(Block::Row {
                key: runs(cells.first().copied().unwrap_or("")),
                text: runs(&cells[1.min(cells.len())..].join(" · ")),
                head,
            });
        } else {
            para.push(line);
        }
    }
    flush(&mut para, &mut out);
    out
}

/// Split a line into runs: `**bold**`, `*italic*` and `` `code` ``. A mark
/// with no partner is just a character.
pub fn runs(text: &str) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Run> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let push = |out: &mut Vec<Run>, text: String, mark: Mark| {
        if !text.is_empty() {
            out.push(Run { text, mark });
        }
    };
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        let (open, mark) = if rest.starts_with("**") {
            ("**", Mark::Bold)
        } else if chars[i] == '`' {
            ("`", Mark::Code)
        } else if chars[i] == '*' {
            ("*", Mark::Italic)
        } else {
            plain.push(chars[i]);
            i += 1;
            continue;
        };
        let after = &rest[open.len()..];
        match after.find(open) {
            Some(end) if end > 0 => {
                push(&mut out, std::mem::take(&mut plain), Mark::Plain);
                push(&mut out, after[..end].to_string(), mark);
                i += open.chars().count() + after[..end].chars().count() + open.chars().count();
            }
            _ => {
                plain.push(chars[i]);
                i += 1;
            }
        }
    }
    push(&mut out, plain, Mark::Plain);
    out
}

/// Every key a key table documents, with the `##` heading it's under: each
/// `code` run in a row, in either column.
#[cfg(test)]
pub fn table_keys(body: &str) -> Vec<(String, String)> {
    let text = |runs: &[Run]| runs.iter().map(|r| r.text.as_str()).collect::<String>();
    let mut section = String::new();
    let mut out = Vec::new();
    for b in parse(body) {
        match b {
            Block::Heading(2, runs) => section = text(&runs),
            Block::Row {
                key,
                text: does,
                head: false,
            } => out.extend(
                key.into_iter()
                    .chain(does)
                    .filter(|r| r.mark == Mark::Code)
                    .map(|r| (section.clone(), r.text)),
            ),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_a_title_and_a_unique_id() {
        for (i, t) in TOPICS.iter().enumerate() {
            let title = t.body.lines().next().unwrap_or("");
            assert_eq!(
                title,
                format!("# {}", t.name),
                "{}'s title is its name",
                t.id
            );
            assert!(TOPICS[..i].iter().all(|o| o.id != t.id), "{} twice", t.id);
        }
        assert!(TOPICS.len() >= 20);
    }

    #[test]
    fn marks_become_runs_and_lonely_marks_stay_characters() {
        let r = runs("press `Esc` for **the menu**, *always* — 5 * 3");
        let marks: Vec<(String, Mark)> = r.into_iter().map(|r| (r.text, r.mark)).collect();
        assert_eq!(
            marks,
            vec![
                ("press ".into(), Mark::Plain),
                ("Esc".into(), Mark::Code),
                (" for ".into(), Mark::Plain),
                ("the menu".into(), Mark::Bold),
                (", ".into(), Mark::Plain),
                ("always".into(), Mark::Italic),
                (" — 5 * 3".into(), Mark::Plain),
            ]
        );
    }

    #[test]
    fn an_article_reads_as_blocks() {
        let b = parse(
            "# Title\n\nOne line\nand the next.\n\n- a bullet\n\n| Key | Does |\n|---|---|\n| `n` | new |\n",
        );
        assert!(matches!(&b[0], Block::Heading(1, _)));
        assert_eq!(b[1], Block::Paragraph(runs("One line and the next.")));
        assert!(matches!(&b[2], Block::Bullet(_)));
        assert!(matches!(&b[3], Block::Row { head: true, .. }));
        assert!(matches!(&b[4], Block::Row { head: false, .. }));
        assert_eq!(b.len(), 5);
    }

    #[test]
    fn search_finds_names_first_then_mentions() {
        let found = search("typewriter");
        assert_eq!(TOPICS[found[0]].id, "focus", "the topic named for it first");
        assert!(found.len() > 1, "and the ones that mention it");
        assert_eq!(search("").len(), TOPICS.len());
        assert!(search("zzqqxx").is_empty());
    }
}
