//! The Pomodoro: a small world that doubles as the timer.
//!
//! The sun crosses the sky over the course of a writing session, so you read
//! the time remaining off the light rather than a countdown, and the ground
//! beneath fills in behind it. Break time is night: the moon comes up and the
//! world's night things come out. Which world depends on the theme — see
//! [`crate::scenery`].

// The scene is a character grid; indexing it by column reads better than
// zipped iterators.
#![allow(clippy::needless_range_loop)]

use std::time::{Duration, Instant};

pub const W: usize = 28;
pub const H: usize = 9;

pub use crate::scenery::{Moment, Scenery};

/// Rows 0 and 1 are sky; the sun or moon rides them.
const SKY_BOTTOM: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Focus,
    Break,
}

pub struct Pomodoro {
    pub focus_len: Duration,
    pub break_len: Duration,
    pub phase: Phase,
    /// When the current phase started; None while paused or idle.
    running_since: Option<Instant>,
    /// Time banked in this phase before the current run.
    banked: Duration,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self {
            focus_len: Duration::from_secs(25 * 60),
            break_len: Duration::from_secs(5 * 60),
            phase: Phase::Idle,
            running_since: None,
            banked: Duration::ZERO,
        }
    }
}

impl Pomodoro {
    /// For the README's screenshots: a writing session `elapsed` in.
    #[cfg(test)]
    pub fn run_for_shots(&mut self, elapsed: Duration) {
        self.phase = Phase::Focus;
        self.banked = Duration::ZERO;
        self.running_since = Instant::now().checked_sub(elapsed);
    }

    pub fn elapsed(&self) -> Duration {
        self.banked + self.running_since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    pub fn len(&self) -> Duration {
        match self.phase {
            Phase::Break => self.break_len,
            _ => self.focus_len,
        }
    }

    pub fn remaining(&self) -> Duration {
        self.len().saturating_sub(self.elapsed())
    }

    /// 0.0 at the start of the phase, 1.0 at the end.
    pub fn progress(&self) -> f64 {
        let len = self.len().as_secs_f64();
        if len <= 0.0 {
            return 0.0;
        }
        (self.elapsed().as_secs_f64() / len).clamp(0.0, 1.0)
    }

    pub fn running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Start, pause, or resume.
    pub fn toggle(&mut self) {
        match self.running_since.take() {
            Some(t) => self.banked += t.elapsed(),
            None => {
                if self.phase == Phase::Idle {
                    self.phase = Phase::Focus;
                    self.banked = Duration::ZERO;
                }
                self.running_since = Some(Instant::now());
            }
        }
    }

    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.running_since = None;
        self.banked = Duration::ZERO;
    }

    /// Roll focus into break and back. Returns true when a phase just ended.
    pub fn tick(&mut self) -> bool {
        if self.running_since.is_none() || self.phase == Phase::Idle {
            return false;
        }
        if self.elapsed() < self.len() {
            return false;
        }
        self.phase = if self.phase == Phase::Focus {
            Phase::Break
        } else {
            Phase::Focus
        };
        self.banked = Duration::ZERO;
        self.running_since = Some(Instant::now());
        true
    }

    pub fn label(&self) -> String {
        match self.phase {
            // "focus" is focus mode's word (Ctrl-D); the timer's working
            // stretch is just writing.
            Phase::Idle => "F2 timer".into(),
            _ => {
                let r = self.remaining().as_secs();
                let tag = if self.phase == Phase::Focus {
                    "writing"
                } else {
                    "break"
                };
                let pause = if self.running() { "" } else { " (paused)" };
                format!("{tag} · {:02}:{:02}{pause}", r / 60, r % 60)
            }
        }
    }
}

/// What each cell of the scene is, so the UI layer can colour it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Sky,
    Star,
    Sun,
    Moon,
    Tree,
    Trunk,
    /// An animal: rabbits, cats, the owl, bats, birds.
    Creature,
    Flower,
    Ground,
    /// Water, and the foam and glints on it.
    Water,
    Foam,
    /// Rock, walls, towers, buildings.
    Stone,
    Snow,
    /// Lamplight: lit windows, lanterns, fire, eyes in the dark.
    Light,
    /// Something that glows through the theme's greens to its pinks — the
    /// aurora, a jellyfish. 0 is foliage, 255 is bloom.
    Glow(u8),
    /// One band of a rainbow, 0 (outside, red) to 4 (inside); faint at night.
    Band {
        n: u8,
        faint: bool,
    },
    /// The theme's lead colour, where a world wants it.
    Accent,
    /// Warm: barns, lighthouse stripes, balloons.
    Warn,
    Cloud,
    Sand,
    /// The ground the session has already crossed.
    Trail,
    /// A paused sun or moon.
    Dim,
    /// A colour chosen outright — the Visualizer paints its own
    /// (`crate::viz_view`), a look per theme.
    Paint(ratatui::style::Color),
}

pub type Cell = (char, Ink);

/// Render the Pomodoro. Takes the phase and progress rather than the whole
/// timer so it can be drawn at any point in a session without waiting 25
/// minutes.
pub fn render(
    scenery: Scenery,
    phase: Phase,
    progress: f64,
    paused: bool,
    frame: u64,
) -> Vec<Vec<Cell>> {
    let m = Moment {
        night: phase == Phase::Break,
        idle: phase == Phase::Idle,
        paused,
        frame,
        progress: progress.clamp(0.0, 1.0),
    };
    let mut g = vec![vec![(' ', Ink::Sky); W]; H];

    // ── sky: stars at night, a few drifting motes by day ──────────────
    let star_seats = [2usize, 7, 11, 16, 21, 25];
    for (i, &x) in star_seats.iter().enumerate() {
        let y = i % 2;
        // Twinkle on staggered periods so it never looks like a metronome.
        let on = !(frame / (6 + i as u64 % 4)).is_multiple_of(3);
        // All six at night; by day, every third drifts past.
        if on && (m.night || i % 3 == 0) {
            g[y][x] = ('·', Ink::Star);
        }
    }

    scenery.paint(&mut g, &m);

    // ── the clock: the sun crosses the sky, the moon at night ─────────
    // Position is the time. Height is just an arc so it feels like a day.
    // Paused, it waits, dimmed.
    if !scenery.draws_own_sun() || m.night {
        let (x, y) = if m.idle {
            (W / 2, SKY_BOTTOM)
        } else {
            let span = W.saturating_sub(4);
            let x = 2 + ((m.progress * span as f64).round() as usize).min(span.saturating_sub(1));
            let arc = (std::f64::consts::PI * m.progress).sin();
            (x, if arc > 0.5 { 0 } else { SKY_BOTTOM })
        };
        let (ch, ink) = if m.night {
            (scenery.moon(), Ink::Moon)
        } else {
            (scenery.sun(), Ink::Sun)
        };
        g[y][x] = (ch, if paused { Ink::Dim } else { ink });
    }

    // ── the ground fills in behind the sun as the session goes ────────
    if !m.idle {
        let head = (m.progress * W as f64).round() as usize;
        for x in 0..head.min(W) {
            if g[H - 1][x].0 != ' ' {
                g[H - 1][x].1 = Ink::Trail;
            }
        }
    }

    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(g: &[Vec<Cell>], ink: Ink) -> Option<(usize, usize)> {
        for (y, row) in g.iter().enumerate() {
            for (x, &(_, i)) in row.iter().enumerate() {
                if i == ink {
                    return Some((x, y));
                }
            }
        }
        None
    }

    #[test]
    fn sun_crosses_the_sky_with_progress() {
        let at = |t: f64| {
            find(
                &render(Scenery::Forest, Phase::Focus, t, false, 0),
                Ink::Sun,
            )
            .expect("sun")
            .0
        };
        let (start, mid, end) = (at(0.0), at(0.5), at(1.0));
        assert!(
            start < mid && mid < end,
            "sun should advance: {start} {mid} {end}"
        );
        assert!(
            end >= W - 4,
            "sun should finish near the right edge, got {end}"
        );
    }

    #[test]
    fn sun_arcs_higher_at_midday() {
        let y = |t: f64| {
            find(
                &render(Scenery::Forest, Phase::Focus, t, false, 0),
                Ink::Sun,
            )
            .expect("sun")
            .1
        };
        assert!(y(0.5) < y(0.0), "midday sun should sit above the dawn sun");
    }

    #[test]
    fn break_swaps_sun_for_moon_and_flowers_for_fireflies() {
        let night = render(Scenery::Forest, Phase::Break, 0.3, false, 0);
        assert!(
            find(&night, Ink::Moon).is_some(),
            "break should show a moon"
        );
        assert!(find(&night, Ink::Sun).is_none(), "break should have no sun");
        assert!(
            find(&night, Ink::Flower).is_none(),
            "flowers close at night"
        );
    }

    #[test]
    fn grid_is_exactly_the_advertised_size() {
        let g = render(Scenery::Forest, Phase::Focus, 0.4, false, 7);
        assert_eq!(g.len(), H);
        assert!(g.iter().all(|r| r.len() == W));
    }

    #[test]
    fn rabbits_never_collide_with_the_treeline() {
        // Sampling frames because the rabbits animate.
        for f in 0..200 {
            let g = render(Scenery::Forest, Phase::Focus, 0.5, false, f);
            for row in g.iter().take(crate::scenery::TRUNK_ROW + 1) {
                assert!(
                    !row.iter().any(|&(_, i)| i == Ink::Creature),
                    "a rabbit climbed into the canopy on frame {f}"
                );
            }
        }
    }

    const PHASES: [(Phase, f64); 4] = [
        (Phase::Idle, 0.0),
        (Phase::Focus, 0.0),
        (Phase::Focus, 0.63),
        (Phase::Break, 0.4),
    ];

    /// Every world, every phase, a few hundred frames: the grid is always
    /// exactly W×H, every glyph one cell wide, and the sky rows hold only sky
    /// things — so nothing a world draws can wander into the clock's path.
    #[test]
    fn every_world_keeps_to_its_grid_and_leaves_the_sky_to_the_clock() {
        for s in Scenery::ALL {
            for (phase, p) in PHASES {
                for f in (0..400).step_by(3) {
                    let g = render(s, phase, p, false, f);
                    assert_eq!(g.len(), H, "{s:?}");
                    for (y, row) in g.iter().enumerate() {
                        assert_eq!(row.len(), W, "{s:?} row {y}");
                        for &(ch, ink) in row {
                            let w = ratatui::text::Span::raw(ch.to_string()).width();
                            assert!(w == 1, "{s:?} draws {ch:?} {w} cells wide");
                            if y < 2 && s != Scenery::Castle && s != Scenery::Peaks {
                                assert!(
                                    matches!(
                                        ink,
                                        Ink::Sky
                                            | Ink::Star
                                            | Ink::Sun
                                            | Ink::Moon
                                            | Ink::Cloud
                                            | Ink::Snow
                                            | Ink::Flower
                                            | Ink::Light
                                            | Ink::Creature
                                            | Ink::Dim
                                    ),
                                    "{s:?} put {ch:?} ({ink:?}) in the sky on frame {f}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_world_shows_its_clock_while_the_timer_runs() {
        for s in Scenery::ALL {
            for f in [0, 7, 99] {
                let day = render(s, Phase::Focus, 0.3, false, f);
                assert!(find(&day, Ink::Sun).is_some(), "{s:?} has no sun");
                let night = render(s, Phase::Break, 0.3, false, f);
                assert!(find(&night, Ink::Moon).is_some(), "{s:?} has no moon");
            }
        }
    }

    #[test]
    fn the_sun_crosses_every_world() {
        for s in Scenery::ALL {
            let at = |t: f64| {
                find(&render(s, Phase::Focus, t, false, 0), Ink::Sun)
                    .expect("sun")
                    .0
            };
            assert!(at(0.0) < at(0.5) && at(0.5) < at(1.0), "{s:?}");
        }
    }

    #[test]
    fn no_two_worlds_look_alike() {
        let idle: Vec<_> = Scenery::ALL
            .iter()
            .map(|&s| render(s, Phase::Idle, 0.0, false, 0))
            .collect();
        for i in 0..idle.len() {
            for j in i + 1..idle.len() {
                assert_ne!(
                    idle[i],
                    idle[j],
                    "{:?} and {:?}",
                    Scenery::ALL[i],
                    Scenery::ALL[j]
                );
            }
        }
    }

    #[test]
    fn the_ground_fills_in_behind_the_sun() {
        let trail = |t: f64| {
            render(Scenery::Forest, Phase::Focus, t, false, 0)[H - 1]
                .iter()
                .filter(|(_, i)| *i == Ink::Trail)
                .count()
        };
        assert_eq!(trail(0.0), 0);
        assert!(trail(0.25) > 0 && trail(0.25) < trail(0.75));
        assert_eq!(trail(1.0), W);
        let idle = render(Scenery::Forest, Phase::Idle, 0.5, false, 0);
        assert!(
            idle[H - 1].iter().all(|(_, i)| *i != Ink::Trail),
            "no trail before F2"
        );
    }

    #[test]
    fn a_paused_sun_waits_dimmed() {
        let g = render(Scenery::Forest, Phase::Focus, 0.4, true, 0);
        assert!(find(&g, Ink::Sun).is_none());
        assert!(find(&g, Ink::Dim).is_some());
    }

    #[test]
    fn themes_get_their_own_worlds_and_custom_gets_the_forest() {
        let names: Vec<String> = crate::theme::presets()
            .into_iter()
            .map(|t| t.name)
            .collect();
        let worlds: Vec<Scenery> = names.iter().map(|n| Scenery::for_theme(n)).collect();
        for i in 0..worlds.len() {
            for j in i + 1..worlds.len() {
                assert_ne!(
                    worlds[i], worlds[j],
                    "{} and {} share a world",
                    names[i], names[j]
                );
            }
        }
        assert_eq!(Scenery::for_theme("Lost Forest"), Scenery::Forest);
        assert_eq!(Scenery::for_theme("Custom"), Scenery::Forest);
        assert_eq!(worlds.len(), Scenery::ALL.len());
    }

    #[test]
    fn timer_phases_roll_over() {
        let mut p = Pomodoro {
            focus_len: Duration::from_millis(1),
            break_len: Duration::from_millis(1),
            ..Default::default()
        };
        p.toggle();
        assert_eq!(p.phase, Phase::Focus);
        std::thread::sleep(Duration::from_millis(5));
        assert!(p.tick(), "focus should have ended");
        assert_eq!(p.phase, Phase::Break);
    }

    #[test]
    fn pausing_banks_time_instead_of_losing_it() {
        let mut p = Pomodoro::default();
        p.toggle();
        std::thread::sleep(Duration::from_millis(20));
        p.toggle(); // pause
        let banked = p.elapsed();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(p.elapsed(), banked, "a paused timer must not advance");
        assert!(!p.running());
    }
}

// ── other things the pane can show ───────────────────────────────────

/// What the small pane under the tree is displaying: the Pomodoro timer or
/// the music Visualizer. Left/Right switches between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Pomodoro,
    Visualizer,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Pomodoro => Mode::Visualizer,
            Mode::Visualizer => Mode::Pomodoro,
        }
    }
    pub fn prev(self) -> Mode {
        self.next()
    }

    pub const ALL: [Mode; 2] = [Mode::Pomodoro, Mode::Visualizer];

    /// The name the switcher on the pane's bottom edge gives this view.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Pomodoro => "pomodoro",
            Mode::Visualizer => "visualizer",
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn the_two_views_toggle_both_ways() {
        let m = Mode::Pomodoro;
        assert_eq!(m.next(), Mode::Visualizer);
        assert_eq!(m.next().next(), m);
        assert_eq!(m.prev(), Mode::Visualizer);
        assert_eq!(Mode::Visualizer.prev(), Mode::Pomodoro);
    }
}
