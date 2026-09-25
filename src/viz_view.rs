//! The Visualizer's look.
//!
//! [`crate::visualizer`] hears whatever the machine is playing; this draws
//! it, and draws it differently for every theme. A [`Look`] is a handful of
//! choices — the glyphs a bar is built from, how wide the bars stand and how
//! far apart, whether they rise from the floor or open out from the middle,
//! what lies beneath them, how colour runs through them, and what the
//! playback row is made of. [`look_for`] picks one per preset; Custom keeps
//! the original: dense eighth-blocks over a rippling pond.
//!
//! The song playing is always named: in the pane's title, or — for the looks
//! whose playback row is the title itself — along the bottom, lit up to the
//! point the song has reached.

// A character grid; indexing it by column reads better than zipped iterators.
#![allow(clippy::needless_range_loop)]

use crate::app::App;
use crate::music::State as MusicState;
use crate::scene::{Cell, H, Ink, W};
use crate::theme::Theme;
use crate::ui::{bar_colour, blend, truncate};
use crate::visualizer::Analyzer;
use ratatui::style::Color;

/// What a bar is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fill {
    /// Eighth blocks: eight steps a row, the finest there is.
    Eighths,
    /// Shade blocks: a softer, grainier bar.
    Shade,
    /// Braille dots: four steps a row, fine and airy.
    Braille,
    /// A string of beads.
    Beads,
    /// A thin line.
    Thin,
    /// A heavy line.
    Heavy,
    /// Stacked segments with dark gaps between them, like a hardware meter.
    Led,
    /// Stacked squares, a heat-map column.
    Squares,
    /// Buildings: walls with lit windows.
    Skyline,
}

impl Fill {
    /// Steps of height in one row.
    fn res(self) -> usize {
        match self {
            Fill::Eighths => 8,
            Fill::Braille => 4,
            Fill::Shade => 3,
            Fill::Beads | Fill::Thin | Fill::Heavy => 2,
            Fill::Led | Fill::Squares | Fill::Skyline => 1,
        }
    }

    fn body(self) -> char {
        match self {
            Fill::Eighths => '█',
            Fill::Shade => '▓',
            Fill::Braille => '⣿',
            Fill::Beads => '●',
            Fill::Thin => '│',
            Fill::Heavy => '┃',
            Fill::Led => '▄',
            Fill::Squares => '■',
            Fill::Skyline => '▓',
        }
    }

    /// The top of a bar that ends part-way up a row, `part` of `res()` steps.
    fn rising(self, part: usize) -> char {
        match self {
            Fill::Eighths => [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇'][part],
            Fill::Shade => [' ', '░', '▒'][part],
            Fill::Braille => [' ', '⣀', '⣤', '⣶'][part],
            Fill::Beads => [' ', '•'][part],
            Fill::Thin => [' ', '╷'][part],
            Fill::Heavy => [' ', '╻'][part],
            _ => ' ',
        }
    }

    /// The same, hanging down from the middle line.
    fn hanging(self, part: usize) -> char {
        match self {
            Fill::Eighths => [' ', '▔', '▔', '▀', '▀', '▀', '▀', '█'][part],
            Fill::Braille => [' ', '⠉', '⠛', '⠿'][part],
            Fill::Thin => [' ', '╵'][part],
            Fill::Heavy => [' ', '╹'][part],
            other => other.rising(part),
        }
    }
}

/// Which way the bars grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Rising,
    /// Out from a line across the middle, up and down at once.
    Mirrored,
}

/// What lies beneath the bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Floor {
    None,
    /// The bars' reflection, squashed and rippling.
    Pond,
    /// A soft shadow of the tallest bars.
    Shadow,
    /// A shelf the bars stand on.
    Shelf,
    /// Drifts of snow.
    Snow,
    /// A horizon and a grid running out to it.
    Grid,
    /// A rolling wave.
    Waves,
    /// Sand dunes.
    Dunes,
    /// A street under the buildings.
    Street,
    /// Grass, with the odd flower.
    Meadow,
}

impl Floor {
    fn rows(self) -> usize {
        match self {
            Floor::None => 0,
            Floor::Pond | Floor::Shadow | Floor::Grid => 2,
            _ => 1,
        }
    }
}

/// A colour of the theme's, named so a look can pick from any theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Accent,
    Text,
    Warn,
    Sun,
    Moon,
    Foliage,
    Bark,
    Bloom,
    Turf,
}

impl Role {
    fn of(self, t: &Theme) -> Color {
        match self {
            Role::Accent => t.accent,
            Role::Text => t.text,
            Role::Warn => t.warn,
            Role::Sun => t.sun,
            Role::Moon => t.moon,
            Role::Foliage => t.foliage,
            Role::Bark => t.bark,
            Role::Bloom => t.bloom,
            Role::Turf => t.turf,
        }
    }
}

/// How colour runs through the bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paint {
    /// Root to tip through foliage, accent, sun and bloom (red to violet
    /// under Rainbow).
    Height,
    /// Root to tip through three of the theme's colours.
    Ramp([Role; 3]),
    /// Left to right through three of the theme's colours.
    Column([Role; 3]),
    /// A meter's green, amber and red bands.
    Heat,
    /// Each bar its own colour of the theme's, darker at the root.
    Stripes,
    /// One colour, the tips in another.
    Duo(Role, Role),
    /// Greener the higher: a heat-map column.
    Contribution,
    /// Night walls with lit windows.
    Skyline,
}

/// What the playback row along the bottom is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Progress {
    /// Heavy where played, light ahead.
    Line,
    /// Dots.
    Dots,
    /// Filled and empty parallelograms.
    Beads,
    /// The song's own name, lit up to the point it has reached.
    Title,
}

/// Everything that makes one theme's Visualizer its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Look {
    pub fill: Fill,
    pub shape: Shape,
    /// Columns per bar, and columns between them.
    pub bar: usize,
    pub gap: usize,
    /// What's left above a bar after it falls; None for no caps.
    pub cap: Option<char>,
    pub floor: Floor,
    pub paint: Paint,
    /// Sparks off the beat, new to nearly gone.
    pub sparks: [char; 3],
    pub progress: Progress,
}

/// The original look, kept for Custom.
pub(crate) const ORIGINAL: Look = Look {
    fill: Fill::Eighths,
    shape: Shape::Rising,
    bar: 1,
    gap: 0,
    cap: Some('▔'),
    floor: Floor::Pond,
    paint: Paint::Height,
    sparks: ['✦', '•', '·'],
    progress: Progress::Line,
};

/// Each preset's Visualizer. Every one differs from every other in the shape
/// of what's drawn, not only its colours.
pub(crate) fn look_for(t: &Theme) -> Look {
    use Fill::*;
    use Role::*;
    let l = |fill, shape, bar, gap, cap, floor, paint, sparks, progress| Look {
        fill,
        shape,
        bar,
        gap,
        cap,
        floor,
        paint,
        sparks,
        progress,
    };
    let (r, m) = (Shape::Rising, Shape::Mirrored);
    let dots = ['·', '·', '·'];
    match t.name.as_str() {
        // An illuminated manuscript: broad bars on a shelf, the song written
        // along the bottom.
        "Grimoire" => l(
            Eighths,
            r,
            2,
            1,
            Some('▔'),
            Floor::Shelf,
            Paint::Height,
            ['✦', '•', '·'],
            Progress::Title,
        ),
        // A hardware meter: green, amber, red.
        "Gruvbox Dark" => l(
            Led,
            r,
            1,
            1,
            Some('▔'),
            Floor::None,
            Paint::Heat,
            dots,
            Progress::Dots,
        ),
        // Braille frost over snowdrifts, snow on the beat.
        "Nord" => l(
            Braille,
            r,
            1,
            0,
            Some('·'),
            Floor::Snow,
            Paint::Column([Moon, Accent, Text]),
            ['*', '·', '·'],
            Progress::Line,
        ),
        // Open from the middle, like wings.
        "Dracula" => l(
            Eighths,
            m,
            1,
            1,
            None,
            Floor::None,
            Paint::Height,
            ['✦', '•', '·'],
            Progress::Line,
        ),
        // Fine lines, a ruler's worth.
        "Solarized Dark" => l(
            Thin,
            r,
            1,
            1,
            Some('─'),
            Floor::None,
            Paint::Column([Moon, Foliage, Sun]),
            dots,
            Progress::Dots,
        ),
        // Pastel beads.
        "Catppuccin Mocha" => l(
            Beads,
            r,
            1,
            1,
            Some('•'),
            Floor::None,
            Paint::Column([Bloom, Accent, Moon]),
            ['•', '·', '·'],
            Progress::Title,
        ),
        // A city at night: buildings with lit windows over a street.
        "Tokyo Night" => l(
            Skyline,
            r,
            2,
            1,
            None,
            Floor::Street,
            Paint::Skyline,
            dots,
            Progress::Line,
        ),
        // Stems in a meadow, flowers for caps.
        "Everforest" => l(
            Eighths,
            r,
            1,
            1,
            Some('•'),
            Floor::Meadow,
            Paint::Height,
            ['✿', '·', '·'],
            Progress::Line,
        ),
        // The house look: dense bars over a pond, fireflies on the beat.
        "Lost Forest" => l(
            Eighths,
            r,
            1,
            0,
            Some('▔'),
            Floor::Pond,
            Paint::Height,
            ['•', '•', '·'],
            Progress::Line,
        ),
        // Braille columns and falling petals.
        "Rosé Pine" => l(
            Braille,
            r,
            2,
            1,
            None,
            Floor::None,
            Paint::Column([Bloom, Accent, Moon]),
            ['❀', '✿', '·'],
            Progress::Title,
        ),
        // Clean blue bars with white tips over a soft shadow.
        "One Dark" => l(
            Eighths,
            r,
            1,
            1,
            Some('▔'),
            Floor::Shadow,
            Paint::Duo(Accent, Text),
            dots,
            Progress::Line,
        ),
        // Every bar a different Monokai colour.
        "Monokai" => l(
            Shade,
            r,
            1,
            1,
            Some('▔'),
            Floor::None,
            Paint::Stripes,
            ['✦', '•', '·'],
            Progress::Dots,
        ),
        // Deep water to foam, over a rolling wave.
        "Kanagawa" => l(
            Eighths,
            r,
            1,
            0,
            Some('~'),
            Floor::Waves,
            Paint::Ramp([Moon, Accent, Text]),
            dots,
            Progress::Line,
        ),
        // Heat haze over the dunes.
        "Ayu Mirage" => l(
            Shade,
            r,
            2,
            1,
            Some('░'),
            Floor::Dunes,
            Paint::Ramp([Bark, Accent, Warn]),
            dots,
            Progress::Dots,
        ),
        // Heavy lines under a sky of stars.
        "Night Owl" => l(
            Heavy,
            r,
            1,
            1,
            Some('✧'),
            Floor::None,
            Paint::Column([Moon, Accent, Bloom]),
            ['✧', '·', '·'],
            Progress::Line,
        ),
        // Soft braille and its shadow.
        "Material Palenight" => l(
            Braille,
            r,
            1,
            1,
            None,
            Floor::Shadow,
            Paint::Column([Accent, Bloom, Sun]),
            dots,
            Progress::Beads,
        ),
        // A sunset over the grid.
        "Synthwave '84" => l(
            Eighths,
            r,
            1,
            1,
            Some('▔'),
            Floor::Grid,
            Paint::Ramp([Turf, Bloom, Sun]),
            ['✦', '·', '·'],
            Progress::Line,
        ),
        // A column of green squares.
        "GitHub Dark" => l(
            Squares,
            r,
            1,
            1,
            None,
            Floor::None,
            Paint::Contribution,
            dots,
            Progress::Dots,
        ),
        // A prism: every hue, opening out from the middle.
        "Rainbow" => l(
            Eighths,
            m,
            1,
            0,
            None,
            Floor::None,
            Paint::Height,
            ['✦', '•', '·'],
            Progress::Title,
        ),
        _ => ORIGINAL,
    }
}

/// What's playing, as "title — artist", or just the title if there's no
/// artist; None when nothing is loaded.
fn song(app: &App) -> Option<String> {
    match &app.music.state {
        MusicState::Playing(tr) if !tr.title.is_empty() => Some(if tr.artist.is_empty() {
            tr.title.clone()
        } else {
            format!("{} — {}", tr.title, tr.artist)
        }),
        _ => None,
    }
}

/// The pane's title and its contents, for the view switcher's Visualizer.
/// `width` is the pane's outer width, so the title fits between its corners.
pub(crate) fn view(app: &App, t: &Theme, width: u16) -> (String, Vec<Vec<Cell>>) {
    let look = if t.name == "Custom" {
        ORIGINAL
    } else {
        look_for(t)
    };
    let (frac, playing) = match &app.music.state {
        MusicState::Playing(tr) => (
            if tr.duration > 0.0 {
                tr.progress / tr.duration
            } else {
                0.0
            },
            tr.playing,
        ),
        _ => (0.0, false),
    };
    let song = song(app);
    let note = app.viz.note(playing);
    let label = label(&look, song.as_deref(), width);
    let grid = render(&look, t, &app.viz, frac, song.as_deref(), note.as_deref());
    (label, grid)
}

/// The pane's title: the song, unless the look writes it along the bottom.
pub(crate) fn label(look: &Look, song: Option<&str>, width: u16) -> String {
    // The title sits between the corners, a space either side.
    let room = (width as usize).saturating_sub(4).max(1);
    match song {
        Some(s) if look.progress != Progress::Title => truncate(&format!("♪ {s}"), room),
        _ => "visualizer".into(),
    }
}

/// Draw a look into the pane's 28×9 grid. `note`, when there is one, takes
/// the place of the bars (the floor and the playback row stay), and says why
/// there's nothing to show.
pub(crate) fn render(
    look: &Look,
    t: &Theme,
    a: &Analyzer,
    frac: f64,
    song: Option<&str>,
    note: Option<&str>,
) -> Vec<Vec<Cell>> {
    let mut g = vec![vec![(' ', Ink::Paint(t.border)); W]; H];
    let rows = H - 1 - look.floor.rows();
    let beat = a.beat.clamp(0.0, 1.0);

    // Lay the bars out across the pane, centred.
    let n = ((W + look.gap) / (look.bar + look.gap)).max(1);
    let span = n * look.bar + (n - 1) * look.gap;
    let x0 = (W - span.min(W)) / 2;
    let columns = |i: usize| {
        let x = x0 + i * (look.bar + look.gap);
        x..(x + look.bar).min(W)
    };
    // Each bar the loudest of the analyser's bands it covers.
    let sample = |v: &[f32], i: usize| -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        let (from, to) = (
            i * v.len() / n,
            ((i + 1) * v.len() / n).max(i * v.len() / n + 1),
        );
        v[from..to.min(v.len())]
            .iter()
            .fold(0.0f32, |m, &x| m.max(x))
            .clamp(0.0, 1.0)
    };
    let levels: Vec<f32> = (0..n)
        .map(|i| {
            if note.is_some() {
                0.0
            } else {
                sample(&a.levels, i)
            }
        })
        .collect();

    // A note too long for the space above the floor takes the floor's rows
    // too, rather than lose its last words.
    let tall = note.is_some_and(|text| wrap(text, W - 2).len() > resting_row(look, rows));
    if let Some(text) = note {
        draw_note(
            &mut g,
            look,
            t,
            if tall { H - 1 } else { rows },
            text,
            !tall,
        );
    } else {
        for i in 0..n {
            draw_bar(
                &mut g, look, t, a, i, n, rows, levels[i], beat, &columns, &sample,
            );
        }
        draw_sparks(&mut g, look, t, a, rows);
    }
    if !tall {
        draw_floor(&mut g, look, t, a, rows, &levels, &columns, n);
    }
    draw_progress(&mut g, look, t, frac, beat, song);
    g
}

/// Colour at height `h` (0 at the root, 1 at the tip) of bar `i` of `n`.
fn paint(look: &Look, t: &Theme, i: usize, n: usize, h: f32) -> Color {
    let across = if n > 1 {
        i as f32 / (n - 1) as f32
    } else {
        0.0
    };
    match look.paint {
        Paint::Height => bar_colour(t, (h.clamp(0.0, 1.0) * 255.0) as u8),
        Paint::Ramp(stops) => ramp(t, stops, h),
        Paint::Column(stops) => blend(ramp(t, stops, across), t.text, h * 0.12),
        Paint::Heat => {
            if h < 0.55 {
                t.foliage
            } else if h < 0.8 {
                t.sun
            } else {
                t.warn
            }
        }
        Paint::Stripes => {
            let hues = [t.bloom, t.sun, t.foliage, t.moon, t.accent, t.warn];
            blend(hues[i % hues.len()], t.border, (1.0 - h) * 0.35)
        }
        Paint::Duo(body, _) => body.of(t),
        Paint::Contribution => {
            let q = (h * 4.0).floor().min(3.0) / 3.0;
            blend(blend(t.turf, t.border, 0.3), t.foliage, q)
        }
        Paint::Skyline => blend(t.border, t.accent, 0.35),
    }
}

/// Evenly spaced stops, `x` from 0 to 1.
fn ramp(t: &Theme, stops: [Role; 3], x: f32) -> Color {
    let x = x.clamp(0.0, 1.0) * 2.0;
    if x <= 1.0 {
        blend(stops[0].of(t), stops[1].of(t), x)
    } else {
        blend(stops[1].of(t), stops[2].of(t), x - 1.0)
    }
}

/// A stable pseudo-random number for a cell, so windows and drifts don't
/// flicker from frame to frame.
fn hash(a: usize, b: usize) -> usize {
    let mut x = (a as u32).wrapping_mul(0x9e37_79b9) ^ (b as u32).wrapping_mul(0x85eb_ca6b);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    (x ^ (x >> 12)) as usize
}

#[allow(clippy::too_many_arguments)]
fn draw_bar(
    g: &mut [Vec<Cell>],
    look: &Look,
    t: &Theme,
    a: &Analyzer,
    i: usize,
    n: usize,
    rows: usize,
    level: f32,
    beat: f32,
    columns: &dyn Fn(usize) -> std::ops::Range<usize>,
    sample: &dyn Fn(&[f32], usize) -> f32,
) {
    let res = look.fill.res();
    // Up and down from the middle, or up from the floor.
    let (halves, reach) = match look.shape {
        Shape::Rising => (1, rows),
        Shape::Mirrored => (2, rows / 2),
    };
    let steps = (level * (reach * res) as f32).round() as usize;
    let (full, part) = (steps / res, steps % res);
    let top = if part > 0 {
        full
    } else {
        full.saturating_sub(1)
    };
    for half in 0..halves {
        for k in 0..reach {
            let ch = if k < full {
                look.fill.body()
            } else if k == full && part > 0 {
                if half == 0 {
                    look.fill.rising(part)
                } else {
                    look.fill.hanging(part)
                }
            } else {
                break;
            };
            let y = match (look.shape, half) {
                (Shape::Rising, _) => rows - 1 - k,
                (Shape::Mirrored, 0) => rows / 2 - 1 - k,
                _ => rows / 2 + k,
            };
            let h = (k as f32 + 0.5) / reach as f32;
            let tip = k == top;
            let mut colour = paint(look, t, i, n, h);
            let mut ch = ch;
            if tip && let Paint::Duo(_, tip_role) = look.paint {
                colour = tip_role.of(t);
            }
            if look.paint == Paint::Skyline {
                // A lit window here and there, fixed to its building and floor.
                if hash(i, k).is_multiple_of(3) {
                    ch = '▪';
                    colour = t.sun;
                } else if tip {
                    ch = '▀';
                }
            }
            // The beat lifts every bar a little, the tips a little more.
            let glow = beat * 0.3 + if tip { 0.15 } else { 0.0 };
            colour = blend(colour, t.text, glow);
            for x in columns(i) {
                g[y][x] = (ch, Ink::Paint(colour));
            }
        }
    }

    // A cap that lingers above the bar after it falls.
    if let (Some(cap), Shape::Rising) = (look.cap, look.shape) {
        let peak = (sample(&a.peaks, i) * (rows * res) as f32).round() as usize;
        if peak >= steps + res + res / 2 {
            let y = rows - 1 - (peak / res).min(rows - 1);
            let holding =
                !a.hold.is_empty() && a.hold[(i * a.hold.len() / n).min(a.hold.len() - 1)] > 0.0;
            let colour = blend(t.dim, t.moon, if holding { 1.0 } else { 0.45 });
            for x in columns(i) {
                if g[y][x].0 == ' ' {
                    g[y][x] = (cap, Ink::Paint(colour));
                }
            }
        }
    }
}

fn draw_sparks(g: &mut [Vec<Cell>], look: &Look, t: &Theme, a: &Analyzer, rows: usize) {
    for s in &a.sparks {
        let (x, r) = ((s.x * W as f32) as usize, (s.y * rows as f32) as usize);
        if x >= W || r >= rows {
            continue;
        }
        let y = rows - 1 - r;
        if g[y][x].0 != ' ' {
            continue;
        }
        let life = s.life.clamp(0.0, 1.0);
        let ch = if life > 0.66 {
            look.sparks[0]
        } else if life > 0.33 {
            look.sparks[1]
        } else {
            look.sparks[2]
        };
        let bright = if t.is_rainbow() {
            crate::theme::hue(x as f32 * 12.0)
        } else {
            blend(t.sun, t.text, 0.35)
        };
        g[y][x] = (ch, Ink::Paint(blend(t.dim, bright, life)));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_floor(
    g: &mut [Vec<Cell>],
    look: &Look,
    t: &Theme,
    a: &Analyzer,
    rows: usize,
    levels: &[f32],
    columns: &dyn Fn(usize) -> std::ops::Range<usize>,
    n: usize,
) {
    let clock = a.clock;
    let level_at = |x: usize| -> f32 {
        (0..n)
            .find(|&i| columns(i).contains(&x))
            .map_or(0.0, |i| levels[i])
    };
    match look.floor {
        Floor::None => {}
        Floor::Pond => {
            // Every bar reflected upside down and squashed to fit; a slow wave
            // nudges each row sideways, so the water seems to move.
            for d in 0..2 {
                for x in 0..W {
                    let wave = (clock * 2.3 + d as f32 * 1.9 + x as f32 * 0.7).sin() * 0.8;
                    let from =
                        (x as isize + wave.round() as isize).clamp(0, W as isize - 1) as usize;
                    let level = level_at(from);
                    let wet = level * level * 2.0 - d as f32;
                    let ch = if wet >= 0.5 {
                        '▀'
                    } else if wet > 0.15 {
                        '▔'
                    } else {
                        continue;
                    };
                    let h = (d as f32 + 0.5) / 2.0;
                    let colour = blend(paint(look, t, 0, 1, h), t.border, 0.45 + 0.2 * d as f32);
                    g[rows + d][x] = (ch, Ink::Paint(colour));
                }
            }
        }
        Floor::Shadow => {
            for x in 0..W {
                let level = level_at(x);
                let i = (0..n).find(|&i| columns(i).contains(&x)).unwrap_or(0);
                let base = blend(paint(look, t, i, n, 0.2), t.border, 0.65);
                let (near, far) = match look.fill {
                    Fill::Braille => ('⠛', '⠉'),
                    _ => ('▀', '░'),
                };
                if level > 0.12 {
                    g[rows][x] = (near, Ink::Paint(base));
                }
                if level > 0.5 {
                    g[rows + 1][x] = (far, Ink::Paint(blend(base, t.border, 0.4)));
                }
            }
        }
        Floor::Shelf => {
            for x in 0..W {
                g[rows][x] = ('▀', Ink::Paint(t.bark));
            }
        }
        Floor::Snow => {
            for x in 0..W {
                let ch = ['▁', '▁', '▂', '▁', '▃', '▂'][hash(x, 7) % 6];
                g[rows][x] = (ch, Ink::Paint(blend(t.text, t.moon, 0.3)));
            }
        }
        Floor::Grid => {
            // The horizon, then lines running out from its middle; every
            // other beat of a slow clock a crossbar rolls toward you.
            for x in 0..W {
                g[rows][x] = ('━', Ink::Paint(t.bloom));
            }
            let mid = W / 2;
            let roll = ((clock * 0.8) as usize).is_multiple_of(2);
            for x in 0..W {
                let off = x as isize - mid as isize;
                let ch = if off == 0 {
                    '│'
                } else if [2, 5, 9, 13].contains(&off.unsigned_abs()) {
                    if off < 0 { '╱' } else { '╲' }
                } else if roll {
                    '─'
                } else {
                    ' '
                };
                g[rows + 1][x] = (ch, Ink::Paint(blend(t.turf, t.moon, 0.35)));
            }
        }
        Floor::Waves => {
            // A slow swell: the pattern drifts one cell every second or so.
            let shift = (clock * 0.9) as usize;
            for x in 0..W {
                let ch = ['∿', '~', '≈', '~'][(x + shift) % 4];
                let crest = hash(x + shift, 3).is_multiple_of(5);
                let colour = if crest {
                    t.text
                } else {
                    blend(t.moon, t.border, 0.3)
                };
                g[rows][x] = (ch, Ink::Paint(colour));
            }
        }
        Floor::Dunes => {
            for x in 0..W {
                let ch = ['▁', '▂', '▃', '▂', '▁', '▁'][(x / 2 + hash(x / 5, 1) % 3) % 6];
                g[rows][x] = (ch, Ink::Paint(t.bark));
            }
        }
        Floor::Street => {
            for x in 0..W {
                // The lights of the windows above, smeared on the wet road.
                let lit = level_at(x) > 0.3 && hash(x, 11).is_multiple_of(2);
                let colour = if lit {
                    blend(t.sun, t.border, 0.5)
                } else {
                    blend(t.border, t.moon, 0.3)
                };
                g[rows][x] = ('▔', Ink::Paint(colour));
            }
        }
        Floor::Meadow => {
            for x in 0..W {
                let (ch, colour) = if hash(x, 5).is_multiple_of(9) {
                    ('•', t.bloom)
                } else {
                    (['▁', '▂', '▁', '▃'][hash(x, 2) % 4], t.turf)
                };
                g[rows][x] = (ch, Ink::Paint(colour));
            }
        }
    }
}

/// The bottom row: how far through the song, lit on the beat.
fn draw_progress(
    g: &mut [Vec<Cell>],
    look: &Look,
    t: &Theme,
    frac: f64,
    beat: f32,
    song: Option<&str>,
) {
    let head = (frac.clamp(0.0, 1.0) * W as f64).round() as usize;
    let played = blend(t.accent, t.bloom, beat * 0.5);
    let ahead = t.turf;
    let y = H - 1;
    match (look.progress, song) {
        (Progress::Title, Some(s)) => {
            // The name itself is the bar: lit as far as the song has got.
            let text: Vec<char> = truncate(&format!("♪ {s}"), W).chars().collect();
            for x in 0..W {
                let ch = text
                    .get(x)
                    .copied()
                    .unwrap_or(if x == text.len() { ' ' } else { '─' });
                let colour = match (x < head, ch) {
                    (true, _) => played,
                    (false, '─') => ahead,
                    (false, _) => t.dim,
                };
                g[y][x] = (ch, Ink::Paint(colour));
            }
        }
        (Progress::Dots, _) => {
            for x in 0..W {
                g[y][x] = if x < head {
                    ('•', Ink::Paint(played))
                } else {
                    ('·', Ink::Paint(ahead))
                };
            }
        }
        (Progress::Beads, _) => {
            for x in 0..W {
                g[y][x] = if x < head {
                    ('▰', Ink::Paint(played))
                } else {
                    ('▱', Ink::Paint(ahead))
                };
            }
        }
        _ => {
            for x in 0..W {
                g[y][x] = if x < head {
                    ('━', Ink::Paint(played))
                } else {
                    ('─', Ink::Paint(ahead))
                };
            }
        }
    }
}

/// Why there's nothing to show, where the bars would be: a resting line of
/// the look's own glyph along the bottom (when `resting`), the reason centred
/// above it — the first line a shade brighter, the rest dim.
fn draw_note(g: &mut [Vec<Cell>], look: &Look, t: &Theme, rows: usize, text: &str, resting: bool) {
    // An empty heat-map is still a grid of squares; the rest rest on a
    // thin line — across the middle, for the looks that open out from it.
    let rest = match look.fill {
        Fill::Squares => '■',
        Fill::Beads => '•',
        Fill::Braille | Fill::Thin | Fill::Heavy => look.fill.rising(1),
        _ => '▁',
    };
    let line = resting_row(look, rows);
    if resting && rows > 2 {
        for x in 0..W {
            g[line][x] = (rest, Ink::Paint(blend(t.border, t.dim, 0.35)));
        }
    }
    let lines = wrap(text, W - 2);
    // Above the resting line, or the whole pane if it had to give that up.
    let space = if resting { line } else { rows }.max(1);
    let top = space.saturating_sub(lines.len()) / 2;
    for (k, line) in lines.iter().enumerate().take(space) {
        let x0 = W.saturating_sub(line.chars().count()) / 2;
        let colour = if k == 0 {
            blend(t.dim, t.text, 0.45)
        } else {
            t.dim
        };
        for (j, c) in line.chars().enumerate().take(W - x0) {
            g[top + k][x0 + j] = (c, Ink::Paint(colour));
        }
    }
}

/// The row a resting look's line lies along — the bottom of the bars, or
/// the middle for the looks that open out from it. A note goes above it.
fn resting_row(look: &Look, rows: usize) -> usize {
    match look.shape {
        Shape::Rising => rows.saturating_sub(1),
        Shape::Mirrored => (rows / 2).saturating_sub(1),
    }
}

/// Word-wrap for the short notes the pane can show.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::presets;
    use ratatui::text::Span;

    const NO_TOOLS: &str =
        "the visualizer needs pw-record (PipeWire) or parec (PulseAudio) to hear what's playing";
    const MAC_SILENT: &str = "hearing only silence. add your terminal to Privacy & Security › \
        Screen & System Audio Recording › System Audio Recording Only";

    /// A loud, uneven signal: every bar a different height, some peaks held
    /// above, a beat just landing.
    fn signal() -> Analyzer {
        let mut a = Analyzer::new(W);
        for x in 0..W {
            let v = ((x as f32 * 0.9).sin() * 0.5 + 0.5) * (0.35 + 0.65 * (x % 7) as f32 / 6.0);
            a.levels[x] = v;
            a.peaks[x] = (v + 0.25).min(1.0);
            a.hold[x] = if x % 3 == 0 { 0.2 } else { 0.0 };
        }
        a.beat = 0.6;
        a.clock = 3.7;
        a
    }

    fn draw(t: &Theme, note: Option<&str>) -> Vec<Vec<Cell>> {
        render(
            &look_for(t),
            t,
            &signal(),
            0.4,
            Some("Light Years — The National"),
            note,
        )
    }

    #[test]
    fn every_preset_draws_inside_the_pane_in_single_cells() {
        for t in presets() {
            for note in [
                None,
                Some("nothing playing"),
                Some(NO_TOOLS),
                Some(MAC_SILENT),
            ] {
                let g = draw(&t, note);
                assert_eq!(g.len(), H, "{}", t.name);
                for row in &g {
                    assert_eq!(row.len(), W, "{}", t.name);
                    for (c, _) in row {
                        assert_eq!(
                            Span::raw(c.to_string()).width(),
                            1,
                            "{} draws a wide {c:?}",
                            t.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn no_two_presets_draw_the_same_shape() {
        // Compared by glyph alone: each theme's look differs in what's drawn,
        // not only in its colours.
        let shapes: Vec<(String, String)> = presets()
            .iter()
            .map(|t| {
                let g = draw(t, None);
                (
                    t.name.clone(),
                    g.iter().flatten().map(|(c, _)| *c).collect(),
                )
            })
            .collect();
        for i in 0..shapes.len() {
            for j in i + 1..shapes.len() {
                assert_ne!(
                    shapes[i].1, shapes[j].1,
                    "{} looks like {}",
                    shapes[i].0, shapes[j].0
                );
            }
        }
    }

    #[test]
    fn a_long_note_keeps_its_last_words_in_every_look() {
        for t in presets() {
            let g = draw(&t, Some(MAC_SILENT));
            let text: String = g
                .iter()
                .map(|r| r.iter().map(|(c, _)| *c).collect::<String>())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                text.contains("Only"),
                "{} cut the note short:\n{text}",
                t.name
            );
        }
    }

    // ---- the original look (Custom), as scene.rs used to test it ----

    fn count(g: &[Vec<Cell>], c: char) -> usize {
        g.iter().flatten().filter(|(ch, _)| *ch == c).count()
    }

    /// An analyser frozen with every bar at `level` and every cap at `peak`.
    fn viz(level: f32, peak: f32) -> Analyzer {
        let mut a = Analyzer::new(W);
        a.levels = vec![level; W];
        a.peaks = vec![peak; W];
        a
    }

    fn original(a: &Analyzer, frac: f64, note: Option<&str>) -> Vec<Vec<Cell>> {
        render(
            &ORIGINAL,
            &crate::theme::default_theme(),
            a,
            frac,
            None,
            note,
        )
    }

    const BARS: usize = H - 1 - 2;

    #[test]
    fn bars_climb_in_eighths_of_a_cell() {
        let half = original(&viz(0.5, 0.0), 0.0, None);
        assert_eq!(
            count(&half[..BARS], '█'),
            W * BARS / 2,
            "half height fills half the rows"
        );
        let one_more = 0.5 + 1.0 / (BARS * 8) as f32;
        let g = original(&viz(one_more, 0.0), 0.0, None);
        assert_eq!(
            count(&g[..BARS], '▁'),
            W,
            "an extra eighth shows as a partial block"
        );
    }

    #[test]
    fn silence_draws_no_bars() {
        let g = original(&viz(0.0, 0.0), 0.0, None);
        assert!(g[..H - 1].iter().flatten().all(|(c, _)| *c == ' '));
    }

    #[test]
    fn a_falling_bar_leaves_its_peak_cap_behind() {
        let g = original(&viz(0.1, 0.9), 0.0, None);
        assert_eq!(
            count(&g[..BARS], '▔'),
            W,
            "every bar should show a cap above it"
        );
    }

    #[test]
    fn bars_shade_from_root_to_tip() {
        let g = original(&viz(1.0, 0.0), 0.0, None);
        let (root, mid, tip) = (g[BARS - 1][0].1, g[BARS / 2][0].1, g[0][0].1);
        assert!(root != mid && mid != tip, "shade should climb with the bar");
    }

    #[test]
    fn the_beat_lights_the_bars() {
        let at = |beat: f32| {
            let mut a = viz(0.5, 0.0);
            a.beat = beat;
            original(&a, 0.0, None)[BARS - 1][0].1
        };
        assert_ne!(at(1.0), at(0.0));
    }

    #[test]
    fn the_pond_reflects_the_bars() {
        let pond = |level: f32| {
            original(&viz(level, 0.0), 0.0, None)[BARS..H - 1]
                .iter()
                .flatten()
                .filter(|(c, _)| *c != ' ')
                .count()
        };
        assert_eq!(pond(0.0), 0, "nothing to reflect in silence");
        assert_eq!(pond(1.0), W * 2, "full bars fill the whole pond");
        assert!(pond(0.3) < pond(1.0));
    }

    #[test]
    fn sparks_fly_in_open_sky_only() {
        let mut a = viz(0.5, 0.0);
        a.sparks = vec![
            crate::visualizer::Spark::at(0.5, 0.9),
            crate::visualizer::Spark::at(0.5, 0.1),
        ];
        assert_eq!(
            count(&original(&a, 0.0, None), '✦'),
            1,
            "the one inside a bar is hidden by it"
        );
    }

    #[test]
    fn bottom_row_is_the_playback_position() {
        let played = |f: f64| count(&original(&viz(0.0, 0.0), f, None)[H - 1..], '━');
        assert_eq!(played(0.0), 0);
        assert!(
            played(0.9) > played(0.1),
            "more of the row fills as the track plays"
        );
    }

    #[test]
    fn odd_inputs_still_fill_the_grid() {
        for t in presets() {
            let look = look_for(&t);
            for g in [
                render(&look, &t, &Analyzer::new(0), 0.0, None, None),
                render(
                    &look,
                    &t,
                    &Analyzer::new(0),
                    0.0,
                    None,
                    Some("nothing playing"),
                ),
                render(
                    &look,
                    &t,
                    &Analyzer::new(7),
                    2.0,
                    Some("x"),
                    Some("a-note-far-longer-than-the-pane-is-wide"),
                ),
            ] {
                assert_eq!(g.len(), H, "{}", t.name);
                assert!(g.iter().all(|r| r.len() == W), "{}", t.name);
            }
        }
    }

    #[test]
    fn custom_keeps_the_original_look() {
        let mut t = crate::theme::default_theme();
        t.name = "Custom".into();
        assert_eq!(look_for(&t), ORIGINAL);
    }

    #[test]
    fn a_note_replaces_the_bars_but_keeps_the_floor_and_the_row() {
        let t = presets()
            .into_iter()
            .find(|t| t.name == "Synthwave '84")
            .unwrap();
        let g = draw(&t, Some("nothing playing"));
        let text: String = g
            .iter()
            .map(|r| r.iter().map(|(c, _)| *c).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("nothing playing"));
        assert!(text.contains('━'), "the horizon stays");
        assert!(!text.contains('█'), "no bars behind a note");
    }

    #[test]
    fn the_song_is_named_in_the_title_or_along_the_bottom() {
        let long = "An Extraordinarily Long Song Title That Keeps Going — Somebody";
        for t in presets() {
            let look = look_for(&t);
            let label = label(&look, Some(long), (W + 2) as u16);
            let g = render(&look, &t, &signal(), 0.5, Some(long), None);
            let bottom: String = g[H - 1].iter().map(|(c, _)| *c).collect();
            if look.progress == Progress::Title {
                assert!(
                    bottom.starts_with("♪ An Extraord") && bottom.ends_with('…'),
                    "{}: {bottom}",
                    t.name
                );
            } else {
                assert!(
                    label.starts_with("♪ An Extraord") && label.ends_with('…'),
                    "{}: {label}",
                    t.name
                );
                assert!(label.chars().count() <= W - 2, "{}: {label}", t.name);
            }
        }
        // Nothing playing: the view's own name.
        assert_eq!(label(&ORIGINAL, None, 30), "visualizer");
    }
}
