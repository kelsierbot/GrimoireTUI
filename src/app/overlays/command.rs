//! Ctrl-K, the command palette: any action, scene, note or theme by name.

use super::*;

impl App {
    pub(super) fn on_palette_key(&mut self, key: Key) {
        let Overlay::Palette {
            query,
            sel,
            entries,
        } = &mut self.overlay
        else {
            return;
        };
        if list_nav(key, sel, palette::filter(entries, query).len(), TYPED) {
            return;
        }
        match key {
            Key::Char(c) if !c.is_control() => {
                query.push(c);
                *sel = 0;
            }
            Key::Backspace => {
                query.pop();
                *sel = 0;
            }
            Key::Enter => {
                let hits = palette::filter(entries, query);
                if let Some(e) = hits.get((*sel).min(hits.len().saturating_sub(1))) {
                    let action = e.action.clone();
                    self.run_action(action);
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

pub(super) fn draw_palette(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Palette {
        query,
        sel,
        entries,
    } = &app.overlay
    else {
        return;
    };
    let hits = crate::palette::filter(entries, query);
    let rows = 14usize;
    let w = area.width.saturating_sub(4).min(84);
    let h = rows as u16 + 5;
    let box_area = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + 2.min(area.height.saturating_sub(h)),
        width: w,
        height: h.min(area.height),
    };
    f.render_widget(Clear, box_area);
    let block = pane_block("FIND ANYTHING", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let iw = inner.width as usize;
    let dim = Style::default().fg(t.dim);
    let sel = (*sel).min(hits.len().saturating_sub(1));
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" › ", Style::default().fg(t.accent)),
            Span::styled(query.clone(), Style::default().fg(t.text)),
            Span::styled("█", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled("─".repeat(iw), Style::default().fg(t.border))),
    ];
    let start = sel.saturating_sub(rows.saturating_sub(1));
    for (i, e) in hits.iter().enumerate().skip(start).take(rows) {
        let on = i == sel;
        let key_w = e.key.chars().count();
        let label_w = e.label.chars().count().min(iw.saturating_sub(key_w + 6));
        let label = truncate(&e.label, label_w);
        let detail_room = iw.saturating_sub(label.chars().count() + key_w + 7);
        let detail = if e.detail.is_empty() || detail_room < 6 {
            String::new()
        } else {
            format!("  {}", truncate(&e.detail, detail_room))
        };
        let used = 3 + label.chars().count() + detail.chars().count();
        let pad = iw.saturating_sub(used + key_w + 1);
        let row = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                label,
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::styled(detail, dim),
            Span::raw(" ".repeat(pad)),
            Span::styled(e.key.clone(), Style::default().fg(t.sun)),
            Span::raw(" "),
        ]);
        lines.push(if on {
            row.style(Style::default().bg(t.sel))
        } else {
            row
        });
    }
    if hits.is_empty() {
        lines.push(Line::from(Span::styled("   nothing by that name", dim)));
    }
    while lines.len() < rows + 2 {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(
        " type to search   ↑↓ choose   ↵ do it   esc close",
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
