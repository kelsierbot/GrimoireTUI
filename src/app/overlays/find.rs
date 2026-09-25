//! Find and replace, in the scene (a bar under the prose) and across the book.

use super::*;

impl App {
    pub(super) fn on_find_key(&mut self, key: Key) {
        let Overlay::Find {
            query,
            with,
            on_with,
            from,
        } = &mut self.overlay
        else {
            return;
        };
        let (q, from_pos) = (query.clone(), *from);
        match key {
            Key::Char(c) if !c.is_control() => {
                if *on_with {
                    with.get_or_insert_with(String::new).push(c);
                } else {
                    query.push(c);
                    let q = query.clone();
                    self.find_step(&q, true, Some(from_pos));
                }
            }
            Key::Backspace => {
                if *on_with {
                    if let Some(w) = with {
                        w.pop();
                    }
                } else {
                    query.pop();
                    let q = query.clone();
                    self.find_step(&q, true, Some(from_pos));
                }
            }
            Key::Tab | Key::BackTab => {
                if with.is_none() {
                    *with = Some(String::new());
                }
                *on_with = !*on_with;
            }
            Key::Enter if *on_with => {
                let w = with.clone().unwrap_or_default();
                self.replace_current(&q, &w);
            }
            Key::Enter | Key::Down => self.find_step(&q, true, None),
            Key::Up => self.find_step(&q, false, None),
            Key::Esc => {
                self.overlay = Overlay::None;
                self.focus = Focus::Editor;
            }
            _ => {}
        }
    }

    pub(super) fn on_find_book_key(&mut self, key: Key) {
        let Overlay::FindBook {
            query,
            with,
            on_with,
            hits,
            sel,
            confirm,
        } = &mut self.overlay
        else {
            return;
        };
        if *confirm {
            match key {
                Key::Char('y') | Key::Char('Y') => {
                    let (q, w) = (query.clone(), with.clone().unwrap_or_default());
                    self.overlay = Overlay::None;
                    self.replace_in_book(&q, &w);
                }
                _ => *confirm = false,
            }
            return;
        }
        let mut changed = false;
        match key {
            Key::Char(c) if !c.is_control() => {
                if *on_with {
                    with.get_or_insert_with(String::new).push(c);
                } else {
                    query.push(c);
                    changed = true;
                }
            }
            Key::Backspace => {
                if *on_with {
                    if let Some(w) = with {
                        w.pop();
                    }
                } else {
                    query.pop();
                    changed = true;
                }
            }
            Key::Tab | Key::BackTab => {
                if with.is_none() {
                    *with = Some(String::new());
                }
                *on_with = !*on_with;
            }
            k if list_nav(k, sel, hits.len(), TYPED) => {}
            Key::Enter if *on_with => {
                if !hits.is_empty() && with.is_some() {
                    *confirm = true;
                }
            }
            Key::Enter => {
                if let Some(h) = hits.get(*sel).cloned() {
                    self.go_to_hit(&h.path, h.line, h.start, h.end);
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
        if changed
            && let Overlay::FindBook {
                query, hits, sel, ..
            } = &mut self.overlay
        {
            *hits = search::book(&self.project, &self.parents, query, self.find_opts);
            *sel = 0;
        }
    }
}

pub(super) fn draw_find(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Find {
        query,
        with,
        on_with,
        ..
    } = &app.overlay
    else {
        return;
    };
    let editor = app.rect_editor;
    let h = if with.is_some() { 5 } else { 4 };
    let w = editor.width.saturating_add(4).min(area.width);
    let bar = Rect {
        x: editor.x.saturating_sub(2),
        y: (editor.y + editor.height).saturating_sub(h).max(area.y),
        width: w,
        height: h,
    };
    f.render_widget(Clear, bar);
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(t.accent));
    let inner = block.inner(bar);
    f.render_widget(block, bar);
    let field = |label: &str, text: &str, active: bool| {
        vec![
            Span::styled(
                format!(" {label:>7} ▸ "),
                Style::default().fg(if active { t.accent } else { t.dim }),
            ),
            Span::styled(text.to_string(), Style::default().fg(t.text)),
            Span::styled(
                if active { "█" } else { " " },
                Style::default().fg(t.accent),
            ),
        ]
    };
    let m = app.mod_label();
    let mut first = field("find", query, !*on_with);
    let pos = app.find_position(query);
    first.push(Span::styled(
        format!("  {pos}"),
        Style::default().fg(if pos == "no matches" { t.warn } else { t.sun }),
    ));
    first.extend(hint_spans(
        &format!("   ↵ next  ↑ previous  Tab replace  {m}F whole book  esc close"),
        t,
    ));
    let mut lines = vec![Line::from(first)];
    if let Some(w) = with {
        let mut second = field("replace", w, *on_with);
        second.extend(hint_spans(
            &format!("   ↵ replace this one  {m}R replace all in this scene"),
            t,
        ));
        lines.push(Line::from(second));
    }
    lines.push(find_opts_line(app, t));
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_find_book(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::FindBook {
        query,
        with,
        on_with,
        hits,
        sel,
        confirm,
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(
        area,
        area.width.saturating_sub(4).min(110),
        area.height.saturating_sub(2).min(38),
    );
    f.render_widget(Clear, box_area);
    let scenes = {
        let mut seen: Vec<&std::path::Path> = Vec::new();
        for h in hits {
            if !seen.contains(&h.path.as_path()) {
                seen.push(&h.path);
            }
        }
        seen.len()
    };
    let head = if query.is_empty() {
        "FIND IN THE BOOK".to_string()
    } else {
        format!(
            "FIND IN THE BOOK · {} match{} in {} scene{}",
            hits.len(),
            if hits.len() == 1 { "" } else { "es" },
            scenes,
            if scenes == 1 { "" } else { "s" }
        )
    };
    let block = pane_block(&head, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let iw = inner.width as usize;
    let dim = Style::default().fg(t.dim);
    let field = |label: &str, text: &str, active: bool| {
        Line::from(vec![
            Span::styled(
                format!(" {label:>7} ▸ "),
                Style::default().fg(if active { t.accent } else { t.dim }),
            ),
            Span::styled(text.to_string(), Style::default().fg(t.text)),
            Span::styled(
                if active { "█" } else { " " },
                Style::default().fg(t.accent),
            ),
        ])
    };
    let mut lines = vec![field("find", query, !*on_with)];
    if let Some(w) = with {
        lines.push(field("replace", w, *on_with));
    }
    lines.push(find_opts_line(app, t));
    lines.push(Line::from(Span::styled(
        "─".repeat(iw),
        Style::default().fg(t.border),
    )));

    // Results, grouped under the scene they're in, scrolled to keep
    // the selection in view.
    let mut rows: Vec<(Option<usize>, Line)> = Vec::new();
    let mut last: Option<&std::path::Path> = None;
    let counts = |p: &std::path::Path| hits.iter().filter(|h| h.path == p).count();
    let match_bg = blend(t.border, t.sun, 0.45);
    for (i, h) in hits.iter().enumerate() {
        if last != Some(h.path.as_path()) {
            last = Some(&h.path);
            rows.push((
                None,
                Line::from(vec![
                    Span::styled(
                        format!(" {}", truncate(&h.place, iw.saturating_sub(8))),
                        Style::default().fg(t.accent),
                    ),
                    Span::styled(format!("  {}", counts(&h.path)), dim),
                ]),
            ));
        }
        let chars: Vec<char> = h.text.chars().collect();
        let room = iw.saturating_sub(10);
        let lead = h.start.saturating_sub(room / 3);
        let before: String = chars[lead..h.start].iter().collect();
        let hit: String = chars[h.start..h.end].iter().collect();
        let after_room = room.saturating_sub(before.chars().count() + hit.chars().count());
        let after: String = chars[h.end..].iter().take(after_room).collect();
        let on = i == *sel;
        let row = Line::from(vec![
            Span::styled(
                if on { "  ▸ " } else { "    " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("{}{before}", if lead > 0 { "…" } else { "" }),
                Style::default().fg(t.text),
            ),
            Span::styled(hit, Style::default().fg(t.text).bg(match_bg)),
            Span::styled(after, Style::default().fg(t.text)),
        ]);
        rows.push((
            Some(i),
            if on {
                row.style(Style::default().bg(t.sel))
            } else {
                row
            },
        ));
    }
    let room = (inner.height as usize).saturating_sub(lines.len() + 2);
    let sel_row = rows.iter().position(|(i, _)| *i == Some(*sel)).unwrap_or(0);
    let start = sel_row.saturating_sub(room.saturating_sub(2));
    if query.is_empty() {
        lines.push(hint_line(" type to search every scene and note", t));
    } else if hits.is_empty() {
        lines.push(Line::from(Span::styled(" nothing found", dim)));
    }
    lines.extend(rows.into_iter().skip(start).take(room).map(|(_, l)| l));
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    let m = app.mod_label();
    lines.push(if *confirm {
                Line::from(
                    [
                        vec![Span::styled(
                            format!(" Replace {} match{} ({}) in {} scene{} with “{}”? ", hits.len(), if hits.len() == 1 { "" } else { "es" }, app.find_opts_label(), scenes, if scenes == 1 { "" } else { "s" }, with.clone().unwrap_or_default()),
                            Style::default().fg(t.warn),
                        )],
                        hint_spans("y replace   any other key cancels", t),
                    ]
                    .concat(),
                )
            } else if with.is_some() {
                hint_line(&format!(" ↑↓ choose   ↵ go to it   Tab switch field   ↵ in replace (or {m}R) replaces all   esc close"), t)
            } else {
                hint_line(" ↑↓ choose   ↵ go to it   Tab replace   esc close", t)
            });
    f.render_widget(Paragraph::new(lines), inner);
}

/// How the find bar is matching, and the two keys that change it. Loosened
/// settings show in the warning colour: they're the ones that can surprise.
pub(super) fn find_opts_line(app: &App, t: &Theme) -> Line<'static> {
    let o = app.find_opts;
    let setting = |on: bool, yes: &str, no: &str| {
        Span::styled(
            if on { yes.to_string() } else { no.to_string() },
            Style::default().fg(if on { t.dim } else { t.warn }),
        )
    };
    Line::from(
        [
            vec![
                Span::styled("  ", Style::default().fg(t.dim)),
                setting(o.whole_words, "whole words", "inside words too"),
                Span::styled(" · ", Style::default().fg(t.dim)),
                setting(o.match_case, "exact case", "any case"),
            ],
            hint_spans("   ^W/^E change", t),
        ]
        .concat(),
    )
}
