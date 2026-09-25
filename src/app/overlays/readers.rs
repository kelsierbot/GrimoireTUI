//! Export for readers: formats, which parts, then what was written.

use super::*;

/// The Paperback's row among the formats.
const PAPERBACK: usize = 2;

impl App {
    pub(super) fn on_export_key(&mut self, key: Key) {
        let Overlay::Export {
            formats,
            trim,
            parts,
            sel,
            done,
        } = &mut self.overlay
        else {
            return;
        };
        if done.is_some() {
            self.overlay = Overlay::None;
            return;
        }
        // Rows: four formats, each act, then the export button.
        let n = formats.len();
        if list_nav(key, sel, n + parts.len() + 1, LIST) {
            return;
        }
        match key {
            // The paperback's trim size, on its row.
            Key::Right | Key::Char('l') if *sel == PAPERBACK => {
                *trim = trim.next();
                formats[PAPERBACK] = true;
            }
            Key::Left | Key::Char('h') if *sel == PAPERBACK => {
                *trim = trim.prev();
                formats[PAPERBACK] = true;
            }
            Key::Char(' ') | Key::Enter if *sel < n => formats[*sel] = !formats[*sel],
            Key::Char(' ') | Key::Enter if *sel < n + parts.len() => {
                let p = &mut parts[*sel - n];
                p.2 = !p.2;
            }
            Key::Enter | Key::Char('x') => {
                if !formats.iter().any(|&f| f) {
                    self.msg = "choose at least one format".into();
                } else if !parts.is_empty() && !parts.iter().any(|p| p.2) {
                    self.msg = format!("choose at least one {}", self.project.meta.part_noun());
                } else {
                    let (f, t, p) = (*formats, *trim, parts.clone());
                    let lines = self.run_export(f, t, &p);
                    if let Overlay::Export { done, .. } = &mut self.overlay {
                        *done = Some(lines);
                    }
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

pub(super) fn draw_export(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Export {
        formats,
        trim,
        parts,
        sel,
        done,
    } = &app.overlay
    else {
        return;
    };
    let n = formats.len();
    let h = (n as u16 + parts.len() as u16 + 10).min(area.height);
    let box_area = centred(area, area.width.saturating_sub(4).clamp(40, 76), h);
    f.render_widget(Clear, box_area);
    let block = pane_block("EXPORT FOR READERS", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(result) = done {
        for l in result {
            let style = if l.starts_with('✓') {
                Style::default().fg(t.accent)
            } else if l.starts_with("couldn't") {
                Style::default().fg(t.warn)
            } else {
                Style::default().fg(t.text)
            };
            lines.push(Line::from(Span::styled(format!(" {l}"), style)));
        }
        while lines.len() < (inner.height as usize).saturating_sub(1) {
            lines.push(Line::from(""));
        }
        lines.push(hint_line(" any key closes", t));
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }
    // Cursor, tick box and label take 23 columns; the note gets the rest.
    let note_room = (inner.width as usize).saturating_sub(24);
    let row = |i: usize, on: bool, label: String, note: &str, lines: &mut Vec<Line>| {
        let cursor = i == *sel;
        let l = Line::from(vec![
            Span::styled(
                if cursor { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                if on { "[x] " } else { "[ ] " },
                Style::default().fg(if on { t.accent } else { t.dim }),
            ),
            Span::styled(
                format!("{label:<16}"),
                Style::default().fg(if cursor { t.accent } else { t.text }),
            ),
            Span::styled(truncate(note, note_room), dim),
        ]);
        lines.push(if cursor {
            l.style(Style::default().bg(t.sel))
        } else {
            l
        });
    };
    lines.push(Line::from(Span::styled(
        " Formats",
        Style::default().fg(t.text).add_modifier(Modifier::BOLD),
    )));
    row(
        0,
        formats[0],
        "Word document".into(),
        "manuscript format, for agents and editors",
        &mut lines,
    );
    row(
        1,
        formats[1],
        "EPUB".into(),
        "for phones and e-readers",
        &mut lines,
    );
    let pdf_note = format!("◂ {} ▸  print-ready DOCX + PDF", trim.label());
    row(
        PAPERBACK,
        formats[PAPERBACK],
        "Paperback".into(),
        &pdf_note,
        &mut lines,
    );
    row(
        3,
        formats[3],
        "Markdown".into(),
        "the plain compiled text",
        &mut lines,
    );
    if !parts.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Include",
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )));
        for (i, (_, title, on)) in parts.iter().enumerate() {
            row(n + i, *on, title.clone(), "", &mut lines);
        }
    }
    lines.push(Line::from(""));
    let button = n + parts.len();
    let on = *sel == button;
    let b = Line::from(vec![
        Span::styled(
            if on { " ▸ " } else { "   " },
            Style::default().fg(t.accent),
        ),
        Span::styled(
            " Export to exports/ ",
            Style::default()
                .fg(if on { t.sel } else { t.accent })
                .bg(if on { t.accent } else { t.sel }),
        ),
    ]);
    lines.push(b);
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(
        if *sel == PAPERBACK {
            " ←→ trim size   space/↵ tick   x export   esc close"
        } else {
            " ↑↓ choose   space/↵ tick   x export   esc close"
        },
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
