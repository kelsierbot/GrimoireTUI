//! A small word-wrapping text buffer.
//!
//! Prose is stored as logical lines (a paragraph is one long line). The editor
//! lays those out into *visual* rows for a given width, and cursor movement is
//! visual — pressing Down inside a wrapped paragraph moves one screen row, not
//! one paragraph, which is the only behaviour that feels right for prose.

use unicode_width::UnicodeWidthStr;

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
}

impl Editor {
    pub fn from_str(s: &str) -> Self {
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
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
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

    pub fn row_text(&self, r: VisRow) -> String {
        self.lines[r.line]
            .chars()
            .skip(r.start)
            .take(r.end - r.start)
            .collect()
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
        let mut chars = self.line_chars(self.cy);
        let at = self.cx.min(chars.len());
        chars.insert(at, ch);
        self.lines[self.cy] = chars.into_iter().collect();
        self.cx = at + 1;
        self.goal = None;
    }

    pub fn newline(&mut self) {
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
        let from = if r.line == l0 { c0.max(r.start) } else { r.start };
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

    fn ed(s: &str) -> Editor {
        Editor::from_str(s)
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
        let mid = VisRow { line: 0, start: 4, end: 8 };
        assert_eq!(e.row_selection(mid), Some((4, 8)));
        // The first row is only selected from char 2.
        let first = VisRow { line: 0, start: 0, end: 4 };
        assert_eq!(e.row_selection(first), Some((2, 4)));
        // A row past the selection end is untouched.
        let last = VisRow { line: 0, start: 10, end: 12 };
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
        assert_eq!(e.row_selection(VisRow { line: 0, start: 0, end: 3 }), None);
        assert_eq!(e.row_selection(VisRow { line: 2, start: 0, end: 5 }), None);
        assert_eq!(e.row_selection(VisRow { line: 1, start: 0, end: 3 }), Some((0, 3)));
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
