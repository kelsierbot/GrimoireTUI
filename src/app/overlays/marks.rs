//! The writing aids' boxes: the notes & TK list (Ctrl-T) and starting a sprint.

use super::super::aids::filter_marks;
use super::*;

impl App {
    pub(super) fn on_marks_key(&mut self, key: Key) {
        let Overlay::Marks { query, sel, rows } = &mut self.overlay else {
            return;
        };
        match key {
            Key::Char(c) if !c.is_control() => {
                query.push(c);
                *sel = 0;
            }
            Key::Backspace => {
                query.pop();
                *sel = 0;
            }
            Key::Down => *sel += 1,
            Key::Up => *sel = sel.saturating_sub(1),
            Key::PageDown => *sel += 10,
            Key::PageUp => *sel = sel.saturating_sub(10),
            Key::Enter => {
                let hits = filter_marks(rows, query);
                if let Some(r) = hits.get((*sel).min(hits.len().saturating_sub(1))) {
                    let r = (*r).clone();
                    self.overlay = Overlay::None;
                    self.go_to_mark(&r);
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
        if let Overlay::Marks { query, sel, rows } = &mut self.overlay {
            let n = filter_marks(rows, query).len();
            *sel = (*sel).min(n.saturating_sub(1));
        }
    }

    pub(super) fn on_sprint_key(&mut self, key: Key) {
        let Overlay::Sprint {
            words,
            minutes,
            on_minutes,
            fresh,
        } = &mut self.overlay
        else {
            return;
        };
        let field: &mut String = if *on_minutes {
            &mut *minutes
        } else {
            &mut *words
        };
        match key {
            Key::Char(c) if c.is_ascii_digit() => {
                if *fresh {
                    field.clear();
                    *fresh = false;
                }
                if field.len() < 5 {
                    field.push(c);
                }
            }
            Key::Backspace => {
                *fresh = false;
                field.pop();
            }
            Key::Tab | Key::BackTab | Key::Up | Key::Down => {
                *on_minutes = !*on_minutes;
                *fresh = true;
            }
            Key::Enter => {
                let goal = words.parse::<usize>().unwrap_or(0);
                let mins = minutes.parse::<u64>().unwrap_or(0);
                if goal == 0 || mins == 0 {
                    self.msg = "a sprint needs some words and some minutes".into();
                    return;
                }
                self.overlay = Overlay::None;
                self.start_sprint(goal, Duration::from_secs(mins * 60));
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

/// Ctrl-T: every note and TK in the book, filtered by what's typed.
pub(super) fn draw_marks(
    f: &mut Frame,
    area: Rect,
    query: &str,
    sel: usize,
    rows: &[crate::app::MarkRow],
    t: &Theme,
) {
    use grimoire_core::notes::MarkKind;
    let hits = crate::app::aids_filter(rows, query);
    let room = 14usize;
    let w = area.width.saturating_sub(4).min(96);
    let h = room as u16 + 5;
    let box_area = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + 2.min(area.height.saturating_sub(h)),
        width: w,
        height: h.min(area.height),
    };
    f.render_widget(Clear, box_area);
    let notes = rows.iter().filter(|r| r.kind == MarkKind::Note).count();
    let title = format!(
        "NOTES & TKS · {notes} note{} · {} TK{}",
        if notes == 1 { "" } else { "s" },
        rows.len() - notes,
        if rows.len() - notes == 1 { "" } else { "s" }
    );
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let iw = inner.width as usize;
    let dim = Style::default().fg(t.dim);
    let sel = sel.min(hits.len().saturating_sub(1));
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" › ", Style::default().fg(t.accent)),
            Span::styled(query.to_string(), Style::default().fg(t.text)),
            Span::styled("█", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled("─".repeat(iw), Style::default().fg(t.border))),
    ];
    let start = sel.saturating_sub(room.saturating_sub(1));
    for (i, r) in hits.iter().enumerate().skip(start).take(room) {
        let on = i == sel;
        let (tag, tag_style) = match r.kind {
            MarkKind::Note => ("%% ", dim.add_modifier(Modifier::ITALIC)),
            MarkKind::Tk => (
                "TK ",
                Style::default().fg(t.sun).add_modifier(Modifier::BOLD),
            ),
        };
        let place_w = r.place.chars().count().min(iw / 3);
        let place = truncate(&r.place, place_w);
        let snip_w = iw.saturating_sub(3 + 3 + place.chars().count() + 3);
        let snippet = truncate(&r.snippet, snip_w);
        let pad = iw.saturating_sub(6 + snippet.chars().count() + place.chars().count() + 1);
        let row = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(tag, tag_style),
            Span::styled(
                snippet,
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(place, dim),
            Span::raw(" "),
        ]);
        lines.push(if on {
            row.style(Style::default().bg(t.sel))
        } else {
            row
        });
    }
    if hits.is_empty() {
        lines.push(Line::from(Span::styled("   nothing matches", dim)));
    }
    while lines.len() < room + 2 {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(
        " type to filter   ↑↓ choose   ↵ go there   esc close",
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Starting a sprint: a word goal and a length, then the timer runs.
pub(super) fn draw_sprint(
    f: &mut Frame,
    area: Rect,
    words: &str,
    minutes: &str,
    on_minutes: bool,
    t: &Theme,
) {
    let box_area = centred(area, 46, 9);
    f.render_widget(Clear, box_area);
    let block = pane_block("START A SPRINT", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let field = |label: &str, value: &str, on: bool, unit: &str| {
        Line::from(vec![
            Span::styled(
                format!("  {label:<9}"),
                Style::default().fg(if on { t.accent } else { t.dim }),
            ),
            Span::styled(
                value.to_string(),
                Style::default().fg(t.text).add_modifier(if on {
                    Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                }),
            ),
            Span::styled(if on { "█" } else { " " }, Style::default().fg(t.accent)),
            Span::styled(format!(" {unit}"), Style::default().fg(t.dim)),
        ])
    };
    let lines = vec![
        Line::from(""),
        field("words", words, !on_minutes, "to write"),
        field("minutes", minutes, on_minutes, "on the timer"),
        Line::from(""),
        Line::from(Span::styled(
            "  the timer starts; the status bar keeps count",
            Style::default().fg(t.dim),
        )),
        Line::from(""),
        hint_line(" Tab switch   ↵ start   esc cancel", t),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
