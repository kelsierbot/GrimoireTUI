//! The Esc menu and its Settings: the discoverable way to everything.

use super::*;

impl App {
    pub(super) fn on_menu_key(&mut self, key: Key) {
        let nested = matches!(self.overlay, Overlay::Settings { .. });
        let items: Vec<Action> = if nested {
            self.settings_menu()
        } else {
            self.menu()
        }
        .into_iter()
        .map(|(_, a)| a)
        .collect();
        let (Overlay::Menu { sel } | Overlay::Settings { sel }) = &mut self.overlay else {
            return;
        };
        if list_nav(key, sel, items.len(), RING) {
            return;
        }
        match key {
            Key::Enter | Key::Char(' ') | Key::Right | Key::Char('l') => {
                let (at, action) = (*sel, items[*sel].clone());
                // A setting switched on or off stays in Settings, so
                // the change shows on its row.
                let stay = nested && action.is_setting_toggle();
                self.run_action(action);
                if stay && self.overlay == Overlay::None {
                    self.overlay = Overlay::Settings { sel: at };
                }
            }
            Key::Esc | Key::Left | Key::Char('h') if nested => self.run_action(Action::MenuBack),
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

pub(super) fn draw_menu(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (Overlay::Menu { sel } | Overlay::Settings { sel }) = &app.overlay else {
        return;
    };
    let nested = matches!(app.overlay, Overlay::Settings { .. });
    let items: Vec<String> = if nested {
        app.settings_menu()
    } else {
        app.menu()
    }
    .into_iter()
    .map(|(label, _)| label)
    .collect();
    let box_area = centred(area, 42, items.len() as u16 + 4);
    f.render_widget(Clear, box_area);
    let block = pane_block(
        if nested {
            "GRIMOIRE › SETTINGS"
        } else {
            "GRIMOIRE"
        },
        true,
        t,
    );
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    // A short terminal shows the rows around the highlight rather
    // than cutting off the end of the menu.
    let room = (inner.height as usize).saturating_sub(2).max(1);
    let start = (*sel + 1).saturating_sub(room);
    let mut lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(i, label)| {
            let on = i == *sel;
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    label.clone(),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            })
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(hint_line(
        if nested {
            " j/k move   ↵ choose   esc back"
        } else {
            " j/k move   ↵ choose   esc close"
        },
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
