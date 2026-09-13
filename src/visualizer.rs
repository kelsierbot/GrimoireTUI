//! Listens to what the machine is playing and turns it into spectrum bars.
//!
//! Audio comes from Core Audio loopback (through cpal) on macOS 14.6+: a tap
//! on the default output device, so it works whichever source is playing —
//! YouTube Music, Spotify, Jellyfin, Plex, anything. Nothing is recorded or
//! kept; samples live in a ring buffer a few hundredths of a second long.
//!
//! Capture runs only while the spectrum view is on screen, so the one-time
//! system-audio permission prompt only ever reaches people who open it.

use std::f32::consts::PI;
use std::time::Instant;

/// Samples per analysis window: about 43 ms at 48 kHz, 23 Hz per bin.
const FFT_LEN: usize = 2048;
/// One bar per column of the pane.
const BARS: usize = crate::scene::W;

const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 12_000.0;
/// Bars span this many dB below the loudest recent band.
const RANGE_DB: f32 = 44.0;
/// Bends the bar heights so loud bands stand out from the rest. A busy mix
/// keeps nearly every band within 20 dB of the loudest, so on a straight scale
/// the bottom half of the pane is a solid slab and only the tips move. At 2.0
/// the rows fill at roughly 26, 19, 13, 8 and 4 dB below the loudest band.
const CONTRAST: f32 = 2.0;
/// How fast the top of the pane eases down after a loud moment, in dB per
/// second, so a quiet passage fills the pane again soon after.
const GAIN_RELEASE: f32 = 10.0;
/// Music carries most of its energy in the bass; tilting the highs up gives
/// every bar a fair share of the pane.
const TILT_DB_PER_OCTAVE: f32 = 3.0;
/// Bars and caps fall under gravity, in pane-heights per second squared, so
/// they drop slowly at first and then quickly. Caps hold a moment before.
const GRAVITY: f32 = 7.0;
const PEAK_GRAVITY: f32 = 1.2;
const PEAK_HOLD: f32 = 0.35;
/// Sparks: a bar that leaps this far in one frame throws one off, and every
/// beat throws them from the tallest bars. They fly up, arc over and fade.
const SPARK_JUMP: f32 = 0.35;
const SPARK_GRAVITY: f32 = 2.2;
const MAX_SPARKS: usize = 36;
/// How long "playing" may stay silent before the pane explains why.
const SILENT_HINT_AFTER: f32 = 3.0;
/// macOS mutes the tap, rather than failing, until the terminal is allowed.
const SILENT_HINT: &str = "hearing only silence. add your terminal to Privacy & Security › \
    Screen & System Audio Recording › System Audio Recording Only";

/// A spark thrown off the top of a bar. Position is in fractions of the bar
/// area — `x` across, `y` up — so it draws at any size.
#[derive(Debug, Clone, Copy)]
pub struct Spark {
    pub x: f32,
    pub y: f32,
    /// What's left of its life, 1.0 new to 0.0 gone.
    pub life: f32,
    vx: f32,
    vy: f32,
    ttl: f32,
}

impl Spark {
    /// A spark placed by hand, for drawing tests.
    #[cfg(test)]
    pub fn at(x: f32, y: f32) -> Spark {
        Spark { x, y, life: 1.0, vx: 0.0, vy: 0.0, ttl: 1.0 }
    }
}

/// Turns windows of samples into bar heights. No audio hardware involved, so
/// it can be tested with synthetic signals.
pub struct Analyzer {
    pub levels: Vec<f32>,
    pub peaks: Vec<f32>,
    /// Seconds each peak cap still holds before it starts to fall.
    pub hold: Vec<f32>,
    /// 1.0 on a beat, fading to 0.0 over a quarter of a second.
    pub beat: f32,
    /// Seconds of unbroken digital silence.
    pub silent_for: f32,
    pub sparks: Vec<Spark>,
    /// Seconds analysed so far; the pond's ripple runs on it.
    pub clock: f32,
    /// How fast each bar, and each cap, is falling right now.
    fall: Vec<f32>,
    peak_fall: Vec<f32>,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    top_db: f32,
    prev_bass: f32,
    flux_avg: f32,
    since_beat: f32,
    seed: u32,
}

impl Analyzer {
    pub fn new(bars: usize) -> Analyzer {
        let hann = (0..FFT_LEN)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_LEN - 1) as f32).cos())
            .collect();
        Analyzer {
            levels: vec![0.0; bars],
            peaks: vec![0.0; bars],
            hold: vec![0.0; bars],
            beat: 0.0,
            silent_for: 0.0,
            sparks: Vec::new(),
            clock: 0.0,
            fall: vec![0.0; bars],
            peak_fall: vec![0.0; bars],
            window: hann,
            re: vec![0.0; FFT_LEN],
            im: vec![0.0; FFT_LEN],
            top_db: -30.0,
            prev_bass: 0.0,
            flux_avg: 0.0,
            since_beat: 1.0,
            seed: 0x9e37_79b9,
        }
    }

    /// Analyse the latest mono samples (oldest first; short input is padded)
    /// and advance the bars by `dt` seconds.
    pub fn update(&mut self, samples: &[f32], rate: f32, dt: f32) {
        let targets = self.targets(samples, rate, dt);
        self.clock += dt;

        // Bars jump up at once and fall back under gravity; caps hold, then
        // fall the same way, only slower.
        let mut leapt = Vec::new();
        for (i, &t) in targets.iter().enumerate() {
            let l = &mut self.levels[i];
            if t >= *l {
                if t - *l > SPARK_JUMP {
                    leapt.push(i);
                }
                *l = t;
                self.fall[i] = 0.0;
            } else {
                self.fall[i] += GRAVITY * dt;
                *l = (*l - self.fall[i] * dt).max(t);
            }
            if *l >= self.peaks[i] {
                self.peaks[i] = *l;
                self.hold[i] = PEAK_HOLD;
                self.peak_fall[i] = 0.0;
            } else if self.hold[i] > 0.0 {
                self.hold[i] -= dt;
            } else {
                self.peak_fall[i] += PEAK_GRAVITY * dt;
                self.peaks[i] = (self.peaks[i] - self.peak_fall[i] * dt).max(*l);
            }
        }

        // A beat is a sudden rise in the bass bars, well above its recent average.
        let bass_n = (targets.len() / 7).max(1);
        let bass = targets[..bass_n].iter().sum::<f32>() / bass_n as f32;
        let flux = (bass - self.prev_bass).max(0.0);
        self.prev_bass = bass;
        self.since_beat += dt;
        if flux > (self.flux_avg * 2.5).max(0.12) && self.since_beat > 0.25 {
            self.beat = 1.0;
            self.since_beat = 0.0;
            // The three tallest bars go up in sparks.
            let mut tall: Vec<usize> = (0..self.levels.len()).collect();
            tall.sort_by(|&a, &b| self.levels[b].total_cmp(&self.levels[a]));
            for &i in tall.iter().take(3) {
                self.spark(i);
                self.spark(i);
            }
        } else {
            self.beat = (self.beat - 4.0 * dt).max(0.0);
        }
        self.flux_avg += (flux - self.flux_avg) * (dt / 0.5).min(1.0);
        for i in leapt {
            self.spark(i);
        }

        for s in &mut self.sparks {
            s.vy -= SPARK_GRAVITY * dt;
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.life -= dt / s.ttl;
        }
        self.sparks
            .retain(|s| s.life > 0.0 && s.y > 0.0 && (0.0..1.0).contains(&s.x));
    }

    /// Throw a spark up off the top of bar `i`.
    fn spark(&mut self, i: usize) {
        if self.sparks.len() >= MAX_SPARKS {
            return;
        }
        let bars = self.levels.len() as f32;
        let (a, b, c) = (self.rand(), self.rand(), self.rand());
        self.sparks.push(Spark {
            x: (i as f32 + 0.5) / bars,
            y: self.levels[i].max(0.02),
            life: 1.0,
            vx: (a - 0.5) * 0.3,
            vy: 0.9 + b * 0.9,
            ttl: 0.7 + c * 0.6,
        });
    }

    /// xorshift: plenty for scattering sparks, and repeatable in tests.
    fn rand(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Where each bar wants to be this frame, 0..1.
    fn targets(&mut self, samples: &[f32], rate: f32, dt: f32) -> Vec<f32> {
        let bars = self.levels.len();
        let n = samples.len().min(FFT_LEN);
        let tail = &samples[samples.len() - n..];
        if tail.iter().all(|s| s.abs() < 1e-4) {
            self.silent_for += dt;
            return vec![0.0; bars];
        }
        self.silent_for = 0.0;

        let pad = FFT_LEN - n;
        for i in 0..FFT_LEN {
            self.re[i] = if i < pad { 0.0 } else { tail[i - pad] * self.window[i] };
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im);

        // Scaled so a full-scale sine reads 0 dB through the Hann window.
        let norm = 4.0 / FFT_LEN as f32;
        let bin_hz = rate / FFT_LEN as f32;
        let half = FFT_LEN / 2;
        let edge = |b: usize| LOW_HZ * (HIGH_HZ / LOW_HZ).powf(b as f32 / bars as f32);
        let db: Vec<f32> = (0..bars)
            .map(|b| {
                let (lo, hi) = (edge(b), edge(b + 1));
                let first = ((lo / bin_hz).floor() as usize).clamp(1, half - 1);
                let last = ((hi / bin_hz).ceil() as usize).clamp(first + 1, half);
                let mag = (first..last)
                    .map(|k| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt())
                    .fold(0.0f32, f32::max)
                    * norm;
                let centre = (lo * hi).sqrt();
                20.0 * mag.max(1e-9).log10() + TILT_DB_PER_OCTAVE * (centre / LOW_HZ).log2()
            })
            .collect();

        // Auto-gain: the top of the pane follows the loudest recent band,
        // easing down GAIN_RELEASE dB a second, but never so low that hiss
        // fills it.
        let loudest = db.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        self.top_db = loudest.max(self.top_db - GAIN_RELEASE * dt).max(-40.0);
        let floor = self.top_db - RANGE_DB;
        db.iter()
            .map(|d| ((d - floor) / RANGE_DB).clamp(0.0, 1.0).powf(CONTRAST))
            .collect()
    }
}

/// In-place iterative radix-2 FFT. `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

enum Status {
    Off,
    Listening,
    Unavailable(String),
}

/// The analyser plus the capture that feeds it, switched with the view.
pub struct Visualizer {
    analyzer: Analyzer,
    capture: Option<capture::Capture>,
    status: Status,
    last: Option<Instant>,
    samples: Vec<f32>,
}

impl std::ops::Deref for Visualizer {
    type Target = Analyzer;
    fn deref(&self) -> &Analyzer {
        &self.analyzer
    }
}

impl Visualizer {
    pub fn new() -> Visualizer {
        Visualizer {
            analyzer: Analyzer::new(BARS),
            capture: None,
            status: Status::Off,
            last: None,
            samples: Vec::with_capacity(FFT_LEN),
        }
    }

    /// Start listening when the view appears; stop, and release the tap,
    /// when it goes. A failed start is retried the next time the view opens.
    pub fn set_active(&mut self, on: bool) {
        match (on, matches!(self.status, Status::Off)) {
            (true, true) => {
                self.status = match capture::Capture::start() {
                    Ok(c) => {
                        self.capture = Some(c);
                        Status::Listening
                    }
                    Err(why) => Status::Unavailable(why),
                };
                self.last = None;
            }
            (false, false) => {
                self.capture = None;
                self.status = Status::Off;
                self.analyzer = Analyzer::new(BARS);
            }
            _ => {}
        }
    }

    /// Pull whatever audio arrived since the last frame and move the bars.
    pub fn update(&mut self) {
        let now = Instant::now();
        let dt = self
            .last
            .map_or(1.0 / 30.0, |t| now.duration_since(t).as_secs_f32())
            .min(0.25);
        self.last = Some(now);
        let Some(cap) = &self.capture else {
            return;
        };
        cap.latest(&mut self.samples);
        self.analyzer.update(&self.samples, cap.rate, dt);
    }

    /// A message to show instead of bars, when there's a reason worth giving.
    pub fn note(&self, playing: bool) -> Option<String> {
        match &self.status {
            Status::Unavailable(why) => Some(why.clone()),
            Status::Listening if self.analyzer.silent_for > SILENT_HINT_AFTER && playing => {
                Some(SILENT_HINT.into())
            }
            Status::Listening if self.analyzer.silent_for > 0.5 && !playing => {
                Some("nothing playing".into())
            }
            _ => None,
        }
    }
}

#[cfg(all(feature = "audio", target_os = "macos"))]
mod capture {
    use super::FFT_LEN;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    /// The most recent `FFT_LEN` mono samples, overwritten as audio arrives.
    struct Ring {
        buf: Vec<f32>,
        pos: usize,
        filled: bool,
    }

    impl Ring {
        fn push(&mut self, s: f32) {
            self.buf[self.pos] = s;
            self.pos = (self.pos + 1) % self.buf.len();
            self.filled |= self.pos == 0;
        }
    }

    pub struct Capture {
        // Dropping the stream tears down the tap and its aggregate device.
        _stream: cpal::Stream,
        ring: Arc<Mutex<Ring>>,
        pub rate: f32,
    }

    impl Capture {
        /// An input stream on the *output* device: cpal turns that into a
        /// Core Audio process tap covering everything the machine plays.
        pub fn start() -> Result<Capture, String> {
            let device = cpal::default_host()
                .default_output_device()
                .ok_or_else(|| "no audio output to listen to".to_string())?;
            let cfg = device
                .default_output_config()
                .map_err(|e| format!("can't read the output format: {e}"))?;
            if cfg.sample_format() != cpal::SampleFormat::F32 {
                return Err(format!("output format {:?} isn't supported yet", cfg.sample_format()));
            }
            let channels = (cfg.channels() as usize).max(1);
            let ring = Arc::new(Mutex::new(Ring {
                buf: vec![0.0; FFT_LEN],
                pos: 0,
                filled: false,
            }));
            let sink = Arc::clone(&ring);
            let stream = device
                .build_input_stream(
                    &cfg.config(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut r) = sink.lock() {
                            for frame in data.chunks(channels) {
                                r.push(frame.iter().sum::<f32>() / frame.len() as f32);
                            }
                        }
                    },
                    |_| {},
                    None,
                )
                .map_err(|e| format!("can't listen to system audio: {e}"))?;
            stream
                .play()
                .map_err(|e| format!("can't start listening: {e}"))?;
            Ok(Capture {
                _stream: stream,
                ring,
                rate: cfg.sample_rate() as f32,
            })
        }

        /// Copy out the ring, oldest sample first.
        pub fn latest(&self, out: &mut Vec<f32>) {
            out.clear();
            if let Ok(r) = self.ring.lock() {
                if r.filled {
                    out.extend_from_slice(&r.buf[r.pos..]);
                }
                out.extend_from_slice(&r.buf[..r.pos]);
            }
        }
    }
}

#[cfg(not(all(feature = "audio", target_os = "macos")))]
mod capture {
    pub struct Capture {
        pub rate: f32,
    }

    impl Capture {
        pub fn start() -> Result<Capture, String> {
            Err(if cfg!(feature = "audio") {
                "the spectrum listens through Core Audio, so it needs macOS 14.6 or later for now"
            } else {
                "this build has no audio support"
            }
            .into())
        }

        pub fn latest(&self, _out: &mut Vec<f32>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;
    const DT: f32 = 1.0 / 30.0;

    fn tone(hz: f32, amp: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| amp * (2.0 * PI * hz * i as f32 / RATE).sin()).collect()
    }

    fn loudest_bar(a: &Analyzer) -> usize {
        (0..a.levels.len())
            .max_by(|&x, &y| a.levels[x].total_cmp(&a.levels[y]))
            .unwrap()
    }

    #[test]
    fn fft_puts_a_sine_in_its_bin() {
        let mut re: Vec<f32> = (0..64).map(|i| (2.0 * PI * 5.0 * i as f32 / 64.0).sin()).collect();
        let mut im = vec![0.0; 64];
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..32).map(|k| re[k].hypot(im[k])).collect();
        let top = (0..32).max_by(|&a, &b| mag[a].total_cmp(&mag[b])).unwrap();
        assert_eq!(top, 5);
    }

    #[test]
    fn bass_lights_the_left_and_treble_the_right() {
        let mut low = Analyzer::new(BARS);
        low.update(&tone(80.0, 0.5, FFT_LEN), RATE, DT);
        assert!(loudest_bar(&low) < BARS / 4, "80 Hz landed on bar {}", loudest_bar(&low));

        let mut high = Analyzer::new(BARS);
        high.update(&tone(6000.0, 0.5, FFT_LEN), RATE, DT);
        assert!(loudest_bar(&high) > BARS * 3 / 4, "6 kHz landed on bar {}", loudest_bar(&high));
    }

    #[test]
    fn bars_fall_smoothly_rather_than_vanishing() {
        let mut a = Analyzer::new(BARS);
        a.update(&tone(200.0, 0.5, FFT_LEN), RATE, DT);
        let bar = loudest_bar(&a);
        let before = a.levels[bar];
        a.update(&vec![0.0; FFT_LEN], RATE, DT);
        assert!(a.levels[bar] > 0.0 && a.levels[bar] < before, "one frame of silence should dip, not drop");
        for _ in 0..30 {
            a.update(&vec![0.0; FFT_LEN], RATE, DT);
        }
        assert_eq!(a.levels[bar], 0.0, "a second of silence should empty the bar");
        assert!(a.silent_for > 0.9);
    }

    #[test]
    fn peak_caps_outlast_the_bars() {
        let mut a = Analyzer::new(BARS);
        a.update(&tone(200.0, 0.5, FFT_LEN), RATE, DT);
        let bar = loudest_bar(&a);
        for _ in 0..8 {
            a.update(&vec![0.0; FFT_LEN], RATE, DT);
        }
        assert!(a.peaks[bar] > a.levels[bar] + 0.2, "the cap should hang above the falling bar");
    }

    #[test]
    fn kicks_register_as_beats() {
        // A 60 Hz thump for 80 ms every half second, silence between, 4 s long.
        let len = (RATE * 4.0) as usize;
        let signal: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f32 / RATE;
                if t % 0.5 < 0.08 { 0.8 * (2.0 * PI * 60.0 * t).sin() } else { 0.0 }
            })
            .collect();
        let hop = (RATE * DT) as usize;
        let mut a = Analyzer::new(BARS);
        let mut beats = 0;
        let mut was = 0.0;
        let mut end = FFT_LEN;
        while end <= len {
            a.update(&signal[end - FFT_LEN..end], RATE, DT);
            if a.beat == 1.0 && was < 1.0 {
                beats += 1;
            }
            was = a.beat;
            end += hop;
        }
        assert!((6..=9).contains(&beats), "8 kicks should give about 8 beats, got {beats}");
    }

    #[test]
    fn beats_throw_sparks_that_burn_out() {
        let len = (RATE * 2.0) as usize;
        let signal: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f32 / RATE;
                if t % 0.5 < 0.08 { 0.8 * (2.0 * PI * 60.0 * t).sin() } else { 0.0 }
            })
            .collect();
        let hop = (RATE * DT) as usize;
        let mut a = Analyzer::new(BARS);
        let mut most = 0;
        let mut end = FFT_LEN;
        while end <= len {
            a.update(&signal[end - FFT_LEN..end], RATE, DT);
            most = most.max(a.sparks.len());
            assert!(a.sparks.iter().all(|s| (0.0..1.0).contains(&s.x) && s.life > 0.0));
            end += hop;
        }
        assert!(most >= 3, "kicks should throw sparks, saw at most {most}");
        for _ in 0..90 {
            a.update(&vec![0.0; FFT_LEN], RATE, DT);
        }
        assert!(a.sparks.is_empty(), "three seconds of silence should leave none");
    }
}
