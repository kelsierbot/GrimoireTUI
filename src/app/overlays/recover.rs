//! Words that couldn't be saved last time, offered back on launch.

use super::*;

impl App {
    pub(super) fn on_recover_key(&mut self, key: Key) {
        let Overlay::Recover { items, sel } = &mut self.overlay else {
            return;
        };
        match key {
            Key::Up | Key::Char('k') => {
                *sel = sel.saturating_sub(1);
                return;
            }
            Key::Down | Key::Char('j') => {
                *sel = (*sel + 1).min(RECOVER_CHOICES.len() - 1);
                return;
            }
            _ => {}
        }
        match (key, *sel) {
            (Key::Enter, 0) => {
                let items = std::mem::take(items);
                self.overlay = Overlay::None;
                let mut restored = 0;
                for it in &items {
                    let Some(idx) = self.project.nodes.iter().position(|n| n.path == it.scene)
                    else {
                        continue;
                    };
                    let (front, body) = grimoire_core::project::split_frontmatter(&it.text);
                    self.project.nodes[idx].front = front;
                    if self.open == Some(idx) {
                        self.editor.set_text(&body);
                    }
                    self.project.nodes[idx].body = body;
                    self.mark_changed(idx);
                    restored += 1;
                }
                if self.commit_saves() {
                    self.msg = format!(
                        "restored {restored} scene{} — the saved version is in its history",
                        if restored == 1 { "" } else { "s" }
                    );
                }
            }
            (Key::Enter, 1) => {
                // Set aside, not destroyed: the recovered copies go to the
                // trash, where they can still be opened and copied from.
                let root = self.project.root.clone();
                let mut failed = 0;
                for it in items.iter() {
                    if project::trash(&root, &it.file).is_err() {
                        failed += 1;
                    }
                }
                self.overlay = Overlay::None;
                if let Err(e) = self.reload_tree() {
                    self.msg = format!("couldn't re-read the tree: {e}");
                } else if failed > 0 {
                    self.msg = format!(
                        "kept the saved versions — {failed} recovered cop{} couldn't be moved and will be offered again",
                        if failed == 1 { "y" } else { "ies" }
                    );
                } else {
                    self.msg =
                        "kept the saved versions — the recovered words are in the trash".into();
                }
            }
            (Key::Enter, _) | (Key::Esc, _) => {
                self.overlay = Overlay::None;
                self.msg = "left for now — offered again next time".into();
            }
            _ => {}
        }
    }
}

pub(super) fn draw_recover(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Recover { items, sel } = &app.overlay else {
        return;
    };
    let h = (items.len() as u16).min(8) + 10;
    let box_area = centred(area, 70, h);
    f.render_widget(Clear, box_area);
    let block = pane_block("RECOVERED WORDS", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let mut lines = vec![
        Line::from(Span::styled(
            " These changes were never saved last time. They're safe:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];
    for it in items.iter().take(8) {
        let when = it.when.map(|w| {
            let dt: chrono::DateTime<chrono::Local> = w.into();
            format!(" · {}", when_label(dt))
        });
        lines.push(Line::from(vec![
            Span::styled(" · ", Style::default().fg(t.dim)),
            Span::styled(it.title.clone(), Style::default().fg(t.accent)),
            Span::styled(
                format!(
                    "  {} words kept, {} in the saved version{}",
                    thousands(it.recovered_words),
                    thousands(it.saved_words),
                    when.unwrap_or_default()
                ),
                dim,
            ),
        ]));
    }
    if items.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("   and {} more", items.len() - 8),
            dim,
        )));
    }
    lines.push(Line::from(""));
    for (i, choice) in crate::app::RECOVER_CHOICES.iter().enumerate() {
        let on = i == *sel;
        lines.push(Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                choice.to_string(),
                if on {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.text)
                },
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" ↑↓", key_style(t)),
        Span::styled(" choose   ", dim),
        Span::styled("↵", key_style(t)),
        Span::styled(" do it   ", dim),
        Span::styled("esc", key_style(t)),
        Span::styled(" decide later", dim),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}
