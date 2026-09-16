//! Rendering. Panes, status line, and the theme picker overlay.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use crate::app::{App, Focus, Overlay};
use crate::create::{self, New};
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
    app.screen = (f.area().width, f.area().height);

    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
    let [left, edit_area] =
        Layout::horizontal([Constraint::Length(LEFT_W), Constraint::Min(24)]).areas(main);

    // Give up the ornaments before the tree gets unusable. With music switched
    // off there is no music pane at all, and the tree has that room.
    let music_h = if app.music.enabled { MUSIC_H } else { 0 };
    let (tree_area, scene_area, music_area) = if !app.music.enabled && left.height >= SCENE_H + MIN_TREE {
        let [a, b] =
            Layout::vertical([Constraint::Min(MIN_TREE), Constraint::Length(SCENE_H)]).areas(left);
        (a, b, Rect::default())
    } else if !app.music.enabled {
        (left, Rect::default(), Rect::default())
    } else if left.height >= SCENE_H + music_h + MIN_TREE {
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
        let dragging_to = app.tree_drag.is_some_and(|(from, to)| from != to && row == to);
        let base = if dragging_to {
            Style::default().bg(t.sel).add_modifier(Modifier::UNDERLINED)
        } else if selected && focused {
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
    draw_create_keys(f, app, area, focused, t);
}

/// The create keys, on the tree's bottom edge where the eye already is —
/// "n scene  c chapter  p part". Clicking one does what pressing it would.
fn draw_create_keys(f: &mut Frame, app: &mut App, area: Rect, focused: bool, t: &Theme) {
    app.create_hits.clear();
    if area.height < 3 || area.width < 4 {
        return;
    }
    let row = Rect {
        x: area.x + 1,
        y: area.y + area.height - 1,
        width: area.width - 2,
        height: 1,
    };
    let (key_style, word_style) = if focused {
        (
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            Style::default().fg(t.text),
        )
    } else {
        (Style::default().fg(t.dim), Style::default().fg(t.dim))
    };
    let sel = app.visible.get(app.sel).copied();
    let mut spans = vec![Span::raw(" ")];
    let mut x = row.x + 1;
    let offers = create::offers(&app.project, sel);
    for (i, (key, word)) in offers.iter().enumerate() {
        let gap = if i > 0 { 2 } else { 0 };
        let width = 2 + word.chars().count() as u16;
        if x + gap + width > row.x + row.width {
            break;
        }
        spans.push(Span::raw(" ".repeat(gap as usize)));
        x += gap;
        spans.push(Span::styled(key.to_string(), key_style));
        spans.push(Span::styled(format!(" {word}"), word_style));
        if let Some(want) = New::from_key(*key) {
            app.create_hits.push((Rect { x, y: row.y, width, height: 1 }, want));
        }
        x += width;
    }
    spans.push(Span::raw(" "));
    f.render_widget(Paragraph::new(Line::from(spans)), row);
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

    // Each row is painted in layers, char by char, then merged into runs:
    // find matches, then the selection on top.
    let find = match &app.overlay {
        Overlay::Find { query, .. } if !query.is_empty() => Some(query.as_str()),
        _ => None,
    };
    let match_bg = blend(t.border, t.sun, 0.45);
    let mut line_matches: std::collections::HashMap<usize, Vec<(usize, usize)>> = std::collections::HashMap::new();
    let mut line_spelling: std::collections::HashMap<usize, Vec<(usize, usize)>> = std::collections::HashMap::new();
    let visible: Vec<Line> = rows
        .iter()
        .skip(app.editor.scroll)
        .take(app.edit_height)
        .map(|&r| {
            let plain = Style::default().fg(t.text);
            let chars: Vec<char> = app.editor.lines[r.line].chars().skip(r.start).take(r.end - r.start).collect();
            let mut styles = vec![plain; chars.len()];
            let mut paint = |from: usize, to: usize, f: &dyn Fn(Style) -> Style| {
                for c in from.max(r.start)..to.min(r.end) {
                    styles[c - r.start] = f(styles[c - r.start]);
                }
            };
            if let Some(q) = find {
                let hits = line_matches
                    .entry(r.line)
                    .or_insert_with(|| crate::search::matches(&app.editor.lines[r.line], q));
                for &(s, e) in hits.iter() {
                    paint(s, e, &|st| st.bg(match_bg));
                }
            }
            // Misspellings: a warn-coloured underline, never on the word being typed.
            if app.spell_on {
                let bad = line_spelling.entry(r.line).or_insert_with(|| app.misspellings(r.line));
                for &(s, e) in bad.iter() {
                    paint(s, e, &|st| st.underline_color(t.warn).add_modifier(Modifier::UNDERLINED));
                }
            }
            if let Some((from, to)) = app.editor.row_selection(r) {
                paint(from, to, &|st| st.bg(t.sel).fg(if find.is_some() { t.accent } else { t.text }));
            }
            let mut spans: Vec<Span> = Vec::new();
            let mut run = String::new();
            let mut run_style = plain;
            for (ch, st) in chars.iter().zip(styles) {
                if st != run_style && !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), run_style));
                }
                run_style = st;
                run.push(*ch);
            }
            if !run.is_empty() {
                spans.push(Span::styled(run, run_style));
            }
            Line::from(spans)
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

    // Autosave keeps this quiet: a dim note while a change waits to be
    // written, a brief tick once it is, and red only when saving failed.
    let dirty = app.project.dirty_count();
    match &app.save_state {
        crate::app::SaveState::Failed(_) if dirty > 0 => spans.push(Span::styled(
            format!("  ● {dirty} not saved"),
            Style::default().fg(t.warn),
        )),
        _ if dirty > 0 => spans.push(Span::styled("  ○ saving", Style::default().fg(t.dim))),
        crate::app::SaveState::Saved(at) if at.elapsed() < std::time::Duration::from_secs(3) => {
            spans.push(Span::styled("  ✓ saved", Style::default().fg(t.dim)))
        }
        _ => {}
    }
    if !app.msg.is_empty() {
        spans.push(Span::styled(
            format!("  {}", app.msg),
            Style::default().fg(t.accent),
        ));
    }

    // Keep a gap after the message; drop whole hints rather than run into it.
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let hints = fit_hints(&app.hints(), (area.width as usize).saturating_sub(used + 3));
    let pad = (area.width as usize)
        .saturating_sub(used)
        .saturating_sub(hints.chars().count() + 1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(format!("{hints} "), Style::default().fg(t.dim)));

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

        Overlay::Palette { query, sel, entries } => {
            let hits = crate::palette::filter(entries, query);
            let rows = 14usize;
            let w = area.width.saturating_sub(4).min(84);
            let h = rows as u16 + 5;
            let box_area = Rect { x: area.x + (area.width - w) / 2, y: area.y + 2.min(area.height.saturating_sub(h)), width: w, height: h.min(area.height) };
            f.render_widget(Clear, box_area);
            let block = pane_block("FIND ANYTHING", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let iw = inner.width as usize;
            let dim = Style::default().fg(t.dim);
            let sel = (*sel).min(hits.len().saturating_sub(1));
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(" › ", Style::default().fg(t.accent)),
                    Span::styled(query.clone(), Style::default().fg(t.text)),
                    Span::styled("█", Style::default().fg(t.accent)),
                ]),
                Line::from(Span::styled("─".repeat(iw), Style::default().fg(t.border))),
            ];
            let start = sel.saturating_sub(rows.saturating_sub(1));
            for (i, e) in hits.iter().enumerate().skip(start).take(rows) {
                let on = i == sel;
                let key_w = e.key.chars().count();
                let label_w = e.label.chars().count().min(iw.saturating_sub(key_w + 6));
                let label = truncate(&e.label, label_w);
                let detail_room = iw.saturating_sub(label.chars().count() + key_w + 7);
                let detail = if e.detail.is_empty() || detail_room < 6 { String::new() } else { format!("  {}", truncate(&e.detail, detail_room)) };
                let used = 3 + label.chars().count() + detail.chars().count();
                let pad = iw.saturating_sub(used + key_w + 1);
                let row = Line::from(vec![
                    Span::styled(if on { " ▸ " } else { "   " }, Style::default().fg(t.accent)),
                    Span::styled(label, Style::default().fg(if on { t.accent } else { t.text })),
                    Span::styled(detail, dim),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(e.key.clone(), Style::default().fg(t.sun)),
                    Span::raw(" "),
                ]);
                lines.push(if on { row.style(Style::default().bg(t.sel)) } else { row });
            }
            if hits.is_empty() {
                lines.push(Line::from(Span::styled("   nothing by that name", dim)));
            }
            while lines.len() < rows + 2 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(" type to search   ↑↓ choose   ↵ do it   esc close", dim)));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Find { query, with, on_with, .. } => {
            let editor = app.rect_editor;
            let h = if with.is_some() { 4 } else { 3 };
            let w = editor.width.saturating_add(4).min(area.width);
            let bar = Rect {
                x: editor.x.saturating_sub(2),
                y: (editor.y + editor.height).saturating_sub(h).max(area.y),
                width: w,
                height: h,
            };
            f.render_widget(Clear, bar);
            let block = Block::default().borders(Borders::TOP | Borders::BOTTOM).border_style(Style::default().fg(t.accent));
            let inner = block.inner(bar);
            f.render_widget(block, bar);
            let dim = Style::default().fg(t.dim);
            let field = |label: &str, text: &str, active: bool| {
                vec![
                    Span::styled(format!(" {label:>7} ▸ "), Style::default().fg(if active { t.accent } else { t.dim })),
                    Span::styled(text.to_string(), Style::default().fg(t.text)),
                    Span::styled(if active { "█" } else { " " }, Style::default().fg(t.accent)),
                ]
            };
            let m = app.mod_label();
            let mut first = field("find", query, !*on_with);
            let pos = app.find_position(query);
            first.push(Span::styled(format!("  {pos}"), Style::default().fg(if pos == "no matches" { t.warn } else { t.sun })));
            first.push(Span::styled(format!("   ↵ next  ↑ previous  Tab replace  {m}F whole book  esc close"), dim));
            let mut lines = vec![Line::from(first)];
            if let Some(w) = with {
                let mut second = field("replace", w, *on_with);
                second.push(Span::styled(format!("   ↵ replace this one  {m}R replace all in this scene"), dim));
                lines.push(Line::from(second));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::FindBook { query, with, on_with, hits, sel, confirm } => {
            let box_area = centred(area, area.width.saturating_sub(4).min(110), area.height.saturating_sub(2).min(38));
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
                format!("FIND IN THE BOOK · {} match{} in {} scene{}", hits.len(), if hits.len() == 1 { "" } else { "es" }, scenes, if scenes == 1 { "" } else { "s" })
            };
            let block = pane_block(&head, true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let iw = inner.width as usize;
            let dim = Style::default().fg(t.dim);
            let field = |label: &str, text: &str, active: bool| {
                Line::from(vec![
                    Span::styled(format!(" {label:>7} ▸ "), Style::default().fg(if active { t.accent } else { t.dim })),
                    Span::styled(text.to_string(), Style::default().fg(t.text)),
                    Span::styled(if active { "█" } else { " " }, Style::default().fg(t.accent)),
                ])
            };
            let mut lines = vec![field("find", query, !*on_with)];
            if let Some(w) = with {
                lines.push(field("replace", w, *on_with));
            }
            lines.push(Line::from(Span::styled("─".repeat(iw), Style::default().fg(t.border))));

            // Results, grouped under the scene they're in, scrolled to keep
            // the selection in view.
            let mut rows: Vec<(Option<usize>, Line)> = Vec::new();
            let mut last: Option<&std::path::Path> = None;
            let counts = |p: &std::path::Path| hits.iter().filter(|h| h.path == p).count();
            let match_bg = blend(t.border, t.sun, 0.45);
            for (i, h) in hits.iter().enumerate() {
                if last != Some(h.path.as_path()) {
                    last = Some(&h.path);
                    rows.push((None, Line::from(vec![
                        Span::styled(format!(" {}", truncate(&h.place, iw.saturating_sub(8))), Style::default().fg(t.accent)),
                        Span::styled(format!("  {}", counts(&h.path)), dim),
                    ])));
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
                    Span::styled(if on { "  ▸ " } else { "    " }, Style::default().fg(t.accent)),
                    Span::styled(format!("{}{before}", if lead > 0 { "…" } else { "" }), Style::default().fg(t.text)),
                    Span::styled(hit, Style::default().fg(t.text).bg(match_bg)),
                    Span::styled(after, Style::default().fg(t.text)),
                ]);
                rows.push((Some(i), if on { row.style(Style::default().bg(t.sel)) } else { row }));
            }
            let room = (inner.height as usize).saturating_sub(lines.len() + 2);
            let sel_row = rows.iter().position(|(i, _)| *i == Some(*sel)).unwrap_or(0);
            let start = sel_row.saturating_sub(room.saturating_sub(2));
            if query.is_empty() {
                lines.push(Line::from(Span::styled(" type to search every scene and note", dim)));
            } else if hits.is_empty() {
                lines.push(Line::from(Span::styled(" nothing found", dim)));
            }
            lines.extend(rows.into_iter().skip(start).take(room).map(|(_, l)| l));
            while lines.len() < (inner.height as usize).saturating_sub(1) {
                lines.push(Line::from(""));
            }
            let m = app.mod_label();
            lines.push(if *confirm {
                Line::from(vec![
                    Span::styled(
                        format!(" Replace {} match{} in {} scene{} with “{}”? ", hits.len(), if hits.len() == 1 { "" } else { "es" }, scenes, if scenes == 1 { "" } else { "s" }, with.clone().unwrap_or_default()),
                        Style::default().fg(t.warn),
                    ),
                    Span::styled("y replace   any other key cancels", Style::default().fg(t.text)),
                ])
            } else if with.is_some() {
                Line::from(Span::styled(format!(" ↑↓ choose   ↵ go to it   Tab switch field   ↵ in replace (or {m}R) replaces all   esc close"), dim))
            } else {
                Line::from(Span::styled(" ↑↓ choose   ↵ go to it   Tab replace   esc close", dim))
            });
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Export { formats, parts, sel, done } => {
            let h = (3 + parts.len() as u16 + 10).min(area.height);
            let box_area = centred(area, 66, h);
            f.render_widget(Clear, box_area);
            let block = pane_block("EXPORT FOR READERS", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let dim = Style::default().fg(t.dim);
            let mut lines: Vec<Line> = Vec::new();
            if let Some(result) = done {
                for l in result {
                    let style = if l.starts_with('✓') { Style::default().fg(t.accent) } else if l.starts_with("couldn't") { Style::default().fg(t.warn) } else { Style::default().fg(t.text) };
                    lines.push(Line::from(Span::styled(format!(" {l}"), style)));
                }
                while lines.len() < (inner.height as usize).saturating_sub(1) {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(Span::styled(" any key closes", dim)));
                f.render_widget(Paragraph::new(lines), inner);
                return;
            }
            let row = |i: usize, on: bool, label: String, note: &str, lines: &mut Vec<Line>| {
                let cursor = i == *sel;
                let l = Line::from(vec![
                    Span::styled(if cursor { " ▸ " } else { "   " }, Style::default().fg(t.accent)),
                    Span::styled(if on { "[x] " } else { "[ ] " }, Style::default().fg(if on { t.accent } else { t.dim })),
                    Span::styled(format!("{label:<16}"), Style::default().fg(if cursor { t.accent } else { t.text })),
                    Span::styled(note.to_string(), dim),
                ]);
                lines.push(if cursor { l.style(Style::default().bg(t.sel)) } else { l });
            };
            lines.push(Line::from(Span::styled(" Formats", Style::default().fg(t.text).add_modifier(Modifier::BOLD))));
            row(0, formats[0], "Word document".into(), "standard manuscript format, for agents and editors", &mut lines);
            row(1, formats[1], "EPUB".into(), "for phones and e-readers", &mut lines);
            row(2, formats[2], "Markdown".into(), "the plain compiled text", &mut lines);
            if !parts.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" Include", Style::default().fg(t.text).add_modifier(Modifier::BOLD))));
                for (i, (_, title, on)) in parts.iter().enumerate() {
                    row(3 + i, *on, title.clone(), "", &mut lines);
                }
            }
            lines.push(Line::from(""));
            let button = 3 + parts.len();
            let on = *sel == button;
            let b = Line::from(vec![
                Span::styled(if on { " ▸ " } else { "   " }, Style::default().fg(t.accent)),
                Span::styled(" Export to exports/ ", Style::default().fg(if on { t.sel } else { t.accent }).bg(if on { t.accent } else { t.sel })),
            ]);
            lines.push(b);
            while lines.len() < (inner.height as usize).saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(" ↑↓ choose   space/↵ tick   x export   esc close", dim)));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Spelling { word, suggestions, sel, line, end, .. } => {
            // Sits just under the word, clamped to the screen.
            let rows = app.editor.layout(app.edit_width);
            let vis = rows.iter().position(|r| r.line == *line && *end >= r.start && *end <= r.end).unwrap_or(0);
            let y = app.rect_editor.y + (vis.saturating_sub(app.editor.scroll)) as u16 + 1;
            let h = suggestions.len() as u16 + 6;
            let w = 44u16.min(area.width);
            let y = y.min(area.height.saturating_sub(h));
            let x = app.rect_editor.x.min(area.width.saturating_sub(w));
            let box_area = Rect { x, y, width: w, height: h };
            f.render_widget(Clear, box_area);
            let title = format!("“{}”", truncate(word, 30));
            let block = pane_block(&title, true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let dim = Style::default().fg(t.dim);
            let mut lines: Vec<Line> = Vec::new();
            if suggestions.is_empty() {
                lines.push(Line::from(Span::styled(" no suggestions", dim)));
            }
            let row = |i: usize, label: String, key: String, lines: &mut Vec<Line>| {
                let on = i == *sel;
                let l = Line::from(vec![
                    Span::styled(if on { " ▸ " } else { "   " }, Style::default().fg(t.accent)),
                    Span::styled(format!("{label:<30}"), Style::default().fg(if on { t.accent } else { t.text })),
                    Span::styled(key, Style::default().fg(t.sun)),
                ]);
                lines.push(if on { l.style(Style::default().bg(t.sel)) } else { l });
            };
            for (i, s) in suggestions.iter().enumerate() {
                row(i, truncate(s, 30), format!("{}", i + 1), &mut lines);
            }
            lines.push(Line::from(""));
            row(suggestions.len(), "add to this book's dictionary".into(), "a".into(), &mut lines);
            row(suggestions.len() + 1, "leave it".into(), "esc".into(), &mut lines);
            lines.push(Line::from(Span::styled(" ↵ choose   F8 skip to the next", dim)));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Cork { scope, sel, pov, typing } => draw_cork(f, app, area, t, scope, *sel, pov.as_deref(), typing.as_ref()),

        Overlay::Names { drifts, sel } => {
            let h = (drifts.len() as u16 * 2).min(20) + 6;
            let box_area = centred(area, area.width.saturating_sub(4).min(90), h);
            f.render_widget(Clear, box_area);
            let block = pane_block("NAMES THAT DRIFTED", true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let dim = Style::default().fg(t.dim);
            let mut lines = vec![
                Line::from(Span::styled(" Spellings one or two letters away from a name in your notebook:", Style::default().fg(t.text))),
                Line::from(""),
            ];
            for (i, d) in drifts.iter().enumerate().take(10) {
                let on = i == *sel;
                let mut scenes: Vec<&str> = Vec::new();
                for h in &d.hits {
                    let title = h.place.rsplit(" › ").next().unwrap_or(&h.place);
                    if !scenes.contains(&title) {
                        scenes.push(title);
                    }
                }
                let row = Line::from(vec![
                    Span::styled(if on { " ▸ " } else { "   " }, Style::default().fg(t.accent)),
                    Span::styled(d.variant.clone(), Style::default().fg(t.warn)),
                    Span::styled(format!(" ×{}", d.hits.len()), dim),
                    Span::styled(" — the ", dim),
                    Span::styled(d.name.section.clone(), dim),
                    Span::styled(" note says ", dim),
                    Span::styled(d.name.name.clone(), Style::default().fg(t.accent)),
                ]);
                lines.push(if on { row.style(Style::default().bg(t.sel)) } else { row });
                lines.push(Line::from(Span::styled(format!("     {}", truncate(&scenes.join(" · "), inner.width.saturating_sub(6) as usize)), dim)));
            }
            while lines.len() < (inner.height as usize).saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(" ↑↓ choose   ↵ go to the first one   f fix them all   esc close", dim)));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Recover { items } => {
            let h = (items.len() as u16).min(8) + 8;
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
                    Span::styled(" ▸ ", Style::default().fg(t.accent)),
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
                lines.push(Line::from(Span::styled(format!("   and {} more", items.len() - 8), dim)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Restoring keeps the saved version in each scene's history.",
                dim,
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" y ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                Span::styled("restore them   ", Style::default().fg(t.text)),
                Span::styled("n ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                Span::styled("keep the saved versions   ", Style::default().fg(t.text)),
                Span::styled("esc decide later", dim),
            ]));
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::History { scene, title, versions, sel, scroll } => {
            let box_area = centred(area, area.width.saturating_sub(4).min(118), area.height.saturating_sub(2).min(40));
            f.render_widget(Clear, box_area);
            let head = format!("HISTORY · {} · {} version{}", title.to_uppercase(), versions.len(), if versions.len() == 1 { "" } else { "s" });
            let block = pane_block(&head, true, t);
            let inner = block.inner(box_area);
            f.render_widget(block, box_area);
            let [list_area, _, diff_area] = Layout::horizontal([
                Constraint::Length(34),
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
            let now_words = current.split_whitespace().count() as i64;

            // The versions, newest first.
            let room = list_area.height.saturating_sub(3) as usize;
            let start = sel.saturating_sub(room.saturating_sub(1));
            let mut left: Vec<Line> = vec![Line::from(Span::styled(" kept versions", dim)), Line::from("")];
            for (i, v) in versions.iter().enumerate().skip(start).take(room) {
                let on = i == *sel;
                let words = v.words() as i64;
                let delta = words - now_words;
                let change = match delta {
                    0 => "same length".to_string(),
                    d if d < 0 => format!("{} fewer", thousands((-d) as usize)),
                    d => format!("{} more", thousands(d as usize)),
                };
                let row = Line::from(vec![
                    Span::styled(if on { " ▸ " } else { "   " }, Style::default().fg(t.accent)),
                    Span::styled(format!("{:<17}", when_label(v.when)), Style::default().fg(if on { t.accent } else { t.text })),
                    Span::styled(truncate(&change, 13), dim),
                ]);
                left.push(if on { row.style(Style::default().bg(t.sel)) } else { row });
            }
            f.render_widget(Paragraph::new(left), list_area);

            // What changed between that version and now.
            let v = &versions[*sel];
            let old = v.body();
            let pieces = crate::history::diff(&old, &current);
            let width = diff_area.width.saturating_sub(1) as usize;
            let (mut body, first_change) = diff_lines(&pieces, width, t);
            let header = vec![
                Line::from(vec![
                    Span::styled(format!("{} · {} words", when_label(v.when), thousands(v.words())), Style::default().fg(t.accent)),
                    Span::styled(format!("  ·  now {} words", thousands(now_words as usize)), dim),
                ]),
                Line::from(vec![
                    Span::styled("struck", Style::default().fg(t.warn).add_modifier(Modifier::CROSSED_OUT)),
                    Span::styled(" was in this version and is gone now · ", dim),
                    Span::styled("underlined", Style::default().fg(t.accent).add_modifier(Modifier::UNDERLINED)),
                    Span::styled(" is new since", dim),
                ]),
                Line::from(""),
            ];
            let room = (diff_area.height as usize).saturating_sub(header.len() + 2);
            let top = first_change.saturating_sub(2) + *scroll;
            let top = top.min(body.len().saturating_sub(1));
            let mut lines = header;
            if pieces.iter().all(|p| matches!(p, crate::history::Piece::Same(_))) {
                lines.push(Line::from(Span::styled("identical to the scene as it is now", dim)));
            } else {
                lines.extend(body.drain(..).skip(top).take(room));
            }
            while lines.len() < (diff_area.height as usize).saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                "↑↓ pick a version   PgUp PgDn scroll   ↵ restore it   esc close",
                dim,
            )));
            f.render_widget(Paragraph::new(lines), diff_area);
        }

        Overlay::Menu { sel } => {
            let items = app.menu();
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
                            label.clone(),
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

        Overlay::Create { plan, buf, fresh } => {
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
                (Style::default().fg(t.text), " type a name   ↵ create   esc cancel")
            };
            let lines = vec![
                Line::from(Span::styled(format!(" {}", plan.place), Style::default().fg(t.dim))),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" ▸ ", Style::default().fg(t.accent)),
                    Span::styled(buf.clone(), name),
                    Span::styled("█", Style::default().fg(t.accent)),
                ]),
                Line::from(""),
                Line::from(Span::styled(keys, Style::default().fg(t.dim))),
            ];
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Rename { buf, fresh, noun, .. } => {
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
                Line::from(Span::styled(keys, Style::default().fg(t.dim))),
            ];
            f.render_widget(Paragraph::new(lines), inner);
        }

        Overlay::Confirm { name, noun, words, permanent, .. } => {
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
            let lines = vec![
                Line::from(vec![
                    Span::styled(" Delete ", Style::default().fg(t.text)),
                    Span::styled(name.clone(), Style::default().fg(t.accent)),
                    Span::styled("?", Style::default().fg(t.text)),
                ]),
                Line::from(""),
                Line::from(Span::styled(format!(" {toll}"), Style::default().fg(t.warn))),
                Line::from(Span::styled(
                    if *permanent {
                        " it's already in the trash — this is for good"
                    } else {
                        " it moves to the trash, so it isn't gone for good"
                    },
                    Style::default().fg(if *permanent { t.warn } else { t.dim }),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " y delete   esc keep it",
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

#[allow(clippy::too_many_arguments)]
fn draw_cork(
    f: &mut Frame,
    app: &App,
    area: Rect,
    t: &Theme,
    scope: &Option<std::path::PathBuf>,
    sel: usize,
    pov: Option<&str>,
    typing: Option<&(crate::app::CardField, String)>,
) {
    use crate::cork;
    let box_area = Rect { x: area.x, y: area.y, width: area.width, height: area.height.saturating_sub(1) };
    f.render_widget(Clear, box_area);
    let part = app.cork_scope(scope);
    let groups = cork::board(&app.project, part);
    let where_ = part.map(|i| app.project.nodes[i].title.to_uppercase()).unwrap_or_else(|| "THE BOOK".into());
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
        legend.push(Span::styled(format!("{p}   "), Style::default().fg(if pov.is_none_or(|x| x == p) { t.text } else { t.dim })));
    }
    if povs.is_empty() {
        legend.push(Span::styled("none set yet — v on a card sets one", dim));
    }
    f.render_widget(Paragraph::new(Line::from(legend)), Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 });

    let cols = app.cork_cols();
    const CARD_H: u16 = 6;
    use crate::app::CARD_W;
    let layout = cork::layout(&groups, cols);
    let cards: Vec<&cork::Card> = groups.iter().flat_map(|g| &g.cards).collect();
    let sel = sel.min(cards.len().saturating_sub(1));

    // Rows of the board, with a heading line before each chapter's first row.
    let body = Rect { x: inner.x, y: inner.y + 2, width: inner.width, height: inner.height.saturating_sub(4) };
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
            let title = if g.title.is_empty() { "scenes".to_string() } else { g.title.clone() };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {title}"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {} scene{} · {} words", g.cards.len(), if g.cards.len() == 1 { "" } else { "s" }, thousands(words)), dim),
                ])),
                Rect { x: body.x, y: body.y + gy - offset, width: body.width, height: 1 },
            );
        }
    }

    for (n, &(_, r, c)) in layout.iter().enumerate() {
        let top = y_of_row[r];
        if top < offset || top - offset + CARD_H > body.height {
            continue;
        }
        let card = cards[n];
        let rect = Rect { x: body.x + 1 + c as u16 * CARD_W, y: body.y + top - offset, width: CARD_W - 1, height: CARD_H };
        let lit = pov.is_none_or(|p| card.pov.as_deref().is_some_and(|cp| cp.eq_ignore_ascii_case(p)));
        let on = n == sel;
        let edge = if on { t.accent } else if lit { colour_of(card.pov.as_deref()) } else { t.border };
        let text = if lit { t.text } else { t.border };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(edge).add_modifier(if on { Modifier::BOLD } else { Modifier::empty() }))
            .title(Span::styled(format!(" {} ", truncate(&card.title, (CARD_W - 5) as usize)), Style::default().fg(if on { t.accent } else { text })));
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
            Span::styled(truncate(&who, w.saturating_sub(status.chars().count() + 1)), Style::default().fg(if lit { colour_of(card.pov.as_deref()) } else { t.border })),
            Span::raw(" ".repeat(pad)),
            Span::styled(status, Style::default().fg(if lit { status_col } else { t.border })),
        ])];
        let synopsis = match typing {
            Some((crate::app::CardField::Synopsis, buf)) if editing_here => format!("{buf}█"),
            _ => card.synopsis.clone().unwrap_or_default(),
        };
        let wrapped = wrap_words(&synopsis, w);
        for i in 0..2 {
            lines.push(Line::from(Span::styled(wrapped.get(i).cloned().unwrap_or_default(), Style::default().fg(if lit { t.text } else { t.border }))));
        }
        if let Some((crate::app::CardField::Pov, buf)) = typing.filter(|_| editing_here) {
            lines[0] = Line::from(vec![Span::styled("POV ▸ ", Style::default().fg(t.accent)), Span::styled(format!("{buf}█"), Style::default().fg(t.text))]);
        }
        let bar_w = w.saturating_sub(6).max(4);
        let words = thousands(card.words);
        let bar = match card.target {
            Some(target) => {
                let filled = ((card.words * bar_w) / target.max(1)).min(bar_w);
                vec![
                    Span::styled("█".repeat(filled), Style::default().fg(if lit { t.accent } else { t.border })),
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
            if cork::parts(&app.project).len() > 1 { "   [ ] other acts" } else { "" }
        )
    };
    f.render_widget(Paragraph::new(Line::from(Span::styled(keys, dim))), Rect { x: inner.x, y: inner.y + inner.height.saturating_sub(1), width: inner.width, height: 1 });
}

/// Greedy word wrap for short card text.
fn wrap_words(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let need = if cur.is_empty() { word.chars().count() } else { cur.chars().count() + 1 + word.chars().count() };
        if need > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|l| truncate(&l, width)).collect()
}

/// "Today 9:14 pm", "Yesterday 6:02 pm", "Tue 16 Sep 9:14 pm".
fn when_label(dt: chrono::DateTime<chrono::Local>) -> String {
    let today = chrono::Local::now().date_naive();
    let clock = dt.format("%-I:%M %P").to_string();
    let day = dt.date_naive();
    if day == today {
        format!("Today {clock}")
    } else if Some(day) == today.pred_opt() {
        format!("Yesterday {clock}")
    } else {
        format!("{} {clock}", dt.format("%a %-d %b"))
    }
}

/// Lay a word diff out as wrapped lines. Returns the lines and the index of
/// the first line with a change, so the view can open where it matters.
fn diff_lines(pieces: &[crate::history::Piece], width: usize, t: &Theme) -> (Vec<Line<'static>>, usize) {
    use crate::history::Piece;

    struct Wrap {
        width: usize,
        lines: Vec<Line<'static>>,
        cur: Vec<Span<'static>>,
        used: usize,
        changed: bool,
        first_change: Option<usize>,
    }
    impl Wrap {
        fn end_line(&mut self) {
            if self.changed && self.first_change.is_none() {
                self.first_change = Some(self.lines.len());
            }
            self.lines.push(Line::from(std::mem::take(&mut self.cur)));
            self.used = 0;
            self.changed = false;
        }
        fn push(&mut self, token: &str, style: Style, changed: bool) {
            let w = token.chars().count();
            let space = token.chars().all(char::is_whitespace);
            if self.used + w > self.width && self.used > 0 {
                self.end_line();
                if space {
                    return;
                }
            }
            self.cur.push(Span::styled(token.to_string(), style));
            self.used += w;
            self.changed |= changed;
        }
    }

    let mut wrap = Wrap { width: width.max(10), lines: Vec::new(), cur: Vec::new(), used: 0, changed: false, first_change: None };
    let plain = Style::default().fg(t.text);
    let gone = Style::default().fg(t.warn).add_modifier(Modifier::CROSSED_OUT);
    let new = Style::default().fg(t.accent).add_modifier(Modifier::UNDERLINED);

    for p in pieces {
        let (text, style, changed) = match p {
            Piece::Same(s) => (s.as_str(), plain, false),
            Piece::Removed(s) => (s.as_str(), gone, true),
            Piece::Added(s) => (s.as_str(), new, true),
        };
        let mut token = String::new();
        for ch in text.chars() {
            if ch == '\n' {
                if !token.is_empty() {
                    wrap.push(&token, style, changed);
                    token.clear();
                }
                wrap.changed |= changed;
                wrap.end_line();
                continue;
            }
            let boundary = token.chars().last().is_some_and(|c| c.is_whitespace() != ch.is_whitespace());
            if boundary {
                wrap.push(&token, style, changed);
                token.clear();
            }
            token.push(ch);
        }
        if !token.is_empty() {
            wrap.push(&token, style, changed);
        }
    }
    if !wrap.cur.is_empty() {
        wrap.end_line();
    }
    let first = wrap.first_change.unwrap_or(0);
    (wrap.lines, first)
}


/// As many whole hints as fit in `room`, from the left. Hints are separated
/// by two spaces; cutting one mid-word reads as a glitch.
fn fit_hints(hints: &str, room: usize) -> String {
    let mut out = String::new();
    for hint in hints.trim_end().split("  ") {
        let sep = if out.is_empty() { 0 } else { 2 };
        if out.chars().count() + sep + hint.chars().count() > room {
            break;
        }
        if sep > 0 {
            out.push_str("  ");
        }
        out.push_str(hint);
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_that_do_not_fit_drop_whole_from_the_right() {
        let hints = "Tab pane  ↵ fold  n scene  c chapter  F1 menu ";
        assert_eq!(fit_hints(hints, 100), "Tab pane  ↵ fold  n scene  c chapter  F1 menu");
        assert_eq!(fit_hints(hints, 30), "Tab pane  ↵ fold  n scene");
        assert_eq!(fit_hints(hints, 5), "");
    }
}
