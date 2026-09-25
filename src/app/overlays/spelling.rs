//! Spelling suggestions for one word, and near-miss spellings of names.

use super::*;

/// The rows of the spelling popup after its suggestions.
const IGNORES: [(Ignore, &str); 3] = [
    (Ignore::Now, "Ignore for now"),
    (Ignore::Book, "Always ignore — this book"),
    (Ignore::Everywhere, "Always ignore — every book"),
];

/// Where the popup sits: under the word (over it when there's no room
/// below), clamped to the screen. Shared by drawing and by clicks.
struct Popup {
    frame: Rect,
    inner: Rect,
    /// How many suggestions it lists.
    n: usize,
}

impl Popup {
    /// The row a click lands on: a suggestion, or one of the ignore rows
    /// (numbered after the suggestions, as the selection is).
    fn row_at(&self, y: u16) -> Option<usize> {
        let rel = y.checked_sub(self.inner.y)? as usize;
        let gap = self.n.max(1) + 1;
        if rel < self.n {
            Some(rel)
        } else if (gap..gap + IGNORES.len()).contains(&rel) {
            Some(self.n + rel - gap)
        } else {
            None
        }
    }
}

fn popup(app: &App, area: Rect) -> Option<Popup> {
    let Overlay::Spelling {
        line,
        start,
        suggestions,
        ..
    } = &app.overlay
    else {
        return None;
    };
    let rows = app.editor.layout(app.edit_width);
    let vis = rows
        .iter()
        .position(|r| r.line == *line && *start >= r.start && *start < r.end.max(r.start + 1))
        .unwrap_or(0);
    let word_x =
        app.rect_editor.x + rows.get(vis).map_or(0, |r| start.saturating_sub(r.start)) as u16;
    let word_y = app.rect_editor.y + vis.saturating_sub(app.editor.scroll) as u16;
    let n = suggestions.len();
    // Suggestions (or "no suggestions"), a rule, the three ignores, a hint.
    let h = (n.max(1) + 1 + IGNORES.len() + 1) as u16 + 2;
    let w = 34u16.min(area.width);
    let below = word_y + 1;
    let y = if below + h <= area.bottom().saturating_sub(1) {
        below
    } else {
        word_y.saturating_sub(h)
    };
    let x = word_x
        .saturating_sub(1)
        .min(area.right().saturating_sub(w))
        .max(area.x);
    let frame = Rect::new(x, y, w, h.min(area.height));
    let inner = pane_block("", true, &app.theme).inner(frame);
    Some(Popup { frame, inner, n })
}

impl App {
    pub(super) fn on_spelling_key(&mut self, key: Key) {
        let Overlay::Spelling {
            suggestions,
            sel,
            inline,
            ..
        } = &mut self.overlay
        else {
            return;
        };
        let (n, inline) = (suggestions.len(), *inline);
        let rows = n + IGNORES.len();
        // Clicked open, letters are for writing: only the arrows move here.
        let nav = if inline {
            Nav {
                letters: false,
                wrap: true,
                pages: false,
            }
        } else {
            LIST
        };
        if list_nav(key, sel, rows, nav) {
            return;
        }
        match key {
            Key::Char(c @ '1'..='9') if (c as usize - '1' as usize) < n => {
                self.act_spelling(c as usize - '1' as usize)
            }
            Key::Char('a') if !inline => self.act_spelling(n + 1),
            Key::Enter => {
                let i = *sel;
                self.act_spelling(i);
            }
            Key::F(8) => {
                // Leave this one and go on to the next.
                if let Overlay::Spelling { line, end, .. } = self.overlay {
                    self.overlay = Overlay::None;
                    self.editor.place(line, end);
                    self.spelling();
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            // Anything else was meant for the prose: the popup steps aside.
            other if inline => {
                self.overlay = Overlay::None;
                self.on_editor_key(other);
            }
            _ => {}
        }
    }

    /// Do what row `i` of the popup says: take a suggestion, or ignore the
    /// word for now, in this book, or in every book.
    fn act_spelling(&mut self, i: usize) {
        let Overlay::Spelling {
            line,
            start,
            end,
            word,
            suggestions,
            ..
        } = std::mem::replace(&mut self.overlay, Overlay::None)
        else {
            return;
        };
        match suggestions.get(i) {
            Some(with) => self.apply_spelling(line, start, end, &word, with),
            None => {
                if let Some(&(scope, _)) = IGNORES.get(i - suggestions.len()) {
                    self.ignore_word(&word, scope);
                }
            }
        }
    }

    /// A click while the popup is up. True if the popup took it; a click
    /// anywhere else closes it and carries on as a normal click.
    pub(crate) fn click_spelling(&mut self, x: u16, y: u16) -> bool {
        let area = Rect::new(0, 0, self.screen.0, self.screen.1);
        let Some(p) = popup(self, area) else {
            return false;
        };
        if !hit(p.frame, x, y) {
            self.overlay = Overlay::None;
            return false;
        }
        if let Some(i) = p.row_at(y) {
            self.act_spelling(i);
        }
        true
    }
}

impl App {
    pub(super) fn on_names_key(&mut self, key: Key) {
        let Overlay::Names { drifts, sel } = &mut self.overlay else {
            return;
        };
        if list_nav(key, sel, drifts.len(), LIST) {
            return;
        }
        match key {
            Key::Enter => {
                if let Some(h) = drifts.get(*sel).and_then(|d| d.hits.first()).cloned() {
                    self.go_to_hit(&h.path, h.line, h.start, h.end);
                }
            }
            Key::Char('f') | Key::Char('F') => {
                if let Some(d) = drifts.get(*sel).cloned() {
                    drifts.remove(*sel);
                    if *sel >= drifts.len() {
                        *sel = drifts.len().saturating_sub(1);
                    }
                    if drifts.is_empty() {
                        self.overlay = Overlay::None;
                    }
                    self.fix_name(&d.variant, &d.name.name);
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

pub(super) fn draw_spelling(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Spelling {
        word,
        suggestions,
        sel,
        inline,
        ..
    } = &app.overlay
    else {
        return;
    };
    let Some(p) = popup(app, area) else {
        return;
    };
    f.render_widget(Clear, p.frame);
    let title = format!("“{}”", truncate(word, 24));
    f.render_widget(pane_block(&title, true, t), p.frame);
    let dim = Style::default().fg(t.dim);
    let label_w = p.inner.width.saturating_sub(6) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let row = |i: usize, label: &str, key: String, lines: &mut Vec<Line>| {
        let on = i == *sel;
        let l = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("{:<label_w$}", truncate(label, label_w)),
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::styled(key, Style::default().fg(t.sun)),
        ]);
        lines.push(if on {
            l.style(Style::default().bg(t.sel))
        } else {
            l
        });
    };
    if suggestions.is_empty() {
        lines.push(Line::from(Span::styled("   no suggestions", dim)));
    }
    for (i, s) in suggestions.iter().enumerate() {
        row(i, s, format!("{}", i + 1), &mut lines);
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", "─".repeat(p.inner.width.saturating_sub(2) as usize)),
        Style::default().fg(t.border),
    )));
    for (k, (scope, label)) in IGNORES.iter().enumerate() {
        let key = if !inline && *scope == Ignore::Book {
            "a"
        } else {
            ""
        };
        row(suggestions.len() + k, label, key.into(), &mut lines);
    }
    lines.push(hint_line(
        if *inline {
            " ↵ choose  esc close  or keep typing"
        } else {
            " ↵ choose  esc close  F8 next"
        },
        t,
    ));
    f.render_widget(Paragraph::new(lines), p.inner);
}

pub(super) fn draw_names(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Names { drifts, sel } = &app.overlay else {
        return;
    };
    let h = (drifts.len() as u16 * 2).min(20) + 6;
    let box_area = centred(area, area.width.saturating_sub(4).min(90), h);
    f.render_widget(Clear, box_area);
    let block = pane_block("NAMES THAT DRIFTED", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let mut lines = vec![
        Line::from(Span::styled(
            " Spellings one or two letters away from a name in your notebook:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];
    for (i, d) in drifts.iter().enumerate().take(10) {
        let on = i == *sel;
        let mut scenes: Vec<&str> = Vec::new();
        for h in &d.hits {
            let title = h.place.rsplit(" › ").next().unwrap_or(&h.place);
            if !scenes.contains(&title) {
                scenes.push(title);
            }
        }
        let row = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(d.variant.clone(), Style::default().fg(t.warn)),
            Span::styled(format!(" ×{}", d.hits.len()), dim),
            Span::styled(" — the ", dim),
            Span::styled(d.name.section.clone(), dim),
            Span::styled(" note says ", dim),
            Span::styled(d.name.name.clone(), Style::default().fg(t.accent)),
        ]);
        lines.push(if on {
            row.style(Style::default().bg(t.sel))
        } else {
            row
        });
        lines.push(Line::from(Span::styled(
            format!(
                "     {}",
                truncate(&scenes.join(" · "), inner.width.saturating_sub(6) as usize)
            ),
            dim,
        )));
    }
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(
        " ↑↓ choose   ↵ go to the first one   f fix them all   esc close",
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
