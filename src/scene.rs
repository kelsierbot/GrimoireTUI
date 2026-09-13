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
    Spectrum,
    Growth,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Clearing => Mode::Spectrum,
            Mode::Spectrum => Mode::Growth,
            Mode::Growth => Mode::Clearing,
        }
    }
    pub fn prev(self) -> Mode {
        self.next().next()
    }
}

/// Partial blocks, zero to seven eighths of a cell, so a bar can end mid-row.
const EIGHTHS: [char; 8] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇'];

/// A spectrum analyser for whatever the machine is playing.
///
/// `levels` and `peaks` are 0..1 per bar, from [`crate::visualizer`], which
/// listens to the real audio. Bars are drawn in eighths of a cell, so eight
/// rows give 64 steps of height. The bottom row is the real playback position,
/// and its played part flashes on a beat. `note` replaces the bars with a short
/// message, for when there is nothing to show and a reason worth giving.
pub fn render_spectrum(
    levels: &[f32],
    peaks: &[f32],
    beat: f32,
    frac: f64,
    note: Option<&str>,
) -> Vec<Vec<Cell>> {
    let mut g = vec![vec![(' ', Ink::Sky); W]; H];
    let rows = H - 1;
    let steps = rows * 8;
    let at = |v: &[f32], x: usize| -> f32 {
        if v.is_empty() {
            0.0
        } else {
            v[x * v.len() / W].clamp(0.0, 1.0)
        }
    };

    if let Some(text) = note {
        let lines = wrap(text, W - 2);
        let top = rows.saturating_sub(lines.len()) / 2;
        for (i, line) in lines.iter().enumerate().take(rows) {
            let x0 = W.saturating_sub(line.chars().count()) / 2;
            for (j, c) in line.chars().enumerate().take(W - x0) {
                g[top + i][x0 + j] = (c, Ink::Star);
            }
        }
    } else {
        for x in 0..W {
            let fill = (at(levels, x) * steps as f32).round() as usize;
            let (full, part) = (fill / 8, fill % 8);
            for r in 0..rows {
                let ch = if r < full {
                    '█'
                } else if r == full && part > 0 {
                    EIGHTHS[part]
                } else {
                    continue;
                };
                // Green at the roots, gold through the middle, bloom at the top.
                let ink = if r >= rows - 2 {
                    Ink::Flower
                } else if r >= rows / 2 {
                    Ink::Sun
                } else {
                    Ink::Tree
                };
                g[rows - 1 - r][x] = (ch, ink);
            }
            // A cap that lingers above the bar after it falls.
            let cap = (at(peaks, x) * steps as f32).round() as usize;
            if cap > fill + 4 {
                let y = rows - 1 - (cap / 8).min(rows - 1);
                if g[y][x].0 == ' ' {
                    g[y][x] = ('▔', Ink::Moon);
                }
            }
        }
    }

    let head = (frac.clamp(0.0, 1.0) * W as f64).round() as usize;
    let played = if beat > 0.5 { Ink::Flower } else { Ink::Sun };
    for x in 0..W {
        g[H - 1][x] = ('▔', if x < head { played } else { Ink::Ground });
    }
    g
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

/// Words per step of growth: every 50 words, something in the garden changes.
pub const WORDS_PER_STEP: usize = 50;

/// How tall each of the fourteen plants grows before it flowers. Uneven on
/// purpose, so the finished garden has a skyline rather than a hedge.
const HEIGHTS: [usize; 14] = [4, 6, 3, 5, 4, 6, 5, 3, 6, 4, 5, 3, 6, 4];

/// A garden that grows with the words you write today.
///
/// Every 50 words adds one cell: the current plant climbs a row (a leaf on
/// every other one), or flowers once it's as tall as it gets, and the next
/// seed starts. `glint` makes the newest growth sparkle so a step is noticed.
/// Reaching the daily target brings the sun out; past a full garden, about
/// 3,900 words, each step adds a firefly to the sky instead.
pub fn render_growth(today: usize, target: usize, glint: bool, frame: u64) -> Vec<Vec<Cell>> {
    let mut g = vec![vec![(' ', Ink::Sky); W]; H];
    let soil = H - 1;
    let base = soil - 1;
    let mut left = today / WORDS_PER_STEP;
    let mut newest = None;

    for (i, &h) in HEIGHTS.iter().enumerate() {
        let x = 1 + 2 * i;
        let k = left.min(h + 1);
        left -= k;
        if k == 0 {
            g[base][x] = ('.', Ink::Trunk); // a seed, waiting its turn
            continue;
        }
        for j in 0..k.min(h) {
            let tip = j + 1 == k && k <= h;
            g[base - j][x] = (if tip { '╷' } else { '│' }, Ink::Tree);
            if j % 2 == 1 && x + 1 < W {
                g[base - j][x + 1] = ('❧', Ink::Tree);
            }
            newest = Some((base - j, x));
        }
        if k == h + 1 {
            g[base - h][x] = (if i % 2 == 0 { '✿' } else { '❀' }, Ink::Flower);
            newest = Some((base - h, x));
        }
    }

    if target > 0 && today >= target {
        g[0][W - 4] = ('☀', Ink::Sun);
    }

    // A full garden: further steps gather fireflies in whatever sky is free,
    // in a scattered but fixed order so each new one lands somewhere new.
    if left > 0 {
        let mut sky: Vec<(usize, usize)> = (0..2)
            .flat_map(|y| (0..W).map(move |x| (y, x)))
            .filter(|&(y, x)| g[y][x].0 == ' ')
            .collect();
        sky.sort_by_key(|&(y, x)| (y * W + x) * 7919 % 97);
        for &(y, x) in sky.iter().take(left) {
            g[y][x] = ('˙', Ink::Star);
            newest = Some((y, x));
        }
    }

    if let (true, Some((y, x))) = (glint, newest) {
        if frame % 2 == 0 {
            g[y][x] = ('✦', Ink::Moon);
        }
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
        assert_eq!(Mode::Spectrum.prev(), Mode::Clearing);
    }

    fn count(g: &[Vec<Cell>], c: char) -> usize {
        g.iter().flatten().filter(|(ch, _)| *ch == c).count()
    }

    #[test]
    fn bars_climb_in_eighths_of_a_cell() {
        let half = render_spectrum(&[0.5; W], &[0.0; W], 0.0, 0.0, None);
        assert_eq!(count(&half, '█'), W * (H - 1) / 2, "half height fills half the rows");

        let one_more = 0.5 + 1.0 / ((H - 1) * 8) as f32;
        let g = render_spectrum(&[one_more; W], &[0.0; W], 0.0, 0.0, None);
        assert_eq!(count(&g, '▁'), W, "an extra eighth shows as a partial block");
    }

    #[test]
    fn silence_draws_no_bars() {
        let g = render_spectrum(&[0.0; W], &[0.0; W], 0.0, 0.0, None);
        assert!(g[..H - 1].iter().flatten().all(|(c, _)| *c == ' '));
    }

    #[test]
    fn a_falling_bar_leaves_its_peak_cap_behind() {
        let g = render_spectrum(&[0.1; W], &[0.9; W], 0.0, 0.0, None);
        assert_eq!(count(&g[..H - 1], '▔'), W, "every bar should show a cap above it");
    }

    #[test]
    fn bottom_row_is_the_playback_position() {
        let played = |f: f64| {
            render_spectrum(&[0.0; W], &[0.0; W], 0.0, f, None)[H - 1]
                .iter()
                .filter(|(_, i)| *i == Ink::Sun)
                .count()
        };
        assert_eq!(played(0.0), 0);
        assert!(played(0.9) > played(0.1), "more of the row fills as the track plays");
    }

    #[test]
    fn a_note_replaces_the_bars() {
        let g = render_spectrum(&[1.0; W], &[1.0; W], 0.0, 0.0, Some("allow audio capture in settings"));
        let text: String = g.iter().flatten().map(|(c, _)| *c).collect();
        assert!(text.contains("allow"), "the note should be drawn");
        assert_eq!(count(&g, '█'), 0, "and the bars hidden behind it");
    }

    #[test]
    fn every_fifty_words_grows_something() {
        // Past the 78 steps of a full garden, into the fireflies.
        let mut prev = render_growth(0, 1000, false, 0);
        for step in 1..=90 {
            let g = render_growth(step * WORDS_PER_STEP, 1000, false, 0);
            assert_ne!(g, prev, "{} words should look different from {}", step * 50, (step - 1) * 50);
            prev = g;
        }
    }

    #[test]
    fn words_between_steps_change_nothing() {
        assert_eq!(render_growth(100, 1000, false, 0), render_growth(149, 1000, false, 0));
    }

    #[test]
    fn plants_flower_once_they_are_full_grown() {
        let flowers = |w: usize| {
            render_growth(w, 1000, false, 0)
                .iter()
                .flatten()
                .filter(|(_, i)| *i == Ink::Flower)
                .count()
        };
        assert_eq!(flowers(4 * WORDS_PER_STEP), 0, "the first plant is still growing");
        assert_eq!(flowers(5 * WORDS_PER_STEP), 1, "and flowers on its fifth step");
    }

    #[test]
    fn the_sun_comes_out_at_the_daily_target() {
        let sun = |w: usize| render_growth(w, 1000, false, 0).iter().flatten().any(|(_, i)| *i == Ink::Sun);
        assert!(!sun(950));
        assert!(sun(1000));
    }

    #[test]
    fn new_growth_glints() {
        let g = render_growth(500, 1000, true, 0);
        assert!(g.iter().flatten().any(|(c, _)| *c == '✦'));
        let quiet = render_growth(500, 1000, false, 0);
        assert!(!quiet.iter().flatten().any(|(c, _)| *c == '✦'));
    }

    #[test]
    fn every_mode_renders_the_advertised_grid() {
        for g in [
            render_spectrum(&[0.3; W], &[0.6; W], 1.0, 0.5, None),
            render_spectrum(&[], &[], 0.0, 0.0, Some("nothing playing")),
            render_spectrum(&[0.2; 7], &[], 0.0, 2.0, Some("a-note-far-longer-than-the-pane-is-wide")),
            render_growth(300, 1000, true, 5),
            render_growth(9000, 1000, false, 0),
        ] {
            assert_eq!(g.len(), H);
            assert!(g.iter().all(|r| r.len() == W));
        }
    }
}
