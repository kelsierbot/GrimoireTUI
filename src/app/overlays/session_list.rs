//! Writing sessions: turning them on, the list, and one scene's changes.

use super::*;

impl App {
    pub(super) fn on_sessions_off_key(&mut self, key: Key) {
        match key {
            Key::Char('y') | Key::Char('Y') => {
                self.overlay = Overlay::None;
                self.commit_saves();
                self.save_resume(true);
                match sessions::enable(&self.project.root) {
                    Ok(()) => {
                        self.sessions_on = true;
                        self.msg =
                            "session history is on — each session is kept when you quit".into();
                        self.open_sessions();
                    }
                    Err(e) => self.msg = format!("couldn't turn on session history: {e}"),
                }
            }
            _ => self.overlay = Overlay::None,
        }
    }

    pub(super) fn on_sessions_key(&mut self, key: Key) {
        let Overlay::Sessions {
            list, sel, changes, ..
        } = &mut self.overlay
        else {
            return;
        };
        if let Some((which, items, csel)) = changes {
            if list_nav(key, csel, items.len(), LIST) {
                return;
            }
            match key {
                Key::Enter => {
                    if let (Some(s), Some(c)) =
                        (list.get(*which).cloned(), items.get(*csel).cloned())
                    {
                        let root = self.project.root.clone();
                        let parent = format!("{}^", s.hash);
                        let old_path = match &c.kind {
                            sessions::ChangeKind::Renamed { from } => from.clone(),
                            _ => c.path.clone(),
                        };
                        let before = sessions::file_at(&root, &parent, &old_path)
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let after = sessions::file_at(&root, &s.hash, &c.path)
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let title = c
                            .path
                            .file_stem()
                            .map(|x| x.to_string_lossy().to_string())
                            .unwrap_or_default();
                        self.overlay = Overlay::SessionDiff {
                            title,
                            label: s.label.clone(),
                            before: grimoire_core::project::split_frontmatter(&before).1,
                            after: grimoire_core::project::split_frontmatter(&after).1,
                            scroll: 0,
                        };
                    }
                }
                Key::Esc | Key::Left | Key::Char('h') => *changes = None,
                _ => {}
            }
            return;
        }
        if list_nav(key, sel, list.len(), LIST) {
            return;
        }
        match key {
            Key::Enter | Key::Right | Key::Char('h') => {
                if let Some(s) = list.get(*sel) {
                    match sessions::changes(&self.project.root, &s.hash) {
                        Ok(items) => *changes = Some((*sel, items, 0)),
                        Err(e) => self.msg = format!("couldn't read that session: {e}"),
                    }
                }
            }
            Key::Char('s') => {
                self.overlay = Overlay::None;
                if let Some(label) = self.save_session() {
                    self.msg = format!("session saved: {label}");
                }
                self.open_sessions();
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

    pub(super) fn on_session_diff_key(&mut self, key: Key) {
        let Overlay::SessionDiff { scroll, .. } = &mut self.overlay else {
            return;
        };
        match key {
            Key::Down | Key::PageDown | Key::Char(' ') => *scroll += 5,
            Key::Up | Key::PageUp => *scroll = scroll.saturating_sub(5),
            _ => self.open_sessions_keeping_place(),
        }
    }
}

pub(super) fn draw_sessions_off(f: &mut Frame, _app: &App, area: Rect, t: &Theme) {
    let box_area = centred(area, 70, 12);
    f.render_widget(Clear, box_area);
    let block = pane_block("WRITING SESSIONS", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let text = Style::default().fg(t.text);
    let lines = vec![
        Line::from(Span::styled(
            " Keep each writing session as a snapshot of the whole book, labelled",
            text,
        )),
        Line::from(Span::styled(
            " like a diary line — “Tuesday evening · Act Two · 1,240 words” — so you",
            text,
        )),
        Line::from(Span::styled(
            " can see what changed on any evening, and get any of it back.",
            text,
        )),
        Line::from(""),
        Line::from(Span::styled(
            " It uses Git inside this book's folder: ordinary files any Git tool can",
            dim,
        )),
        Line::from(Span::styled(
            " read. Connect a remote and sessions back themselves up.",
            dim,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " y ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("turn it on for this book   ", text),
            Span::styled("any other key", key_style(t)),
            Span::styled(": not now", dim),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_sessions(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Sessions {
        list,
        sel,
        pending,
        backup,
        changes,
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(
        area,
        area.width.saturating_sub(4).min(96),
        area.height.saturating_sub(2).min(34),
    );
    f.render_widget(Clear, box_area);
    let block = pane_block("WRITING SESSIONS", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let iw = inner.width as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!(
                " {} session{}",
                list.len(),
                if list.len() == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.text),
        ),
        Span::styled(
            format!("   {backup}"),
            Style::default().fg(if backup.contains('✓') {
                t.accent
            } else {
                t.dim
            }),
        ),
    ])];
    if let Some(p) = pending {
        lines.push(Line::from(vec![
            Span::styled(" now: ", dim),
            Span::styled(p.clone(), Style::default().fg(t.sun)),
            Span::styled("  — saved when you quit, or s to save it now", dim),
        ]));
    }
    lines.push(Line::from(""));
    let footer = 2;
    let room = (inner.height as usize).saturating_sub(lines.len() + footer);
    match changes {
        None => {
            let start = sel.saturating_sub(room.saturating_sub(1));
            for (i, s) in list.iter().enumerate().skip(start).take(room) {
                let on = i == *sel;
                let when = s.when.format("%a %-d %b %-I:%M %P").to_string();
                let row = Line::from(vec![
                    Span::styled(
                        " ● ",
                        Style::default().fg(if i == 0 { t.sun } else { t.border }),
                    ),
                    Span::styled(
                        format!(
                            "{:<width$}",
                            truncate(&s.label, iw.saturating_sub(24)),
                            width = iw.saturating_sub(24)
                        ),
                        Style::default().fg(if on { t.accent } else { t.text }),
                    ),
                    Span::styled(format!("{when:>19}"), dim),
                ]);
                lines.push(if on {
                    row.style(Style::default().bg(t.sel))
                } else {
                    row
                });
            }
            while lines.len() < (inner.height as usize).saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(hint_line(
                " ↑↓ choose   ↵ what changed that session   s save this session now   esc close",
                t,
            ));
        }
        Some((which, items, csel)) => {
            let s = &list[*which];
            lines.push(Line::from(vec![
                Span::styled(format!(" {}", s.label), Style::default().fg(t.accent)),
                Span::styled(format!("  {}", s.when.format("%a %-d %b %-I:%M %P")), dim),
            ]));
            lines.push(Line::from(""));
            if items.is_empty() {
                lines.push(Line::from(Span::styled(" no scenes or notes changed", dim)));
            }
            for (i, c) in items.iter().enumerate().take(room.saturating_sub(2)) {
                let on = i == *csel;
                let delta = c.words_after as i64 - c.words_before as i64;
                let what = match &c.kind {
                    grimoire_core::sessions::ChangeKind::Added => {
                        format!("new · {} words", thousands(c.words_after))
                    }
                    grimoire_core::sessions::ChangeKind::Deleted => {
                        format!("removed · {} words", thousands(c.words_before))
                    }
                    grimoire_core::sessions::ChangeKind::Renamed { .. } if delta == 0 => {
                        "moved".to_string()
                    }
                    _ if delta > 0 => format!("{} words added", thousands(delta as usize)),
                    _ if delta < 0 => format!("{} words cut", thousands((-delta) as usize)),
                    _ => "revised".to_string(),
                };
                let name = c.path.to_string_lossy().replace('\\', "/");
                let row = Line::from(vec![
                    Span::styled(
                        if on { " ▸ " } else { "   " },
                        Style::default().fg(t.accent),
                    ),
                    Span::styled(
                        format!(
                            "{:<width$}",
                            truncate(&name, iw.saturating_sub(28)),
                            width = iw.saturating_sub(28)
                        ),
                        Style::default().fg(if on { t.accent } else { t.text }),
                    ),
                    Span::styled(format!("{what:>24}"), dim),
                ]);
                lines.push(if on {
                    row.style(Style::default().bg(t.sel))
                } else {
                    row
                });
            }
            while lines.len() < (inner.height as usize).saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(hint_line(
                " ↑↓ choose   ↵ see the changes   esc back to sessions",
                t,
            ));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_session_diff(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::SessionDiff {
        title,
        label,
        before,
        after,
        scroll,
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(
        area,
        area.width.saturating_sub(4).min(110),
        area.height.saturating_sub(2).min(40),
    );
    f.render_widget(Clear, box_area);
    let head = format!("{} · {}", title, label);
    let block = pane_block(&head, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let pieces = grimoire_core::history::diff(before, after);
    let (body, first) = diff_lines(&pieces, inner.width.saturating_sub(1) as usize, t);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "struck",
                Style::default()
                    .fg(t.warn)
                    .add_modifier(Modifier::CROSSED_OUT),
            ),
            Span::styled(" cut that session · ", dim),
            Span::styled(
                "underlined",
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(" written that session", dim),
        ]),
        Line::from(""),
    ];
    let room = (inner.height as usize).saturating_sub(4);
    lines.extend(
        body.into_iter()
            .skip(first.saturating_sub(2) + *scroll)
            .take(room),
    );
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(hint_line("↑↓ scroll   any other key: back to sessions", t));
    f.render_widget(Paragraph::new(lines), inner);
}
