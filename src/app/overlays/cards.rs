//! The corkboard: index cards for one part, chapter by chapter.

use super::*;

impl App {
    pub(super) fn on_cork_key(&mut self, key: Key) {
        let cols = self.cork_cols();
        let Overlay::Cork {
            scope,
            sel,
            pov,
            typing,
        } = &mut self.overlay
        else {
            return;
        };
        let scope_idx = scope
            .as_ref()
            .and_then(|p| self.project.nodes.iter().position(|n| &n.path == p));
        let groups = cork::board(&self.project, scope_idx);
        let cards: Vec<cork::Card> = groups.iter().flat_map(|g| g.cards.clone()).collect();
        if cards.is_empty() {
            self.overlay = Overlay::None;
            return;
        }
        *sel = (*sel).min(cards.len() - 1);
        let card = cards[*sel].clone();

        if let Some((field, buf)) = typing {
            match key {
                Key::Char(c) if !c.is_control() && buf.chars().count() < 200 => buf.push(c),
                Key::Backspace => {
                    buf.pop();
                }
                Key::Enter => {
                    let (field, value) = (*field, buf.clone());
                    *typing = None;
                    let key = match field {
                        CardField::Synopsis => "synopsis",
                        CardField::Pov => "pov",
                    };
                    self.project.nodes[card.idx].set_meta(key, &value);
                    self.mark_changed(card.idx);
                    self.msg = format!("{} · {key} saved", card.title);
                }
                Key::Esc => *typing = None,
                _ => {}
            }
            return;
        }

        match key {
            Key::Left | Key::Char('h') => *sel = cork::step(&groups, cols, *sel, -1, 0),
            Key::Right | Key::Char('l') => *sel = cork::step(&groups, cols, *sel, 1, 0),
            Key::Up | Key::Char('k') => *sel = cork::step(&groups, cols, *sel, 0, -1),
            Key::Down | Key::Char('j') => *sel = cork::step(&groups, cols, *sel, 0, 1),
            Key::Char('[') | Key::Char(']') => {
                let parts = cork::parts(&self.project);
                if let Some(at) = scope_idx.and_then(|s| parts.iter().position(|&p| p == s)) {
                    let next = if key == Key::Char('[') {
                        at.checked_sub(1)
                    } else {
                        (at + 1 < parts.len()).then_some(at + 1)
                    };
                    if let Some(n) = next {
                        *scope = Some(self.project.nodes[parts[n]].path.clone());
                        *sel = 0;
                    }
                }
            }
            Key::Char('p') => {
                let all = cork::povs(&groups);
                *pov = match pov
                    .as_ref()
                    .and_then(|cur| all.iter().position(|x| x == cur))
                {
                    None if !all.is_empty() && pov.is_none() => Some(all[0].clone()),
                    Some(i) if i + 1 < all.len() => Some(all[i + 1].clone()),
                    _ => None,
                };
            }
            Key::Char('s') => {
                let next = cork::next_status(card.status.as_deref());
                self.project.nodes[card.idx].set_meta("status", next);
                self.mark_changed(card.idx);
            }
            Key::Char('e') => {
                *typing = Some((
                    CardField::Synopsis,
                    card.synopsis.clone().unwrap_or_default(),
                ))
            }
            Key::Char('v') => {
                *typing = Some((CardField::Pov, card.pov.clone().unwrap_or_default()))
            }
            Key::Enter => {
                self.overlay = Overlay::None;
                self.reveal(card.idx);
                self.open_scene(card.idx);
            }
            Key::Esc | Key::Char('b') => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_cork(
    f: &mut Frame,
    app: &App,
    area: Rect,
    t: &Theme,
    scope: &Option<std::path::PathBuf>,
    sel: usize,
    pov: Option<&str>,
    typing: Option<&(crate::app::CardField, String)>,
) {
    use grimoire_core::cork;
    let box_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    f.render_widget(Clear, box_area);
    let part = app.cork_scope(scope);
    let groups = cork::board(&app.project, part);
    let where_ = part
        .map(|i| app.project.nodes[i].title.to_uppercase())
        .unwrap_or_else(|| "THE BOOK".into());
    let head = match pov {
        Some(p) => format!("CORKBOARD · {where_} · POV: {p}"),
        None => format!("CORKBOARD · {where_}"),
    };
    let block = pane_block(&head, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);

    // Each POV keeps one colour for the whole board.
    let povs = cork::povs(&groups);
    let palette = [t.accent, t.moon, t.sun, t.bloom, t.warn, t.text];
    let colour_of = |name: Option<&str>| {
        name.and_then(|n| povs.iter().position(|p| p.eq_ignore_ascii_case(n)))
            .map(|i| palette[i % palette.len()])
            .unwrap_or(t.dim)
    };
    let mut legend = vec![Span::styled(" POV  ", dim)];
    for p in &povs {
        legend.push(Span::styled("■ ", Style::default().fg(colour_of(Some(p)))));
        legend.push(Span::styled(
            format!("{p}   "),
            Style::default().fg(if pov.is_none_or(|x| x == p) {
                t.text
            } else {
                t.dim
            }),
        ));
    }
    if povs.is_empty() {
        legend.push(Span::styled("none set yet — v on a card sets one", dim));
    }
    f.render_widget(
        Paragraph::new(Line::from(legend)),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );

    let cols = app.cork_cols();
    const CARD_H: u16 = 6;
    use crate::app::CARD_W;
    let layout = cork::layout(&groups, cols);
    let cards: Vec<&cork::Card> = groups.iter().flat_map(|g| &g.cards).collect();
    let sel = sel.min(cards.len().saturating_sub(1));

    // Rows of the board, with a heading line before each chapter's first row.
    let body = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(4),
    };
    let mut y_of_row: Vec<u16> = Vec::new();
    let mut y = 0u16;
    let mut group_first_row = Vec::new();
    let mut row = 0usize;
    for g in &groups {
        group_first_row.push((row, y));
        y += 1; // heading
        for _ in 0..g.cards.len().div_ceil(cols) {
            y_of_row.push(y);
            y += CARD_H;
            row += 1;
        }
        y += 1; // gap
    }
    let sel_row = layout.get(sel).map(|&(_, r, _)| r).unwrap_or(0);
    let sel_bottom = y_of_row.get(sel_row).copied().unwrap_or(0) + CARD_H;
    let offset = sel_bottom.saturating_sub(body.height);

    for (gi, g) in groups.iter().enumerate() {
        let (_, gy) = group_first_row[gi];
        if gy >= offset && gy - offset < body.height {
            let words: usize = g.cards.iter().map(|c| c.words).sum();
            let title = if g.title.is_empty() {
                "scenes".to_string()
            } else {
                g.title.clone()
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" {title}"),
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "  {} scene{} · {} words",
                            g.cards.len(),
                            if g.cards.len() == 1 { "" } else { "s" },
                            thousands(words)
                        ),
                        dim,
                    ),
                ])),
                Rect {
                    x: body.x,
                    y: body.y + gy - offset,
                    width: body.width,
                    height: 1,
                },
            );
        }
    }

    for (n, &(_, r, c)) in layout.iter().enumerate() {
        let top = y_of_row[r];
        if top < offset || top - offset + CARD_H > body.height {
            continue;
        }
        let card = cards[n];
        let rect = Rect {
            x: body.x + 1 + c as u16 * CARD_W,
            y: body.y + top - offset,
            width: CARD_W - 1,
            height: CARD_H,
        };
        let lit = pov.is_none_or(|p| {
            card.pov
                .as_deref()
                .is_some_and(|cp| cp.eq_ignore_ascii_case(p))
        });
        let on = n == sel;
        let edge = if on {
            t.accent
        } else if lit {
            colour_of(card.pov.as_deref())
        } else {
            t.border
        };
        let text = if lit { t.text } else { t.border };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(edge).add_modifier(if on {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }))
            .title(Span::styled(
                format!(" {} ", truncate(&card.title, (CARD_W - 5) as usize)),
                Style::default().fg(if on { t.accent } else { text }),
            ));
        let cin = block.inner(rect);
        f.render_widget(block, rect);
        let w = cin.width as usize;
        let status = card.status.clone().unwrap_or_default();
        let status_col = match status.to_lowercase().as_str() {
            "done" => t.bloom,
            "revised" => t.accent,
            "draft" => t.sun,
            _ => t.dim,
        };
        let who = card.pov.clone().unwrap_or_else(|| "no POV".into());
        let pad = w.saturating_sub(who.chars().count() + status.chars().count());
        let editing_here = on && typing.is_some();
        let mut lines = vec![Line::from(vec![
            Span::styled(
                truncate(&who, w.saturating_sub(status.chars().count() + 1)),
                Style::default().fg(if lit {
                    colour_of(card.pov.as_deref())
                } else {
                    t.border
                }),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(
                status,
                Style::default().fg(if lit { status_col } else { t.border }),
            ),
        ])];
        // No synopsis yet: the scene's own first line stands in, dimmed.
        let (synopsis, standin) = match typing {
            Some((crate::app::CardField::Synopsis, buf)) if editing_here => {
                (format!("{buf}█"), false)
            }
            _ => match card.synopsis.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(s) => (s.to_string(), false),
                None => (first_line(&app.project.nodes[card.idx].body), true),
            },
        };
        let wrapped = wrap_words(&synopsis, w);
        for i in 0..2 {
            lines.push(Line::from(Span::styled(
                wrapped.get(i).cloned().unwrap_or_default(),
                Style::default().fg(match (lit, standin) {
                    (true, false) => t.text,
                    (true, true) => t.dim,
                    (false, _) => t.border,
                }),
            )));
        }
        if let Some((crate::app::CardField::Pov, buf)) = typing.filter(|_| editing_here) {
            lines[0] = Line::from(vec![
                Span::styled("POV ▸ ", Style::default().fg(t.accent)),
                Span::styled(format!("{buf}█"), Style::default().fg(t.text)),
            ]);
        }
        let bar_w = w.saturating_sub(6).max(4);
        let words = thousands(card.words);
        let bar = match card.target {
            Some(target) => {
                let filled = ((card.words * bar_w) / target.max(1)).min(bar_w);
                vec![
                    Span::styled(
                        "█".repeat(filled),
                        Style::default().fg(if lit { t.accent } else { t.border }),
                    ),
                    Span::styled("░".repeat(bar_w - filled), Style::default().fg(t.border)),
                    Span::styled(format!("{words:>6}"), dim),
                ]
            }
            None => vec![Span::styled(format!("{words} words"), dim)],
        };
        lines.push(Line::from(bar));
        f.render_widget(Paragraph::new(lines), cin);
    }

    let keys = if typing.is_some() {
        " type   ↵ save   esc cancel".to_string()
    } else {
        format!(
            " ←→↑↓ move   ↵ open   s status   e synopsis   v POV   p filter by POV{}   esc close",
            if cork::parts(&app.project).len() > 1 {
                "   [ ] other acts"
            } else {
                ""
            }
        )
    };
    f.render_widget(
        Paragraph::new(hint_line(&keys, t)),
        Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        },
    );
}

/// A scene's first line of prose, for a card with no synopsis.
pub(super) fn first_line(body: &str) -> String {
    body.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_without_a_synopsis_shows_the_first_line_of_prose() {
        assert_eq!(first_line("\n\n# A heading\nThe rain.\n"), "A heading");
        assert_eq!(first_line("\n  The rain came.\nMore.\n"), "The rain came.");
        assert_eq!(first_line("\n\n"), "");
    }
}
