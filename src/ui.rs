//! Rendering. Two panes and a status line — tree on the left, prose on the right.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus};
use crate::project::Kind;

const ACCENT: Color = Color::Rgb(214, 173, 96);
const DIM: Color = Color::Rgb(108, 108, 122);
const SEL_BG: Color = Color::Rgb(48, 48, 60);
const BORDER: Color = Color::Rgb(70, 70, 84);
const TEXT: Color = Color::Rgb(222, 220, 212);

pub const TREE_WIDTH: u16 = 26;

pub fn draw(f: &mut Frame, app: &mut App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
    let [tree_area, edit_area] =
        Layout::horizontal([Constraint::Length(TREE_WIDTH), Constraint::Min(20)]).areas(main);

    draw_tree(f, app, tree_area);
    draw_editor(f, app, edit_area);
    draw_status(f, app, status);
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let border = if focused { ACCENT } else { BORDER };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused { ACCENT } else { DIM })
                .add_modifier(Modifier::BOLD),
        ))
}

fn draw_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Tree;
    let block = pane_block("MANUSCRIPT", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let height = inner.height as usize;
    if app.sel < app.tree_scroll {
        app.tree_scroll = app.sel;
    } else if height > 0 && app.sel >= app.tree_scroll + height {
        app.tree_scroll = app.sel + 1 - height;
    }

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (row, &idx) in app
        .visible
        .iter()
        .enumerate()
        .skip(app.tree_scroll)
        .take(height)
    {
        let n = &app.project.nodes[idx];
        let selected = row == app.sel;

        if n.kind == Kind::Divider {
            let label = format!("── {} ", n.title);
            let pad = width.saturating_sub(label.chars().count());
            lines.push(Line::from(Span::styled(
                format!("{label}{}", "─".repeat(pad)),
                Style::default().fg(BORDER),
            )));
            continue;
        }

        let indent = "  ".repeat(n.depth);
        let marker = match n.kind {
            Kind::Container => {
                if n.expanded {
                    "▾ "
                } else {
                    "▸ "
                }
            }
            Kind::Scene => {
                if Some(idx) == app.open {
                    "● "
                } else {
                    "• "
                }
            }
            Kind::Divider => "",
        };

        let words = app.project.subtree_words(idx);
        let count = if words > 0 {
            thousands(words)
        } else {
            String::new()
        };

        let left = format!("{indent}{marker}{}", n.title);
        let left_w = left.chars().count();
        let gap = width
            .saturating_sub(left_w)
            .saturating_sub(count.chars().count() + 1);
        let left = if left_w + count.chars().count() + 1 > width {
            truncate(&left, width.saturating_sub(count.chars().count() + 2))
        } else {
            left
        };
        let gap = if left_w + count.chars().count() + 1 > width {
            1
        } else {
            gap
        };

        let name_style = match n.kind {
            Kind::Container => Style::default()
                .fg(TEXT)
                .add_modifier(Modifier::BOLD),
            _ if Some(idx) == app.open => Style::default().fg(ACCENT),
            _ => Style::default().fg(TEXT),
        };
        let base = if selected && focused {
            Style::default().bg(SEL_BG)
        } else {
            Style::default()
        };

        lines.push(
            Line::from(vec![
                Span::styled(left, name_style.patch(base)),
                Span::styled(" ".repeat(gap), base),
                Span::styled(count, Style::default().fg(DIM).patch(base)),
                Span::styled(" ", base),
            ])
            .style(base),
        );
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Editor;
    let title = app.open_title();
    let block = pane_block(&title, focused).padding(Padding::new(2, 2, 0, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);

    app.edit_width = inner.width as usize;
    app.edit_height = inner.height as usize;

    if app.open.is_none() {
        let hint = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Select a scene on the left and press Enter.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(Paragraph::new(hint), inner);
        return;
    }

    let rows = app.editor.layout(app.edit_width);
    app.editor.clamp_scroll(&rows, app.edit_height);

    let visible: Vec<Line> = rows
        .iter()
        .skip(app.editor.scroll)
        .take(app.edit_height)
        .map(|&r| {
            Line::from(Span::styled(
                app.editor.row_text(r),
                Style::default().fg(TEXT),
            ))
        })
        .collect();

    f.render_widget(Paragraph::new(visible), inner);

    if focused {
        let (r, col) = app.editor.cursor_vis(&rows);
        if r >= app.editor.scroll && r < app.editor.scroll + app.edit_height {
            let x = inner.x + col.min(app.edit_width.saturating_sub(1)) as u16;
            let y = inner.y + (r - app.editor.scroll) as u16;
            f.set_cursor_position((x, y));
        }
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let total = app.project.total_words();
    let target = app.project.meta.target_words.max(1);
    let filled = (total * 10 / target).min(10);
    let bar: String = "▓".repeat(filled) + &"░".repeat(10 - filled);
    let today = total.saturating_sub(app.baseline);

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(thousands(total), Style::default().fg(TEXT)),
        Span::styled(
            format!(" / {} ", thousands(target)),
            Style::default().fg(DIM),
        ),
        Span::styled(bar, Style::default().fg(ACCENT)),
        Span::styled(
            format!("  today {}", thousands(today)),
            Style::default().fg(if today >= app.project.meta.daily_target {
                ACCENT
            } else {
                DIM
            }),
        ),
    ];

    let dirty = app.project.dirty_count();
    if dirty > 0 {
        spans.push(Span::styled(
            format!("  ● {dirty} unsaved"),
            Style::default().fg(Color::Rgb(200, 120, 100)),
        ));
    }
    if !app.msg.is_empty() {
        spans.push(Span::styled(
            format!("  {}", app.msg),
            Style::default().fg(ACCENT),
        ));
    }

    let hints = "Tab pane  ^S save  ^Q quit ";
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize)
        .saturating_sub(used)
        .saturating_sub(hints.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(hints, Style::default().fg(DIM)));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
