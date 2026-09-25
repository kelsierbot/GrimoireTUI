//! A scene's kept versions, with what changed since each.

use super::*;

impl App {
    pub(super) fn on_history_key(&mut self, key: Key) {
        let Overlay::History {
            scene,
            versions,
            sel,
            scroll,
            ..
        } = &mut self.overlay
        else {
            return;
        };
        // PgUp/PgDn scroll the changes, so only ↑↓ and j/k move the list.
        if list_nav(key, sel, versions.len(), LINES) {
            *scroll = 0;
            return;
        }
        match key {
            Key::PageDown | Key::Char(' ') => *scroll += 10,
            Key::PageUp => *scroll = scroll.saturating_sub(10),
            Key::Enter => {
                let (scene, text) = (scene.clone(), versions[*sel].text.clone());
                self.overlay = Overlay::None;
                self.restore_version(&scene, &text);
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }
}

pub(super) fn draw_history(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::History {
        scene,
        title,
        versions,
        sel,
        scroll,
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(
        area,
        area.width.saturating_sub(4).min(118),
        area.height.saturating_sub(2).min(40),
    );
    f.render_widget(Clear, box_area);
    let head = format!(
        "HISTORY · {} · {} version{}",
        title.to_uppercase(),
        versions.len(),
        if versions.len() == 1 { "" } else { "s" }
    );
    let block = pane_block(&head, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let [list_area, _, diff_area] = Layout::horizontal([
        Constraint::Length(36),
        Constraint::Length(2),
        Constraint::Min(20),
    ])
    .areas(inner);
    let dim = Style::default().fg(t.dim);
    let current = app
        .project
        .nodes
        .iter()
        .find(|n| &n.path == scene)
        .map(|n| n.body.clone())
        .unwrap_or_default();
    let now_words = grimoire_core::notes::count_words(&current) as i64;

    // The versions, newest first.
    let room = list_area.height.saturating_sub(3) as usize;
    let start = sel.saturating_sub(room.saturating_sub(1));
    let mut left: Vec<Line> = vec![
        Line::from(Span::styled(" kept versions · length vs now", dim)),
        Line::from(""),
    ];
    for (i, v) in versions.iter().enumerate().skip(start).take(room) {
        let on = i == *sel;
        let words = v.words() as i64;
        let delta = words - now_words;
        let change = length_vs_now(delta);
        let row = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("{:<17}", when_label(v.when)),
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::styled(truncate(&change, 16), dim),
        ]);
        left.push(if on {
            row.style(Style::default().bg(t.sel))
        } else {
            row
        });
    }
    f.render_widget(Paragraph::new(left), list_area);

    // What changed between that version and now.
    let v = &versions[*sel];
    let old = v.body();
    let pieces = grimoire_core::history::diff(&old, &current);
    let width = diff_area.width.saturating_sub(1) as usize;
    let (mut body, first_change) = diff_lines(&pieces, width, t);
    let header = vec![
        Line::from(vec![
            Span::styled(
                format!("{} · {} words", when_label(v.when), thousands(v.words())),
                Style::default().fg(t.accent),
            ),
            Span::styled(
                format!("  ·  now {} words", thousands(now_words as usize)),
                dim,
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "struck",
                Style::default()
                    .fg(t.warn)
                    .add_modifier(Modifier::CROSSED_OUT),
            ),
            Span::styled(" was in this version and is gone now · ", dim),
            Span::styled(
                "underlined",
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(" is new since", dim),
        ]),
        Line::from(""),
    ];
    let room = (diff_area.height as usize).saturating_sub(header.len() + 2);
    let top = first_change.saturating_sub(2) + *scroll;
    let top = top.min(body.len().saturating_sub(1));
    let mut lines = header;
    if pieces
        .iter()
        .all(|p| matches!(p, grimoire_core::history::Piece::Same(_)))
    {
        lines.push(Line::from(Span::styled(
            "identical to the scene as it is now",
            dim,
        )));
    } else {
        lines.extend(body.drain(..).skip(top).take(room));
    }
    while lines.len() < (diff_area.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(hint_line(
        "↑↓ pick a version   PgUp PgDn scroll   ↵ restore it   esc close",
        t,
    ));
    f.render_widget(Paragraph::new(lines), diff_area);
}

/// A kept version's length against the scene now, in 16 columns or fewer:
/// "6 words shorter", "1,204 words longer", "12,345 shorter", "same length".
pub(super) fn length_vs_now(delta: i64) -> String {
    let n = thousands(delta.unsigned_abs() as usize);
    let (way, one) = match delta {
        0 => return "same length".into(),
        d if d < 0 => ("shorter", "word"),
        _ => ("longer", "word"),
    };
    let plural = if delta.unsigned_abs() == 1 {
        one.to_string()
    } else {
        format!("{one}s")
    };
    let long = format!("{n} {plural} {way}");
    if long.chars().count() <= 16 {
        long
    } else {
        format!("{n} {way}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_says_how_its_length_compares_with_now() {
        assert_eq!(length_vs_now(0), "same length");
        assert_eq!(length_vs_now(-6), "6 words shorter");
        assert_eq!(length_vs_now(1), "1 word longer");
        assert_eq!(length_vs_now(12), "12 words longer");
        assert_eq!(length_vs_now(-12_345), "12,345 shorter");
        for d in [-99_999i64, -999, 7, 99_999] {
            assert!(length_vs_now(d).chars().count() <= 16, "{d}");
        }
    }
}
