//! Music, by remote control.
//!
//! There is no official YouTube Music API, and anything that extracts stream
//! URLs both violates the terms and side-steps the subscription you already
//! pay for. So Grimoire never plays audio. It drives a desktop client that is
//! already signed into your real account — your playlists, your Premium, your
//! playback. We just press the buttons.
//!
//! Playlists are the one place we read from YouTube directly. The client's API
//! can list your playlists (through search) but not what's in them, so the
//! songs come from YouTube Music's public web endpoint — the same listing the
//! website shows anyone with the link — and are queued in the client one by
//! one. Still metadata only: no audio ever leaves the app.
//!
//! The backend is th-ch/youtube-music's API Server plugin, chosen because it
//! ships a .dmg *and* an .AppImage, so the same setup works on macOS and
//! Linux. (YTMDesktop was the original target; its Homebrew cask was disabled
//! on 2026-09-01 for failing Apple's Gatekeeper check.)
//!
//! Note the macOS build is *ad-hoc signed, not notarised* — macOS quarantines
//! it and refuses to launch until the flag is cleared. `grimoire music-setup`
//! handles that; it is the step users get stuck on.
//!
//!   https://github.com/th-ch/youtube-music

use crate::library;
use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

mod local;
mod setup;
mod spotify;
mod ytm;

use local::Local;
pub use setup::{authenticate, setup_for};
use spotify::Spotify;
use ytm::Ytm;

pub const APP_ID: &str = "grimoiretui";
pub const DEFAULT_PORT: u16 = 26538;

/// Where the music comes from. The first two are remote controls for a player
/// you already run; the last two are your own server, which Grimoire can
/// actually play from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    YouTubeMusic,
    Spotify,
    Jellyfin,
    Plex,
}

impl Source {
    pub const ALL: [Source; 4] = [
        Source::YouTubeMusic,
        Source::Spotify,
        Source::Jellyfin,
        Source::Plex,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Source::YouTubeMusic => "youtube-music",
            Source::Spotify => "spotify",
            Source::Jellyfin => "jellyfin",
            Source::Plex => "plex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::YouTubeMusic => "YouTube Music",
            Source::Spotify => "Spotify",
            Source::Jellyfin => "Jellyfin",
            Source::Plex => "Plex",
        }
    }

    pub fn parse(s: &str) -> Option<Source> {
        Source::ALL
            .into_iter()
            .find(|x| x.slug() == s.trim().to_lowercase())
    }

    /// True when Grimoire plays the audio itself rather than driving another app.
    pub fn plays_audio(self) -> bool {
        matches!(self, Source::Jellyfin | Source::Plex)
    }
}

const POLL: Duration = Duration::from_millis(1500);
const HTTP_TIMEOUT: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone)]
pub struct Config {
    /// Off unless you turn it on. Music is an extra, not part of the desk, so
    /// a new install never polls a player or shows a music pane until asked.
    pub enabled: bool,
    pub source: Source,
    /// YouTube Music: where its API Server listens.
    pub host: String,
    pub port: u16,
    pub token: Option<String>,
    /// Jellyfin / Plex: your own server.
    pub server: String,
    pub api_key: String,
    pub user_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            source: Source::YouTubeMusic,
            host: "127.0.0.1".into(),
            port: DEFAULT_PORT,
            token: None,
            server: String::new(),
            api_key: String::new(),
            user_id: String::new(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        grimoire_core::paths::home()
            .join(".config")
            .join("grimoire")
            .join("music.toml")
    }

    /// Missing or unreadable config just means "music off".
    pub fn load() -> Config {
        match std::fs::read_to_string(Config::path()) {
            Ok(s) => Config::parse(&s),
            Err(_) => Config::default(),
        }
    }

    fn parse(s: &str) -> Config {
        let mut cfg = Config::default();
        for line in s.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "enabled" => cfg.enabled = v == "true",
                "source" => {
                    if let Some(x) = Source::parse(&v) {
                        cfg.source = x;
                    }
                }
                "token" if !v.is_empty() => cfg.token = Some(v),
                "host" if !v.is_empty() => cfg.host = v,
                "port" => {
                    if let Ok(p) = v.parse() {
                        cfg.port = p;
                    }
                }
                "server" => cfg.server = v,
                "api_key" => cfg.api_key = v,
                "user_id" => cfg.user_id = v,
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self) -> Result<()> {
        let path = Config::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let body = format!(
            "# Music is off until this says true. Turn it on or off from the menu (Esc › Settings).\n\
             enabled = {}\nsource = \"{}\"\nhost = \"{}\"\nport = {}\ntoken = \"{}\"\n\
             server = \"{}\"\napi_key = \"{}\"\nuser_id = \"{}\"\n",
            self.enabled,
            self.source.slug(),
            self.host,
            self.port,
            self.token.as_deref().unwrap_or(""),
            self.server,
            self.api_key,
            self.user_id,
        );
        grimoire_core::atomic::write_text(&path, &body)?;
        Ok(())
    }

    fn base(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub progress: f64,
    pub duration: f64,
    pub playing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// No token configured — the pane invites you to pair.
    NoToken,
    /// Configured, but the client isn't reachable.
    Offline,
    /// Connected, nothing loaded.
    Idle,
    Playing(Track),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    PlayPause,
    Next,
    Prev,
    /// Seconds forward, or back if negative.
    Seek(i32),
    Shuffle,
    Repeat,
    /// Nudge the volume by this many percent.
    Volume(i32),
    Like,
    /// Play the track at this position in the queue.
    JumpTo(usize),
    /// Queue a track by id straight after this one, and play it if `now`.
    Enqueue {
        id: String,
        now: bool,
    },
    /// Ask for the queue; answered by a fresh `Music::queue`.
    FetchQueue,
    /// Search; answered by `Music::results`.
    Search(String),
    /// Your library's playlists, or YouTube Music's for a query; answered by
    /// `Music::playlists`.
    Playlists(Option<String>),
    /// Replace the queue with a playlist and play it (`now`), or queue the
    /// whole thing straight after this track. Progress arrives as notes.
    Playlist {
        id: String,
        title: String,
        now: bool,
    },
}

/// A row in the queue or in search results.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub title: String,
    pub artist: String,
    /// "4:18", or empty when the source doesn't say.
    pub length: String,
    /// What to hand back to play or queue it (a YouTube video id).
    pub id: String,
    /// Position in the source's own queue, for jumping to it.
    pub pos: usize,
    /// The track playing now.
    pub current: bool,
    /// A music video rather than a song.
    pub video: bool,
}

/// What the poller thread sends back.
enum Update {
    State(State),
    Queue(Vec<Item>),
    Results(Vec<Item>),
    Playlists(Vec<Item>),
    Note(String),
    Modes(Modes),
}

/// Repeat, as the player has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    pub fn label(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
    }
}

/// The player's settings that a keypress changes but playback doesn't show:
/// without them on screen, pressing `r` or `+` looked like nothing happened.
/// `None` where the source can't say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Modes {
    pub repeat: Option<Repeat>,
    pub shuffle: Option<bool>,
    pub volume: Option<u8>,
    pub muted: bool,
    pub liked: Option<bool>,
}

impl Modes {
    /// A few glyphs for a narrow pane: ↻ repeat all, ↻1 repeat one, ⇄
    /// shuffle, ♥ liked, and the volume only when it's muted.
    pub fn badges(&self) -> String {
        let mut v: Vec<&str> = Vec::new();
        match self.repeat {
            Some(Repeat::All) => v.push("↻"),
            Some(Repeat::One) => v.push("↻1"),
            _ => {}
        }
        if self.shuffle == Some(true) {
            v.push("⇄");
        }
        if self.liked == Some(true) {
            v.push("♥");
        }
        let vol;
        if self.muted {
            v.push("muted");
        } else if let Some(n) = self.volume.filter(|&n| n < 100) {
            // Below full, the level is always in sight.
            vol = format!("{n}%");
            v.push(&vol);
        }
        v.join(" ")
    }

    /// Everything known, in words, for the player.
    pub fn line(&self) -> Vec<(String, bool)> {
        let mut v = Vec::new();
        if let Some(r) = self.repeat {
            v.push((format!("repeat {}", r.label()), r != Repeat::Off));
        }
        if let Some(s) = self.shuffle {
            v.push((format!("shuffle {}", if s { "on" } else { "off" }), s));
        }
        if let Some(vol) = self.volume {
            v.push((
                if self.muted {
                    "muted".to_string()
                } else {
                    format!("volume {vol}")
                },
                false,
            ));
        }
        if self.liked == Some(true) {
            v.push(("♥ liked".to_string(), true));
        }
        v
    }

    /// Whether the setting `c` changes differs between `self` and `now`.
    fn moved(&self, now: &Modes, c: &Cmd) -> bool {
        match c {
            Cmd::Repeat => self.repeat != now.repeat,
            Cmd::Shuffle => self.shuffle != now.shuffle,
            Cmd::Volume(_) => self.volume != now.volume || self.muted != now.muted,
            Cmd::Like => self.liked != now.liked,
            _ => true,
        }
    }

    /// What a command just did, said once it's known.
    fn said(&self, c: &Cmd) -> Option<String> {
        match c {
            Cmd::Repeat => self.repeat.map(|r| format!("repeat: {}", r.label())),
            Cmd::Shuffle => self
                .shuffle
                .map(|s| format!("shuffle {}", if s { "on" } else { "off" })),
            Cmd::Volume(d) => self.volume.map(|v| match (v, *d > 0) {
                (100.., true) => "volume 100 — as loud as it goes".to_string(),
                (0, false) => "volume 0".to_string(),
                _ => format!("volume {v}"),
            }),
            Cmd::Like => self
                .liked
                .map(|l| if l { "liked ♥" } else { "like taken back" }.to_string()),
            _ => None,
        }
    }
}

pub struct Music {
    /// False when music is switched off: no pane, no poller, no network.
    pub enabled: bool,
    pub state: State,
    pub source: Source,
    /// The last queue fetched, and the last search's results.
    pub queue: Vec<Item>,
    pub results: Vec<Item>,
    /// Your playlists, or the last playlist search's. `length` holds the
    /// song count and `id` the playlist id.
    pub playlists: Vec<Item>,
    /// A short line for the player: "searching…", or why something failed.
    pub note: Option<String>,
    /// A note arrived since the app last looked (see `take_fresh_note`).
    fresh_note: bool,
    /// Repeat, shuffle, volume, like — as last read from the player.
    pub modes: Modes,
    rx: Option<Receiver<Update>>,
    tx: Option<Sender<Cmd>>,
}

/// One music source. Whether it is a remote control or a real player is an
/// implementation detail above this line.
trait Backend: Send {
    fn state(&mut self) -> Result<State>;
    fn command(&mut self, c: Cmd) -> Result<()>;
    /// The play queue, for sources that can share it.
    fn queue(&mut self) -> Result<Vec<Item>> {
        Err(anyhow!("this source doesn't share its queue"))
    }
    fn search(&mut self, _query: &str) -> Result<Vec<Item>> {
        Err(anyhow!("search works with YouTube Music"))
    }
    /// Your own playlists (`None`), or everyone's that match a query.
    fn playlists(&mut self, _query: Option<&str>) -> Result<Vec<Item>> {
        Err(anyhow!("playlists work with YouTube Music"))
    }
    /// Repeat, shuffle, volume and like, for sources that can say.
    fn modes(&mut self) -> Result<Modes> {
        Ok(Modes::default())
    }
    /// Start queueing a playlist. That takes a while, so it runs in the
    /// background and reports its progress, and the final queue, on `tell`.
    fn load_playlist(
        &mut self,
        _id: String,
        _title: String,
        _now: bool,
        _tell: Sender<Update>,
    ) -> Result<()> {
        Err(anyhow!("playlists work with YouTube Music"))
    }
}

impl Music {
    /// Spawns the poller. Unconfigured sources stay inert and never touch the
    /// network or shell out.
    pub fn spawn(cfg: Config) -> Music {
        let source = cfg.source;
        // Switched off means inert: nothing is spawned, nothing is polled.
        let backend: Option<Box<dyn Backend>> = if !cfg.enabled {
            None
        } else {
            match source {
                Source::YouTubeMusic => cfg
                    .token
                    .clone()
                    .map(|t| Box::new(Ytm::new(&cfg, t)) as Box<dyn Backend>),
                Source::Spotify => Some(Box::new(Spotify) as Box<dyn Backend>),
                Source::Jellyfin => {
                    (!cfg.server.is_empty() && !cfg.api_key.is_empty()).then(|| {
                        Box::new(Local::new(Box::new(library::Jellyfin {
                            server: cfg.server.clone(),
                            token: cfg.api_key.clone(),
                            user_id: cfg.user_id.clone(),
                        }))) as Box<dyn Backend>
                    })
                }
                Source::Plex => (!cfg.server.is_empty() && !cfg.api_key.is_empty()).then(|| {
                    Box::new(Local::new(Box::new(library::Plex {
                        server: cfg.server.clone(),
                        token: cfg.api_key.clone(),
                    }))) as Box<dyn Backend>
                }),
            }
        };

        let Some(mut backend) = backend else {
            return Music {
                enabled: cfg.enabled,
                state: State::NoToken,
                source,
                queue: Vec::new(),
                results: Vec::new(),
                playlists: Vec::new(),
                note: None,
                fresh_note: false,
                modes: Modes::default(),
                rx: None,
                tx: None,
            };
        };

        let (up_tx, up_rx) = mpsc::channel::<Update>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

        thread::spawn(move || {
            let mut polls = 0u32;
            loop {
                let next = backend.state().unwrap_or(State::Offline);
                if up_tx.send(Update::State(next)).is_err() {
                    return;
                }
                // The settings change rarely; look every few polls, and
                // straight after a key that changes one (below).
                if polls.is_multiple_of(4)
                    && let Ok(m) = backend.modes()
                    && up_tx.send(Update::Modes(m)).is_err()
                {
                    return;
                }
                polls = polls.wrapping_add(1);
                // Wait out the poll interval, but act on a command the moment
                // it arrives, and re-poll straight after one that changes
                // playback so the pane catches up with the keypress.
                let deadline = Instant::now() + POLL;
                loop {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    let c = match cmd_rx.recv_timeout(wait) {
                        Ok(c) => c,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                    let repoll = !matches!(c, Cmd::FetchQueue | Cmd::Search(_) | Cmd::Playlists(_));
                    let reply = match c {
                        Cmd::FetchQueue => Some(backend.queue().map(Update::Queue)),
                        Cmd::Search(q) => Some(backend.search(&q).map(Update::Results)),
                        Cmd::Playlists(q) => {
                            Some(backend.playlists(q.as_deref()).map(Update::Playlists))
                        }
                        // Answers for itself, as it goes; only a refusal comes back here.
                        Cmd::Playlist { id, title, now } => backend
                            .load_playlist(id, title, now, up_tx.clone())
                            .err()
                            .map(Err),
                        // These reshape the queue, so send the new one along.
                        c @ (Cmd::JumpTo(_) | Cmd::Enqueue { .. }) => Some(
                            backend
                                .command(c)
                                .and_then(|_| backend.queue())
                                .map(Update::Queue),
                        ),
                        // Settings: do it, then read back what it became, and say so.
                        c @ (Cmd::Repeat | Cmd::Shuffle | Cmd::Volume(_) | Cmd::Like) => {
                            let before = backend.modes().ok();
                            match backend.command(c.clone()) {
                                Ok(()) => {
                                    // YouTube Music answers before it applies the
                                    // change: read until it shows (a volume already
                                    // at the top never will; that's fine).
                                    let mut after = None;
                                    for _ in 0..8 {
                                        thread::sleep(Duration::from_millis(150));
                                        if let Ok(m) = backend.modes() {
                                            let moved =
                                                before.as_ref().is_none_or(|b| b.moved(&m, &c));
                                            after = Some(m);
                                            if moved {
                                                break;
                                            }
                                        }
                                    }
                                    match after {
                                        Some(m) => {
                                            let note = m.said(&c);
                                            if up_tx.send(Update::Modes(m)).is_err() {
                                                return;
                                            }
                                            note.map(|n| Ok(Update::Note(n)))
                                        }
                                        None => None,
                                    }
                                }
                                Err(e) => Some(Err(e)),
                            }
                        }
                        // A refusal used to vanish here; now it's said.
                        c => backend.command(c).err().map(Err),
                    };
                    let sent = match reply {
                        Some(Ok(u)) => up_tx.send(u),
                        Some(Err(e)) => up_tx.send(Update::Note(e.to_string())),
                        None => Ok(()),
                    };
                    if sent.is_err() {
                        return;
                    }
                    if repoll {
                        break;
                    }
                }
            }
        });

        Music {
            enabled: true,
            state: State::Offline,
            source,
            queue: Vec::new(),
            results: Vec::new(),
            playlists: Vec::new(),
            note: None,
            fresh_note: false,
            modes: Modes::default(),
            rx: Some(up_rx),
            tx: Some(cmd_tx),
        }
    }

    /// The note the poller just sent, once: so a key pressed on the music
    /// pane (where the player's note line isn't showing) is answered in the
    /// status bar.
    pub fn take_fresh_note(&mut self) -> Option<String> {
        std::mem::take(&mut self.fresh_note)
            .then(|| self.note.clone())
            .flatten()
    }

    /// Drain whatever the poller has sent since the last frame.
    pub fn drain(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        while let Ok(u) = rx.try_recv() {
            match u {
                Update::State(s) => self.state = s,
                Update::Queue(q) => self.queue = q,
                Update::Results(r) => {
                    self.note = r.is_empty().then(|| "nothing playable found".to_string());
                    self.results = r;
                }
                Update::Playlists(p) => {
                    self.note = p.is_empty().then(|| "no playlists found".to_string());
                    self.playlists = p;
                }
                Update::Note(n) => {
                    self.note = Some(n);
                    self.fresh_note = true;
                }
                Update::Modes(m) => self.modes = m,
            }
        }
    }

    pub fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }
}

#[cfg(test)]
mod config_tests {

    #[test]
    fn the_pane_badges_say_only_what_is_on() {
        let mut m = Modes::default();
        assert_eq!(m.badges(), "");
        m.repeat = Some(Repeat::Off);
        m.shuffle = Some(false);
        assert_eq!(m.badges(), "", "off shows nothing");
        m.repeat = Some(Repeat::One);
        m.shuffle = Some(true);
        m.liked = Some(true);
        assert_eq!(m.badges(), "↻1 ⇄ ♥");
        m.volume = Some(100);
        assert_eq!(m.badges(), "↻1 ⇄ ♥", "full volume isn't shown");
        m.volume = Some(70);
        assert_eq!(m.badges(), "↻1 ⇄ ♥ 70%");
        m.repeat = Some(Repeat::All);
        m.muted = true;
        assert_eq!(m.badges(), "↻ ⇄ ♥ muted");
    }

    #[test]
    fn a_setting_is_said_as_it_now_is() {
        let m = Modes {
            repeat: Some(Repeat::One),
            shuffle: Some(false),
            volume: Some(100),
            muted: false,
            liked: Some(false),
        };
        assert_eq!(m.said(&Cmd::Repeat).as_deref(), Some("repeat: one"));
        assert_eq!(m.said(&Cmd::Shuffle).as_deref(), Some("shuffle off"));
        assert_eq!(
            m.said(&Cmd::Volume(10)).as_deref(),
            Some("volume 100 — as loud as it goes")
        );
        assert_eq!(m.said(&Cmd::Volume(-10)).as_deref(), Some("volume 100"));
        assert_eq!(m.said(&Cmd::Like).as_deref(), Some("like taken back"));
        assert_eq!(m.said(&Cmd::Next), None);
        let words: Vec<String> = m.line().into_iter().map(|(w, _)| w).collect();
        assert_eq!(words, ["repeat one", "shuffle off", "volume 100"]);
    }

    #[test]
    fn a_change_is_seen_only_in_the_setting_it_touches() {
        let before = Modes {
            repeat: Some(Repeat::All),
            ..Modes::default()
        };
        let after = Modes {
            repeat: Some(Repeat::One),
            ..Modes::default()
        };
        assert!(before.moved(&after, &Cmd::Repeat));
        assert!(!before.moved(&after, &Cmd::Shuffle));
        assert!(
            !before.moved(&before, &Cmd::Repeat),
            "unapplied yet: keep reading"
        );
    }

    use super::*;

    #[test]
    fn music_is_off_unless_the_file_turns_it_on() {
        assert!(!Config::default().enabled, "a new install has no music");
        // A music.toml written before the switch existed has no `enabled`
        // line, and stays off.
        let old = Config::parse("source = \"youtube-music\"\ntoken = \"abc\"\n");
        assert!(!old.enabled);
        assert_eq!(old.token.as_deref(), Some("abc"));
        assert!(Config::parse("enabled = true\n").enabled);
        assert!(!Config::parse("enabled = false\n").enabled);
    }
}
