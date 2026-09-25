//! Export for readers: formats, which parts, how much, the manuscript's look
//! and the author's details, then what was written.

use super::*;
use crate::app::{SAMPLE_KINDS, sample_default};
use grimoire_core::submission::Choice;

/// Rows: five formats, each act, then "How much", "Look", "Author" and the
/// Export button.
const FORMATS: usize = 5;
/// The Paperback's row among the formats: ←/→ there changes its trim.
const PAPERBACK: usize = 2;

impl App {
    pub(super) fn on_export_key(&mut self, key: Key) {
        let Overlay::Export {
            formats,
            trim,
            parts,
            sample,
            sel,
            tks,
            done,
        } = &mut self.overlay
        else {
            return;
        };
        if done.is_some() {
            self.overlay = Overlay::None;
            return;
        }
        let how_much = FORMATS + parts.len();
        let (look_row, author_row, button) = (how_much + 1, how_much + 2, how_much + 3);
        if list_nav(key, sel, button + 1, LIST) {
            return;
        }
        // Anything changed after a TK warning asks again.
        let reset = |tks: &mut Option<Vec<String>>| *tks = None;
        match key {
            // The paperback's trim size, on its row; choosing one ticks it.
            Key::Right | Key::Char('l') if *sel == PAPERBACK => {
                *trim = trim.next();
                formats[PAPERBACK] = true;
                reset(tks);
            }
            Key::Left | Key::Char('h') if *sel == PAPERBACK => {
                *trim = trim.prev();
                formats[PAPERBACK] = true;
                reset(tks);
            }
            Key::Char(' ') | Key::Enter if *sel < FORMATS => {
                formats[*sel] = !formats[*sel];
                reset(tks);
            }
            Key::Char(' ') | Key::Enter if *sel < how_much => {
                let p = &mut parts[*sel - FORMATS];
                p.2 = !p.2;
                reset(tks);
            }
            Key::Left | Key::Right if *sel == how_much => {
                let n = SAMPLE_KINDS.len();
                sample.0 = if key == Key::Right {
                    (sample.0 + 1) % n
                } else {
                    (sample.0 + n - 1) % n
                };
                sample.1 = sample_default(sample.0);
                reset(tks);
            }
            Key::Char(c) if *sel == how_much && (c.is_ascii_digit() || c == '-' || c == ',') => {
                if sample.0 == 0 {
                    sample.0 = if c == '-' { 2 } else { 1 };
                    sample.1.clear();
                }
                if sample.1 == sample_default(sample.0) {
                    sample.1.clear();
                }
                sample.1.push(c);
                reset(tks);
            }
            Key::Backspace if *sel == how_much => {
                sample.1.pop();
                reset(tks);
            }
            Key::Enter | Key::Char(' ') if *sel == look_row => {
                reset(tks);
                let back = Box::new(self.overlay.clone());
                self.open_look(Some(back));
            }
            Key::Enter | Key::Char(' ') if *sel == author_row => {
                reset(tks);
                let back = Box::new(self.overlay.clone());
                self.open_author(Some(back));
            }
            Key::Enter | Key::Char('x') => self.export_pressed(),
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
        sample,
        sel,
        tks,
        done,
    } = &app.overlay
    else {
        return;
    };
    let warn_rows = tks.as_ref().map_or(0, |v| v.len().min(3) + 2) as u16;
    let h = (FORMATS as u16 + parts.len() as u16 + 15 + warn_rows).min(area.height);
    let box_area = centred(area, area.width.saturating_sub(4).clamp(40, 80), h);
    f.render_widget(Clear, box_area);
    let block = pane_block("EXPORT FOR READERS", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let bold = Style::default().fg(t.text).add_modifier(Modifier::BOLD);
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
    let cursor_line = |i: usize, spans: Vec<Span<'static>>| -> Line<'static> {
        let l = Line::from(spans);
        if i == *sel {
            l.style(Style::default().bg(t.sel))
        } else {
            l
        }
    };
    let marker = |i: usize| {
        Span::styled(
            if i == *sel { " ▸ " } else { "   " },
            Style::default().fg(t.accent),
        )
    };
    let tick_row = |i: usize, on: bool, label: String, note: String| {
        cursor_line(
            i,
            vec![
                marker(i),
                Span::styled(
                    if on { "[x] " } else { "[ ] " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    format!("{label:<16}"),
                    Style::default().fg(if i == *sel { t.accent } else { t.text }),
                ),
                Span::styled(truncate(&note, note_room), dim),
            ],
        )
    };

    lines.push(Line::from(Span::styled(" Formats", bold)));
    let pdf_note = if app.has_office() {
        "the same pages, made by LibreOffice"
    } else {
        "needs LibreOffice (free) — libreoffice.org"
    };
    let paperback_note = if app.has_office() {
        format!("◂ {} ▸  print-ready DOCX + PDF", trim.label())
    } else {
        format!(
            "◂ {} ▸  print-ready DOCX (PDF needs LibreOffice)",
            trim.label()
        )
    };
    let notes = [
        (
            "Word document",
            "manuscript format, for agents and editors".to_string(),
        ),
        ("PDF", pdf_note.to_string()),
        ("Paperback", paperback_note),
        ("EPUB", "for phones and e-readers".to_string()),
        ("Markdown", "the plain compiled text".to_string()),
    ];
    for (i, (label, note)) in notes.into_iter().enumerate() {
        lines.push(tick_row(i, formats[i], label.to_string(), note));
    }
    if !parts.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Include", bold)));
        for (i, (_, title, on)) in parts.iter().enumerate() {
            lines.push(tick_row(FORMATS + i, *on, title.clone(), String::new()));
        }
    }
    let how_much = FORMATS + parts.len();
    let what = match sample.0 {
        0 => "the whole book".to_string(),
        1 => format!("the first {} chapters", sample.1),
        2 => format!("chapters {}", sample.1),
        _ => format!("the first {} words", sample.1),
    };
    lines.push(cursor_line(
        how_much,
        vec![
            marker(how_much),
            Span::styled(format!("{:<20}", "How much"), Style::default().fg(t.text)),
            Span::styled(
                format!("◂ {what} ▸"),
                Style::default().fg(if *sel == how_much { t.accent } else { t.text }),
            ),
        ],
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" Manuscript", bold)));
    let m = &app.project.meta;
    let row2 = |i: usize, label: &str, value: String, note_style: Style| {
        cursor_line(
            i,
            vec![
                marker(i),
                Span::styled(
                    format!("{label:<20}"),
                    Style::default().fg(if i == *sel { t.accent } else { t.text }),
                ),
                Span::styled(truncate(&value, note_room.saturating_sub(4)), note_style),
            ],
        )
    };
    lines.push(row2(how_much + 1, "Look…", m.manuscript.summary(), dim));
    let who = if m.contact.is_empty() {
        (
            "not set — the title page wants your name and email".to_string(),
            Style::default().fg(t.warn),
        )
    } else {
        let name = if m.contact.legal_name.trim().is_empty() {
            m.author.clone()
        } else {
            m.contact.legal_name.clone()
        };
        let reach = if m.contact.email.trim().is_empty() {
            m.contact.phone.clone()
        } else {
            m.contact.email.clone()
        };
        (format!("{name} · {reach}"), dim)
    };
    lines.push(row2(how_much + 2, "Author details…", who.0, who.1));
    lines.push(Line::from(""));
    let button = how_much + 3;
    let on = *sel == button;
    lines.push(Line::from(vec![
        marker(button),
        Span::styled(
            " Export to exports/ ",
            Style::default()
                .fg(if on { t.sel } else { t.accent })
                .bg(if on { t.accent } else { t.sel }),
        ),
    ]));
    if let Some(found) = tks {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                " {} TK{} still in the text — x again exports anyway, ^T finds them",
                found.len(),
                if found.len() == 1 { " is" } else { "s are" }
            ),
            Style::default().fg(t.warn),
        )));
        for l in found.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!(
                    "   {}",
                    truncate(l, (inner.width as usize).saturating_sub(4))
                ),
                dim,
            )));
        }
    }
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    let hint = if *sel == how_much {
        " ←→ whole book / first chapters / chapters / words   type the number   x export"
    } else if *sel == PAPERBACK {
        " ←→ trim size   space/↵ tick   x export   esc close"
    } else {
        " ↑↓ choose   space/↵ tick or open   x export   esc close"
    };
    lines.push(hint_line(hint, t));
    let lines = fit_rows(lines, inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

/// More rows than the box holds — a short terminal, a book of many parts, a
/// TK warning: the spacer lines go first, then the list scrolls to keep the
/// cursor (the `▸` row) in view. The hint row at the bottom always stays.
fn fit_rows(mut lines: Vec<Line<'static>>, room: usize) -> Vec<Line<'static>> {
    if lines.len() <= room || room < 3 {
        return lines;
    }
    let hint = lines.pop().expect("the hint row");
    while lines.len() >= room {
        match lines.iter().rposition(|l| l.width() == 0) {
            Some(i) => {
                lines.remove(i);
            }
            None => break,
        }
    }
    let body = room - 1;
    if lines.len() > body {
        let cursor = lines
            .iter()
            .position(|l| l.spans.first().is_some_and(|s| s.content == " ▸ "))
            .unwrap_or(0);
        let start = (cursor + 1).saturating_sub(body).min(lines.len() - body);
        lines = lines.split_off(start);
        lines.truncate(body);
    }
    lines.push(hint);
    lines
}

// ── the manuscript's look ──────────────────────────────────────────────

/// The look dialog's rows, as (label, current value).
fn look_rows(look: &grimoire_core::submission::Manuscript) -> Vec<(&'static str, String)> {
    vec![
        ("Format", look.format.label().to_string()),
        ("Paper", look.paper.label().to_string()),
        ("Spacing", look.spacing.label().to_string()),
        ("Chapter headings", look.chapter_heading.label().to_string()),
        ("First paragraph", look.first_paragraph.label().to_string()),
        ("At the end", look.ending.label().to_string()),
        ("Spaced hyphen", look.spaced_hyphen.label().to_string()),
        (
            "Header keyword",
            if look.header_keyword.trim().is_empty() {
                "(the title's first word)".to_string()
            } else {
                look.header_keyword.clone()
            },
        ),
    ]
}

pub(super) const KEYWORD_ROW: usize = 7;

impl App {
    pub(super) fn on_look_key(&mut self, key: Key) {
        let Overlay::Look { sel, look, back } = &mut self.overlay else {
            return;
        };
        let rows = look_rows(look).len() + 1; // + Done
        // Letters are for typing the keyword, so j/k don't move here.
        if list_nav(key, sel, rows, TYPED) {
            return;
        }
        let done = rows - 1;
        match key {
            Key::Left | Key::Right | Key::Char(' ') | Key::Enter if *sel < KEYWORD_ROW => {
                let fwd = key != Key::Left;
                match *sel {
                    0 => look.format = look.format.step(fwd),
                    1 => look.paper = look.paper.step(fwd),
                    2 => look.spacing = look.spacing.step(fwd),
                    3 => look.chapter_heading = look.chapter_heading.step(fwd),
                    4 => look.first_paragraph = look.first_paragraph.step(fwd),
                    5 => look.ending = look.ending.step(fwd),
                    _ => look.spaced_hyphen = look.spaced_hyphen.step(fwd),
                }
            }
            Key::Char(c) if *sel == KEYWORD_ROW && !c.is_control() => {
                if look.header_keyword.chars().count() < 40 {
                    look.header_keyword.push(c);
                }
            }
            Key::Backspace if *sel == KEYWORD_ROW => {
                look.header_keyword.pop();
            }
            Key::Enter if *sel == KEYWORD_ROW => *sel += 1,
            Key::Enter | Key::Esc if *sel == done || key == Key::Esc => {
                let (look, back) = (look.clone(), back.take());
                let m = &self.project.meta;
                let (byline, contact) = (m.author.clone(), m.contact.clone());
                if self.save_submission(byline, contact, look) {
                    self.msg = "manuscript look saved".into();
                    self.overlay = back.map(|b| *b).unwrap_or(Overlay::None);
                } else if let Overlay::Look { back: b, .. } = &mut self.overlay {
                    *b = back;
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_author_key(&mut self, key: Key) {
        let Overlay::Author { sel, fields, back } = &mut self.overlay else {
            return;
        };
        let n = fields.len();
        match key {
            Key::Up => *sel = sel.saturating_sub(1),
            Key::Down | Key::Tab => *sel = (*sel + 1).min(n - 1),
            Key::BackTab => *sel = sel.saturating_sub(1),
            Key::Char(c) if !c.is_control() => {
                if fields[*sel].chars().count() < 120 {
                    fields[*sel].push(c);
                }
            }
            Key::Backspace => {
                fields[*sel].pop();
            }
            Key::Enter if *sel + 1 < n => *sel += 1,
            Key::Enter | Key::Esc => {
                let (f, back) = (fields.clone(), back.take());
                let keep = |v: &[String]| -> Vec<String> {
                    v.iter()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                };
                let contact = grimoire_core::submission::Contact {
                    legal_name: f[1].trim().to_string(),
                    address: keep(&f[2..5]),
                    phone: f[5].trim().to_string(),
                    email: f[6].trim().to_string(),
                    agent: keep(&f[7..9]),
                };
                let look = self.project.meta.manuscript.clone();
                if self.save_submission(f[0].trim().to_string(), contact, look) {
                    self.msg = "author details saved".into();
                    self.overlay = back.map(|b| *b).unwrap_or(Overlay::None);
                } else if let Overlay::Author { back: b, .. } = &mut self.overlay {
                    *b = back;
                }
            }
            _ => {}
        }
    }
}

pub(super) fn draw_look(f: &mut Frame, _app: &App, area: Rect, t: &Theme) {
    let Overlay::Look { sel, look, .. } = &_app.overlay else {
        return;
    };
    let rows = look_rows(look);
    let box_area = centred(area, 72, rows.len() as u16 + 8);
    f.render_widget(Clear, box_area);
    let block = pane_block("MANUSCRIPT LOOK", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        " How the Word and PDF manuscript is set (Shunn's modern format by default)",
        Style::default().fg(t.dim),
    ))];
    lines.push(Line::from(""));
    for (i, (label, value)) in rows.iter().enumerate() {
        let on = i == *sel;
        let value = if i == KEYWORD_ROW && on {
            format!("{}▏", look.header_keyword)
        } else if i < KEYWORD_ROW && on {
            format!("◂ {value} ▸")
        } else {
            value.clone()
        };
        let l = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("{label:<18}"),
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::styled(
                value,
                Style::default().fg(if on { t.accent } else { t.dim }),
            ),
        ]);
        lines.push(if on {
            l.style(Style::default().bg(t.sel))
        } else {
            l
        });
    }
    let done = rows.len();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            if *sel == done { " ▸ " } else { "   " },
            Style::default().fg(t.accent),
        ),
        Span::styled(
            " Done ",
            Style::default()
                .fg(if *sel == done { t.sel } else { t.accent })
                .bg(if *sel == done { t.accent } else { t.sel }),
        ),
    ]));
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(" ↑↓ choose   ←→ change   esc save & back", t));
    f.render_widget(Paragraph::new(lines), inner);
}

const AUTHOR_LABELS: [&str; 9] = [
    "Byline (pen name)",
    "Legal name",
    "Address",
    "",
    "",
    "Phone",
    "Email",
    "Agent",
    "",
];

pub(super) fn draw_author(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Author { sel, fields, .. } = &app.overlay else {
        return;
    };
    let box_area = centred(area, 72, fields.len() as u16 + 7);
    f.render_widget(Clear, box_area);
    let block = pane_block("AUTHOR DETAILS", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        " For the title page: the byline, and where an agent writes back",
        Style::default().fg(t.dim),
    ))];
    lines.push(Line::from(""));
    let room = (inner.width as usize).saturating_sub(24);
    for (i, value) in fields.iter().enumerate() {
        let on = i == *sel;
        let shown = if on {
            format!("{}▏", truncate(value, room.saturating_sub(1)))
        } else {
            truncate(value, room)
        };
        let l = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("{:<19}", AUTHOR_LABELS[i]),
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::styled(
                shown,
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
        ]);
        lines.push(if on {
            l.style(Style::default().bg(t.sel))
        } else {
            l
        });
    }
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(" ↑↓ field   type   ↵ next   esc save & back", t));
    f.render_widget(Paragraph::new(lines), inner);
}
