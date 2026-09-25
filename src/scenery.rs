//! The Pomodoro's scenery: a different small world for every theme.
//!
//! Each painter draws rows 2 to 8 of the 28×9 grid (rows 0 and 1 are sky,
//! where [`crate::scene::render`] puts the stars and the sun or moon whose
//! position is the clock). They draw in [`Ink`] roles, never colours, so every
//! theme's own palette applies. Motion is slow and sparse on the 250 ms frame
//! clock: this sits beside someone writing.

// Grids are indexed by column; that reads better than zipped iterators.
#![allow(clippy::needless_range_loop)]

use crate::scene::{Cell, H, Ink, W};

/// Where the timer is, for a painter: night is the break, idle is before F2.
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    pub night: bool,
    pub idle: bool,
    pub paused: bool,
    pub frame: u64,
    /// 0.0 to 1.0 through the current phase.
    pub progress: f64,
}

/// One small world per theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenery {
    /// Lost Forest, the house theme, and any custom palette.
    Forest,
    Tower,
    Farm,
    Peaks,
    Castle,
    Lighthouse,
    Cats,
    Skyline,
    Lake,
    Blossom,
    Space,
    Desert,
    Wave,
    Balloons,
    Owl,
    Reef,
    Synthwave,
    Campsite,
    Rainbow,
}

impl Scenery {
    /// Which world a theme gets, by the theme's name. Anything unknown —
    /// a custom palette — gets the forest.
    pub fn for_theme(name: &str) -> Scenery {
        match name {
            "Grimoire" => Scenery::Tower,
            "Gruvbox Dark" => Scenery::Farm,
            "Nord" => Scenery::Peaks,
            "Dracula" => Scenery::Castle,
            "Solarized Dark" => Scenery::Lighthouse,
            "Catppuccin Mocha" => Scenery::Cats,
            "Tokyo Night" => Scenery::Skyline,
            "Everforest" => Scenery::Lake,
            "Rosé Pine" => Scenery::Blossom,
            "One Dark" => Scenery::Space,
            "Monokai" => Scenery::Desert,
            "Kanagawa" => Scenery::Wave,
            "Ayu Mirage" => Scenery::Balloons,
            "Night Owl" => Scenery::Owl,
            "Material Palenight" => Scenery::Reef,
            "Synthwave '84" => Scenery::Synthwave,
            "GitHub Dark" => Scenery::Campsite,
            "Rainbow" => Scenery::Rainbow,
            _ => Scenery::Forest,
        }
    }

    #[cfg(test)]
    pub const ALL: [Scenery; 19] = [
        Scenery::Forest,
        Scenery::Tower,
        Scenery::Farm,
        Scenery::Peaks,
        Scenery::Castle,
        Scenery::Lighthouse,
        Scenery::Cats,
        Scenery::Skyline,
        Scenery::Lake,
        Scenery::Blossom,
        Scenery::Space,
        Scenery::Desert,
        Scenery::Wave,
        Scenery::Balloons,
        Scenery::Owl,
        Scenery::Reef,
        Scenery::Synthwave,
        Scenery::Campsite,
        Scenery::Rainbow,
    ];

    /// The sun and the moon, as this world draws them.
    pub fn sun(self) -> char {
        match self {
            Scenery::Space => '✺',
            Scenery::Desert => '☼',
            _ => '☀',
        }
    }

    pub fn moon(self) -> char {
        match self {
            // A full moon over the castle.
            Scenery::Castle => '●',
            _ => '☾',
        }
    }

    /// The synthwave sun is the scenery itself: it slides along the horizon,
    /// so the usual one isn't drawn over it.
    pub fn draws_own_sun(self) -> bool {
        self == Scenery::Synthwave
    }

    pub fn paint(self, g: &mut [Vec<Cell>], m: &Moment) {
        match self {
            Scenery::Forest => forest(g, m),
            Scenery::Tower => tower(g, m),
            Scenery::Farm => farm(g, m),
            Scenery::Peaks => peaks(g, m),
            Scenery::Castle => castle(g, m),
            Scenery::Lighthouse => lighthouse(g, m),
            Scenery::Cats => cats(g, m),
            Scenery::Skyline => skyline(g, m),
            Scenery::Lake => lake(g, m),
            Scenery::Blossom => blossom(g, m),
            Scenery::Space => space(g, m),
            Scenery::Desert => desert(g, m),
            Scenery::Wave => wave(g, m),
            Scenery::Balloons => balloons(g, m),
            Scenery::Owl => owl(g, m),
            Scenery::Reef => reef(g, m),
            Scenery::Synthwave => synthwave(g, m),
            Scenery::Campsite => campsite(g, m),
            Scenery::Rainbow => rainbow(g, m),
        }
    }
}

// ── drawing helpers ──────────────────────────────────────────────────

/// Draw `s` at (x, y) in one ink; spaces are transparent. `x` may be
/// negative or past the edge — anything off the grid is clipped, so moving
/// things can slide in and out.
fn draw(g: &mut [Vec<Cell>], x: i32, y: usize, s: &str, ink: Ink) {
    art(g, x, y, &[s], |c| (c != ' ').then_some(ink));
}

/// Draw rows of art from (x, y); `ink` picks each character's role, or
/// None to leave the cell as it was.
fn art(g: &mut [Vec<Cell>], x: i32, y: usize, rows: &[&str], ink: impl Fn(char) -> Option<Ink>) {
    for (dy, row) in rows.iter().enumerate() {
        let yy = y + dy;
        if yy >= g.len() {
            break;
        }
        for (dx, ch) in row.chars().enumerate() {
            let xx = x + dx as i32;
            if xx < 0 || xx as usize >= W {
                continue;
            }
            if let Some(i) = ink(ch) {
                g[yy][xx as usize] = (ch, i);
            }
        }
    }
}

/// Like [`draw`], but only into empty sky: for things that pass behind the
/// scenery.
fn behind(g: &mut [Vec<Cell>], x: i32, y: usize, s: &str, ink: Ink) {
    for (dx, ch) in s.chars().enumerate() {
        let xx = x + dx as i32;
        if ch != ' ' && (0..W as i32).contains(&xx) && y < g.len() && g[y][xx as usize].0 == ' ' {
            g[y][xx as usize] = (ch, ink);
        }
    }
}

fn ground(g: &mut [Vec<Cell>], ch: char, ink: Ink) {
    for x in 0..W {
        g[H - 1][x] = (ch, ink);
    }
}

/// True for `on` beats out of every `of`, on a period of `len` frames,
/// offset by `seed` so neighbours don't blink together.
fn beat(frame: u64, len: u64, seed: u64, on: u64, of: u64) -> bool {
    ((frame + seed * 7) / len.max(1) + seed) % of < on
}

/// A position that drifts from `from` to `to` and wraps, one step every
/// `len` frames.
fn drift(frame: u64, len: u64, seed: u64, from: i32, to: i32) -> i32 {
    let span = (to - from).max(1) as u64;
    from + ((frame / len.max(1) + seed) % span) as i32
}

/// A few falling specks (snow, petals, leaves, sparks) in the rows given,
/// each on its own slow path.
fn falling(
    g: &mut [Vec<Cell>],
    m: &Moment,
    count: u64,
    rows: (usize, usize),
    glyphs: &[char],
    ink: Ink,
) {
    let (top, bottom) = rows;
    let depth = (bottom - top + 1) as u64;
    for i in 0..count {
        let speed = 5 + i % 3;
        let t = m.frame / speed + i * 13;
        let y = top + (t % depth) as usize;
        let x = ((i * 11 + 3 + (t / depth) * 5 + (t % depth) / 2) % W as u64) as usize;
        if g[y][x].0 == ' ' {
            g[y][x] = (glyphs[(i as usize + y) % glyphs.len()], ink);
        }
    }
}

// ── Lost Forest: the glade, as it always was ─────────────────────────

// Row budget: 0-1 sky, 2-4 foliage, 5 trunks, 6-7 rabbits, 8 ground.
pub const CANOPY_BASE: usize = 4;
pub const TRUNK_ROW: usize = 5;
pub const RABBIT_TOP: usize = 6;

fn forest(g: &mut [Vec<Cell>], m: &Moment) {
    let frame = m.frame;
    // Treeline: centred triangles, bottom-aligned on the canopy row.
    let trees = [(4usize, 3usize), (11, 3), (19, 3), (25, 2)];
    for &(cx, size) in &trees {
        for layer in 0..size {
            let row = CANOPY_BASE + 1 - size + layer;
            for dx in 0..=(layer * 2) {
                let px = (cx + dx).saturating_sub(layer);
                if px < W && row < H {
                    g[row][px] = ('▲', Ink::Tree);
                }
            }
        }
        g[TRUNK_ROW][cx] = ('┃', Ink::Trunk);
    }

    // Rabbits: ears flick and heads turn on slow, mutually-prime cycles, so
    // the two of them never move in lockstep.
    let ears = |seed: u64, len: u64, of: u64| {
        if ((frame + seed) / len).is_multiple_of(of) {
            "(\\ /)"
        } else {
            "(\\_/)"
        }
    };
    let face = |seed: u64, len: u64, of: u64| {
        if ((frame + seed) / len).is_multiple_of(of) {
            "(-ᴥ-)"
        } else {
            "(•ᴥ•)"
        }
    };
    draw(g, 3, RABBIT_TOP, ears(5, 9, 7), Ink::Creature);
    draw(g, 3, RABBIT_TOP + 1, face(13, 23, 5), Ink::Creature);
    draw(g, 20, RABBIT_TOP, ears(31, 11, 9), Ink::Creature);
    draw(g, 20, RABBIT_TOP + 1, face(44, 17, 6), Ink::Creature);

    // Flowers by day; fireflies drifting between the two by night.
    if m.night {
        let seats = [
            (RABBIT_TOP, 12usize),
            (RABBIT_TOP + 1, 15),
            (CANOPY_BASE + 1, 10),
        ];
        for (i, &(fy, fx)) in seats.iter().enumerate() {
            if !(frame / (5 + i as u64 * 3)).is_multiple_of(4) {
                g[fy][fx] = ('˙', Ink::Star);
            }
        }
    } else {
        g[RABBIT_TOP + 1][12] = ('❀', Ink::Flower);
        g[RABBIT_TOP + 1][15] = ('✿', Ink::Flower);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Grimoire: a wizard's tower on its hill ──────────────────────────

fn tower(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        0,
        2,
        &[
            "                            ",
            "                            ",
            "                  █W█       ",
            "                  ███       ",
            "          ▁▂▃▃▂▁▁▁█W█▁▁     ",
            "  ▁▁▂▃▄▅▆▇▇▇▇▇▇▇▇▇▇▇▇▇▇▆▄▂▁ ",
        ],
        |c| match c {
            '█' => Some(Ink::Stone),
            '▁'..='▇' => Some(Ink::Tree),
            _ => None,
        },
    );
    // The roof, in the theme's own colour.
    draw(g, 19, 2, "▲", Ink::Accent);
    draw(g, 18, 3, "▟█▙", Ink::Accent);
    // The windows: dark by day, lamplit at night.
    for (x, y) in [(19, 4), (19, 6)] {
        g[y][x] = if m.night {
            ('▪', Ink::Light)
        } else {
            ('▪', Ink::Sky)
        };
    }
    // Wisps drift about the tower at night; two birds cross by day.
    if m.night {
        for i in 0..3u64 {
            if beat(m.frame, 5, i, 3, 4) {
                let x = drift(m.frame, 9, i * 5, 11, 27) as usize;
                let y = 2 + (i as usize * 2 + (m.frame / 13) as usize) % 3;
                if g[y][x].0 == ' ' {
                    g[y][x] = ('∘', Ink::Light);
                }
            }
        }
    } else {
        let x = drift(m.frame, 10, 6, -2, 30);
        draw(g, x, 3, "ˇ ˇ", Ink::Creature);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Gruvbox: an autumn farm with a windmill ──────────────────────────

fn farm(g: &mut [Vec<Cell>], m: &Moment) {
    // The windmill's sails turn, slowly.
    let sails: [&str; 3] = if (m.frame / 6).is_multiple_of(2) {
        ["╲ ╱", " ◆ ", "╱ ╲"]
    } else {
        [" │ ", "─◆─", " │ "]
    };
    art(g, 4, 2, &sails, |c| match c {
        '◆' => Some(Ink::Trunk),
        ' ' => None,
        _ => Some(Ink::Snow),
    });
    art(g, 4, 5, &[" ▐▌", "▗██▖"], |c| {
        (c != ' ').then_some(Ink::Trunk)
    });
    // The barn.
    art(
        g,
        14,
        3,
        &["  ▗▄▄▄▄▖ ", " ▟██████▙", " █W█▀▀█W█", " █▀█▐▌█▀█"],
        |c| match c {
            'W' => None,
            '▐' | '▌' => Some(Ink::Trunk),
            _ => Some(Ink::Warn),
        },
    );
    for x in [16, 22] {
        g[5][x] = if m.night {
            ('▪', Ink::Light)
        } else {
            ('▪', Ink::Trunk)
        };
    }
    // Pumpkins and a fence along the field.
    art(
        g,
        0,
        7,
        &["┼─┼─┼─┼─┼─┼─┼   ● ●  ┼─┼─┼─┼"],
        |c| match c {
            '●' => Some(Ink::Sun),
            ' ' => None,
            _ => Some(Ink::Trunk),
        },
    );
    if !m.night {
        // Leaves let go of somewhere off to the left and drift across.
        falling(g, m, 3, (2, 6), &['❧', '⁎'], Ink::Warn);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Nord: snow on the peaks, the aurora at night ─────────────────────

fn peaks(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        0,
        2,
        &[
            "         ◢◣                 ",
            "   ◢◣   ◢SS◣      ◢◣        ",
            "  ◢SS◣ ◢████◣    ◢SS◣   ▲   ",
            " ◢████◢██████◣  ◢████◣ ▲▲▲  ",
            "◢█████████████◣◢██████◣▲▲▲▲▲",
        ],
        |c| match c {
            '◢' | '◣' => Some(Ink::Snow),
            'S' => None,
            '█' => Some(Ink::Stone),
            '▲' => Some(Ink::Tree),
            _ => None,
        },
    );
    // Snow on the shoulders of the peaks.
    for (x, y) in [(9, 3), (10, 3), (3, 4), (4, 4), (18, 4), (19, 4)] {
        g[y][x] = ('█', Ink::Snow);
    }
    // A cabin, its window lit at night, smoke curling by day.
    art(g, 2, 6, &["▟▀▙", "█W█"], |c| match c {
        'W' => None,
        _ => Some(Ink::Trunk),
    });
    g[7][3] = if m.night {
        ('▪', Ink::Light)
    } else {
        ('▪', Ink::Stone)
    };
    art(g, 23, 7, &["┃"], |_| Some(Ink::Trunk));
    if m.night {
        // The aurora: a slow ribbon across the top of the sky.
        for x in 0..W {
            let phase = x as f64 * 0.45 + m.frame as f64 / 14.0;
            let y = if phase.sin() > 0.2 { 0 } else { 1 };
            if g[y][x].0 == ' ' {
                let v = ((phase * 0.6).sin() * 0.5 + 0.5) * 255.0;
                g[y][x] = ('~', Ink::Glow(v as u8));
            }
        }
    } else {
        if beat(m.frame, 4, 0, 3, 4) {
            g[5][3 + (m.frame / 8 % 2) as usize] = ('°', Ink::Cloud);
        }
        falling(g, m, 4, (0, 5), &['·', '*'], Ink::Snow);
    }
    ground(g, '▔', Ink::Snow);
}

// ── Dracula: the castle on its crag, and bats at night ───────────────

fn castle(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        0,
        2,
        &[
            "     ▙▟▙▟        ▙▟▙▟       ",
            "     █W██▙▟▙▟▙▟▙▟█W██       ",
            "     ████████W███████       ",
            "    ▟████▛▀▀▀▜███████▙      ",
        ],
        |c| match c {
            'W' | ' ' => None,
            _ => Some(Ink::Stone),
        },
    );
    // The crag it stands on.
    art(
        g,
        0,
        6,
        &[
            "  ▗▟█████▌   ▐█████████▙▖   ",
            "▗▟██████████████████████▙▄▖ ",
        ],
        |c| (c != ' ').then_some(Ink::Ground),
    );
    for (x, y) in [(6, 3), (18, 3), (13, 4)] {
        g[y][x] = if m.night {
            ('▪', Ink::Light)
        } else {
            ('▪', Ink::Sky)
        };
    }
    if m.night {
        // Bats: three, wheeling on their own slow loops.
        for i in 0..3u64 {
            let x = drift(m.frame, 3 + i, i * 9 + 4, -1, 29);
            let y = 1 + ((m.frame / (7 + i) + i) % 3) as usize;
            let wing = if beat(m.frame, 2, i, 1, 2) {
                "⋎"
            } else {
                "⋏"
            };
            draw(g, x, y, wing, Ink::Creature);
        }
    } else {
        // Mist drifts past below the battlements, behind the castle.
        let x = drift(m.frame, 14, 8, -6, 28);
        behind(g, x, 5, "~ ~~", Ink::Cloud);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Solarized: a lighthouse over the sea ─────────────────────────────

fn lighthouse(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        17,
        2,
        &["   ▲", "  ▐L▌", "  ▐█▌", "  ▐▀▌", " ▟███▙", "▟█████▙"],
        |c| match c {
            '▲' => Some(Ink::Accent),
            'L' => None,
            '▀' => Some(Ink::Snow),
            ' ' => None,
            _ => Some(Ink::Warn),
        },
    );
    g[3][20] = ('◉', Ink::Light);
    // Waves roll in along the bottom two rows.
    let off = (m.frame / 5) as usize;
    for x in 0..W {
        if g[7][x].0 == ' ' {
            g[7][x] = if (x + off).is_multiple_of(6) {
                ('◠', Ink::Foam)
            } else {
                ('≈', Ink::Water)
            };
        }
        g[8][x] = if (x + off / 2) % 5 == 1 {
            ('◠', Ink::Foam)
        } else {
            ('≈', Ink::Water)
        };
    }
    if m.night {
        // The beam sweeps: out to the left, then to the right.
        let left = (m.frame / 8).is_multiple_of(2);
        if left {
            draw(g, 7, 3, "════════════", Ink::Light);
        } else {
            draw(g, 22, 3, "══════", Ink::Light);
        }
    } else {
        for i in 0..2u64 {
            let x = drift(m.frame, 8 + i * 3, i * 11, -1, 17);
            let y = 3 + i as usize;
            let bird = if beat(m.frame, 3, i, 1, 2) {
                "v"
            } else {
                "⌄"
            };
            draw(g, x, y, bird, Ink::Creature);
        }
    }
}

// ── Catppuccin: two cats on a fence, and a mug ───────────────────────

fn cats(g: &mut [Vec<Cell>], m: &Moment) {
    // By day one naps; at night both are up, eyes shining.
    let blink = beat(m.frame, 3, 5, 1, 17);
    let awake = |base: bool| base && !blink;
    let face = |open: bool| if open { "( o.o )" } else { "( -.- )" };
    let left_open = awake(true);
    let right_open = awake(m.night);
    for (x, open, tail) in [
        (2, left_open, beat(m.frame, 6, 0, 1, 2)),
        (15, right_open, beat(m.frame, 7, 3, 1, 2)),
    ] {
        art(
            g,
            x,
            3,
            &[
                " /\\_/\\",
                face(open),
                if tail { " > ^ <╮" } else { " > ^ <╯" },
            ],
            |c| match c {
                ' ' => None,
                'o' => Some(if m.night { Ink::Light } else { Ink::Creature }),
                '^' => Some(Ink::Flower),
                '╮' | '╯' => Some(Ink::Creature),
                _ => Some(Ink::Creature),
            },
        );
    }
    // The fence they sit on.
    draw(g, 0, 6, "▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀", Ink::Trunk);
    draw(g, 0, 7, " ▌  ▌  ▌  ▌  ▌  ▌  ▌  ▌  ▌  ", Ink::Trunk);
    // A mug on the post, steam rising.
    draw(g, 24, 5, "c▆", Ink::Accent);
    if !m.night || beat(m.frame, 4, 1, 1, 2) {
        let lift = (m.frame / 5 % 2) as usize;
        g[3 + lift][25] = ('∫', Ink::Cloud);
        g[4 - lift][24] = ('∫', Ink::Cloud);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Tokyo Night: a skyline, windows coming and going ─────────────────

fn skyline(g: &mut [Vec<Cell>], m: &Moment) {
    let rows = [
        "      ▗▖         ▗▄▖        ",
        "  ▗▄▖ █W  ▗▄▄▖   █W█  ▄▄    ",
        "  █W█ █▌  █WW█ ▗ ███ ▐WW▌ ▄▄",
        "▄ ███ ██▄ ████ █ █W█ ▐██▌ █W",
        "█▄█W█▄███▄█W██▄█▄███▄▐WW▌▄██",
    ];
    art(g, 0, 2, &rows, |c| match c {
        ' ' => None,
        'W' => Some(Ink::Stone),
        _ => Some(Ink::Stone),
    });
    // Windows: a few lit by day, most at night, one changing every so often.
    let mut n = 0u64;
    for (dy, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c != 'W' {
                continue;
            }
            n += 1;
            let lit = if m.night {
                !(n * 7 + m.frame / 20).is_multiple_of(5)
            } else {
                (n * 3 + m.frame / 24).is_multiple_of(4)
            };
            g[2 + dy][x] = if lit {
                ('▪', Ink::Light)
            } else {
                ('▪', Ink::Sky)
            };
        }
    }
    // The line along the bottom, and a train that comes by now and then.
    let x = (m.frame / 2 % 90) as i32 - 30;
    draw(g, x, 7, "▐▀▀▀▀▀▀▌▐▀▀▀▀▀▀▌", Ink::Accent);
    draw(g, x + 2, 7, "▪ ▪ ▪   ▪ ▪ ▪", Ink::Light);
    ground(g, '═', Ink::Trunk);
}

// ── Everforest: a lake in the woods ─────────────────────────────────

fn lake(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        0,
        2,
        &[
            " ▲                       ▲  ",
            "▲▲▲  ▲                ▲ ▲▲▲ ",
            "▲▲▲▲▲▲▲              ▲▲▲▲▲▲▲",
            " ┃  ┃                  ┃  ┃ ",
        ],
        |c| match c {
            '▲' => Some(Ink::Tree),
            '┃' => Some(Ink::Trunk),
            _ => None,
        },
    );
    // The water, rippling.
    let off = (m.frame / 6) as usize;
    for y in 5..8 {
        let (from, to) = if y == 5 { (7, 21) } else { (4, 24) };
        for x in from..to {
            g[y][x] = if (x + y * 3 + off).is_multiple_of(7) {
                ('∼', Ink::Foam)
            } else {
                ('≈', Ink::Water)
            };
        }
    }
    // The sun (or moon) shines back off the lake beneath where it hangs.
    let t = if m.idle { 0.5 } else { m.progress };
    let x = (2.0 + t * (W - 5) as f64).round() as usize;
    let x = x.clamp(5, 22);
    g[6][x] = if m.night {
        ('◦', Ink::Moon)
    } else {
        ('◦', Ink::Sun)
    };
    // A rowboat drifting across.
    let bx = drift(m.frame, 16, 4, 8, 17);
    draw(g, bx, 5, "╲▁▁╱", Ink::Trunk);
    if m.night {
        for i in 0..3u64 {
            if beat(m.frame, 4, i, 2, 3) {
                let x = drift(m.frame, 11, i * 7, 6, 22) as usize;
                let y = 3 + (i as usize) % 2;
                if g[y][x].0 == ' ' {
                    g[y][x] = ('˙', Ink::Light);
                }
            }
        }
    }
    ground(g, '▔', Ink::Ground);
}

// ── Rosé Pine: a blossom tree, petals on the wind ────────────────────

fn blossom(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        0,
        2,
        &[
            "   ❀✿❀✿❀",
            " ✿❀✿❀✿❀✿❀",
            "  ❀✿❀✿❀✿",
            "     ┃╲",
            "     ┃",
            "    ▟█▙",
        ],
        |c| match c {
            '❀' | '✿' => Some(Ink::Flower),
            ' ' => None,
            _ => Some(Ink::Trunk),
        },
    );
    // A pine across the way.
    art(
        g,
        23,
        2,
        &["  ▲", " ▲▲▲", "▲▲▲▲▲", " ▲▲▲", "  ┃"],
        |c| match c {
            '▲' => Some(Ink::Tree),
            '┃' => Some(Ink::Trunk),
            _ => None,
        },
    );
    // A stone lantern, lit at night.
    art(
        g,
        15,
        3,
        &[" ▄▄▄", "▔▐L▌▔", "  █", " ▄█▄"],
        |c| match c {
            'L' | ' ' => None,
            _ => Some(Ink::Stone),
        },
    );
    g[4][17] = if m.night {
        ('◉', Ink::Light)
    } else {
        ('◌', Ink::Stone)
    };
    falling(
        g,
        m,
        if m.night { 3 } else { 6 },
        (2, 7),
        &['·', '˙', '❀'],
        Ink::Flower,
    );
    ground(g, '▔', Ink::Ground);
}

// ── One Dark: a ringed planet out in the dark ────────────────────────

fn space(g: &mut [Vec<Cell>], m: &Moment) {
    // A ringed planet, the ring passing in front.
    art(
        g,
        1,
        3,
        &["  ▗▄▄▄▖", "━━█████━━", "  ▝▀▀▀▘"],
        |c| match c {
            '━' => Some(Ink::Accent),
            ' ' => None,
            _ => Some(Ink::Moon),
        },
    );
    draw(g, 11, 2, "•", Ink::Stone);
    // Stars, far and near, twinkling out of step.
    let seats = [
        (3usize, 14usize),
        (2, 20),
        (4, 25),
        (6, 4),
        (5, 17),
        (7, 9),
        (6, 22),
        (3, 18),
        (7, 26),
        (4, 12),
    ];
    for (i, &(y, x)) in seats.iter().enumerate() {
        if beat(
            m.frame,
            5 + i as u64 % 3,
            i as u64,
            if m.night { 3 } else { 2 },
            4,
        ) {
            g[y][x] = (if i % 3 == 0 { '✦' } else { '·' }, Ink::Star);
        }
    }
    // A satellite crossing.
    let x = drift(m.frame, 10, 18, -4, 30);
    draw(g, x, 6, "▭╪▭", Ink::Stone);
    if m.night {
        // And a comet, now and then.
        let cx = (m.frame / 3 % 70) as i32 - 10;
        draw(g, cx, 2, "∙∙∙✶", Ink::Light);
    }
    for x in 0..W {
        g[H - 1][x] = (if x % 3 == 1 { '∙' } else { '·' }, Ink::Stone);
    }
}

// ── Monokai: the desert ──────────────────────────────────────────────

fn desert(g: &mut [Vec<Cell>], m: &Moment) {
    // Mesas on the horizon.
    art(
        g,
        0,
        4,
        &["   ▗▄▄▄▄▄▖            ▗▄▄▖ ", "  ▟███████▙          ▟████▙"],
        |c| (c != ' ').then_some(Ink::Trunk),
    );
    // Saguaros.
    for (x, y) in [(12usize, 4usize), (22, 5)] {
        art(
            g,
            x as i32,
            y,
            &["╻ ┃ ╻", "┗━╋━┛", "  ┃"],
            |c| (c != ' ').then_some(Ink::Tree),
        );
    }
    // A tumbleweed rolls through every so often.
    let tx = (m.frame / 2 % 80) as i32 - 20;
    let tumble = if (m.frame / 2).is_multiple_of(2) {
        "✱"
    } else {
        "✲"
    };
    draw(g, tx, 7, tumble, Ink::Trunk);
    if !m.night {
        // Heat shimmer over the sand.
        let off = (m.frame / 4) as usize;
        for x in 0..W {
            if (x + off).is_multiple_of(9) && g[7][x].0 == ' ' {
                g[7][x] = ('∼', Ink::Cloud);
            }
        }
    }
    ground(g, '▁', Ink::Sand);
}

// ── Kanagawa: the great wave, and the mountain ───────────────────────

fn wave(g: &mut [Vec<Cell>], m: &Moment) {
    g[4][19] = ('█', Ink::Snow);
    g[4][20] = ('█', Ink::Snow);
    // The claws of foam at the crest reach, then curl back.
    let claws = if (m.frame / 7).is_multiple_of(2) {
        "◠◠"
    } else {
        "◡◠"
    };
    art(
        g,
        0,
        2,
        &[
            "   ▄▄▄▄",
            " ▄█▀  ▀█▄",
            "██▀ CC  ▀█▄",
            "█▀        ▀██▄",
            "            ▀██▄▄▄▄▄▄▄▄▄▄▄",
        ],
        |c| match c {
            'C' | ' ' => None,
            _ => Some(Ink::Water),
        },
    );
    draw(g, 4, 4, claws, Ink::Foam);
    draw(g, 2, 3, "◠", Ink::Foam);
    // Fuji, far off, its snow cap catching the light.
    art(
        g,
        16,
        3,
        &["   ◢◣", "  ◢██◣", " ◢████◣"],
        |c| match c {
            '◢' | '◣' => Some(Ink::Snow),
            '█' => Some(Ink::Stone),
            _ => None,
        },
    );
    let off = (m.frame / 5) as usize;
    for x in 0..W {
        g[7][x] = if (x + off).is_multiple_of(5) {
            ('◠', Ink::Foam)
        } else {
            ('≈', Ink::Water)
        };
        g[8][x] = ('≈', Ink::Water);
    }
    // A boat riding the trough.
    draw(g, 11, 6 + (m.frame / 9 % 2) as usize, "▁▂▁", Ink::Trunk);
}

// ── Ayu Mirage: balloons over the fields ─────────────────────────────

fn balloons(g: &mut [Vec<Cell>], m: &Moment) {
    // Rolling fields.
    art(
        g,
        0,
        6,
        &[
            "      ▁▂▂▁▁           ▁▂▂▁ ",
            "▁▂▃▄▅▆▇▇▇▇▇▆▅▄▃▂▁▂▃▄▅▆▇▇▇▇▆▅",
        ],
        |c| (c != ' ').then_some(Ink::Tree),
    );
    // Two balloons, striped, bobbing and drifting; at night the burners
    // glow.
    for (i, (a, b), seed) in [
        (0u64, (Ink::Warn, Ink::Sun), 8u64),
        (1, (Ink::Accent, Ink::Flower), 22),
    ] {
        let x = drift(m.frame, 20 + i * 6, seed, -4, 30);
        let y = 2 + ((m.frame / (9 + i * 2) + i) % 2) as usize;
        art(
            g,
            x,
            y,
            &[" ▄▄", "▐██▌", " ▀▀", " ▗▖"],
            |c| match c {
                ' ' => None,
                '▗' | '▖' => Some(Ink::Trunk),
                _ => Some(a),
            },
        );
        // The right half of the envelope takes the second colour.
        art(g, x + 2, y, &["▄", "█▌", "▀"], |c| {
            (c != ' ').then_some(b)
        });
        if m.night && beat(m.frame, 2, i, 1, 3) {
            draw(g, x + 1, y + 3, "▴", Ink::Light);
        }
    }
    ground(g, '▔', Ink::Ground);
}

// ── Night Owl: the owl on its branch ─────────────────────────────────

fn owl(g: &mut [Vec<Cell>], m: &Moment) {
    // The old tree at the right, and the branch the owl sits on.
    art(
        g,
        0,
        2,
        &[
            "                   ▗▆▆▆▆▆▖  ",
            "                  ▐███████▌ ",
            "                    ▀▜█▛▀   ",
        ],
        |c| match c {
            ' ' => None,
            _ => Some(Ink::Tree),
        },
    );
    g[4][22] = ('█', Ink::Trunk);
    for y in 5..8 {
        g[y][22] = ('█', Ink::Trunk);
    }
    draw(g, 4, 6, "━━━━━━━━━━━━━━━━━━┫", Ink::Trunk);
    // Asleep by day; at night awake, blinking now and then.
    let awake = m.night && !beat(m.frame, 3, 2, 1, 19);
    art(
        g,
        10,
        2,
        &[
            " ,___,",
            if awake { " (O,O)" } else { " (-,-)" },
            " /)_)",
            "  \" \"",
        ],
        |c| match c {
            'O' => Some(Ink::Light),
            '"' => Some(Ink::Sun),
            ' ' => None,
            _ => Some(Ink::Creature),
        },
    );
    draw(g, 5, 5, "❦", Ink::Tree);
    if !m.night {
        falling(g, m, 2, (3, 7), &['❧'], Ink::Tree);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Palenight: under the sea ─────────────────────────────────────────

fn reef(g: &mut [Vec<Cell>], m: &Moment) {
    // The surface, glittering.
    let off = (m.frame / 5) as usize;
    for x in 0..W {
        g[2][x] = if (x + off).is_multiple_of(6) {
            ('◠', Ink::Foam)
        } else {
            ('≈', Ink::Water)
        };
    }
    // Kelp sways.
    let sway = (m.frame / 6).is_multiple_of(2);
    for &x in &[2usize, 6, 21, 25] {
        for y in 5..8 {
            let left = (y + x) % 2 == 0;
            g[y][x] = (if left == sway { 'ʃ' } else { 'ʅ' }, Ink::Tree);
        }
    }
    // Fish going about their business in both directions.
    let fx = drift(m.frame, 6, 10, -4, 30);
    draw(g, fx, 4, "><>", Ink::Sun);
    let fx2 = 27 - drift(m.frame, 8, 12, -4, 30);
    draw(g, fx2, 6, "<><", Ink::Accent);
    // Bubbles rising.
    for i in 0..3u64 {
        let y = 7 - ((m.frame / 4 + i * 2) % 5) as usize;
        let x = [9usize, 14, 18][i as usize];
        if g[y][x].0 == ' ' {
            g[y][x] = (if y < 5 { '°' } else { '∘' }, Ink::Foam);
        }
    }
    if m.night {
        // A jellyfish, glowing.
        let y = 3 + ((m.frame / 10) % 2) as usize;
        let v = ((m.frame % 40) as f64 / 40.0 * std::f64::consts::TAU).sin() * 0.5 + 0.5;
        art(g, 12, y, &["◠", "┆"], |_| {
            Some(Ink::Glow((v * 255.0) as u8))
        });
    }
    art(
        g,
        0,
        8,
        &["▁▂▁▁▂▃▂▁▁▁▂▁▁▂▁▁▁▂▃▂▁▁▂▁▁▂▁▁"],
        |_| Some(Ink::Sand),
    );
}

// ── Synthwave '84: the sun sliding along a neon horizon ──────────────

fn synthwave(g: &mut [Vec<Cell>], m: &Moment) {
    // The horizon and a perspective grid rushing toward you.
    draw(g, 0, 5, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━", Ink::Accent);
    let v = 13.5f64;
    for y in 6..9 {
        let depth = (y - 5) as f64;
        for k in -4i32..=4 {
            let xi = (v + k as f64 * depth * 3.3).round() as i32;
            if (0..W as i32).contains(&xi) {
                let ch = match k.signum() {
                    -1 => '╱',
                    1 => '╲',
                    _ => '│',
                };
                g[y][xi as usize] = (ch, Ink::Accent);
            }
        }
    }
    let rush = 6 + (m.frame / 4 % 3) as usize;
    for x in 0..W {
        if g[rush][x].0 == ' ' {
            g[rush][x] = ('─', Ink::Accent);
        }
    }
    // Palms at either edge.
    for x in [0, 23] {
        art(g, x, 2, &["▄▀█▀▄", "  ▐", "  ▐"], |c| match c {
            ' ' => None,
            '▐' => Some(Ink::Trunk),
            _ => Some(Ink::Tree),
        });
    }
    // The sun is the clock: it slides along the horizon through the session,
    // striped yellow to pink, and is down (the moon's out instead) for the
    // break.
    if !m.night {
        let t = if m.idle { 0.5 } else { m.progress };
        let x = (5.0 + t * 11.0).round() as i32;
        let (top, mid, low) = if m.paused {
            (Ink::Dim, Ink::Dim, Ink::Dim)
        } else {
            (Ink::Sun, Ink::Warn, Ink::Flower)
        };
        draw(g, x, 2, " ▄███▄", top);
        draw(g, x, 3, "▀▀▀▀▀▀▀", mid);
        draw(g, x, 4, "▔▔▔▔▔▔▔", low);
    }
}

// ── GitHub Dark: a campsite in the pines ─────────────────────────────

fn campsite(g: &mut [Vec<Cell>], m: &Moment) {
    art(
        g,
        0,
        2,
        &[
            "   ▲                    ▲   ",
            "  ▲▲▲                  ▲▲▲  ",
            " ▲▲▲▲▲                ▲▲▲▲▲ ",
            "   ┃                    ┃   ",
        ],
        |c| match c {
            '▲' => Some(Ink::Tree),
            '┃' => Some(Ink::Trunk),
            _ => None,
        },
    );
    // The tent, a lamp in its doorway at night.
    art(
        g,
        9,
        4,
        &["   ▲", "  ▟█▙", " ▟█▀█▙", "▟██ ██▙"],
        |c| (c != ' ').then_some(Ink::Accent),
    );
    if m.night {
        g[7][12] = ('▪', Ink::Light);
    }
    // The fire: embers by day, flames and sparks at night.
    draw(g, 18, 7, "▞▚", Ink::Trunk);
    if m.night {
        let tall = (m.frame / 2).is_multiple_of(2);
        draw(g, 19, 6, if tall { "▲" } else { "▴" }, Ink::Light);
        g[5][19] = if tall {
            ('▴', Ink::Warn)
        } else {
            (' ', Ink::Sky)
        };
        for i in 0..2u64 {
            let y = 4 - ((m.frame / 3 + i * 2) % 3) as usize;
            let x = 19 + (i as usize + (m.frame / 5) as usize) % 2;
            if g[y][x].0 == ' ' {
                g[y][x] = ('·', Ink::Light);
            }
        }
    } else {
        draw(g, 19, 6, "·", Ink::Warn);
        let y = 5 - ((m.frame / 6) % 3) as usize;
        draw(g, 19 + (m.frame / 9 % 2) as i32, y, "°", Ink::Cloud);
    }
    ground(g, '▔', Ink::Ground);
}

// ── Rainbow: a fine arc of five ribbons, standing in two clouds ──────

/// The rainbow is drawn in braille, two dots across and four down per cell,
/// so its bands curve smoothly instead of stepping. Dots are about square,
/// so these are plain circles in dot space, centred on the ground's top edge.
const DOTS_X: usize = W * 2;
const ARC_CX: f64 = DOTS_X as f64 / 2.0;
const ARC_CY: f64 = ((H - 1) * 4) as f64;
/// Outer edge radius, the distance from one band to the next, and each
/// band's thickness, all in dots: two-dot ribbons a cell apart.
const ARC_R: f64 = 24.0;
const ARC_PITCH: f64 = 4.0;
const ARC_THICK: f64 = 2.0;
const BANDS: u8 = 5;

/// Braille dot `(col, row)` in a cell, as the bit Unicode gives it.
fn dot_bit(col: usize, row: usize) -> u32 {
    match (col, row) {
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        (0, r) => 1 << r,
        (_, r) => 1 << (r + 3),
    }
}

/// Which band a dot at (`x`, `y`) in dot space belongs to, if any.
fn band_at(x: usize, y: usize) -> Option<u8> {
    let r = (x as f64 + 0.5 - ARC_CX).hypot(y as f64 + 0.5 - ARC_CY);
    let depth = ARC_R - r;
    if depth < 0.0 {
        return None;
    }
    let k = (depth / ARC_PITCH) as u8;
    (k < BANDS && depth - f64::from(k) * ARC_PITCH < ARC_THICK).then_some(k)
}

/// A soft cloud: a few overlapping puffs, in dot space.
fn in_cloud(x: f64, y: f64, at: f64) -> bool {
    [(3.5, 29.0, 3.2), (7.5, 27.0, 4.2), (11.5, 29.5, 3.0)]
        .iter()
        .any(|&(px, py, r)| (x - px - at).hypot(y - py) <= r)
}

fn rainbow(g: &mut [Vec<Cell>], m: &Moment) {
    // By day a light runs slowly along the outer band, left to right, then
    // rests; its angle from the left foot, 0 to 180 degrees.
    let run = (m.frame % 160) as f64 * 1.5;
    for cy in 2..H - 1 {
        for cx in 0..W {
            let (mut bits, mut votes) = (0u32, [0u8; BANDS as usize]);
            for col in 0..2 {
                for row in 0..4 {
                    if let Some(k) = band_at(cx * 2 + col, cy * 4 + row) {
                        bits |= dot_bit(col, row);
                        votes[k as usize] += 1;
                    }
                }
            }
            if bits == 0 {
                continue;
            }
            // A cell shows one colour: the band with most dots in it.
            let n = (0..BANDS)
                .max_by_key(|&k| (votes[k as usize], BANDS - k))
                .unwrap_or(0);
            let angle = (ARC_CY - cy as f64 * 4.0 - 2.0)
                .atan2(ARC_CX - cx as f64 * 2.0 - 1.0)
                .to_degrees();
            let glint = n == 0 && !m.night && !m.paused && (angle - run).abs() < 7.0;
            let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
            g[cy][cx] = (
                ch,
                Ink::Band {
                    n,
                    faint: m.night,
                    glint,
                },
            );
        }
    }
    // The feet stand in two clouds, which drift a dot or two.
    for (left, seed) in [(0.0, 0u64), (DOTS_X as f64 - 15.0, 7)] {
        let at = left + drift(m.frame, 20, seed, 0, 3) as f64;
        for cy in H - 3..H - 1 {
            for cx in 0..W {
                let mut bits = 0u32;
                for col in 0..2 {
                    for row in 0..4 {
                        let (x, y) = ((cx * 2 + col) as f64 + 0.5, (cy * 4 + row) as f64 + 0.5);
                        if in_cloud(x, y, at) {
                            bits |= dot_bit(col, row);
                        }
                    }
                }
                if bits != 0 {
                    g[cy][cx] = (char::from_u32(0x2800 + bits).unwrap_or(' '), Ink::Cloud);
                }
            }
        }
    }
    ground(g, '▔', Ink::Ground);
}

#[cfg(test)]
mod rainbow_tests {
    use super::*;

    fn moment(night: bool) -> Moment {
        Moment {
            night,
            idle: false,
            paused: false,
            frame: 0,
            progress: 0.5,
        }
    }

    fn painted(night: bool) -> Vec<Vec<Cell>> {
        let mut g = vec![vec![(' ', Ink::Sky); W]; H];
        rainbow(&mut g, &moment(night));
        g
    }

    #[test]
    fn the_rainbow_is_fine_ribbons_not_blocks() {
        let g = painted(false);
        let bands: Vec<(char, u8)> = g
            .iter()
            .flatten()
            .filter_map(|&(c, i)| match i {
                Ink::Band { n, .. } => Some((c, n)),
                _ => None,
            })
            .collect();
        assert!(bands.len() > 40, "an arc, not a smudge");
        assert!(
            bands
                .iter()
                .all(|&(c, _)| ('\u{2801}'..='\u{28ff}').contains(&c)),
            "drawn in braille dots, never solid blocks"
        );
        for k in 0..BANDS {
            assert!(bands.iter().any(|&(_, n)| n == k), "band {k} is missing");
        }
    }

    #[test]
    fn red_is_outside_and_the_bands_nest_down_the_middle() {
        let g = painted(false);
        let mid = W / 2;
        let down: Vec<u8> = (0..H)
            .filter_map(|y| match g[y][mid].1 {
                Ink::Band { n, .. } => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(down.first(), Some(&0), "red on top");
        assert!(down.windows(2).all(|w| w[0] < w[1]), "outside in: {down:?}");
    }

    #[test]
    fn night_dims_it_and_puts_the_light_out() {
        let g = painted(true);
        assert!(g.iter().flatten().all(|&(_, i)| match i {
            Ink::Band { faint, glint, .. } => faint && !glint,
            _ => true,
        }));
    }
}
