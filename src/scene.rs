//! A quiet forest that doubles as the timer.
//!
//! The sun crosses the sky over the course of a focus session, so you read the
//! time remaining off the light rather than a countdown. Break time is night:
//! the moon comes up and the fireflies come out.

use std::time::{Duration, Instant};

pub const W: usize = 28;
pub const H: usize = 9;

// Row budget: 0-1 sky, 2-4 foliage, 5 trunks, 6-7 rabbits, 8 ground.
const SKY_BOTTOM: usize = 1;
const CANOPY_BASE: usize = 4;
const TRUNK_ROW: usize = 5;
const RABBIT_TOP: usize = 6;

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
            Phase::Idle => "press F2 to begin".into(),
            _ => {
                let r = self.remaining().as_secs();
                let tag = if self.phase == Phase::Focus { "focus" } else { "break" };
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
    Rabbit,
    Flower,
    Ground,
}

pub type Cell = (char, Ink);

/// Render the scene. Takes phase and progress rather than the whole timer so
/// it can be exercised at any point in a session without waiting 25 minutes.
pub fn render(phase: Phase, progress: f64, frame: u64) -> Vec<Vec<Cell>> {
    let night = phase == Phase::Break;
    let mut g = vec![vec![(' ', Ink::Sky); W]; H];

    // ── sky: stars at night, a few drifting motes by day ──────────────
    let star_seats = [2usize, 7, 11, 16, 21, 25];
    for (i, &x) in star_seats.iter().enumerate() {
        let y = i % 2;
        // Twinkle on staggered periods so it never looks like a metronome.
        let on = (frame / (6 + i as u64 % 4)) % 3 != 0;
        if night && on {
            g[y][x] = ('·', Ink::Star);
        } else if !night && i % 3 == 0 && on {
            g[y][x] = ('·', Ink::Star);
        }
    }

    // ── the timer: sun crosses the sky, moon rises at night ───────────
    // Position is the clock. Height is just an arc so it feels like a day.
    let t = progress.clamp(0.0, 1.0);
    let span = W.saturating_sub(4);
    let x = 2 + ((t * span as f64).round() as usize).min(span.saturating_sub(1));
    let arc = (std::f64::consts::PI * t).sin();
    let y = if arc > 0.5 { 0 } else { SKY_BOTTOM };

    if phase == Phase::Idle {
        g[SKY_BOTTOM][W / 2] = ('☀', Ink::Sun);
    } else if night {
        g[y][x] = ('☾', Ink::Moon);
    } else {
        g[y][x] = ('☀', Ink::Sun);
    }

    // ── treeline: centred triangles, bottom-aligned on the canopy row ──
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
        if cx < W {
            g[TRUNK_ROW][cx] = ('┃', Ink::Trunk);
        }
    }

    // ── rabbits ───────────────────────────────────────────────────────
    // Ears flick and heads turn on slow, mutually-prime cycles, so the two
    // of them never move in lockstep.
    let ears_a = if ((frame + 5) / 9) % 7 == 0 { "(\\ /)" } else { "(\\_/)" };
    let ears_b = if ((frame + 31) / 11) % 9 == 0 { "(\\ /)" } else { "(\\_/)" };
    let face_a = if ((frame + 13) / 23) % 5 == 0 { "(-ᴥ-)" } else { "(•ᴥ•)" };
    let face_b = if ((frame + 44) / 17) % 6 == 0 { "(-ᴥ-)" } else { "(•ᴥ•)" };

    put(&mut g, 3, RABBIT_TOP, ears_a, Ink::Rabbit);
    put(&mut g, 3, RABBIT_TOP + 1, face_a, Ink::Rabbit);
    put(&mut g, 20, RABBIT_TOP, ears_b, Ink::Rabbit);
    put(&mut g, 20, RABBIT_TOP + 1, face_b, Ink::Rabbit);

    // Flowers by day; fireflies drifting between the two by night.
    if night {
        let seats = [(RABBIT_TOP, 12usize), (RABBIT_TOP + 1, 15), (CANOPY_BASE + 1, 10)];
        for (i, &(fy, fx)) in seats.iter().enumerate() {
            if (frame / (5 + i as u64 * 3)) % 4 != 0 && fy < H && fx < W {
                g[fy][fx] = ('˙', Ink::Star);
            }
        }
    } else {
        g[RABBIT_TOP + 1][12] = ('❀', Ink::Flower);
        g[RABBIT_TOP + 1][15] = ('✿', Ink::Flower);
    }

    // ── ground ────────────────────────────────────────────────────────
    for x in 0..W {
        g[H - 1][x] = ('▔', Ink::Ground);
    }

    g
}

fn put(g: &mut [Vec<Cell>], x: usize, y: usize, s: &str, ink: Ink) {
    if y >= g.len() {
        return;
    }
    for (i, ch) in s.chars().enumerate() {
        let cx = x + i;
        if cx < W {
            g[y][cx] = (ch, ink);
        }
    }
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
        let at = |t: f64| find(&render(Phase::Focus, t, 0), Ink::Sun).expect("sun").0;
        let (start, mid, end) = (at(0.0), at(0.5), at(1.0));
        assert!(start < mid && mid < end, "sun should advance: {start} {mid} {end}");
        assert!(end >= W - 4, "sun should finish near the right edge, got {end}");
    }

    #[test]
    fn sun_arcs_higher_at_midday() {
        let y = |t: f64| find(&render(Phase::Focus, t, 0), Ink::Sun).expect("sun").1;
        assert!(y(0.5) < y(0.0), "midday sun should sit above the dawn sun");
    }

    #[test]
    fn break_swaps_sun_for_moon_and_flowers_for_fireflies() {
        let night = render(Phase::Break, 0.3, 0);
        assert!(find(&night, Ink::Moon).is_some(), "break should show a moon");
        assert!(find(&night, Ink::Sun).is_none(), "break should have no sun");
        assert!(find(&night, Ink::Flower).is_none(), "flowers close at night");
    }

    #[test]
    fn grid_is_exactly_the_advertised_size() {
        let g = render(Phase::Focus, 0.4, 7);
        assert_eq!(g.len(), H);
        assert!(g.iter().all(|r| r.len() == W));
    }

    #[test]
    fn rabbits_never_collide_with_the_treeline() {
        // Sampling frames because the rabbits animate.
        for f in 0..200 {
            let g = render(Phase::Focus, 0.5, f);
            for row in g.iter().take(TRUNK_ROW + 1) {
                assert!(
                    !row.iter().any(|&(_, i)| i == Ink::Rabbit),
                    "a rabbit climbed into the canopy on frame {f}"
                );
            }
        }
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

/// What the small pane under the tree is displaying. Left/Right cycles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Clearing,
    Waveform,
    Growth,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Clearing => Mode::Waveform,
            Mode::Waveform => Mode::Growth,
            Mode::Growth => Mode::Clearing,
        }
    }
    pub fn prev(self) -> Mode {
        self.next().next()
    }
}

fn hash(s: &str) -> u64 {
    // FNV-1a. Only needs to be stable and well-spread, not cryptographic.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A waveform for the playing track.
///
/// Honest about what this is: the API gives track metadata and position, never
/// audio samples, so the *shape* is a stable fingerprint derived from the
/// title — the same song always draws the same wave — while the playhead and
/// the fill are real playback position. It is a progress bar with a face, not
/// a spectrum analyser, and pretending otherwise would be a lie in pixels.
pub fn render_waveform(title: &str, frac: f64, playing: bool, frame: u64) -> Vec<Vec<Cell>> {
    let mut g = vec![vec![(' ', Ink::Sky); W]; H];
    if title.is_empty() {
        let msg = "nothing playing";
        let x0 = (W - msg.len()) / 2;
        for (i, c) in msg.chars().enumerate() {
            g[H / 2][x0 + i] = (c, Ink::Star);
        }
        return g;
    }

    let seed = hash(title);
    let head = ((frac.clamp(0.0, 1.0)) * (W - 1) as f64).round() as usize;
    let rows = H - 1;

    for x in 0..W {
        // Two mixed frequencies keep it from looking like a sawtooth.
        let a = ((seed >> (x % 48)) & 0x7) as f64 / 7.0;
        let b = (((seed.rotate_left(x as u32 * 3)) >> 5) & 0x7) as f64 / 7.0;
        let mut v = (a * 0.65 + b * 0.35).clamp(0.05, 1.0);

        // Only the playhead breathes, and only while actually playing.
        if playing && x == head {
            v = (v + 0.25 * (((frame / 2) % 4) as f64 / 3.0)).min(1.0);
        }
        let h = ((v * rows as f64).round() as usize).max(1);

        for y in 0..rows {
            if y >= rows - h {
                let ink = if x < head {
                    Ink::Sun
                } else if x == head {
                    Ink::Moon
                } else {
                    Ink::Tree
                };
                g[y][x] = ('█', ink);
            }
        }
    }
    for x in 0..W {
        g[H - 1][x] = ('▔', Ink::Ground);
    }
    g
}

/// A plant that grows with the words you write today.
///
/// Reactive in the way that matters for a writing app: the stem climbs and
/// leaves unfurl as the session count rises toward the daily target, so the
/// pane answers "how is today going" without a number.
pub fn render_growth(today: usize, target: usize, frame: u64) -> Vec<Vec<Cell>> {
    let mut g = vec![vec![(' ', Ink::Sky); W]; H];
    let frac = if target == 0 {
        0.0
    } else {
        (today as f64 / target as f64).clamp(0.0, 1.0)
    };

    let soil = H - 1;
    let max_stem = soil - 1;
    let stem = ((frac * max_stem as f64).round() as usize).min(max_stem);
    let cx = W / 2;

    if stem == 0 {
        // A seed, waiting.
        g[soil - 1][cx] = ('.', Ink::Trunk);
    }

    for i in 0..stem {
        let y = soil - 1 - i;
        // A gentle sway, slow enough to read as alive rather than jittery.
        let sway = if (frame / 12 + i as u64) % 7 == 0 { 1 } else { 0 };
        let x = cx + sway;
        if x < W {
            g[y][x] = ('│', Ink::Tree);
        }
        // Leaves alternate sides every other segment.
        if i > 0 && i % 2 == 0 {
            let (lx, ch) = if (i / 2) % 2 == 0 {
                (x.saturating_sub(1), '❧')
            } else {
                ((x + 1).min(W - 1), '❦')
            };
            g[y][lx] = (ch, Ink::Tree);
        }
    }

    // A bloom once the day's target is met.
    if frac >= 1.0 && stem > 0 {
        let y = soil - 1 - (stem - 1);
        g[y.saturating_sub(1).min(H - 1)][cx] = ('✿', Ink::Flower);
    }

    for x in 0..W {
        g[soil][x] = ('▔', Ink::Ground);
    }
    g
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn modes_cycle_both_ways() {
        let m = Mode::Clearing;
        assert_eq!(m.next().next().next(), m);
        assert_eq!(m.next().prev(), m);
        assert_eq!(Mode::Waveform.prev(), Mode::Clearing);
    }

    #[test]
    fn waveform_is_stable_per_track_but_fills_with_progress() {
        let a = render_waveform("Weightless", 0.0, false, 0);
        let b = render_waveform("Weightless", 0.0, false, 0);
        assert_eq!(a, b, "the same track must always draw the same wave");

        let c = render_waveform("Something Else", 0.0, false, 0);
        assert_ne!(a, c, "different tracks should look different");

        let played = |f: f64| {
            render_waveform("Weightless", f, false, 0)
                .iter()
                .flatten()
                .filter(|(_, i)| *i == Ink::Sun)
                .count()
        };
        assert!(played(0.9) > played(0.1), "more of the wave fills as it plays");
    }

    #[test]
    fn growth_tracks_the_daily_count() {
        let stem = |t: usize| {
            render_growth(t, 1000, 0)
                .iter()
                .flatten()
                .filter(|(c, _)| *c == '│')
                .count()
        };
        assert_eq!(stem(0), 0, "nothing written, nothing grown");
        assert!(stem(500) > 0);
        assert!(stem(1000) > stem(500), "the plant keeps climbing");

        let bloomed = render_growth(1200, 1000, 0)
            .iter()
            .flatten()
            .any(|(c, _)| *c == '✿');
        assert!(bloomed, "hitting the target should flower");
    }

    #[test]
    fn every_mode_renders_the_advertised_grid() {
        for g in [
            render_waveform("x", 0.5, true, 3),
            render_waveform("", 0.0, false, 0),
            render_growth(300, 1000, 5),
        ] {
            assert_eq!(g.len(), H);
            assert!(g.iter().all(|r| r.len() == W));
        }
    }
}
