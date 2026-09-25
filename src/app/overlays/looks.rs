//! How Grimoire looks and sounds: theme presets, the custom-theme editor, and
//! where music comes from.

use super::*;

/// The custom editor's rows: the twelve swatches, then the two borrowed
/// views — which preset's Pomodoro world and Visualizer to draw.
const PICK_WORLD: usize = theme::ROLES.len();
const PICK_LOOK: usize = theme::ROLES.len() + 1;
const CUSTOM_ROWS: usize = theme::ROLES.len() + 2;

impl App {
    pub(super) fn on_themes_key(&mut self, key: Key) {
        let Overlay::Themes { sel, restore } = &mut self.overlay else {
            return;
        };
        let n = theme::presets().len() + 1;
        if list_nav(key, sel, n, RING) {
            // Moving the cursor shows the theme it's on.
            let i = *sel;
            self.preview(i);
            return;
        }
        match key {
            Key::Enter => {
                let i = *sel;
                if i == n - 1 {
                    self.theme.name = "Custom".into();
                    self.overlay = Overlay::Custom {
                        field: 0,
                        buf: String::new(),
                    };
                } else {
                    let _ = theme::save(&self.theme);
                    self.msg = format!("theme: {}", self.theme.name);
                    self.overlay = Overlay::None;
                }
            }
            Key::Esc => {
                self.theme = restore.clone();
                self.overlay = Overlay::None;
            }
            _ => {}
        }
    }

    pub(super) fn on_custom_key(&mut self, key: Key) {
        let Overlay::Custom { field, buf } = &mut self.overlay else {
            return;
        };
        let picking = *field >= theme::ROLES.len();
        let moved = match key {
            Key::Down | Key::Enter if !picking || key == Key::Down => {
                commit(&mut self.theme, *field, buf);
                *field = (*field + 1) % CUSTOM_ROWS;
                buf.clear();
                true
            }
            Key::Up => {
                commit(&mut self.theme, *field, buf);
                *field = (*field + CUSTOM_ROWS - 1) % CUSTOM_ROWS;
                buf.clear();
                true
            }
            _ => false,
        };
        if moved {
            // On a pick row, the pane under the tree shows what's picked.
            match *field {
                PICK_WORLD => self.pane_mode = crate::scene::Mode::Pomodoro,
                PICK_LOOK => self.pane_mode = crate::scene::Mode::Visualizer,
                _ => {}
            }
            return;
        }
        match key {
            // Borrow a preset's Pomodoro world or Visualizer, previewed live.
            Key::Right | Key::Left | Key::Enter | Key::Char(' ') if picking => {
                let forward = key != Key::Left;
                if *field == PICK_WORLD {
                    let next =
                        theme::step_choice(&theme::pomodoro_choices(), self.theme.world(), forward);
                    self.theme.pick_world(next);
                } else {
                    let next = theme::step_choice(
                        &theme::visualizer_choices(),
                        self.theme.look(),
                        forward,
                    );
                    self.theme.pick_look(next);
                }
            }
            Key::Backspace => {
                buf.pop();
            }
            Key::Char(c) if !picking && c.is_ascii_hexdigit() && buf.len() < 6 => {
                buf.push(c.to_ascii_lowercase());
                // Six digits is a complete colour — apply it live.
                if buf.len() == 6 {
                    commit(&mut self.theme, *field, buf);
                }
            }
            Key::Esc => {
                commit(&mut self.theme, *field, buf);
                self.theme.name = "Custom".into();
                let _ = theme::save(&self.theme);
                self.msg = "theme: Custom saved".into();
                self.overlay = Overlay::None;
            }
            _ => {}
        }
    }

    pub(super) fn on_sources_key(&mut self, key: Key) {
        let Overlay::Sources { sel } = &mut self.overlay else {
            return;
        };
        if list_nav(key, sel, music::Source::ALL.len(), RING) {
            return;
        }
        match key {
            Key::Enter | Key::Char(' ') => {
                let chosen = music::Source::ALL[*sel];
                let mut cfg = music::Config::load();
                cfg.source = chosen;
                // Choosing a source is asking for music.
                cfg.enabled = true;
                let _ = cfg.save();
                // Restart the poller against the new source.
                self.music = Music::spawn(cfg);
                self.msg = format!("music: {}", chosen.label());
                self.overlay = Overlay::None;
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

pub(super) fn draw_themes(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Themes { sel, .. } = &app.overlay else {
        return;
    };
    let names = app.theme_names();
    let presets = theme::presets();
    // Twenty themes don't fit a 24-row terminal: the list scrolls, keeping
    // the one under the cursor in view.
    let rows = names
        .len()
        .min(area.height.saturating_sub(6).max(3) as usize);
    let start = (*sel + 1).saturating_sub(rows);
    let box_area = centred(area, 42, rows as u16 + 4);
    f.render_widget(Clear, box_area);
    let block = pane_block("THEME", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    let mut lines: Vec<Line> = names
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .map(|(i, n)| {
            let on = i == *sel;
            let mut spans = vec![
                Span::styled(
                    if on { " ● " } else { " • " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    format!("{n:<21}"),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
            ];
            // A strip of each theme's own colours, to choose by looking.
            if let Some(p) = presets.get(i) {
                for c in [p.accent, p.sun, p.foliage, p.moon, p.bloom, p.warn] {
                    spans.push(Span::styled("● ", Style::default().fg(c)));
                }
            }
            Line::from(spans).style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            })
        })
        .collect();
    let more = match (start > 0, start + rows < names.len()) {
        (true, true) => " ↑↓ more",
        (true, false) => " ↑ more",
        (false, true) => " ↓ more",
        (false, false) => "",
    };
    lines.push(hint_line(more, t));
    lines.push(hint_line(" j/k preview   ↵ apply   esc cancel", t));
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_custom(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Custom { field, buf } = &app.overlay else {
        return;
    };
    // Twelve swatches, a gap, the two picks, a gap and the hint: 17 rows
    // inside, 19 with the frame, so it fits a 24-row terminal.
    let box_area = centred(area, 46, CUSTOM_ROWS as u16 + 5);
    f.render_widget(Clear, box_area);
    let block = pane_block("CUSTOM THEME", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    let mut lines: Vec<Line> = theme::ROLES
        .iter()
        .enumerate()
        .map(|(i, role)| {
            let on = i == *field;
            let col = app.theme.role(i);
            let shown = if on && !buf.is_empty() {
                format!("#{buf}")
            } else {
                theme::hex_of(col)
            };
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(t.accent),
                ),
                Span::styled(format!("{role:<10}"), Style::default().fg(t.text)),
                Span::styled("██ ", Style::default().fg(col)),
                Span::styled(
                    shown,
                    Style::default().fg(if on { t.accent } else { t.dim }),
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
    // Which preset's Pomodoro world and Visualizer to draw — mix and match.
    for (row, label, name) in [
        (PICK_WORLD, "pomodoro", app.theme.world()),
        (PICK_LOOK, "visualizer", app.theme.look()),
    ] {
        let on = row == *field;
        let shown = if on {
            format!("◂ {name} ▸")
        } else {
            name.to_string()
        };
        lines.push(
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(t.accent),
                ),
                Span::styled(format!("{label:<10}"), Style::default().fg(t.text)),
                Span::styled("   ", Style::default()),
                Span::styled(
                    shown,
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            }),
        );
    }
    lines.push(Line::from(""));
    lines.push(hint_line(
        if *field >= theme::ROLES.len() {
            " ←→ choose · ↑↓ move · esc save & close"
        } else {
            " type hex · ↵ next · esc save & close"
        },
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_sources(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Sources { sel } = &app.overlay else {
        return;
    };
    let all = crate::music::Source::ALL;
    let box_area = centred(area, 52, all.len() as u16 + 5);
    f.render_widget(Clear, box_area);
    let block = pane_block("MUSIC SOURCE", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    let mut lines: Vec<Line> = all
        .iter()
        .enumerate()
        .map(|(i, src)| {
            let on = i == *sel;
            let note = if src.plays_audio() {
                "your server · plays here"
            } else {
                "remote control"
            };
            Line::from(vec![
                Span::styled(
                    if on { " ● " } else { " • " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    format!("{:<15}", src.label()),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
                Span::styled(note, Style::default().fg(t.dim)),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            })
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(hint_line(" j/k move   ↵ choose   esc close", t));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Apply a typed hex buffer to a role, ignoring anything unparseable (and
/// the pick rows, which take no hex).
fn commit(t: &mut Theme, field: usize, buf: &str) {
    if field >= theme::ROLES.len() {
        return;
    }
    if let Some(c) = theme::parse_hex(buf) {
        t.set_role(field, c);
        t.name = "Custom".into();
    }
}
