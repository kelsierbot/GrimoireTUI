//! Rendering. Panes, status line, and the theme picker overlay.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use crate::app::{App, Focus, Overlay};
use crate::music::State as MusicState;
use crate::project::Kind;
use crate::scene::{self, Ink, Mode, Phase};
use crate::theme::{self, Theme};

/// Wide enough that the scene's 28 columns fit inside the border.
pub const LEFT_W: u16 = (scene::W + 2) as u16;

const SCENE_H: u16 = (scene::H + 2) as u16;
const MUSIC_H: u16 = 4;
const MIN_TREE: u16 = 6;

pub fn draw(f: &mut Frame, app: &mut App) {
    // Cloned once per frame so sub-renderers can read the theme while `app`
    // stays mutably borrowed for scroll and rect bookkeeping.
    let t = app.theme.clone();

    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
    let [left, edit_area] =
        Layout::horizontal([Constraint::Length(LEFT_W), Constraint::Min(24)]).areas(main);

    // Give up the ornaments before the tree gets unusable.
    let (tree_area, scene_area, music_area) = if left.height >= SCENE_H + MUSIC_H + MIN_TREE {
        let [a, b, c] = Layout::vertical([
            Constraint::Min(MIN_TREE),
            Constraint::Length(SCENE_H),
            Constraint::Length(MUSIC_H),
        ])
        .areas(left);
        (a, b, c)
    } else if left.height >= MUSIC_H + MIN_TREE {
        let [a, c] =
            Layout::vertical([Constraint::Min(MIN_TREE), Constraint::Length(MUSIC_H)]).areas(left);
        (a, Rect::default(), c)
    } else {
        (left, Rect::default(), Rect::default())
    };

    app.scene_visible = scene_area.height > 0;
    app.music_visible = music_area.height > 0;
    app.rect_scene = Rect::default();
    app.rect_music = Rect::default();

    draw_tree(f, app, tree_area, &t);
    if app.scene_visible {
        draw_scene(f, app, scene_area, &t);
    }
    if app.music_visible {
        draw_music(f, app, music_area, &t);
    }
    draw_editor(f, app, edit_area, &t);
    draw_status(f, app, status, &t);

    if !matches!(app.overlay, Overlay::None) {
        draw_overlay(f, app, f.area(), &t);
    }
}

fn pane_block<'a>(title: &'a str, focused: bool, t: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { t.accent } else { t.border }))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused { t.accent } else { t.dim })
                .add_modifier(Modifier::BOLD),
        ))
}

fn draw_tree(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let focused = app.focus == Focus::Tree;
    let block = pane_block("MANUSCRIPT", focused, t);
    let inner = block.inner(area);
    app.rect_tree = inner;
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
                Style::default().fg(t.border),
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
        let overflow = left_w + count.chars().count() + 1 > width;
        let left = if overflow {
            truncate(&left, width.saturating_sub(count.chars().count() + 2))
        } else {
            left
        };
        let gap = if overflow {
            1
        } else {
            width
                .saturating_sub(left_w)
                .saturating_sub(count.chars().count() + 1)
        };

        let name_style = match n.kind {
            Kind::Container => Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            _ if Some(idx) == app.open => Style::default().fg(t.accent),
            _ => Style::default().fg(t.text),
        };
        let base = if selected && focused {
            Style::default().bg(t.sel)
        } else {
            Style::default()
        };

        lines.push(
            Line::from(vec![
                Span::styled(left, name_style.patch(base)),
                Span::styled(" ".repeat(gap), base),
                Span::styled(count, Style::default().fg(t.dim).patch(base)),
                Span::styled(" ", base),
            ])
            .style(base),
        );
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_scene(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    // The pane's title says which of the three views you're on, so ←/→ is
    // discoverable without a legend.
    let (label, grid) = match app.pane_mode {
        Mode::Clearing => (
            app.pomo.label(),
            scene::render(app.pomo.phase, app.pomo.progress(), app.frame),
        ),
        Mode::Spectrum => {
            let (title, frac, playing) = match &app.music.state {
                MusicState::Playing(tr) => (
                    tr.title.clone(),
                    if tr.duration > 0.0 {
                        tr.progress / tr.duration
                    } else {
                        0.0
                    },
                    tr.playing,
                ),
                _ => (String::new(), 0.0, false),
            };
            let head = if title.is_empty() {
                "spectrum".to_string()
            } else {
                truncate(&title, 22)
            };
            let note = app.viz.note(playing);
            (head, scene::render_spectrum(&app.viz, frac, note.as_deref()))
        }
        Mode::Growth => {
            let today = app.project.total_words().saturating_sub(app.baseline);
            let target = app.project.meta.daily_target.max(1);
            // New growth glints for two seconds. The first sighting just records
            // the step, so opening the view doesn't fake a milestone.
            let step = today / scene::WORDS_PER_STEP;
            match app.growth_step {
                Some(seen) if step > seen => app.growth_changed = Some(std::time::Instant::now()),
                _ => {}
            }
            app.growth_step = Some(step);
            let glint = app
                .growth_changed
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2));
            let next = scene::WORDS_PER_STEP - today % scene::WORDS_PER_STEP;
            (
                format!("today · {today} · next in {next}"),
                scene::render_growth(today, target, glint, app.frame),
            )
        }
    };

    let focused = app.focus == Focus::Clearing;
    let mut block = pane_block(&label, focused, t);
    // On a beat the frame flashes bloom, and fades back as the beat does.
    if app.pane_mode == Mode::Spectrum && app.viz.beat > 0.0 {
        let base = if focused { t.accent } else { t.border };
        block = block.border_style(Style::default().fg(blend(base, t.bloom, app.viz.beat)));
    }
    let inner = block.inner(area);
    app.rect_scene = inner;
    f.render_widget(block, area);

    let night = app.pomo.phase == Phase::Break && app.pane_mode == Mode::Clearing;
    let lit = |v: u8| v as f32 / 255.0;

    let lines: Vec<Line> = grid
        .iter()
        .map(|row| {
            Line::from(
                row.iter()
                    .map(|&(ch, ink)| {
                        let col = match ink {
                            Ink::Sky => t.border,
                            Ink::Star => {
                                if night {
                                    t.moon
                                } else {
                                    t.dim
                                }
                            }
                            Ink::Sun => t.sun,
                            Ink::Moon => t.moon,
                            Ink::Tree => t.foliage,
                            Ink::Trunk => t.bark,
                            Ink::Rabbit => t.text,
                            Ink::Flower => t.bloom,
                            Ink::Ground => t.turf,
                            Ink::Bar { h, glow } => blend(bar_colour(t, h), t.text, lit(glow) * 0.6),
                            Ink::Pond { h, depth } => {
                                blend(bar_colour(t, h), t.border, 0.45 + 0.2 * depth as f32)
                            }
                            Ink::Cap { heat } => blend(t.dim, t.moon, lit(heat)),
                            Ink::Spark { life } => blend(t.dim, blend(t.sun, t.text, 0.35), lit(life)),
                            Ink::Played { glow } => blend(t.accent, t.bloom, lit(glow)),
                        };
                        Span::styled(ch.to_string(), Style::default().fg(col))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

/// Mix two truecolour colours, `f` of the way from `a` to `b`. Anything that
/// isn't RGB just switches over halfway.
fn blend(a: ratatui::style::Color, b: ratatui::style::Color, f: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    let f = f.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
            Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
        }
        _ if f < 0.5 => a,
        _ => b,
    }
}

/// A spectrum bar's colour at height `h`, 0 at the roots to 255 at the tip:
/// the theme's foliage, up through its accent and sun, to bloom at the very
/// top. Drawn from the theme, so every palette gets its own.
fn bar_colour(t: &Theme, h: u8) -> ratatui::style::Color {
    let x = h as f32 / 255.0;
    let stops = [(0.0, t.foliage), (0.4, t.accent), (0.75, t.sun), (1.0, t.bloom)];
    for w in stops.windows(2) {
        let ((a, from), (b, to)) = (w[0], w[1]);
        if x <= b {
            return blend(from, to, (x - a) / (b - a));
        }
    }
    t.bloom
}

fn draw_music(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let title = format!("♪ {} · F7", app.music.source.label());
    let block = pane_block(&title, app.focus == Focus::Music, t);
    let inner = block.inner(area);
    app.rect_music = inner;
    f.render_widget(block, area);
    let w = inner.width as usize;

    let lines: Vec<Line> = match &app.music.state {
        MusicState::NoToken => vec![
            Line::from(Span::styled("not set up", Style::default().fg(t.dim))),
            Line::from(Span::styled(
                "grimoire music-setup",
                Style::default().fg(t.border),
            )),
        ],
        MusicState::Offline => vec![
            Line::from(Span::styled(
                format!("{} not running", app.music.source.label()),
                Style::default().fg(t.dim),
            )),
            Line::from(Span::styled(
                "F1 to switch source",
                Style::default().fg(t.border),
            )),
        ],
        MusicState::Idle => vec![
            Line::from(Span::styled("connected", Style::default().fg(t.dim))),
            Line::from(Span::styled(
                "nothing playing",
                Style::default().fg(t.border),
            )),
        ],
        MusicState::Playing(tr) => {
            let glyph = if tr.playing { "▶" } else { "❚❚" };
            let head = format!("{glyph} {}", tr.title);
            let frac = if tr.duration > 0.0 {
                (tr.progress / tr.duration).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let barw = w.saturating_sub(7).max(4);
            let filled = (frac * barw as f64).round() as usize;
            let mins = |s: f64| format!("{:.0}:{:02.0}", (s / 60.0).floor(), s % 60.0);

            vec![
                Line::from(Span::styled(
                    truncate(&head, w),
                    Style::default().fg(if tr.playing { t.accent } else { t.dim }),
                )),
                // Different weights, not just different colours, so the bar
                // still reads on a monochrome terminal.
                Line::from(vec![
                    Span::styled("━".repeat(filled.min(barw)), Style::default().fg(t.accent)),
                    Span::styled(
                        "─".repeat(barw.saturating_sub(filled)),
                        Style::default().fg(t.border),
                    ),
                    Span::styled(format!(" {}", mins(tr.progress)), Style::default().fg(t.dim)),
                ]),
            ]
        }
    };

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let focused = app.focus == Focus::Editor;
    let title = app.open_title();
    let block = pane_block(&title, focused, t).padding(Padding::new(2, 2, 0, 0));
    let inner = block.inner(area);
    app.rect_editor = inner;
    f.render_widget(block, area);

    app.edit_width = inner.width as usize;
    app.edit_height = inner.height as usize;

    if app.open.is_none() {
        let hint = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Select a scene on the left and press Enter.",
                Style::default().fg(t.dim),
            )),
        ];
        f.render_widget(Paragraph::new(hint), inner);
        return;
    }

    let rows = app.editor.layout(app.edit_width);
    app.editor.clamp_scroll(&rows, app.edit_height);

    // Paint the drag selection by splitting each row into up to three runs.
    let visible: Vec<Line> = rows
        .iter()
        .skip(app.editor.scroll)
        .take(app.edit_height)
        .map(|&r| {
            let plain = Style::default().fg(t.text);
            match app.editor.row_selection(r) {
                None => Line::from(Span::styled(app.editor.row_text(r), plain)),
                Some((from, to)) => {
                    let seg = |a: usize, b: usize| app.editor.row_text(scene_slice(r, a, b));
                    Line::from(vec![
                        Span::styled(seg(r.start, from), plain),
                        Span::styled(
                            seg(from, to),
                            Style::default().fg(t.text).bg(t.sel),
                        ),
                        Span::styled(seg(to, r.end), plain),
                    ])
                }
            }
        })
        .collect();

    f.render_widget(Paragraph::new(visible), inner);

    if focused && matches!(app.overlay, Overlay::None) {
        let (r, col) = app.editor.cursor_vis(&rows);
        if r >= app.editor.scroll && r < app.editor.scroll + app.edit_height {
            let x = inner.x + col.min(app.edit_width.saturating_sub(1)) as u16;
            let y = inner.y + (r - app.editor.scroll) as u16;
            f.set_cursor_position((x, y));
        }
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let total = app.project.total_words();
    let target = app.project.meta.target_words.max(1);
    let filled = (total * 10 / target).min(10);
    let bar: String = "▓".repeat(filled) + &"░".repeat(10 - filled);
    let today = total.saturating_sub(app.baseline);

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(thousands(total), Style::default().fg(t.text)),
        Span::styled(
            format!(" / {} ", thousands(target)),
            Style::default().fg(t.dim),
        ),
        Span::styled(bar, Style::default().fg(t.accent)),
        Span::styled(
            format!("  today {}", thousands(today)),
            Style::default().fg(if today >= app.project.meta.daily_target {
                t.accent
            } else {
                t.dim
            }),
        ),
    ];

    let dirty = app.project.dirty_count();
    if dirty > 0 {
        spans.push(Span::styled(
            format!("  ● {dirty} unsaved"),
            Style::default().fg(t.warn),
        ));
    }
    if !app.msg.is_empty() {
        spans.push(Span::styled(
            format!("  {}", app.msg),
            Style::default().fg(t.accent),
        ));
    }

    let hints = app.hints();
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize)
        .saturating_sub(used)
        .saturating_sub(hints.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(hints, Style::default().fg(t.dim)));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── overlays ─────────────────────────────────────────────────────────

fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_overlay(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    match &app.overlay {
        Overlay::None => {}

        Overlay::Menu { sel } => {
            let items = App::MENU;
            let box_area = centred(area, 42, items.len() as u16 + 4);
            f.render_widget(Clear, box_area);
            let block = pane_block("GRIMOIRE", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);

            let mut lines: Vec<Line> = items
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let on = i == *sel;
                    Line::from(vec![
                        Span::styled(
                            if on { " ▸ " } else { "   " },
                            Style::default().fg(if on { t.accent } else { t.dim }),
                        ),
                        Span::styled(
                            *label,
                            Style::default().fg(if on { t.accent } else { t.text }),
                        ),
                    ])
                    .style(if on {
                        Style::default().bg(t.sel)
                    } else {
                        Style::default()
                    })
                })
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " j/k move   ↵ choose   esc close",
                Style::default().fg(t.dim),
            )));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Player { tab, sel, query, find, typing, .. } => {
            let box_area = centred(area, area.width.saturating_sub(4).min(100), area.height.saturating_sub(2).min(30));
            f.render_widget(Clear, box_area);
            let title = format!("♪ {}", app.music.source.label().to_uppercase());
            let block = pane_block(&title, true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let iw = inner.width as usize;
            let dim = Style::default().fg(t.dim);
            let accent = Style::default().fg(t.accent);
            let mins = |s: f64| format!("{:.0}:{:02.0}", (s / 60.0).floor(), s % 60.0);
            let mut lines: Vec<Line> = Vec::new();

            // Now playing, with a real progress bar.
            match &app.music.state {
                MusicState::Playing(tr) => {
                    lines.push(Line::from(vec![
                        Span::styled(if tr.playing { " ▶ " } else { " ❚❚ " }, accent),
                        Span::styled(truncate(&tr.title, iw.saturating_sub(26).max(8)), Style::default().fg(t.text)),
                        Span::styled(format!("  {}", truncate(&tr.artist, 22)), dim),
                    ]));
                    let times = format!(" {} / {}", mins(tr.progress), mins(tr.duration));
                    let barw = iw.saturating_sub(times.chars().count() + 1).max(4);
                    let frac = if tr.duration > 0.0 { (tr.progress / tr.duration).clamp(0.0, 1.0) } else { 0.0 };
                    let filled = ((frac * barw as f64).round() as usize).min(barw);
                    lines.push(Line::from(vec![
                        Span::raw(" "),
                        Span::styled("━".repeat(filled), accent),
                        Span::styled("─".repeat(barw - filled), Style::default().fg(t.border)),
                        Span::styled(times, dim),
                    ]));
                }
                other => {
                    let why = match other {
                        MusicState::NoToken => "not set up yet: F1 › Music source, then run grimoire music-setup".to_string(),
                        MusicState::Offline => format!("{} isn't running. Open it and this fills in.", app.music.source.label()),
                        _ => "nothing playing. Pick something below.".to_string(),
                    };
                    lines.push(Line::from(Span::styled(format!(" {why}"), dim)));
                    lines.push(Line::from(""));
                }
            }
            lines.push(Line::from(""));

            // Tabs, and what the list below is showing.
            use crate::app::Tab;
            let on = accent.add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED);
            let style = |which: Tab| if *tab == which { on } else { dim };
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("QUEUE · {}", app.music.queue.len()), style(Tab::Queue)),
                Span::raw("    "),
                Span::styled("PLAYLISTS", style(Tab::Playlists)),
                Span::raw("    "),
                Span::styled("SEARCH", style(Tab::Search)),
                Span::styled("    tab switches", dim),
            ]));
            let prompt = |label: &str, buf: &str| {
                Line::from(vec![
                    Span::styled(format!(" {label} ▸ "), accent),
                    Span::styled(buf.to_string(), Style::default().fg(t.text)),
                    Span::styled("█", accent),
                ])
            };
            match tab {
                Tab::Search => lines.push(if *typing {
                    prompt("find", query)
                } else if query.is_empty() {
                    Line::from(Span::styled(" press / and type a song or an artist", dim))
                } else {
                    Line::from(Span::styled(format!(" results for “{query}” · / to search again"), dim))
                }),
                Tab::Playlists => lines.push(if *typing {
                    prompt("playlists", find)
                } else if find.is_empty() {
                    Line::from(Span::styled(" your library · / searches every playlist on YouTube Music", dim))
                } else {
                    Line::from(Span::styled(format!(" playlists matching “{find}” · esc for yours"), dim))
                }),
                Tab::Queue => {}
            }
            lines.push(Line::from(""));

            // The list, scrolled to keep the selection in view.
            let footer = 3;
            let room = (inner.height as usize).saturating_sub(lines.len() + footer).max(1);
            let items = match tab {
                Tab::Queue => &app.music.queue,
                Tab::Playlists => &app.music.playlists,
                Tab::Search => &app.music.results,
            };
            if items.is_empty() && *tab == Tab::Queue {
                lines.push(Line::from(Span::styled(
                    " the queue is empty. Tab over to your playlists, or search with /",
                    dim,
                )));
            }
            let start = sel.saturating_sub(room / 2).min(items.len().saturating_sub(room));
            // Playlists show a song count where tracks show a length.
            let len_w = if *tab == Tab::Playlists { 11 } else { 6 };
            let artist_w = (iw / 4).clamp(8, 28);
            let title_w = iw.saturating_sub(artist_w + 8 + len_w);
            for (i, it) in items.iter().enumerate().skip(start).take(room) {
                let who = if it.video { format!("{} · video", it.artist) } else { it.artist.clone() };
                let row = Line::from(vec![
                    Span::styled(format!(" {}{:>3} ", if it.current { "▶" } else { " " }, i + 1), if it.current { accent } else { dim }),
                    Span::styled(
                        format!("{:<title_w$}", truncate(&it.title, title_w)),
                        Style::default().fg(if it.current { t.accent } else { t.text }),
                    ),
                    Span::styled(format!(" {:<artist_w$}", truncate(&who, artist_w)), dim),
                    Span::styled(format!("{:>len_w$}", it.length), dim),
                ]);
                lines.push(if i == *sel { row.style(Style::default().bg(t.sel)) } else { row });
            }

            while lines.len() < (inner.height as usize).saturating_sub(footer) {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(format!(" {}", app.music.note.clone().unwrap_or_default()), accent)));
            let controls = " space pause  ←→ seek 10s  [ ] prev/next  s shuffle  r repeat  +/- volume  l like  esc close";
            let keys = match tab {
                Tab::Queue => " ↵ play this one   ↑↓ choose   / search   tab playlists",
                Tab::Playlists => " ↵ play playlist   a play it next   / find playlists   ↑↓ choose   tab search",
                Tab::Search => " / search   ↵ play now   a play next   ↑↓ choose   tab queue",
            };
            lines.extend([keys, controls].map(|k| Line::from(Span::styled(k, dim))));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Create { folder, label, buf, .. } => {
            let box_area = centred(area, 56, 7);
            f.render_widget(Clear, box_area);
            let block = pane_block(if *folder { "NEW FOLDER" } else { "NEW SCENE" }, true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let lines = vec![
                Line::from(Span::styled(
                    format!(" at the end of {label}"),
                    Style::default().fg(t.dim),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" ▸ ", Style::default().fg(t.accent)),
                    Span::styled(buf.clone(), Style::default().fg(t.text)),
                    Span::styled("█", Style::default().fg(t.accent)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    " type a name   ↵ create   esc cancel",
                    Style::default().fg(t.dim),
                )),
            ];
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Sources { sel } => {
            let all = crate::music::Source::ALL;
            let box_area = centred(area, 52, all.len() as u16 + 5);
            f.render_widget(Clear, box_area);
            let block = pane_block("MUSIC SOURCE", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);

            let mut lines: Vec<Line> = all
                .iter()
                .enumerate()
                .map(|(i, src)| {
                    let on = i == *sel;
                    let note = if src.plays_audio() {
                        "your server · plays here"
                    } else {
                        "remote control"
                    };
                    Line::from(vec![
                        Span::styled(
                            if on { " ● " } else { " • " },
                            Style::default().fg(if on { t.accent } else { t.dim }),
                        ),
                        Span::styled(
                            format!("{:<15}", src.label()),
                            Style::default().fg(if on { t.accent } else { t.text }),
                        ),
                        Span::styled(note, Style::default().fg(t.dim)),
                    ])
                    .style(if on {
                        Style::default().bg(t.sel)
                    } else {
                        Style::default()
                    })
                })
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " j/k move   ↵ choose   esc close",
                Style::default().fg(t.dim),
            )));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Themes { sel, .. } => {
            let names = app.theme_names();
            let box_area = centred(area, 40, names.len() as u16 + 4);
            f.render_widget(Clear, box_area);
            let block = pane_block("THEME", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);

            let mut lines: Vec<Line> = names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let on = i == *sel;
                    Line::from(vec![
                        Span::styled(
                            if on { " ● " } else { " • " },
                            Style::default().fg(if on { t.accent } else { t.dim }),
                        ),
                        Span::styled(
                            n.clone(),
                            Style::default().fg(if on { t.accent } else { t.text }),
                        ),
                    ])
                    .style(if on {
                        Style::default().bg(t.sel)
                    } else {
                        Style::default()
                    })
                })
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " j/k preview   ↵ apply   esc cancel",
                Style::default().fg(t.dim),
            )));
            f.render_widget(Paragraph::new(lines), inner);
        }
        Overlay::Custom { field, buf } => {
            let box_area = centred(area, 46, theme::ROLES.len() as u16 + 4);
            f.render_widget(Clear, box_area);
            let block = pane_block("CUSTOM THEME", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);

            let mut lines: Vec<Line> = theme::ROLES
                .iter()
                .enumerate()
                .map(|(i, role)| {
                    let on = i == *field;
                    let col = app.theme.role(i);
                    let shown = if on && !buf.is_empty() {
                        format!("#{buf}")
                    } else {
                        theme::hex_of(col)
                    };
                    Line::from(vec![
                        Span::styled(
                            if on { " ▸ " } else { "   " },
                            Style::default().fg(t.accent),
                        ),
                        Span::styled(format!("{role:<10}"), Style::default().fg(t.text)),
                        Span::styled("██ ", Style::default().fg(col)),
                        Span::styled(
                            shown,
                            Style::default().fg(if on { t.accent } else { t.dim }),
                        ),
                    ])
                    .style(if on {
                        Style::default().bg(t.sel)
                    } else {
                        Style::default()
                    })
                })
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " type hex · ↵ next · esc save & close",
                Style::default().fg(t.dim),
            )));
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

/// A sub-range of a visual row, for painting selection runs.
fn scene_slice(r: crate::editor::VisRow, start: usize, end: usize) -> crate::editor::VisRow {
    crate::editor::VisRow {
        line: r.line,
        start,
        end,
    }
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
