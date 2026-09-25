//! Rendering. Panes, status line, and the theme picker overlay.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};

use crate::app::{App, Focus, Overlay};
use crate::music::State as MusicState;
use crate::scene::{self, Ink, Mode, Phase};
use crate::theme::Theme;
use grimoire_core::create::{self, New};
use grimoire_core::project::{Area, Kind};

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
    app.check_focus_mode();

    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
    app.rect_scene = Rect::default();
    app.view_hits.clear();
    app.rect_music = Rect::default();

    let edit_area = if app.focus_mode {
        // Focus mode: the prose alone. No tree, clearing or music, and
        // nothing left behind for a click to land on.
        app.rect_tree = Rect::default();
        app.create_hits.clear();
        app.scene_visible = false;
        app.music_visible = false;
        main
    } else {
        draw_left(f, app, main, &t)
    };

    // A note, or another scene, opened from the prose sits beside it, when
    // there's room. Only one at a time.
    app.side_room = edit_area.width >= 70;
    app.rect_codex = Rect::default();
    app.rect_beside = Rect::default();
    if app.side_room && (app.codex.is_some() || app.beside.is_some()) {
        let [ed, side] =
            Layout::horizontal([Constraint::Min(34), Constraint::Percentage(38)]).areas(edit_area);
        draw_editor(f, app, ed, &t);
        if app.codex.is_some() {
            draw_codex(f, app, side, &t);
        } else {
            draw_beside(f, app, side, &t);
        }
    } else {
        draw_editor(f, app, edit_area, &t);
    }
    draw_status(f, app, status, &t);

    if !matches!(app.overlay, Overlay::None) {
        crate::app::overlays::draw(f, app, f.area(), &t);
    }

    if t.is_rainbow() {
        paint_rainbow(f.buffer_mut(), t.accent, app.frame);
    }
}

/// The Rainbow theme's accent isn't one colour: every cell drawn in it — the
/// focused border, titles, the progress bar, the word count, the open scene —
/// takes its hue from where it sits on screen, a diagonal sweep across the
/// spectrum that drifts round the wheel about once a minute.
pub(crate) fn paint_rainbow(
    buf: &mut ratatui::buffer::Buffer,
    accent: ratatui::style::Color,
    frame: u64,
) {
    let area = buf.area;
    let drift = (frame % 240) as f32 * 1.5;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            if cell.fg != accent && cell.bg != accent {
                continue;
            }
            let c = crate::theme::hue(x as f32 * 3.0 + y as f32 * 7.0 + drift);
            if cell.fg == accent {
                cell.fg = c;
            }
            if cell.bg == accent {
                cell.bg = c;
            }
        }
    }
}

/// The left column — tree, clearing, music — and what's left for writing.
fn draw_left(f: &mut Frame, app: &mut App, main: Rect, t: &Theme) -> Rect {
    let [left, edit_area] =
        Layout::horizontal([Constraint::Length(LEFT_W), Constraint::Min(24)]).areas(main);

    // Give up the ornaments before the tree gets unusable. With music switched
    // off there is no music pane at all, and the tree has that room.
    let music_h = if app.music.enabled { MUSIC_H } else { 0 };
    let (tree_area, scene_area, music_area) =
        if !app.music.enabled && left.height >= SCENE_H + MIN_TREE {
            let [a, b] = Layout::vertical([Constraint::Min(MIN_TREE), Constraint::Length(SCENE_H)])
                .areas(left);
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
            let [a, c] = Layout::vertical([Constraint::Min(MIN_TREE), Constraint::Length(MUSIC_H)])
                .areas(left);
            (a, Rect::default(), c)
        } else {
            (left, Rect::default(), Rect::default())
        };

    app.scene_visible = scene_area.height > 0;
    app.music_visible = music_area.height > 0;

    draw_tree(f, app, tree_area, t);
    if app.scene_visible {
        draw_scene(f, app, scene_area, t);
    }
    if app.music_visible {
        draw_music(f, app, music_area, t);
    }
    edit_area
}

/// How a key looks in a popup's footer: lit, so it stands out from what it does.
pub(crate) fn key_style(t: &Theme) -> Style {
    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
}

/// A popup footer like " y delete   esc keep it", its keys lit and the rest dim.
pub(crate) fn hint_line(text: &str, t: &Theme) -> Line<'static> {
    Line::from(hint_spans(text, t))
}

pub(crate) fn hint_spans(text: &str, t: &Theme) -> Vec<Span<'static>> {
    let dim = Style::default().fg(t.dim);
    hint_parts(text)
        .into_iter()
        .map(|(part, key)| Span::styled(part, if key { key_style(t) } else { dim }))
        .collect()
}

/// Split a footer into (text, is it a key). Hints are separated by two or
/// more spaces or " · ", and each starts with its key — "↵ choose",
/// "PgUp PgDn scroll", "any other key cancels" — unless it's typing ("type a
/// name"), which has none.
fn hint_parts(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let gap = if rest.starts_with(" · ") {
            " · ".len()
        } else {
            rest.len() - rest.trim_start_matches(' ').len()
        };
        if gap > 0 {
            out.push((rest[..gap].to_string(), false));
            rest = &rest[gap..];
            continue;
        }
        let end = [rest.find("  "), rest.find(" · ")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(rest.len());
        let hint = &rest[..end];
        let key = if hint == "type" || hint.starts_with("type ") {
            0
        } else {
            ["any other key", "any key", "PgUp PgDn", "[ ]"]
                .into_iter()
                .find(|k| hint.starts_with(k))
                .map_or_else(|| hint.find(' ').unwrap_or(hint.len()), str::len)
        };
        if key > 0 {
            out.push((hint[..key].to_string(), true));
        }
        if key < hint.len() {
            out.push((hint[key..].to_string(), false));
        }
        rest = &rest[end..];
    }
    out
}

pub(crate) fn pane_block<'a>(title: &'a str, focused: bool, t: &Theme) -> Block<'a> {
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

/// An open spellbook, pages sloping away from the reader, with light rising
/// from its spine. Drawn above the tree
/// when the pane is at least this tall, so the art never costs a short
/// terminal its outline.
const BOOK_ART: [&str; 7] = [
    "    ✧   ·    ✦    ·   ✧",
    "      ______ ☾ ______",
    "    _/      ╲│╱      \\_",
    "   // ≈≈≈ ≈≈ │ ≈≈ ≈≈≈ \\\\",
    "  // ≈ ≈≈ ✧  │  ✧ ≈≈ ≈ \\\\",
    " //__________│__________\\\\",
    " `───────────┴───────────'",
];
const BOOK_ART_MIN_TREE: u16 = 24;

/// Cover in the accent (so Rainbow paints it), pages in dim ink, the moon in
/// the moon's colour, and sparkles that twinkle on staggered beats like the
/// clearing's stars.
fn draw_book_art(f: &mut Frame, area: Rect, t: &Theme, frame: u64) {
    let width = BOOK_ART
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let x0 = area.x + area.width.saturating_sub(width) / 2;
    let mut spark = 0u64;
    for (row, line) in BOOK_ART.iter().enumerate() {
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }
        let spans: Vec<Span> = line
            .chars()
            .map(|c| {
                let (glyph, colour) = match c {
                    '✦' | '✧' | '·' if row == 0 => {
                        spark += 1;
                        let beat = (frame / (4 + spark % 3) + spark) % 4;
                        match beat {
                            0 => ('✦', t.sun),
                            1 | 3 => ('✧', blend(t.sun, t.dim, 0.35)),
                            _ => ('·', t.dim),
                        }
                    }
                    '╲' | '╱' => (c, blend(t.sun, t.dim, 0.5)),
                    '☾' => (c, t.moon),
                    '✧' => (c, t.bloom),
                    '≈' => (c, t.dim),
                    ' ' => (c, t.dim),
                    _ => (c, t.accent),
                };
                Span::styled(glyph.to_string(), Style::default().fg(colour))
            })
            .collect();
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(x0, y, width.min(area.right().saturating_sub(x0)), 1),
        );
    }
}

fn draw_tree(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let focused = app.focus == Focus::Tree;
    // The book's own name heads the tree; Manuscript is a section inside it.
    let book = app.project.meta.title.trim().to_uppercase();
    let block = pane_block(if book.is_empty() { "UNTITLED" } else { &book }, focused, t);
    let mut inner = block.inner(area);
    f.render_widget(block, area);
    // A spellbook heads the tree when there's room for it and the tree too.
    if inner.height >= BOOK_ART_MIN_TREE {
        let [art, rest] = Layout::vertical([
            Constraint::Length(BOOK_ART.len() as u16 + 1),
            Constraint::Min(1),
        ])
        .areas(inner);
        draw_book_art(f, art, t, app.frame);
        inner = rest;
    }
    app.rect_tree = inner;

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

        // ▾ ☺ Characters: a fold arrow for anything that folds, then the
        // thing's symbol, then its name.
        let indent = "  ".repeat(n.depth);
        let fold = match n.kind {
            Kind::Scene => "  ",
            _ if n.expanded => "▾ ",
            _ => "▸ ",
        };
        let open = Some(idx) == app.open;
        let (icon, icon_colour) = match n.kind {
            Kind::Category => (n.area.icon(), area_colour(t, n.area)),
            Kind::Container => ("▰", t.dim),
            Kind::Scene if n.area == Area::Format => (n.area.icon(), area_colour(t, n.area)),
            Kind::Scene if open => ("▮", t.accent),
            Kind::Scene => ("▯", t.dim),
        };

        // Counts are for writing: the manuscript and the notebook, not the
        // paperwork or the trash.
        let counted = n.area == Area::Manuscript || n.area.is_notebook();
        let words = if counted {
            app.project.subtree_words(idx)
        } else {
            0
        };
        // A parked copy or a file Grimoire won't write says so where the
        // count would be: two rows with the same name and different words is
        // exactly how the wrong one gets edited.
        let tag = if n.read_only {
            "read-only"
        } else if n.parked {
            "parked copy"
        } else {
            ""
        };
        let count = if !tag.is_empty() {
            tag.to_string()
        } else if words > 0 {
            thousands(words)
        } else {
            String::new()
        };

        let lead = format!("{indent}{fold}");
        let icon = if app.icons_on {
            format!("{icon} ")
        } else {
            String::new()
        };
        let lead_w = lead.chars().count() + icon.chars().count();
        let room = width.saturating_sub(lead_w + count.chars().count() + 1);
        let title = truncate(&n.title, room.max(1));
        let gap = width
            .saturating_sub(lead_w + title.chars().count())
            .saturating_sub(count.chars().count() + 1)
            .max(1);

        let name_style = match n.kind {
            Kind::Category | Kind::Container => {
                Style::default().fg(t.text).add_modifier(Modifier::BOLD)
            }
            _ if open => Style::default().fg(t.accent),
            _ => Style::default().fg(t.text),
        };
        let dragging_to = app
            .tree_drag
            .is_some_and(|(from, to)| from != to && row == to);
        let base = if dragging_to {
            Style::default()
                .bg(t.sel)
                .add_modifier(Modifier::UNDERLINED)
        } else if selected && focused {
            Style::default().bg(t.sel)
        } else {
            Style::default()
        };

        lines.push(
            Line::from(vec![
                Span::styled(lead, Style::default().fg(t.dim).patch(base)),
                Span::styled(icon, Style::default().fg(icon_colour).patch(base)),
                Span::styled(title, name_style.patch(base)),
                Span::styled(" ".repeat(gap), base),
                Span::styled(
                    count,
                    Style::default()
                        .fg(if tag.is_empty() { t.dim } else { t.warn })
                        .patch(base),
                ),
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
            app.create_hits.push((
                Rect {
                    x,
                    y: row.y,
                    width,
                    height: 1,
                },
                want,
            ));
        }
        x += width;
    }
    spans.push(Span::raw(" "));
    f.render_widget(Paragraph::new(Line::from(spans)), row);
}

fn draw_scene(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    // The pane's title says which of the two views you're on, so ←/→ is
    // discoverable without a legend.
    let (label, grid) = match app.pane_mode {
        Mode::Pomodoro => (
            // Idle, the title has room to say where focus mode is.
            if app.pomo.phase == Phase::Idle && app.open.is_some() {
                format!("F2 timer · {}D focus mode", app.mod_label())
            } else {
                app.pomo.label()
            },
            scene::render(
                scene::Scenery::for_theme(&t.name),
                app.pomo.phase,
                app.pomo.progress(),
                app.pomo.phase != Phase::Idle && !app.pomo.running(),
                app.frame,
            ),
        ),
        Mode::Visualizer => crate::viz_view::view(app, t, area.width),
    };

    let focused = app.focus == Focus::Clearing;
    let mut block = pane_block(&label, focused, t);
    // On a beat the frame flashes bloom, and fades back as the beat does.
    if app.pane_mode == Mode::Visualizer && app.viz.beat > 0.0 {
        let base = if focused { t.accent } else { t.border };
        block = block.border_style(Style::default().fg(blend(base, t.bloom, app.viz.beat * 0.5)));
    }
    let inner = block.inner(area);
    app.rect_scene = inner;
    f.render_widget(block, area);

    let night = app.pomo.phase == Phase::Break && app.pane_mode == Mode::Pomodoro;
    let lines: Vec<Line> = grid
        .iter()
        .map(|row| {
            Line::from(
                row.iter()
                    .map(|&(ch, ink)| {
                        Span::styled(
                            ch.to_string(),
                            Style::default().fg(scene_colour(ink, t, night)),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
    draw_view_switch(f, app, area, focused, t);
}

/// The colour of one cell of the Pomodoro or the Visualizer: every world is
/// drawn in roles, so each theme paints it in its own palette.
pub(crate) fn scene_colour(ink: Ink, t: &Theme, night: bool) -> ratatui::style::Color {
    let lit = |v: u8| v as f32 / 255.0;
    match ink {
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
        Ink::Creature => t.text,
        Ink::Flower => t.bloom,
        Ink::Ground => t.turf,
        Ink::Water => blend(t.moon, t.border, 0.45),
        Ink::Foam => blend(t.text, t.moon, 0.35),
        Ink::Stone => blend(t.dim, t.border, 0.25),
        Ink::Snow => t.text,
        Ink::Light => t.sun,
        Ink::Glow(v) => blend(t.foliage, t.bloom, lit(v)),
        Ink::Band { n, faint } => {
            let c = match n {
                0 => t.warn,
                1 => t.sun,
                2 => t.foliage,
                3 => t.moon,
                _ => t.bloom,
            };
            if faint { blend(c, t.border, 0.6) } else { c }
        }
        Ink::Accent => t.accent,
        Ink::Warn => t.warn,
        Ink::Cloud => blend(t.text, t.border, 0.55),
        Ink::Sand => blend(t.sun, t.bark, 0.5),
        Ink::Trail => t.accent,
        Ink::Dim => t.dim,
        // The Visualizer picks its colours itself (crate::viz_view).
        Ink::Paint(c) => c,
    }
}

/// The view switcher, on the scene pane's bottom edge: "◂ pomodoro
/// visualizer ▸", the current view lit. It says ←/→ has somewhere to go, and
/// clicking a name or an arrow goes there.
fn draw_view_switch(f: &mut Frame, app: &mut App, area: Rect, focused: bool, t: &Theme) {
    if area.height < 3 || area.width < 4 {
        return;
    }
    let row = Rect {
        x: area.x + 1,
        y: area.y + area.height - 1,
        width: area.width - 2,
        height: 1,
    };
    let (items, spans) = view_switch(app.pane_mode, row.width);
    let arrow = Style::default().fg(if focused { t.accent } else { t.dim });
    let lit = Style::default()
        .fg(if focused { t.accent } else { t.text })
        .add_modifier(Modifier::BOLD);
    let styled: Vec<Span> = spans
        .into_iter()
        .map(|(text, kind)| match kind {
            Piece::Arrow => Span::styled(text, arrow),
            Piece::Current => Span::styled(text, lit),
            Piece::Other => Span::styled(text, Style::default().fg(t.dim)),
            Piece::Gap => Span::raw(text),
            Piece::Rule => Span::styled(
                text,
                Style::default().fg(if focused { t.accent } else { t.border }),
            ),
        })
        .collect();
    for (x, width, mode) in items {
        app.view_hits.push((
            Rect {
                x: row.x + x,
                y: row.y,
                width,
                height: 1,
            },
            mode,
        ));
    }
    f.render_widget(Paragraph::new(Line::from(styled)), row);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    Arrow,
    Current,
    Other,
    Gap,
    /// The pane's own border line, carried on either side of a centred
    /// switcher.
    Rule,
}

/// Lay out the switcher in `width` cells: the pieces to draw, and each
/// clickable one's (offset, width, view). Names drop from the right before
/// they'd run into the corner; the arrows always stay.
/// Clickable (offset, width, view) spans, and the labelled pieces to draw.
type Switcher = (Vec<(u16, u16, Mode)>, Vec<(String, Piece)>);

fn view_switch(current: Mode, width: u16) -> Switcher {
    let mut hits = vec![(0, 1, current.prev())];
    let mut spans = vec![("◂".to_string(), Piece::Arrow)];
    let mut x = 1u16;
    for mode in Mode::ALL {
        let w = mode.name().chars().count() as u16;
        // A space before the name, and room left for " ▸".
        if x + 1 + w + 2 > width {
            break;
        }
        spans.push((" ".into(), Piece::Gap));
        let kind = if mode == current {
            Piece::Current
        } else {
            Piece::Other
        };
        spans.push((mode.name().into(), kind));
        hits.push((x + 1, w, mode));
        x += 1 + w;
    }
    if x + 2 <= width {
        spans.push((" ".into(), Piece::Gap));
        spans.push(("▸".into(), Piece::Arrow));
        hits.push((x + 1, 1, current.next()));
        x += 2;
    }
    // Two names leave room to spare: centre the switcher on the edge, so it
    // reads as a control rather than a caption.
    let pad = width.saturating_sub(x) / 2;
    if pad > 0 {
        spans.insert(0, ("─".repeat(pad as usize), Piece::Rule));
        for h in &mut hits {
            h.0 += pad;
        }
    }
    (hits, spans)
}

/// A section's symbol colour. The manuscript takes the accent; the notebook
/// grows in foliage; the book's paperwork is sun; the trash stays dim.
fn area_colour(t: &Theme, a: Area) -> ratatui::style::Color {
    match a {
        Area::Manuscript => t.accent,
        Area::Characters | Area::Places | Area::Notes | Area::Research => t.foliage,
        Area::Format | Area::FrontMatter | Area::Templates => t.sun,
        Area::Trash => t.dim,
    }
}

/// Mix two truecolour colours, `f` of the way from `a` to `b`. Anything that
/// isn't RGB just switches over halfway.
pub(crate) fn blend(
    a: ratatui::style::Color,
    b: ratatui::style::Color,
    f: f32,
) -> ratatui::style::Color {
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
pub(crate) fn bar_colour(t: &Theme, h: u8) -> ratatui::style::Color {
    let x = h as f32 / 255.0;
    // The rainbow's bars climb the spectrum itself: red at the roots, violet
    // at the tips.
    if t.is_rainbow() {
        return crate::theme::hue(x * 280.0);
    }
    let stops = [
        (0.0, t.foliage),
        (0.4, t.accent),
        (0.75, t.sun),
        (1.0, t.bloom),
    ];
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
    let focused = app.focus == Focus::Music;
    let mut block = pane_block(&title, focused, t);
    if focused {
        // Focused, the pane's own edge says what the keys do.
        block = block.title_bottom(Line::from(Span::styled(
            " [ prev · space ⏯ · ] next ",
            Style::default().fg(t.accent),
        )));
    }
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
                "Esc › Settings to switch source",
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
            let badges = app.music.modes.badges();
            let extra = if badges.is_empty() {
                0
            } else {
                badges.chars().count() + 1
            };
            let barw = w.saturating_sub(7 + extra).max(4);
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
                    Span::styled(
                        format!(" {}", mins(tr.progress)),
                        Style::default().fg(t.dim),
                    ),
                    Span::styled(
                        if badges.is_empty() {
                            String::new()
                        } else {
                            format!(" {badges}")
                        },
                        Style::default().fg(t.accent),
                    ),
                ]),
            ]
        }
    };

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let focused = app.focus == Focus::Editor;
    let mut title = app.open_title();
    if let Some((words, target)) = app.scene_target() {
        title = format!("{title} · {} / {}", thousands(words), thousands(target));
    }
    let block = if app.focus_mode {
        // Focus mode draws the page, not a pane: no frame, no title.
        Block::default().padding(Padding::new(2, 2, 1, 0))
    } else {
        pane_block(&title, focused, t).padding(Padding::new(2, 2, 0, 0))
    };
    let inner = block.inner(area);
    app.rect_prose = area;
    f.render_widget(block, area);

    // Prose is set in a column no wider than the line width, centred.
    let col = prose_column(inner, app.line_width);
    app.rect_editor = col;
    app.edit_width = col.width as usize;
    app.edit_height = col.height as usize;

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
    app.follow_caret(&rows);

    // Each row is painted in layers, char by char, then merged into runs:
    // find matches, then the selection on top.
    let find = match &app.overlay {
        Overlay::Find { query, .. } if !query.is_empty() => Some(query.as_str()),
        _ => None,
    };
    let match_bg = blend(t.border, t.sun, 0.45);
    let mut line_matches: std::collections::HashMap<usize, Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    let mut line_spelling: std::collections::HashMap<usize, Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    let mut line_names: std::collections::HashMap<usize, Vec<(usize, usize, usize)>> =
        std::collections::HashMap::new();
    // Notes and TKs, which may run across lines, and echoing words when on.
    let marks = grimoire_core::notes::line_spans(&app.editor.lines);
    let echoes = if app.echo_on {
        grimoire_core::revision::echoes(&app.editor.lines, &app.echo_skip())
    } else {
        Vec::new()
    };
    let echo_bg = blend(t.border, t.moon, 0.35);
    let visible: Vec<Line> = rows
        .iter()
        .skip(app.editor.scroll)
        .take(app.edit_height)
        .map(|&r| {
            let plain = Style::default().fg(t.text);
            let chars: Vec<char> = app.editor.lines[r.line]
                .chars()
                .skip(r.start)
                .take(r.end - r.start)
                .collect();
            let mut styles = vec![plain; chars.len()];
            let mut paint = |from: usize, to: usize, f: &dyn Fn(Style) -> Style| {
                for c in from.max(r.start)..to.min(r.end) {
                    styles[c - r.start] = f(styles[c - r.start]);
                }
            };
            // Notebook names in the accent colour.
            let names = line_names.entry(r.line).or_insert_with(|| {
                grimoire_core::codex::spans(&app.editor.lines[r.line], &app.codex_index)
            });
            for &(s, e, _) in names.iter() {
                paint(s, e, &|st| st.fg(t.accent));
            }
            // Notes to yourself, quiet; TKs, gaps to come back to, in the sun.
            for &(s, e, kind) in &marks[r.line] {
                match kind {
                    grimoire_core::notes::MarkKind::Note => {
                        paint(s, e, &|st| st.fg(t.dim).add_modifier(Modifier::ITALIC))
                    }
                    grimoire_core::notes::MarkKind::Tk => {
                        paint(s, e, &|st| st.fg(t.sun).add_modifier(Modifier::BOLD))
                    }
                }
            }
            if let Some(ec) = echoes.get(r.line) {
                for &(s, e) in ec {
                    paint(s, e, &|st| st.bg(echo_bg));
                }
            }
            if let Some(q) = find {
                let hits = line_matches.entry(r.line).or_insert_with(|| {
                    grimoire_core::search::matches_with(&app.editor.lines[r.line], q, app.find_opts)
                });
                for &(s, e) in hits.iter() {
                    paint(s, e, &|st| st.bg(match_bg));
                }
            }
            // Misspellings: a warn-coloured underline, never on the word being typed.
            if app.spell_on {
                let bad = line_spelling.entry(r.line).or_insert_with(|| {
                    App::misspellings_outside_marks(app.misspellings(r.line), &marks[r.line])
                });
                for &(s, e) in bad.iter() {
                    paint(s, e, &|st| {
                        st.underline_color(t.warn)
                            .add_modifier(Modifier::UNDERLINED)
                    });
                }
            }
            if let Some((from, to)) = app.editor.row_selection(r) {
                paint(from, to, &|st| {
                    st.bg(t.sel)
                        .fg(if find.is_some() { t.accent } else { t.text })
                });
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

    // An empty scene shows where the words go. It is only drawn, never
    // part of the text, and the first keystroke replaces it.
    let empty = app.editor.lines.len() == 1 && app.editor.lines[0].is_empty();
    if empty {
        let cue = if focused {
            "Start writing · Esc for the menu"
        } else {
            "Tab or click to write · Esc for the menu"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(cue, col.width as usize),
                Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
            ))),
            col,
        );
    } else {
        f.render_widget(Paragraph::new(visible), col);
    }

    if focused && matches!(app.overlay, Overlay::None) {
        let (r, c) = app.editor.cursor_vis(&rows);
        if r >= app.editor.scroll && r < app.editor.scroll + app.edit_height {
            let x = col.x + c.min(app.edit_width.saturating_sub(1)) as u16;
            let y = col.y + (r - app.editor.scroll) as u16;
            f.set_cursor_position((x, y));
        }
    }
}

/// The column prose is set in: `width` wide at most (0: all of it),
/// centred in `inner`.
fn prose_column(inner: Rect, width: usize) -> Rect {
    let w = match width {
        0 => inner.width,
        w => inner.width.min(w.min(u16::MAX as usize) as u16),
    };
    Rect {
        x: inner.x + (inner.width - w) / 2,
        width: w,
        ..inner
    }
}

/// Another scene, read-only, beside the one being written.
fn draw_beside(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let Some(i) = app.beside_node() else {
        // Moved or deleted since it was opened.
        app.close_beside();
        return;
    };
    let focused = app.focus == Focus::Beside;
    let n = &app.project.nodes[i];
    let title = format!("{} · read-only", n.title);
    let block = pane_block(&title, focused, t).padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.rect_beside = area;

    let w = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    for para in n.body.split('\n') {
        if para.trim().is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        for l in wrap_words(para, w) {
            lines.push(Line::from(Span::styled(l, Style::default().fg(t.text))));
        }
    }
    if n.body.trim().is_empty() {
        lines = vec![Line::from(Span::styled(
            "Nothing written here yet.",
            Style::default().fg(t.dim),
        ))];
    }
    let max = lines.len().saturating_sub(inner.height as usize);
    let Some(pane) = &mut app.beside else { return };
    pane.scroll = pane.scroll.min(max);
    let scroll = pane.scroll;
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(scroll).collect::<Vec<_>>()),
        inner,
    );
}

fn draw_codex(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let Some(pane) = &app.codex else { return };
    let focused = app.focus == Focus::Codex;
    let title = if pane.entry.section.is_empty() {
        pane.entry.title.clone()
    } else {
        format!("{} · {}", pane.entry.title, pane.entry.section)
    };
    let block = pane_block(&title, focused, t).padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.rect_codex = inner;
    let Some(pane) = &app.codex else { return };
    let dim = Style::default().fg(t.dim);
    let w = inner.width as usize;

    // The appearances list takes the bottom; the note fills the rest.
    let list_h = (pane.appears.len() as u16 + 2).min(inner.height / 2).max(3);
    let note_area = Rect {
        height: inner.height.saturating_sub(list_h),
        ..inner
    };
    let list_area = Rect {
        y: inner.y + note_area.height,
        height: list_h,
        ..inner
    };

    let body = app
        .project
        .nodes
        .iter()
        .find(|n| n.path == pane.entry.note)
        .map(|n| n.body.clone())
        .unwrap_or_default();
    let mut note: Vec<Line> = Vec::new();
    for para in body.split('\n') {
        if para.trim().is_empty() {
            note.push(Line::from(""));
            continue;
        }
        for l in wrap_words(para, w) {
            note.push(Line::from(Span::styled(l, Style::default().fg(t.text))));
        }
    }
    let scroll = pane.scroll.min(note.len().saturating_sub(1));
    f.render_widget(
        Paragraph::new(note.into_iter().skip(scroll).collect::<Vec<_>>()),
        note_area,
    );

    let mut list = vec![Line::from(Span::styled(
        match pane.appears.len() {
            0 => "Not in the manuscript yet".to_string(),
            1 => "Appears in 1 scene".to_string(),
            n => format!("Appears in {n} scenes"),
        },
        Style::default().fg(t.accent),
    ))];
    let open_path = app.open.map(|i| app.project.nodes[i].path.clone());
    let room = (list_h as usize).saturating_sub(1);
    let start = pane.sel.saturating_sub(room.saturating_sub(1));
    for (i, a) in pane.appears.iter().enumerate().skip(start).take(room) {
        let here = open_path.as_ref() == Some(&a.scene);
        let on = focused && i == pane.sel;
        let count = format!(" ×{}", a.count);
        let place = truncate(&a.place, w.saturating_sub(count.chars().count() + 2));
        let row = Line::from(vec![
            Span::styled(if here { "● " } else { "  " }, Style::default().fg(t.sun)),
            Span::styled(
                place,
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::styled(count, dim),
        ]);
        list.push(if on {
            row.style(Style::default().bg(t.sel))
        } else {
            row
        });
    }
    f.render_widget(Paragraph::new(list), list_area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let total = app.project.total_words();
    let target = app.project.meta.target_words.max(1);
    let filled = (total * 10 / target).min(10);
    let bar: String = "▓".repeat(filled) + &"░".repeat(10 - filled);
    let today = app.today_words();
    let today_text = if today < 0 {
        format!("−{}", thousands(today.unsigned_abs() as usize))
    } else {
        thousands(today as usize)
    };

    let today_style = Style::default().fg(if today >= app.project.meta.daily_target as i64 {
        t.accent
    } else {
        t.dim
    });

    let mut spans = if app.focus_mode {
        // Focus mode keeps to the scene: its name, its words, and today.
        let (title, words) = app.open.map_or((String::new(), 0), |i| {
            (
                app.project.nodes[i].title.clone(),
                app.project.nodes[i].words(),
            )
        });
        vec![
            Span::raw(" "),
            Span::styled(title, Style::default().fg(t.accent)),
            Span::styled(
                format!("  {} words", thousands(words)),
                Style::default().fg(t.dim),
            ),
            Span::styled(format!("  today {today_text}"), today_style),
        ]
    } else {
        vec![
            Span::raw(" "),
            Span::styled(thousands(total), Style::default().fg(t.text)),
            Span::styled(
                format!(" / {} ", thousands(target)),
                Style::default().fg(t.dim),
            ),
            Span::styled(bar, Style::default().fg(t.accent)),
            Span::styled(format!("  today {today_text}"), today_style),
        ]
    };

    if let Some((label, reached)) = app.sprint_label() {
        spans.push(Span::styled(
            format!("  {label}"),
            Style::default().fg(if reached { t.accent } else { t.sun }),
        ));
    }

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
    spans.push(Span::styled(
        format!("{hints} "),
        Style::default().fg(t.dim),
    ));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── overlays ─────────────────────────────────────────────────────────

pub(crate) fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// Greedy word wrap for short card text.
pub(crate) fn wrap_words(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let need = if cur.is_empty() {
            word.chars().count()
        } else {
            cur.chars().count() + 1 + word.chars().count()
        };
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

pub(crate) fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(crate) fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kept version reads against now, never as a loss: "6 fewer" looked
    /// like six words had gone missing.

    #[test]
    fn hints_that_do_not_fit_drop_whole_from_the_right() {
        let hints = "Tab pane  ↵ fold  n scene  c chapter  F1 menu ";
        assert_eq!(
            fit_hints(hints, 100),
            "Tab pane  ↵ fold  n scene  c chapter  F1 menu"
        );
        assert_eq!(fit_hints(hints, 30), "Tab pane  ↵ fold  n scene");
        assert_eq!(fit_hints(hints, 5), "");
    }

    #[test]
    fn popup_footers_light_their_keys() {
        let keys = |text: &str| -> Vec<String> {
            let parts = hint_parts(text);
            assert_eq!(
                parts.iter().map(|(p, _)| p.as_str()).collect::<String>(),
                text
            );
            parts
                .into_iter()
                .filter(|(_, k)| *k)
                .map(|(p, _)| p)
                .collect()
        };
        assert_eq!(keys(" y delete   esc keep it"), ["y", "esc"]);
        assert_eq!(keys(" type a name   ↵ create   esc cancel"), ["↵", "esc"]);
        assert_eq!(
            keys("↑↓ pick a version   PgUp PgDn scroll   ↵ restore it"),
            ["↑↓", "PgUp PgDn", "↵"]
        );
        assert_eq!(
            keys("↑↓ scroll   any other key: back to sessions"),
            ["↑↓", "any other key"]
        );
        assert_eq!(keys(" type hex · ↵ next · esc save & close"), ["↵", "esc"]);
        assert_eq!(
            keys(" space pause  ←→ seek 10s  [ ] prev/next  +/- volume"),
            ["space", "←→", "[ ]", "+/-"]
        );
        assert_eq!(
            keys("   ↵ next  ↑ previous  Tab replace  ⌘F whole book"),
            ["↵", "↑", "Tab", "⌘F"]
        );
    }

    #[test]
    fn the_view_switcher_names_both_views_inside_the_scene_pane() {
        let inner = LEFT_W - 2;
        let (hits, spans) = view_switch(Mode::Visualizer, inner);
        let text: String = spans.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(text.trim_start_matches('─'), "◂ pomodoro visualizer ▸");
        assert!(text.chars().count() as u16 <= inner);
        let pad = text.chars().take_while(|c| *c == '─').count() as u16;
        assert!(pad >= 2, "centred on the edge: {text:?}");
        let lit: Vec<&str> = spans
            .iter()
            .filter(|(_, k)| *k == Piece::Current)
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(lit, ["visualizer"]);
        // Arrows step from the current view; each name goes to itself.
        let at = |x: u16| {
            hits.iter()
                .find(|(o, w, _)| x >= *o && x < o + w)
                .map(|h| h.2)
        };
        assert_eq!(at(pad), Some(Mode::Pomodoro), "◂");
        assert_eq!(at(pad + 2), Some(Mode::Pomodoro), "pomodoro");
        assert_eq!(at(pad + 11), Some(Mode::Visualizer), "visualizer");
        assert_eq!(at(pad + 22), Some(Mode::Pomodoro), "▸");
        assert_eq!(at(pad + 1), None);
    }

    #[test]
    fn a_narrow_switcher_drops_names_but_keeps_its_arrows() {
        let (_, spans) = view_switch(Mode::Pomodoro, 14);
        let text: String = spans.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(text.trim_start_matches('─'), "◂ pomodoro ▸");
    }
}
