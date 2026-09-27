//! The Progress page: the book against its goal, the words of each recent
//! week, the pace lately and roughly when the goal comes at it. Nothing to
//! keep up with: no streaks, no days in a row, and a quiet week is just a
//! short bar.

use super::*;
use grimoire_core::days;

/// How many weeks the page shows.
const WEEKS: usize = 8;
const BOX_W: u16 = 64;

pub(super) fn draw_progress(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let today = chrono::Local::now().date_naive();
    let record = days::read(&app.project.root);
    let weeks = days::weeks(&record, today, WEEKS);
    let pace = days::pace(&record, today);
    let meta = &app.project.meta;
    let total = app.project.total_words();
    let goal = meta.target_words.max(1);

    let box_h = (weeks.len() as u16 + 12).min(area.height);
    let box_area = centred(area, BOX_W, box_h);
    f.render_widget(Clear, box_area);
    let title = format!("PROGRESS · {}", meta.title.to_uppercase());
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    let dim = Style::default().fg(t.dim);
    let text = Style::default().fg(t.text);
    let accent = Style::default().fg(t.accent);
    let width = inner.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    // The book against its goal.
    let share = (total as f64 / goal as f64).min(1.0);
    let bar_w = width.saturating_sub(7);
    let filled = (share * bar_w as f64).round() as usize;
    lines.push(Line::from(vec![
        Span::styled(format!(" {}", thousands(total)), text),
        Span::styled(format!(" of {} words", thousands(goal)), dim),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("█".repeat(filled), accent),
        Span::styled("░".repeat(bar_w - filled), dim),
        Span::styled(
            if total > 0 && share < 0.01 {
                "  <1%".to_string()
            } else {
                format!(" {:>3}%", (share * 100.0).floor() as u32)
            },
            dim,
        ),
    ]));
    lines.push(Line::from(""));

    // Each week, as a bar against the best of them.
    lines.push(Line::from(Span::styled(" Each week", dim)));
    let most = weeks
        .iter()
        .map(|&(_, w)| w.max(0))
        .max()
        .unwrap_or(0)
        .max(1);
    let this_week = days::week_of(today);
    let label_w = 11;
    let count_w = 8;
    let room = width.saturating_sub(label_w + count_w + 2);
    for (i, &(monday, words)) in weeks.iter().enumerate() {
        let label = if monday == this_week {
            "this week".to_string()
        } else {
            monday.format("%b %-d").to_string()
        };
        let len = (words.max(0) as f64 / most as f64 * room as f64).round() as usize;
        let colour = if t.is_rainbow() {
            crate::theme::hue(i as f32 * 360.0 / WEEKS as f32)
        } else {
            t.accent
        };
        let count = if words < 0 {
            format!("−{}", thousands(words.unsigned_abs() as usize))
        } else {
            thousands(words as usize)
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {label:<w$}", w = label_w - 1), dim),
            Span::styled(
                if len == 0 && words > 0 {
                    "▏".to_string()
                } else {
                    "█".repeat(len)
                },
                Style::default().fg(colour),
            ),
            Span::styled(format!(" {count}"), text),
        ]));
    }
    lines.push(Line::from(""));

    // The pace, and where it leads.
    let mut said: Vec<Line> = Vec::new();
    match pace {
        Some(p) => {
            said.push(Line::from(vec![
                Span::styled(" Lately you write about ", dim),
                Span::styled(
                    format!("{} words a week", thousands(p.round() as usize)),
                    text,
                ),
                Span::styled(".", dim),
            ]));
            match days::arrives(total, goal, pace, today) {
                Some(when) => said.push(Line::from(vec![
                    Span::styled(" At that pace, you'll reach your goal around ", dim),
                    Span::styled(when.format("%B %-d").to_string(), accent),
                    Span::styled(".", dim),
                ])),
                None if total >= goal => said.push(Line::from(Span::styled(
                    " You've reached your goal. Lovely work.",
                    accent,
                ))),
                None => said.push(Line::from(Span::styled(
                    " Your goal is a long way off at that pace, and that's fine.",
                    dim,
                ))),
            }
        }
        None if total >= goal => said.push(Line::from(Span::styled(
            " You've reached your goal. Lovely work.",
            accent,
        ))),
        None => said.push(Line::from(Span::styled(
            " Write a little, and your pace shows up here.",
            dim,
        ))),
    }
    lines.extend(said);
    let written = app.today_words();
    lines.push(Line::from(vec![
        Span::styled(" Today: ", dim),
        Span::styled(
            if written < 0 {
                format!("−{} words", thousands(written.unsigned_abs() as usize))
            } else {
                format!("{} words", thousands(written as usize))
            },
            if written >= meta.daily_target as i64 {
                accent
            } else {
                text
            },
        ),
        Span::styled(
            format!(" of the {} you aim for", thousands(meta.daily_target)),
            dim,
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(hint_line(" esc close   goals live in novel.toml", t));

    // A short window keeps the top and the hint; the weeks give way.
    let room_h = inner.height as usize;
    if lines.len() > room_h && room_h > 2 {
        let hint = lines.pop();
        lines.truncate(room_h - 1);
        lines.extend(hint);
    }
    f.render_widget(Paragraph::new(lines), inner);
}
