//! Spelling suggestions for one word, and near-miss spellings of names.

use super::*;

impl App {
    pub(super) fn on_spelling_key(&mut self, key: Key) {
        let Overlay::Spelling {
            line,
            start,
            end,
            word,
            suggestions,
            sel,
        } = &mut self.overlay
        else {
            return;
        };
        // Rows: each suggestion, then "add to this book", then "leave it".
        if list_nav(key, sel, suggestions.len() + 2, LIST) {
            return;
        }
        match key {
            Key::Char(c @ '1'..='9') if (c as usize - '1' as usize) < suggestions.len() => {
                let (l, s, e, with) = (
                    *line,
                    *start,
                    *end,
                    suggestions[c as usize - '1' as usize].clone(),
                );
                self.overlay = Overlay::None;
                self.apply_spelling(l, s, e, &with);
            }
            Key::Char('a') => {
                let w = word.clone();
                self.overlay = Overlay::None;
                self.add_to_book(&w);
            }
            Key::Enter => {
                let (l, s, e, i) = (*line, *start, *end, *sel);
                let (word, pick) = (word.clone(), suggestions.get(i).cloned());
                let n = suggestions.len();
                self.overlay = Overlay::None;
                match pick {
                    Some(with) => self.apply_spelling(l, s, e, &with),
                    None if i == n => self.add_to_book(&word),
                    None => {}
                }
            }
            Key::F(8) => {
                // Leave this one and go on to the next.
                let (l, e) = (*line, *end);
                self.overlay = Overlay::None;
                self.editor.place(l, e);
                self.spelling();
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

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
        line,
        end,
        ..
    } = &app.overlay
    else {
        return;
    };
    // Sits just under the word, clamped to the screen.
    let rows = app.editor.layout(app.edit_width);
    let vis = rows
        .iter()
        .position(|r| r.line == *line && *end >= r.start && *end <= r.end)
        .unwrap_or(0);
    let y = app.rect_editor.y + (vis.saturating_sub(app.editor.scroll)) as u16 + 1;
    let h = suggestions.len() as u16 + 6;
    let w = 44u16.min(area.width);
    let y = y.min(area.height.saturating_sub(h));
    let x = app.rect_editor.x.min(area.width.saturating_sub(w));
    let box_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, box_area);
    let title = format!("“{}”", truncate(word, 30));
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let mut lines: Vec<Line> = Vec::new();
    if suggestions.is_empty() {
        lines.push(Line::from(Span::styled(" no suggestions", dim)));
    }
    let row = |i: usize, label: String, key: String, lines: &mut Vec<Line>| {
        let on = i == *sel;
        let l = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("{label:<30}"),
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
    for (i, s) in suggestions.iter().enumerate() {
        row(i, truncate(s, 30), format!("{}", i + 1), &mut lines);
    }
    lines.push(Line::from(""));
    row(
        suggestions.len(),
        "add to this book's dictionary".into(),
        "a".into(),
        &mut lines,
    );
    row(
        suggestions.len() + 1,
        "leave it".into(),
        "esc".into(),
        &mut lines,
    );
    lines.push(hint_line(" ↵ choose   F8 skip to the next", t));
    f.render_widget(Paragraph::new(lines), inner);
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
