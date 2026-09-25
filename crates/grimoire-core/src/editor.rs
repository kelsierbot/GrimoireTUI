//! A small word-wrapping text buffer.
//!
//! Prose is stored as logical lines (a paragraph is one long line). The editor
//! lays those out into *visual* rows for a given width, and cursor movement is
//! visual — pressing Down inside a wrapped paragraph moves one screen row, not
//! one paragraph, which is the only behaviour that feels right for prose.

use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

/// Typing pauses longer than this start a new undo step.
const PAUSE: Duration = Duration::from_millis(1200);
/// Undo steps kept per scene. Each is a copy of the scene, and scenes are small.
const DEPTH: usize = 300;

/// What an edit was, for deciding where one undo step ends and the next begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// A letter, digit or punctuation mark.
    Letter,
    /// A space typed after a word: it belongs to that word's step.
    Space,
    Delete,
    /// A new paragraph, a paste, a replace, a restore: always its own step.
    Whole,
}

#[derive(Debug, Clone)]
struct Snap {
    lines: Vec<String>,
    cy: usize,
    cx: usize,
}

/// A scene's undo and redo, kept when you switch to another scene so coming
/// back still undoes.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    last: Option<(Edit, Instant)>,
}

/// One visual row: a char range within a logical line.
#[derive(Debug, Clone, Copy)]
pub struct VisRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

pub struct Editor {
    pub lines: Vec<String>,
    /// Logical line index.
    pub cy: usize,
    /// Char index within the logical line.
    pub cx: usize,
    /// Visual row scroll offset.
    pub scroll: usize,
    /// Remembered column for vertical movement.
    goal: Option<usize>,
    /// Where a drag started, as (line, char). Selection runs from here to the
    /// cursor, in whichever order they happen to be.
    anchor: Option<(usize, usize)>,
    hist: History,
}

impl Editor {
    pub fn from_text(s: &str) -> Self {
        let mut lines: Vec<String> = s.split('\n').map(|l| l.to_string()).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            lines,
            cy: 0,
            cx: 0,
            scroll: 0,
            goal: None,
            anchor: None,
            hist: History::default(),
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    // ---- undo ------------------------------------------------------------

    pub fn take_history(&mut self) -> History {
        std::mem::take(&mut self.hist)
    }

    pub fn set_history(&mut self, h: History) {
        self.hist = h;
    }

    fn snap(&self) -> Snap {
        Snap {
            lines: self.lines.clone(),
            cy: self.cy,
            cx: self.cx,
        }
    }

    fn restore(&mut self, s: Snap) {
        self.lines = s.lines;
        self.cy = s.cy.min(self.lines.len().saturating_sub(1));
        self.cx = s.cx.min(self.line_len(self.cy));
        self.goal = None;
        self.anchor = None;
    }

    /// Record the state before an edit, starting a new undo step when this
    /// edit doesn't belong with the last one. A word and the space after it
    /// are one step; a pause, a change from typing to deleting, a new
    /// paragraph or a paste each start another.
    fn remember(&mut self, kind: Edit) {
        let now = Instant::now();
        let fresh = match self.hist.last {
            None => true,
            Some((last, at)) => {
                now.duration_since(at) > PAUSE
                    || kind == Edit::Whole
                    || last == Edit::Whole
                    || (kind == Edit::Delete) != (last == Edit::Delete)
                    || (last == Edit::Space && kind == Edit::Letter)
            }
        };
        if fresh {
            let s = self.snap();
            self.hist.undo.push(s);
            if self.hist.undo.len() > DEPTH {
                self.hist.undo.remove(0);
            }
        }
        self.hist.redo.clear();
        self.hist.last = Some((kind, now));
    }

    /// Step back. False when there's nothing left to undo.
    pub fn undo(&mut self) -> bool {
        let Some(s) = self.hist.undo.pop() else {
            return false;
        };
        let cur = self.snap();
        self.hist.redo.push(cur);
        self.restore(s);
        self.hist.last = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.hist.redo.pop() else {
            return false;
        };
        let cur = self.snap();
        self.hist.undo.push(cur);
        self.restore(s);
        self.hist.last = None;
        true
    }

    /// Replace the whole text as one undoable step — restoring a version,
    /// replacing across the scene. The cursor stays near where it was.
    pub fn set_text(&mut self, text: &str) {
        self.remember(Edit::Whole);
        self.lines = text.split('\n').map(|l| l.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cy = self.cy.min(self.lines.len() - 1);
        self.cx = self.cx.min(self.line_len(self.cy));
        self.goal = None;
        self.anchor = None;
        self.hist.last = Some((Edit::Whole, Instant::now()));
    }

    /// Put the cursor at (line, char), clamped, with nothing selected.
    pub fn place(&mut self, line: usize, ch: usize) {
        self.cy = line.min(self.lines.len().saturating_sub(1));
        self.cx = ch.min(self.line_len(self.cy));
        self.goal = None;
        self.anchor = None;
    }

    /// Select from (line, char) to (line, char), cursor at the end.
    pub fn select(&mut self, from: (usize, usize), to: (usize, usize)) {
        self.place(to.0, to.1);
        let line = from.0.min(self.lines.len().saturating_sub(1));
        self.anchor = Some((line, from.1.min(self.line_len(line))));
    }

    /// Delete whatever is selected, as the start of an undo step the next
    /// keystroke joins — so typing over a selection undoes in one go.
    /// False if nothing was selected.
    pub fn delete_selection(&mut self) -> bool {
        let Some(((l0, c0), (l1, c1))) = self.selection() else {
            return false;
        };
        self.remember(Edit::Whole);
        let head: String = self.lines[l0].chars().take(c0).collect();
        let tail: String = self.lines[l1].chars().skip(c1).collect();
        self.lines.splice(l0..=l1, [format!("{head}{tail}")]);
        self.cy = l0;
        self.cx = c0;
        self.anchor = None;
        self.goal = None;
        // The deletion and whatever is typed next are one step.
        self.hist.last = Some((Edit::Letter, Instant::now()));
        true
    }

    /// Replace chars `start..end` of paragraph `line` with `with` — a
    /// spelling fix — as one undo step. The cursor lands after the new word.
    pub fn replace_in_line(&mut self, line: usize, start: usize, end: usize, with: &str) {
        let Some(chars) = self
            .lines
            .get(line)
            .map(|l| l.chars().collect::<Vec<char>>())
        else {
            return;
        };
        let (start, end) = (start.min(chars.len()), end.min(chars.len()));
        if start > end {
            return;
        }
        self.remember(Edit::Whole);
        let head: String = chars[..start].iter().collect();
        let tail: String = chars[end..].iter().collect();
        self.lines[line] = format!("{head}{with}{tail}");
        self.cy = line;
        self.cx = start + with.chars().count();
        self.goal = None;
        self.anchor = None;
        self.hist.last = Some((Edit::Whole, Instant::now()));
    }

    /// Insert text that may span paragraphs, as one undo step.
    pub fn insert_str(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() {
            return;
        }
        self.remember(Edit::Whole);
        let chars = self.line_chars(self.cy);
        let at = self.cx.min(chars.len());
        let head: String = chars[..at].iter().collect();
        let tail: String = chars[at..].iter().collect();
        let mut parts: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        let last_len = parts.last().map(|p| p.chars().count()).unwrap_or(0);
        let n = parts.len();
        parts[0] = format!("{head}{}", parts[0]);
        let end_line = self.cy + n - 1;
        let end_col = if n == 1 { at + last_len } else { last_len };
        parts[n - 1].push_str(&tail);
        self.lines.splice(self.cy..=self.cy, parts);
        self.cy = end_line;
        self.cx = end_col;
        self.goal = None;
        self.hist.last = Some((Edit::Whole, Instant::now()));
    }

    fn line_chars(&self, y: usize) -> Vec<char> {
        self.lines[y].chars().collect()
    }

    fn line_len(&self, y: usize) -> usize {
        self.lines[y].chars().count()
    }

    // ---- layout -----------------------------------------------------------

    /// Wrap every logical line to `width`, producing the visual row list.
    pub fn layout(&self, width: usize) -> Vec<VisRow> {
        let width = width.max(1);
        let mut rows = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                rows.push(VisRow {
                    line: i,
                    start: 0,
                    end: 0,
                });
                continue;
            }
            let mut start = 0usize;
            while start < chars.len() {
                let mut w = 0usize;
                let mut last_space: Option<usize> = None;
                let mut end = start;
                while end < chars.len() {
                    let cw = char_width(chars[end]);
                    if w + cw > width {
                        break;
                    }
                    w += cw;
                    if chars[end] == ' ' {
                        last_space = Some(end);
                    }
                    end += 1;
                }
                if end >= chars.len() {
                    rows.push(VisRow {
                        line: i,
                        start,
                        end: chars.len(),
                    });
                    break;
                }
                // Prefer breaking after the last space that fits.
                let brk = match last_space {
                    Some(sp) if sp + 1 > start => sp + 1,
                    _ => end.max(start + 1),
                };
                rows.push(VisRow {
                    line: i,
                    start,
                    end: brk,
                });
                start = brk;
            }
        }
        if rows.is_empty() {
            rows.push(VisRow {
                line: 0,
                start: 0,
                end: 0,
            });
        }
        rows
    }

    /// Which visual row the cursor sits on, and its display column.
    pub fn cursor_vis(&self, rows: &[VisRow]) -> (usize, usize) {
        let mut best = 0usize;
        for (i, r) in rows.iter().enumerate() {
            if r.line != self.cy {
                continue;
            }
            best = i;
            if self.cx >= r.start && self.cx < r.end {
                break;
            }
            if self.cx == r.end {
                // End of a wrapped segment: stay on this row unless another
                // row for the same line starts here.
                let continues = rows
                    .get(i + 1)
                    .map(|n| n.line == self.cy && n.start == self.cx)
                    .unwrap_or(false);
                if !continues {
                    break;
                }
            }
        }
        let r = rows[best];
        let seg: String = self.lines[r.line]
            .chars()
            .skip(r.start)
            .take(self.cx.saturating_sub(r.start))
            .collect();
        (best, seg.width())
    }

    // ---- editing ----------------------------------------------------------

    pub fn insert(&mut self, ch: char) {
        self.remember(if ch.is_whitespace() {
            Edit::Space
        } else {
            Edit::Letter
        });
        let mut chars = self.line_chars(self.cy);
        let at = self.cx.min(chars.len());
        chars.insert(at, ch);
        self.lines[self.cy] = chars.into_iter().collect();
        self.cx = at + 1;
        self.goal = None;
    }

    pub fn newline(&mut self) {
        self.remember(Edit::Whole);
        let chars = self.line_chars(self.cy);
        let at = self.cx.min(chars.len());
        let rest: String = chars[at..].iter().collect();
        let keep: String = chars[..at].iter().collect();
        self.lines[self.cy] = keep;
        self.lines.insert(self.cy + 1, rest);
        self.cy += 1;
        self.cx = 0;
        self.goal = None;
    }

    pub fn backspace(&mut self) {
        if self.cx == 0 && self.cy == 0 {
            return;
        }
        self.remember(Edit::Delete);
        if self.cx > 0 {
            let mut chars = self.line_chars(self.cy);
            chars.remove(self.cx - 1);
            self.lines[self.cy] = chars.into_iter().collect();
            self.cx -= 1;
        } else if self.cy > 0 {
            let cur = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.line_len(self.cy);
            self.lines[self.cy].push_str(&cur);
        }
        self.goal = None;
    }

    pub fn delete(&mut self) {
        let len = self.line_len(self.cy);
        if self.cx >= len && self.cy + 1 >= self.lines.len() {
            return;
        }
        self.remember(Edit::Delete);
        if self.cx < len {
            let mut chars = self.line_chars(self.cy);
            chars.remove(self.cx);
            self.lines[self.cy] = chars.into_iter().collect();
        } else if self.cy + 1 < self.lines.len() {
            let next = self.lines.remove(self.cy + 1);
            self.lines[self.cy].push_str(&next);
        }
        self.goal = None;
    }

    // ---- movement ---------------------------------------------------------

    pub fn left(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.line_len(self.cy);
        }
        self.goal = None;
    }

    pub fn right(&mut self) {
        if self.cx < self.line_len(self.cy) {
            self.cx += 1;
        } else if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = 0;
        }
        self.goal = None;
    }

    pub fn home(&mut self, rows: &[VisRow]) {
        let (r, _) = self.cursor_vis(rows);
        self.cx = rows[r].start;
        self.goal = None;
    }

    pub fn end(&mut self, rows: &[VisRow]) {
        let (r, _) = self.cursor_vis(rows);
        self.cx = rows[r].end;
        self.goal = None;
    }

    pub fn up(&mut self, rows: &[VisRow]) {
        self.vmove(rows, -1);
    }

    pub fn down(&mut self, rows: &[VisRow]) {
        self.vmove(rows, 1);
    }

    fn vmove(&mut self, rows: &[VisRow], delta: isize) {
        let (r, col) = self.cursor_vis(rows);
        let goal = self.goal.unwrap_or(col);
        let target = r as isize + delta;
        if target < 0 || target as usize >= rows.len() {
            return;
        }
        let t = rows[target as usize];
        self.cy = t.line;
        self.cx = char_at_width(&self.lines[t.line], t.start, t.end, goal);
        self.goal = Some(goal);
    }

    // ---- selection ---------------------------------------------------

    pub fn begin_select(&mut self) {
        self.anchor = Some((self.cy, self.cx));
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// Ordered ((line, char), (line, char)), or None if nothing is selected.
    pub fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let a = self.anchor?;
        let b = (self.cy, self.cx);
        if a == b {
            return None;
        }
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// The selected slice of a single visual row, for painting a background.
    pub fn row_selection(&self, r: VisRow) -> Option<(usize, usize)> {
        let ((l0, c0), (l1, c1)) = self.selection()?;
        if r.line < l0 || r.line > l1 {
            return None;
        }
        let from = if r.line == l0 {
            c0.max(r.start)
        } else {
            r.start
        };
        let to = if r.line == l1 { c1.min(r.end) } else { r.end };
        (from < to).then_some((from, to))
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((l0, c0), (l1, c1)) = self.selection()?;
        let take = |line: usize, from: usize, to: usize| -> String {
            self.lines[line]
                .chars()
                .skip(from)
                .take(to.saturating_sub(from))
                .collect()
        };
        if l0 == l1 {
            return Some(take(l0, c0, c1));
        }
        let mut out = take(l0, c0, self.line_len(l0));
        for l in l0 + 1..l1 {
            out.push('\n');
            out.push_str(&self.lines[l]);
        }
        out.push('\n');
        out.push_str(&take(l1, 0, c1));
        Some(out)
    }

    /// Put the cursor where the pointer landed.
    pub fn click(&mut self, rows: &[VisRow], vis_row: usize, col: usize) {
        self.move_to(rows, vis_row, col);
        self.begin_select();
    }

    /// Same as click but leaves the anchor alone, extending the selection.
    pub fn drag(&mut self, rows: &[VisRow], vis_row: usize, col: usize) {
        if self.anchor.is_none() {
            self.begin_select();
        }
        self.move_to(rows, vis_row, col);
    }

    fn move_to(&mut self, rows: &[VisRow], vis_row: usize, col: usize) {
        if rows.is_empty() {
            return;
        }
        let r = rows[vis_row.min(rows.len() - 1)];
        self.cy = r.line;
        self.cx = char_at_width(&self.lines[r.line], r.start, r.end, col);
        self.goal = None;
    }

    /// Keep the cursor row inside the viewport.
    pub fn clamp_scroll(&mut self, rows: &[VisRow], height: usize) {
        let (r, _) = self.cursor_vis(rows);
        let height = height.max(1);
        if r < self.scroll {
            self.scroll = r;
        } else if r >= self.scroll + height {
            self.scroll = r + 1 - height;
        }
        let max = rows.len().saturating_sub(1);
        if self.scroll > max {
            self.scroll = max;
        }
    }

    /// Scroll so at least `below` rows show under the cursor, so a writer
    /// isn't drafting on the pane's last row. Only ever scrolls down; the
    /// room shrinks on a pane too short to spare it.
    pub fn keep_room_below(&mut self, rows: &[VisRow], height: usize, below: usize) {
        self.clamp_scroll(rows, height);
        let height = height.max(1);
        let below = below.min(height.saturating_sub(1) / 2);
        let (r, _) = self.cursor_vis(rows);
        if r + below >= self.scroll + height {
            self.scroll = r + below + 1 - height;
        }
    }

    /// Typewriter scrolling: put the cursor's row in the middle of the pane
    /// (or as near as the top of the scene allows).
    pub fn centre_on_cursor(&mut self, rows: &[VisRow], height: usize) {
        let (r, _) = self.cursor_vis(rows);
        self.scroll = r.saturating_sub(height.max(1) / 2);
    }
}

fn char_width(c: char) -> usize {
    let mut b = [0u8; 4];
    c.encode_utf8(&mut b).width().max(1)
}

/// Within char range [start,end) of `line`, find the char index nearest to
/// display column `goal`.
fn char_at_width(line: &str, start: usize, end: usize, goal: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let mut w = 0usize;
    let mut i = start;
    while i < end.min(chars.len()) {
        let cw = char_width(chars[i]);
        if w + cw > goal {
            break;
        }
        w += cw;
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_replaced_in_place_comes_back_with_one_undo() {
        let mut e = Editor::from_text("Teh cat sat.");
        e.replace_in_line(0, 0, 3, "The");
        assert_eq!(e.text(), "The cat sat.");
        assert_eq!((e.cy, e.cx), (0, 3));
        assert!(e.undo());
        assert_eq!(e.text(), "Teh cat sat.", "one step back is the whole fix");
        assert!(e.redo());
        assert_eq!(e.text(), "The cat sat.");
    }

    fn ed(s: &str) -> Editor {
        Editor::from_text(s)
    }

    fn numbered(n: usize) -> Editor {
        ed(&(0..n)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[test]
    fn room_is_kept_below_the_cursor() {
        let mut e = numbered(40);
        let rows = e.layout(80);
        e.cy = 9;
        // Plain clamping leaves the cursor on the last of 10 rows...
        e.clamp_scroll(&rows, 10);
        assert_eq!(e.scroll, 0);
        // ...keeping room scrolls so three rows show beneath it.
        e.keep_room_below(&rows, 10, 3);
        assert_eq!(e.scroll, 3);
        assert_eq!(e.scroll + 10 - 1 - e.cy, 3);
        // Moving up inside the view doesn't scroll back.
        e.cy = 6;
        e.keep_room_below(&rows, 10, 3);
        assert_eq!(e.scroll, 3);
    }

    #[test]
    fn room_below_shrinks_on_a_short_pane() {
        let mut e = numbered(40);
        let rows = e.layout(80);
        e.cy = 3;
        e.keep_room_below(&rows, 4, 3);
        // A four-row pane can spare one row, not three.
        assert_eq!(e.scroll, 1);
    }

    #[test]
    fn typewriter_keeps_the_cursor_mid_pane() {
        let mut e = numbered(40);
        let rows = e.layout(80);
        e.cy = 25;
        e.centre_on_cursor(&rows, 11);
        assert_eq!(e.cy - e.scroll, 5);
        // Near the top there's nothing to scroll past.
        e.cy = 2;
        e.centre_on_cursor(&rows, 11);
        assert_eq!(e.scroll, 0);
    }

    #[test]
    fn no_selection_until_the_cursor_moves_off_the_anchor() {
        let mut e = ed("hello world");
        e.cx = 3;
        e.begin_select();
        assert!(!e.has_selection(), "anchor == cursor is not a selection");
        e.cx = 7;
        assert!(e.has_selection());
    }

    #[test]
    fn selection_is_ordered_regardless_of_drag_direction() {
        let mut e = ed("hello world");
        e.cx = 8;
        e.begin_select();
        e.cx = 2; // dragged backwards
        assert_eq!(e.selection(), Some(((0, 2), (0, 8))));
        assert_eq!(e.selected_text().as_deref(), Some("llo wo"));
    }

    #[test]
    fn selection_spans_lines() {
        let mut e = ed("one\ntwo\nthree");
        e.cy = 0;
        e.cx = 1;
        e.begin_select();
        e.cy = 2;
        e.cx = 3;
        assert_eq!(e.selected_text().as_deref(), Some("ne\ntwo\nthr"));
    }

    #[test]
    fn row_selection_clips_to_the_visual_row() {
        let mut e = ed("aaaabbbbcccc");
        e.cx = 2;
        e.begin_select();
        e.cx = 10;
        // A wrapped row covering chars 4..8 is entirely inside the selection.
        let mid = VisRow {
            line: 0,
            start: 4,
            end: 8,
        };
        assert_eq!(e.row_selection(mid), Some((4, 8)));
        // The first row is only selected from char 2.
        let first = VisRow {
            line: 0,
            start: 0,
            end: 4,
        };
        assert_eq!(e.row_selection(first), Some((2, 4)));
        // A row past the selection end is untouched.
        let last = VisRow {
            line: 0,
            start: 10,
            end: 12,
        };
        assert_eq!(e.row_selection(last), None);
    }

    #[test]
    fn rows_outside_the_selected_lines_are_untouched() {
        let mut e = ed("one\ntwo\nthree");
        e.cy = 1;
        e.cx = 0;
        e.begin_select();
        e.cy = 1;
        e.cx = 3;
        assert_eq!(
            e.row_selection(VisRow {
                line: 0,
                start: 0,
                end: 3
            }),
            None
        );
        assert_eq!(
            e.row_selection(VisRow {
                line: 2,
                start: 0,
                end: 5
            }),
            None
        );
        assert_eq!(
            e.row_selection(VisRow {
                line: 1,
                start: 0,
                end: 3
            }),
            Some((0, 3))
        );
    }

    fn typed(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.insert(c);
        }
    }

    #[test]
    fn undo_takes_back_a_word_at_a_time() {
        let mut e = ed("");
        typed(&mut e, "Wren ran ");
        typed(&mut e, "home");
        assert_eq!(e.text(), "Wren ran home");
        assert!(e.undo());
        assert_eq!(e.text(), "Wren ran ", "the last word goes first");
        assert!(e.undo());
        assert_eq!(e.text(), "Wren ", "a word and its space are one step");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(!e.undo(), "nothing left");
        assert!(e.redo());
        assert!(e.redo());
        assert_eq!(e.text(), "Wren ran ");
    }

    #[test]
    fn deleting_is_its_own_step_and_a_new_edit_clears_redo() {
        let mut e = ed("");
        typed(&mut e, "hello");
        e.backspace();
        e.backspace();
        assert_eq!(e.text(), "hel");
        assert!(e.undo());
        assert_eq!(e.text(), "hello", "both backspaces undo together");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(e.redo());
        typed(&mut e, "!");
        assert!(!e.redo(), "typing after an undo drops the redo");
    }

    #[test]
    fn typing_over_a_selection_replaces_it_and_undoes_in_one_step() {
        let mut e = ed("the grey lot");
        e.select((0, 4), (0, 8));
        assert_eq!(e.selected_text().as_deref(), Some("grey"));
        assert!(e.delete_selection());
        typed(&mut e, "empty");
        assert_eq!(e.text(), "the empty lot");
        assert!(e.undo());
        assert_eq!(e.text(), "the grey lot");
    }

    #[test]
    fn a_multi_paragraph_paste_lands_as_one_step_with_the_cursor_after_it() {
        let mut e = ed("ab");
        e.cx = 1;
        e.insert_str("1\n2\n3");
        assert_eq!(e.text(), "a1\n2\n3b");
        assert_eq!((e.cy, e.cx), (2, 1));
        assert!(e.undo());
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn replacing_the_whole_text_undoes() {
        let mut e = ed("old words");
        e.set_text("new words entirely");
        assert!(e.undo());
        assert_eq!(e.text(), "old words");
    }

    #[test]
    fn a_selection_across_paragraphs_deletes_cleanly() {
        let mut e = ed("one\ntwo\nthree");
        e.select((0, 1), (2, 2));
        assert!(e.delete_selection());
        assert_eq!(e.text(), "oree");
        assert_eq!((e.cy, e.cx), (0, 1));
    }

    #[test]
    fn clearing_drops_the_selection() {
        let mut e = ed("hello");
        e.begin_select();
        e.cx = 4;
        assert!(e.has_selection());
        e.clear_selection();
        assert!(!e.has_selection());
        assert_eq!(e.selected_text(), None);
    }
}
