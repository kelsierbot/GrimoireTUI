//! Naming a new thing, renaming one, and the delete prompt.

use super::*;

impl App {
    pub(super) fn on_create_key(&mut self, key: Key) {
        let Overlay::Create { plan, buf, fresh } = &mut self.overlay else {
            return;
        };
        match key {
            // The suggestion is selected: typing replaces it, Backspace
            // clears it, → or End keeps it to add to.
            Key::Char(c) if !c.is_control() => {
                if std::mem::take(fresh) {
                    buf.clear();
                }
                if buf.chars().count() < 60 {
                    buf.push(c);
                }
            }
            Key::Backspace => {
                if std::mem::take(fresh) {
                    buf.clear();
                } else {
                    buf.pop();
                }
            }
            Key::Right | Key::End => *fresh = false,
            Key::Enter => {
                let (plan, name) = (plan.clone(), buf.clone());
                self.overlay = Overlay::None;
                self.finish_create(plan, name);
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

    pub(super) fn on_rename_key(&mut self, key: Key) {
        let Overlay::Rename {
            path, buf, fresh, ..
        } = &mut self.overlay
        else {
            return;
        };
        match key {
            Key::Char(c) if !c.is_control() => {
                if std::mem::take(fresh) {
                    buf.clear();
                }
                if buf.chars().count() < 60 {
                    buf.push(c);
                }
            }
            Key::Backspace => {
                if std::mem::take(fresh) {
                    buf.clear();
                } else {
                    buf.pop();
                }
            }
            Key::Right | Key::End => *fresh = false,
            Key::Enter => {
                let (path, name) = (path.clone(), buf.clone());
                self.overlay = Overlay::None;
                self.finish_rename(path, name);
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

    /// Only `y` deletes. Enter is the fold key two rows up and the
    /// fingers know it — it must not be able to destroy a chapter.
    pub(super) fn on_confirm_key(&mut self, key: Key) {
        let Overlay::Confirm {
            path,
            name,
            permanent,
            ..
        } = &mut self.overlay
        else {
            return;
        };
        match key {
            Key::Char('y') | Key::Char('Y') => {
                let (path, name, permanent) = (path.clone(), name.clone(), *permanent);
                self.overlay = Overlay::None;
                self.finish_delete(path, name, permanent);
            }
            Key::Esc | Key::Enter | Key::Char('n') | Key::Char('N') | Key::Char('q') => {
                self.overlay = Overlay::None;
                self.msg = "kept it".into();
            }
            _ => {}
        }
    }
}

pub(super) fn draw_create(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Create { plan, buf, fresh } = &app.overlay else {
        return;
    };
    let box_area = centred(area, 56, 7);
    f.render_widget(Clear, box_area);
    let title = format!("NEW {}", plan.noun.to_uppercase());
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    // A suggested name shows selected, because typing replaces it.
    let (name, keys) = if *fresh {
        (
            Style::default().fg(t.text).bg(t.sel),
            " ↵ create   type to rename   esc cancel",
        )
    } else {
        (
            Style::default().fg(t.text),
            " type a name   ↵ create   esc cancel",
        )
    };
    let lines = vec![
        Line::from(Span::styled(
            format!(" {}", plan.place),
            Style::default().fg(t.dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" ▸ ", Style::default().fg(t.accent)),
            Span::styled(buf.clone(), name),
            Span::styled("█", Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        hint_line(keys, t),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_rename(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Rename {
        buf, fresh, noun, ..
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(area, 56, 7);
    f.render_widget(Clear, box_area);
    let title = format!("RENAME {}", noun.to_uppercase());
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    // The old name shows selected, because typing replaces it.
    let (name, keys) = if *fresh {
        (
            Style::default().fg(t.text).bg(t.sel),
            " type a new name   ↵ rename   esc cancel",
        )
    } else {
        (Style::default().fg(t.text), " ↵ rename   esc cancel")
    };
    let lines = vec![
        Line::from(Span::styled(
            " the file is renamed to match, and keeps its place".to_string(),
            Style::default().fg(t.dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" ▸ ", Style::default().fg(t.accent)),
            Span::styled(buf.clone(), name),
            Span::styled("█", Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        hint_line(keys, t),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_confirm(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Confirm {
        name,
        noun,
        words,
        permanent,
        ..
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(area, 58, 8);
    f.render_widget(Clear, box_area);
    let title = format!("DELETE {}", noun.to_uppercase());
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let toll = match words {
        0 => "nothing written in it yet".to_string(),
        1 => "1 word goes with it".to_string(),
        n => format!("{n} words go with it"),
    };
    // Its conflict copies go too — say so, and which.
    let Overlay::Confirm { path, .. } = &app.overlay else {
        return;
    };
    let copies = if path.is_file() && !*permanent {
        grimoire_core::sync::copies_of(path)
    } else {
        Vec::new()
    };
    let toll = match copies.as_slice() {
        [] => toll,
        [(_, c)] => format!("{toll}, and its {}", c.source.label()),
        more => format!("{toll}, and its {} conflict copies", more.len()),
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(" Delete ", Style::default().fg(t.text)),
            Span::styled(name.clone(), Style::default().fg(t.accent)),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!(" {toll}"),
            Style::default().fg(t.warn),
        )),
        Line::from(Span::styled(
            if *permanent {
                " it's already in the trash — this is for good"
            } else {
                " it moves to the trash, so it isn't gone for good"
            },
            Style::default().fg(if *permanent { t.warn } else { t.dim }),
        )),
        Line::from(""),
        hint_line(" y delete   esc keep it", t),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
