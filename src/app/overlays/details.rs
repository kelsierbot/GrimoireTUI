//! A scene's details, or a notebook entry's, in one box: what the corkboard
//! shows on its card, plus the word target, whether it goes in exports, and
//! the other names the notebook knows someone by. All of it is the scene's
//! frontmatter, written the way it's written by hand.

use super::*;
use grimoire_core::cork::STATUSES;

/// The rows of a Details box, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Status,
    Pov,
    Synopsis,
    Target,
    InExports,
    Aliases,
}

impl App {
    /// Details for the scene being written, or the scene or note the
    /// outline is on.
    pub fn open_details(&mut self) {
        let idx = if self.focus == Focus::Editor {
            self.open
        } else {
            self.visible.get(self.sel).copied()
        };
        let Some(idx) = idx.filter(|&i| self.project.nodes[i].kind == Kind::Scene) else {
            self.msg = "details are for a scene or a note · pick one in the outline".into();
            return;
        };
        let n = &self.project.nodes[idx];
        let get = |k: &str| n.meta(k).unwrap_or_default();
        let manuscript = n.in_manuscript;
        let fields = if manuscript {
            vec![
                Field::Status,
                Field::Pov,
                Field::Synopsis,
                Field::Target,
                Field::InExports,
            ]
        } else {
            vec![Field::Aliases, Field::Synopsis]
        };
        let aliases = n
            .front
            .as_deref()
            .map(grimoire_core::codex::aliases)
            .unwrap_or_default()
            .join(", ");
        self.overlay = Overlay::Details {
            path: n.path.clone(),
            sel: 0,
            fields,
            status: n.status.clone().unwrap_or_default(),
            pov: get("pov"),
            synopsis: get("synopsis"),
            target: get("target"),
            compile: n.compile,
            aliases,
        };
    }

    pub(super) fn on_details_key(&mut self, key: Key) {
        let Overlay::Details {
            sel,
            fields,
            status,
            pov,
            synopsis,
            target,
            compile,
            aliases,
            ..
        } = &mut self.overlay
        else {
            return;
        };
        let field = fields[*sel];
        let text = match field {
            Field::Pov => Some(pov),
            Field::Synopsis => Some(synopsis),
            Field::Aliases => Some(aliases),
            _ => None,
        };
        match key {
            Key::Up | Key::BackTab => *sel = (*sel + fields.len() - 1) % fields.len(),
            Key::Down | Key::Tab => *sel = (*sel + 1) % fields.len(),
            Key::Enter => self.save_details(),
            Key::Esc => {
                self.overlay = Overlay::None;
                self.msg = "details left as they were".into();
            }
            Key::Left | Key::Right | Key::Char(' ') if field == Field::Status => {
                let at = STATUSES
                    .iter()
                    .position(|s| s.eq_ignore_ascii_case(status.trim()));
                let n = STATUSES.len();
                let next = match (at, key) {
                    (None, _) => 0,
                    (Some(i), Key::Left) => (i + n - 1) % n,
                    (Some(i), _) => (i + 1) % n,
                };
                *status = STATUSES[next].to_string();
            }
            Key::Left | Key::Right | Key::Char(' ') if field == Field::InExports => {
                *compile = !*compile;
            }
            Key::Char(c) if field == Field::Target && c.is_ascii_digit() && target.len() < 7 => {
                target.push(c)
            }
            Key::Backspace if field == Field::Target => {
                target.pop();
            }
            Key::Char(c) if !c.is_control() => {
                if let Some(t) = text
                    && t.chars().count() < 500
                {
                    t.push(c);
                }
            }
            Key::Backspace => {
                if let Some(t) = text {
                    t.pop();
                }
            }
            _ => {}
        }
    }

    fn save_details(&mut self) {
        let Overlay::Details {
            path,
            status,
            pov,
            synopsis,
            target,
            compile,
            aliases,
            fields,
            ..
        } = std::mem::replace(&mut self.overlay, Overlay::None)
        else {
            return;
        };
        let Some(idx) = self.project.nodes.iter().position(|n| n.path == path) else {
            self.msg = "that scene isn't there any more".into();
            return;
        };
        let node = &mut self.project.nodes[idx];
        // Only what changed is written, so a key nobody set isn't added.
        let set = |node: &mut project::Node, key: &str, now: &str| {
            if node.meta(key).unwrap_or_default().trim() != now.trim() {
                node.set_meta(key, now);
                true
            } else {
                false
            }
        };
        let mut changed = false;
        for f in &fields {
            changed |= match f {
                Field::Status => set(node, "status", status.as_str()),
                Field::Pov => set(node, "pov", pov.as_str()),
                Field::Synopsis => set(node, "synopsis", synopsis.as_str()),
                Field::Target => set(node, "target", target.as_str()),
                Field::InExports => {
                    if node.compile != compile {
                        node.set_meta("compile", if compile { "true" } else { "false" });
                        node.compile = compile;
                        true
                    } else {
                        false
                    }
                }
                Field::Aliases => {
                    let list: Vec<String> = aliases
                        .split(',')
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect();
                    let was = node
                        .front
                        .as_deref()
                        .map(grimoire_core::codex::aliases)
                        .unwrap_or_default();
                    if was != list {
                        node.set_meta_list("aliases", &list);
                        true
                    } else {
                        false
                    }
                }
            };
        }
        let title = self.project.nodes[idx].title.clone();
        if changed {
            self.mark_changed(idx);
            if fields.contains(&Field::Aliases) {
                self.rebuild_codex();
            }
            self.msg = format!("{title} · details saved");
        } else {
            self.msg = format!("{title} · nothing changed");
        }
    }
}

pub(super) fn draw_details(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Details {
        path,
        sel,
        fields,
        status,
        pov,
        synopsis,
        target,
        compile,
        aliases,
    } = &app.overlay
    else {
        return;
    };
    let title = app
        .project
        .nodes
        .iter()
        .find(|n| &n.path == path)
        .map(|n| n.title.to_uppercase())
        .unwrap_or_default();
    let box_area = centred(area, 70, fields.len() as u16 * 2 + 6);
    f.render_widget(Clear, box_area);
    let heading = format!("DETAILS · {title}");
    let block = pane_block(&heading, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let mut lines = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        let on = i == *sel;
        let (label, value, about) = match field {
            Field::Status => (
                "Status",
                format!("◂ {} ▸", if status.is_empty() { "none" } else { status }),
                "idea, outline, draft, revised, done",
            ),
            Field::Pov => ("POV", pov.clone(), "whose eyes the scene is seen through"),
            Field::Synopsis => ("Synopsis", synopsis.clone(), "what happens, in a line"),
            Field::Target => (
                "Word target",
                if target.is_empty() {
                    String::new()
                } else {
                    thousands(target.parse().unwrap_or(0))
                },
                "how long you mean it to be",
            ),
            Field::InExports => (
                "In exports",
                if *compile {
                    "◂ yes ▸"
                } else {
                    "◂ no, left out ▸"
                }
                .to_string(),
                "a scene left out stays in the book, just not the manuscript",
            ),
            Field::Aliases => (
                "Other names",
                aliases.clone(),
                "commas between: the notebook lights these up too",
            ),
        };
        let shown: String = {
            let room = (inner.width as usize).saturating_sub(18);
            let n = value.chars().count();
            if n > room {
                format!("…{}", value.chars().skip(n + 1 - room).collect::<String>())
            } else {
                value
            }
        };
        lines.push(
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(format!("{label:<13}"), Style::default().fg(t.text)),
                Span::styled(
                    shown,
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
                Span::styled(
                    if on
                        && matches!(
                            field,
                            Field::Pov | Field::Synopsis | Field::Target | Field::Aliases
                        )
                    {
                        "█"
                    } else {
                        ""
                    },
                    Style::default().fg(t.accent),
                ),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            }),
        );
        lines.push(Line::from(Span::styled(
            format!("                {about}"),
            dim.add_modifier(Modifier::ITALIC),
        )));
    }
    lines.push(Line::from(""));
    lines.push(hint_line(
        " ↑ ↓ move   type, or ← → to change   ↵ save   esc cancel",
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
