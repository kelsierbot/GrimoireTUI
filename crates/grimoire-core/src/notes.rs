//! Notes to yourself that never reach the book.
//!
//! Two kinds, both plain text so the file stays readable anywhere:
//!
//! - `%% … %%` — Obsidian's comment syntax, so a vault shows them as comments
//!   too. One may span lines. A `%%` with no partner closes at the end of its
//!   own line rather than swallowing the rest of the scene (Obsidian would hide
//!   the rest of the file), so one stray `%%` can't drop half a chapter out of
//!   an export.
//! - `TK` — the publishing mark for "to come": a gap to fill later. Standalone
//!   and in capitals (`TKTK` too), so "TKO" or "Atka" are left alone.
//!
//! Neither is counted as words, and neither goes into a compile or export.
//! On disk they are kept exactly as typed.

use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    /// `%% … %%`, fences included.
    Note,
    /// `TK`.
    Tk,
}

/// Byte ranges of every note in `text`, fences included. A note may cross
/// lines; one left open ends with its line.
pub fn note_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(a) = text[from..].find("%%").map(|i| from + i) {
        match text[a + 2..].find("%%").map(|i| a + 2 + i) {
            Some(b) => {
                out.push((a, b + 2));
                from = b + 2;
            }
            None => {
                let end = text[a..].find('\n').map_or(text.len(), |i| a + i);
                out.push((a, end));
                from = end;
            }
        }
        if from >= text.len() {
            break;
        }
    }
    out
}

/// Is this whitespace-separated token a TK, allowing the punctuation around
/// it (`TK.`, `(TK)`, `TKTK,`)?
pub fn is_tk_token(token: &str) -> bool {
    let core = token.trim_matches(|c: char| !c.is_alphanumeric());
    tk_letters(core)
}

fn tk_letters(s: &str) -> bool {
    !s.is_empty() && s.len().is_multiple_of(2) && s.as_bytes().chunks(2).all(|p| p == b"TK")
}

/// Byte ranges of the TKs in `text` that sit outside notes.
pub fn tk_ranges(text: &str) -> Vec<(usize, usize)> {
    if !text.contains("TK") {
        return Vec::new();
    }
    let notes = note_ranges(text);
    let in_note = |i: usize| notes.iter().any(|&(a, b)| i >= a && i < b);
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(k) = text[i..].find("TK").map(|k| i + k) {
        // Extend over TKTK…
        let mut end = k + 2;
        while text[end..].starts_with("TK") {
            end += 2;
        }
        let before = text[..k].chars().next_back();
        let after = text[end..].chars().next();
        let standalone = before.is_none_or(|c| !c.is_alphanumeric())
            && after.is_none_or(|c| !c.is_alphanumeric());
        if standalone && !in_note(k) {
            out.push((k, end));
        }
        i = end;
        if i >= bytes.len() {
            break;
        }
    }
    out
}

/// Every note and TK in `text`, in order, as byte ranges.
pub fn ranges(text: &str) -> Vec<(usize, usize, MarkKind)> {
    let mut out: Vec<(usize, usize, MarkKind)> = note_ranges(text)
        .into_iter()
        .map(|(a, b)| (a, b, MarkKind::Note))
        .chain(
            tk_ranges(text)
                .into_iter()
                .map(|(a, b)| (a, b, MarkKind::Tk)),
        )
        .collect();
    out.sort_by_key(|r| r.0);
    out
}

/// Could `text` hold a note or a TK at all? The cheap test that keeps counting
/// ordinary prose as fast as it always was.
fn may_have_marks(text: &str) -> bool {
    text.contains("%%") || text.contains("TK")
}

/// Words, not counting notes or TKs.
pub fn count_words(text: &str) -> usize {
    if !may_have_marks(text) {
        return text.split_whitespace().count();
    }
    strip_notes(text)
        .split_whitespace()
        .filter(|t| !is_tk_token(t))
        .count()
}

/// `text` with the notes taken out. A line that held nothing but a note goes
/// with it (and one blank line beside it, so no empty paragraph is left), and
/// the gap a note leaves mid-sentence is closed to one space.
pub fn strip_notes(text: &str) -> Cow<'_, str> {
    let notes = note_ranges(text);
    if notes.is_empty() {
        return Cow::Borrowed(text);
    }
    // Rebuild line by line, knowing for each line whether a note touched it.
    let mut lines: Vec<Option<String>> = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let end = start + line.len();
        let mut out = String::new();
        let mut touched = false;
        let mut pos = start;
        for &(a, b) in &notes {
            if b <= start || a >= end {
                continue;
            }
            touched = true;
            let (a, b) = (a.max(start), b.min(end));
            if a > pos {
                join(&mut out, &text[pos..a]);
            }
            pos = pos.max(b);
        }
        if pos < end {
            join(&mut out, &text[pos..end]);
        }
        if touched {
            let kept = out.trim_end().to_string();
            lines.push((!kept.trim().is_empty()).then_some(kept));
        } else {
            lines.push(Some(line.to_string()));
        }
        start = end + 1;
    }
    Cow::Owned(assemble(lines))
}

/// Append `piece` after a removed note, keeping a single space at the seam.
fn join(out: &mut String, piece: &str) {
    if out.is_empty() {
        out.push_str(piece);
    } else if out.ends_with(' ') && piece.starts_with(' ') {
        out.push_str(piece.trim_start_matches(' '));
    } else {
        out.push_str(piece);
    }
}

/// Lines back into text; `None` is a line that held only a note, dropped with
/// the blank line after it when that would leave two in a row.
fn assemble(lines: Vec<Option<String>>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut dropped = false;
    for l in lines {
        match l {
            None => dropped = true,
            Some(s) => {
                let blank = s.trim().is_empty();
                let last_blank = out.last().is_none_or(|p| p.trim().is_empty());
                if dropped && blank && last_blank {
                    continue;
                }
                if !blank {
                    dropped = false;
                }
                out.push(s);
            }
        }
    }
    out.join("\n")
}

/// The text as it goes into the book: notes and TKs out. A TK takes the space
/// before it along, and leaves any closing punctuation on the word before
/// ("to TK." becomes "to.").
pub fn strip(text: &str) -> Cow<'_, str> {
    if !may_have_marks(text) {
        return Cow::Borrowed(text);
    }
    let noted = strip_notes(text);
    if !noted.contains("TK") {
        return noted;
    }
    let lines: Vec<Option<String>> = noted
        .split('\n')
        .map(|line| {
            if tk_ranges(line).is_empty() {
                return Some(line.to_string());
            }
            let kept = strip_tks(line);
            (!kept.trim().is_empty()).then_some(kept)
        })
        .collect();
    Cow::Owned(assemble(lines))
}

fn strip_tks(line: &str) -> String {
    let lead: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    let mut words: Vec<String> = Vec::new();
    for token in line.split_whitespace() {
        if !is_tk_token(token) {
            words.push(token.to_string());
            continue;
        }
        let tail: String = token
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim_start_matches(['T', 'K'])
            .chars()
            .filter(|c| ".,;:!?…".contains(*c))
            .collect();
        match words.last_mut() {
            Some(prev) if !tail.is_empty() => prev.push_str(&tail),
            _ => {}
        }
    }
    format!("{lead}{}", words.join(" "))
}

/// For the editor: per line, the char ranges (start, end) of notes and TKs,
/// following notes across lines.
pub fn line_spans(lines: &[String]) -> Vec<Vec<(usize, usize, MarkKind)>> {
    let mut out = vec![Vec::new(); lines.len()];
    if !lines.iter().any(|l| may_have_marks(l)) {
        return out;
    }
    let text = lines.join("\n");
    // Byte offset of each line's start.
    let mut starts = Vec::with_capacity(lines.len());
    let mut at = 0;
    for l in lines {
        starts.push(at);
        at += l.len() + 1;
    }
    for (a, b, kind) in ranges(&text) {
        for (i, l) in lines.iter().enumerate() {
            let (ls, le) = (starts[i], starts[i] + l.len());
            if b <= ls || a > le || (a == le && b > le) {
                continue;
            }
            let (from, to) = (a.max(ls) - ls, b.min(le) - ls);
            if to > from || (kind == MarkKind::Note && a <= ls && b > le) {
                out[i].push((char_at(l, from), char_at(l, to), kind));
            }
        }
    }
    out
}

/// Byte offset → char index within one line.
fn char_at(line: &str, byte: usize) -> usize {
    line[..byte.min(line.len())].chars().count()
}

/// One note or TK found in a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    /// Line and char column where it starts, in the scene's prose, and the
    /// column where it ends on that same line.
    pub line: usize,
    pub col: usize,
    pub end: usize,
    /// A one-line glimpse: the note's words, or the prose around a TK.
    pub snippet: String,
}

/// Every note and TK in one scene's prose, in order.
pub fn marks(text: &str) -> Vec<Mark> {
    if !may_have_marks(text) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (a, b, kind) in ranges(text) {
        let line = text[..a].matches('\n').count();
        let line_start = text[..a].rfind('\n').map_or(0, |i| i + 1);
        let col = text[line_start..a].chars().count();
        let first_end = text[a..b].find('\n').map_or(b, |i| a + i);
        let end = col + text[a..first_end].chars().count();
        let snippet = match kind {
            MarkKind::Note => {
                let inner = text[a..b].trim_start_matches("%%").trim_end_matches("%%");
                one_line(inner)
            }
            MarkKind::Tk => {
                let line_end = text[a..].find('\n').map_or(text.len(), |i| a + i);
                one_line(&text[line_start..line_end])
            }
        };
        out.push(Mark {
            kind,
            line,
            col,
            end,
            snippet,
        });
    }
    out
}

fn one_line(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        "(empty note)".into()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_and_tks_are_not_words() {
        assert_eq!(count_words("one two three"), 3);
        assert_eq!(count_words("one %% not these four words %% two"), 2);
        assert_eq!(count_words("She went to TK and back."), 5);
        assert_eq!(count_words("She went to TK."), 3);
        assert_eq!(count_words("TKO in the (TKTK) fourth"), 4);
        assert_eq!(count_words("Atka TKs"), 2);
    }

    #[test]
    fn a_note_can_cross_lines() {
        let t = "First para.\n\n%%\nremember the gravel\nand the dog\n%%\n\nSecond para.";
        assert_eq!(count_words(t), 4);
        assert_eq!(strip(t), "First para.\n\nSecond para.");
    }

    #[test]
    fn a_stray_opening_only_hides_its_own_line() {
        let t = "Kept words %% forgot to close\nNext line counts.";
        assert_eq!(count_words(t), 5);
        assert_eq!(strip(t), "Kept words\nNext line counts.");
    }

    #[test]
    fn stripping_closes_the_gap_and_keeps_punctuation() {
        assert_eq!(strip("She ran %% faster? %% home."), "She ran home.");
        assert_eq!(strip("She drove to TK."), "She drove to.");
        assert_eq!(strip("to TK and back"), "to and back");
        assert_eq!(strip("%% only a note %%\nProse."), "Prose.");
        assert_eq!(strip("  Indented %%x%% line"), "  Indented line");
    }

    #[test]
    fn plain_prose_is_untouched_and_borrowed() {
        let t = "Nothing to see — here.\n\nAt all.";
        assert!(matches!(strip(t), Cow::Borrowed(_)));
        assert_eq!(strip(t), t);
    }

    #[test]
    fn tks_inside_notes_are_part_of_the_note() {
        let t = "A %% TK later %% B TK";
        let r = ranges(t);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].2, MarkKind::Note);
        assert_eq!(r[1].2, MarkKind::Tk);
        assert_eq!(&t[r[1].0..r[1].1], "TK");
    }

    #[test]
    fn editor_spans_follow_a_note_across_lines() {
        let lines: Vec<String> = ["Before %% open", "inside", "close %% after TK"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = line_spans(&lines);
        assert_eq!(s[0], vec![(7, 14, MarkKind::Note)]);
        assert_eq!(s[1], vec![(0, 6, MarkKind::Note)]);
        assert_eq!(s[2], vec![(0, 8, MarkKind::Note), (15, 17, MarkKind::Tk)]);
    }

    #[test]
    fn marks_give_line_column_and_a_glimpse() {
        let t = "One.\nTwo %% check the\ndates %% three TK four.";
        let m = marks(t);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].line, m[0].col, m[0].kind), (1, 4, MarkKind::Note));
        assert_eq!(m[0].end, 16);
        assert_eq!(m[0].snippet, "check the dates");
        assert_eq!((m[1].line, m[1].kind), (2, MarkKind::Tk));
        assert_eq!(m[1].snippet, "dates %% three TK four.");
    }
}
