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
    lines.push(hint_line(" esc close   g goals", t));

    // A short window keeps the top and the hint; the weeks give way.
    let room_h = inner.height as usize;
    if lines.len() > room_h && room_h > 2 {
        let hint = lines.pop();
        lines.truncate(room_h - 1);
        lines.extend(hint);
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The presets on the Goals page: a name, the book's goal, a day's.
pub const PRESETS: [(&str, usize, usize); 3] = [
    ("A month of drafting", 50_000, 1_667),
    ("A short story", 5_000, 500),
    ("A novel", 80_000, 1_000),
];

impl App {
    pub(super) fn on_goals_key(&mut self, key: Key) {
        let Overlay::Goals { sel, book, day } = &mut self.overlay else {
            return;
        };
        let rows = 2 + PRESETS.len();
        match key {
            Key::Up | Key::BackTab => *sel = (*sel + rows - 1) % rows,
            Key::Down | Key::Tab => *sel = (*sel + 1) % rows,
            Key::Char(c) if c.is_ascii_digit() && *sel < 2 => {
                let field = if *sel == 0 { book } else { day };
                if field.len() < 7 {
                    field.push(c);
                }
            }
            Key::Backspace if *sel < 2 => {
                let field = if *sel == 0 { book } else { day };
                field.pop();
            }
            // A preset fills both numbers in; ↵ on a number keeps them.
            Key::Enter if *sel >= 2 => {
                let (_, b, d) = PRESETS[*sel - 2];
                *book = b.to_string();
                *day = d.to_string();
                *sel = 0;
            }
            Key::Enter => {
                let (b, d) = (book.parse().unwrap_or(0), day.parse().unwrap_or(0));
                self.save_goals(b, d);
            }
            Key::Esc => {
                self.overlay = Overlay::None;
                self.msg = "goals left as they were".into();
            }
            _ => {}
        }
    }

    fn save_goals(&mut self, book: usize, day: usize) {
        if book == 0 || day == 0 {
            self.msg = "a goal needs a number above zero".into();
            return;
        }
        match grimoire_core::submission::save_goals(&self.project.root, book, day) {
            Ok(()) => {
                self.project.meta.target_words = book;
                self.project.meta.daily_target = day;
                self.overlay = Overlay::None;
                self.msg = format!(
                    "goals saved: {} words for the book, {} a day",
                    thousands(book),
                    thousands(day)
                );
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
    }
}

pub(super) fn draw_goals(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Goals { sel, book, day } = &app.overlay else {
        return;
    };
    let box_area = centred(area, 60, 14);
    f.render_widget(Clear, box_area);
    let title = format!("GOALS · {}", app.project.meta.title.to_uppercase());
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let dim = Style::default().fg(t.dim);
    let row = |i: usize, spans: Vec<Span<'static>>| {
        let on = i == *sel;
        let mut all = vec![Span::styled(
            if on { " ▸ " } else { "   " },
            Style::default().fg(if on { t.accent } else { t.dim }),
        )];
        all.extend(spans);
        Line::from(all).style(if on {
            Style::default().bg(t.sel)
        } else {
            Style::default()
        })
    };
    let number = |i: usize, label: &str, value: &str| {
        let on = i == *sel;
        row(
            i,
            vec![
                Span::styled(format!("{label:<12}"), Style::default().fg(t.text)),
                Span::styled(
                    value.to_string(),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
                Span::styled(
                    if on { "█" } else { "" }.to_string(),
                    Style::default().fg(t.accent),
                ),
                Span::styled(" words".to_string(), dim),
            ],
        )
    };
    let mut lines = vec![
        number(0, "The book", book),
        number(1, "Each day", day),
        Line::from(""),
        Line::from(Span::styled(" Or start from one of these:", dim)),
    ];
    for (k, (name, b, d)) in PRESETS.iter().enumerate() {
        lines.push(row(
            k + 2,
            vec![
                Span::styled(format!("{name:<22}"), Style::default().fg(t.text)),
                Span::styled(
                    format!("{} words, {} a day", thousands(*b), thousands(*d)),
                    dim,
                ),
            ],
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " No streaks: a day you don't write is just a day.",
        Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));
    lines.push(hint_line(
        if *sel < 2 {
            " type a number   ↑ ↓ move   ↵ save   esc cancel"
        } else {
            " ↵ use these   ↑ ↓ move   esc cancel"
        },
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
