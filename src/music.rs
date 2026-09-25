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

use crate::library::{self, Jukebox};
use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

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
        grimoire_core::paths::home().join(".config").join("grimoire").join("music.toml")
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
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
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
    Enqueue { id: String, now: bool },
    /// Ask for the queue; answered by a fresh `Music::queue`.
    FetchQueue,
    /// Search; answered by `Music::results`.
    Search(String),
    /// Your library's playlists, or YouTube Music's for a query; answered by
    /// `Music::playlists`.
    Playlists(Option<String>),
    /// Replace the queue with a playlist and play it (`now`), or queue the
    /// whole thing straight after this track. Progress arrives as notes.
    Playlist { id: String, title: String, now: bool },
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
    /// Start queueing a playlist. That takes a while, so it runs in the
    /// background and reports its progress, and the final queue, on `tell`.
    fn load_playlist(&mut self, _id: String, _title: String, _now: bool, _tell: Sender<Update>) -> Result<()> {
        Err(anyhow!("playlists work with YouTube Music"))
    }
}

impl Music {
    /// Spawns the poller. Unconfigured sources stay inert and never touch the
    /// network or shell out.
    pub fn spawn(cfg: Config) -> Music {
        let source = cfg.source;
        // Switched off means inert: nothing is spawned, nothing is polled.
        let backend: Option<Box<dyn Backend>> = if !cfg.enabled { None } else { match source {
            Source::YouTubeMusic => cfg
                .token
                .clone()
                .map(|t| Box::new(Ytm::new(&cfg, t)) as Box<dyn Backend>),
            Source::Spotify => Some(Box::new(Spotify) as Box<dyn Backend>),
            Source::Jellyfin => (!cfg.server.is_empty() && !cfg.api_key.is_empty()).then(|| {
                Box::new(Local::new(Box::new(library::Jellyfin {
                    server: cfg.server.clone(),
                    token: cfg.api_key.clone(),
                    user_id: cfg.user_id.clone(),
                }))) as Box<dyn Backend>
            }),
            Source::Plex => (!cfg.server.is_empty() && !cfg.api_key.is_empty()).then(|| {
                Box::new(Local::new(Box::new(library::Plex {
                    server: cfg.server.clone(),
                    token: cfg.api_key.clone(),
                }))) as Box<dyn Backend>
            }),
        } };

        let Some(mut backend) = backend else {
            return Music {
                enabled: cfg.enabled,
                state: State::NoToken,
                source,
                queue: Vec::new(),
                results: Vec::new(),
                playlists: Vec::new(),
                note: None,
                rx: None,
                tx: None,
            };
        };

        let (up_tx, up_rx) = mpsc::channel::<Update>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

        thread::spawn(move || {
            loop {
                let next = backend.state().unwrap_or(State::Offline);
                if up_tx.send(Update::State(next)).is_err() {
                    return;
                }
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
                        Cmd::Playlists(q) => Some(backend.playlists(q.as_deref()).map(Update::Playlists)),
                        // Answers for itself, as it goes; only a refusal comes back here.
                        Cmd::Playlist { id, title, now } => {
                            backend.load_playlist(id, title, now, up_tx.clone()).err().map(Err)
                        }
                        // These reshape the queue, so send the new one along.
                        c @ (Cmd::JumpTo(_) | Cmd::Enqueue { .. }) => Some(
                            backend
                                .command(c)
                                .and_then(|_| backend.queue())
                                .map(Update::Queue),
                        ),
                        c => {
                            let _ = backend.command(c);
                            None
                        }
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
            rx: Some(up_rx),
            tx: Some(cmd_tx),
        }
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
                Update::Note(n) => self.note = Some(n),
            }
        }
    }

    pub fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }
}

// ── YouTube Music: th-ch API Server ──────────────────────────────────

#[derive(Clone)]
struct Ytm {
    agent: ureq::Agent,
    /// Search goes out to YouTube from inside the app, and a long queue is
    /// megabytes, so these get longer.
    slow: ureq::Agent,
    /// YouTube Music's own web endpoint, for what's in a playlist.
    web: ureq::Agent,
    base: String,
    bearer: String,
    /// Bumped by every playlist load, so a newer one stops an older one.
    loading: Arc<AtomicU64>,
}

/// Large queues run to megabytes of renderer JSON.
const BODY_LIMIT: u64 = 64 << 20;

/// Search filters, as YouTube Music's own site sends them: playlists in your
/// library, and playlists anywhere.
const LIBRARY_PLAYLISTS: &str = "EgWKAQIoAWoKEAUQCRADEAoYBA%3D%3D";
const ALL_PLAYLISTS: &str = "Eg-KAQwIABAAGAAgACgBMABqChAEEAMQCRAFEAo%3D";
const BROWSE: &str = "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false";
/// The web client version to present. YouTube keeps old ones working for a
/// long time; bump it if playlists ever come back empty.
const WEB_CLIENT: &str = "1.20250910.01.00";
/// A playlist is queued one song at a time, so a very long one is cut here:
/// about half a minute of queueing, and still a day of music.
const PLAYLIST_MAX: usize = 300;
/// Between inserts. Go much faster and YouTube answers them further out of order.
const PLAYLIST_GAP: Duration = Duration::from_millis(100);
/// Where queued songs go. The client also takes INSERT_AT_END, but in 3.12.0
/// that silently adds nothing whenever the queue came from a playlist or a
/// radio (YouTube returns no items for it), which is nearly always.
const AFTER_CURRENT: &str = "INSERT_AFTER_CURRENT_VIDEO";

impl Ytm {
    fn new(cfg: &Config, token: String) -> Ytm {
        let agent = |t| ureq::Agent::config_builder().timeout_global(Some(t)).build().new_agent();
        Ytm {
            agent: agent(HTTP_TIMEOUT),
            slow: agent(Duration::from_secs(12)),
            web: agent(Duration::from_secs(20)),
            base: cfg.base(),
            bearer: format!("Bearer {token}"),
            loading: Arc::new(AtomicU64::new(0)),
        }
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.agent.delete(self.url(path)).header("Authorization", &self.bearer).call()?;
        Ok(())
    }

    /// Put a song straight after the one playing.
    fn queue_next(&self, id: &str) -> Result<()> {
        self.post("/queue", Some(serde_json::json!({ "videoId": id, "insertPosition": AFTER_CURRENT })))
    }

    fn fetch_queue(&self) -> Result<serde_json::Value> {
        let mut res = self.slow.get(self.url("/queue")).header("Authorization", &self.bearer).call()?;
        if res.status() == 204 {
            return Ok(serde_json::Value::Null);
        }
        Ytm::read(&mut res)
    }

    /// The queue as the client numbers it: each entry's video id (empty for
    /// anything that isn't a track), and which one is playing.
    fn raw_queue(&self) -> Result<(Vec<String>, Option<usize>)> {
        let v = self.fetch_queue()?;
        let mut ids = Vec::new();
        let mut cur = None;
        for (i, it) in v["items"].as_array().into_iter().flatten().enumerate() {
            let r = track_renderer(it);
            ids.push(r.and_then(|r| r["videoId"].as_str()).unwrap_or_default().to_string());
            if r.is_some_and(|r| r["selected"].as_bool() == Some(true)) {
                cur = Some(i);
            }
        }
        Ok((ids, cur))
    }

    /// The songs of a public or unlisted playlist, in order, up to
    /// `PLAYLIST_MAX`, and whether it runs on past that. A private playlist
    /// comes back empty: YouTube only lists those to the signed-in app.
    fn tracks(&self, playlist: &str) -> Result<(Vec<String>, bool)> {
        use serde_json::json;
        let context = json!({ "client": { "clientName": "WEB_REMIX", "clientVersion": WEB_CLIENT, "hl": "en" } });
        let mut body = json!({ "context": context, "browseId": format!("VL{playlist}") });
        let mut ids = Vec::new();
        // A page is 100 songs; the bound only guards against a looping token.
        for _ in 0..PLAYLIST_MAX / 50 {
            let v = Ytm::read(&mut self.web.post(BROWSE).send_json(&body)?)?;
            collect_tracks(&v, &mut ids);
            let next = continuation(&v);
            if next.is_none() || ids.len() >= PLAYLIST_MAX {
                let more = next.is_some() || ids.len() > PLAYLIST_MAX;
                ids.truncate(PLAYLIST_MAX);
                return Ok((ids, more));
            }
            body = json!({ "context": context, "continuation": next });
        }
        ids.truncate(PLAYLIST_MAX);
        Ok((ids, true))
    }

    /// Queue a playlist in the client, reporting progress through `note`.
    /// `now` replaces the queue and plays it; otherwise it goes after the
    /// playing song. Returns the finished queue, or None if a newer load took
    /// over part way.
    ///
    /// The client can only queue one song at a time, straight after the one
    /// playing, and YouTube answers each insert on its own schedule. So: the
    /// first song goes in and plays at once, the rest go in backwards (each
    /// landing in front of the last), and a final pass moves any that landed
    /// out of turn.
    fn play_list(&self, id: &str, title: &str, now: bool, ticket: u64, note: &dyn Fn(String)) -> Result<Option<Vec<Item>>> {
        use serde_json::json;
        let current = || self.loading.load(Ordering::SeqCst) == ticket;
        let (ids, more) = self.tracks(id)?;
        if ids.is_empty() {
            return Err(anyhow!("“{title}” is private. Make it unlisted in YouTube Music to play it here"));
        }
        if !current() {
            return Ok(None);
        }
        let n = ids.len();

        // Where the run starts, and how long the queue must grow before every
        // song is known to have landed. Counting matters: the same song can
        // also be in the queue already (or arrive by radio), so looking for
        // each id would call a straggler home before it is.
        let (start, before) = if now {
            // With the queue cleared and nothing selected, each song lands at
            // the end: forwards.
            self.delete("/queue")?;
            thread::sleep(Duration::from_millis(300));
            for (k, vid) in ids.iter().enumerate() {
                if !current() {
                    return Ok(None);
                }
                self.queue_next(vid)?;
                progress(note, title, k, n);
                thread::sleep(PLAYLIST_GAP);
            }
            (0, 0)
        } else {
            // Each song lands straight after the playing one: backwards.
            let (queue, cur) = self.raw_queue()?;
            for (k, vid) in ids.iter().rev().enumerate() {
                if !current() {
                    return Ok(None);
                }
                self.queue_next(vid)?;
                progress(note, title, k, n);
                thread::sleep(PLAYLIST_GAP);
            }
            (cur.map_or(0, |c| c + 1), queue.len())
        };

        // Wait for the stragglers, then put the order right: YouTube answers
        // the inserts out of step, so a few land out of turn.
        let mut queue = self.raw_queue()?.0;
        for _ in 0..16 {
            if queue.len() >= before + n {
                break;
            }
            thread::sleep(Duration::from_millis(600));
            queue = self.raw_queue()?.0;
        }
        if !current() {
            return Ok(None);
        }
        let (moves, _) = reorder(&queue, start, &ids);
        for (from, to) in moves {
            self.patch(&format!("/queue/{from}"), json!({ "toIndex": to }))?;
        }

        if now {
            // Only now start it. Starting the first song earlier, with the
            // queue nearly empty, makes the client refill it by radio from
            // whatever played before, and that radio lands in the middle.
            self.patch("/queue", json!({ "index": 0 }))?;
            // It may still append that radio after the last song. A fresh
            // playlist shouldn't end in someone else's.
            thread::sleep(Duration::from_millis(1500));
            let after = self.raw_queue()?.0.len();
            for i in (n..after).rev() {
                self.delete(&format!("/queue/{i}"))?;
            }
        }

        // The queue first, so the list is right by the time the note says so.
        let items = self.items()?;
        let cut = if more { format!(", the first {PLAYLIST_MAX}") } else { String::new() };
        note(if now {
            format!("playing {title} · {n} songs{cut}")
        } else {
            format!("{title} is up next · {n} songs{cut}")
        });
        Ok(Some(items))
    }

    fn items(&self) -> Result<Vec<Item>> {
        Ok(parse_queue(&self.fetch_queue()?))
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    fn get(&self, path: &str) -> Result<ureq::http::Response<ureq::Body>> {
        Ok(self.agent.get(self.url(path)).header("Authorization", &self.bearer).call()?)
    }

    fn post(&self, path: &str, body: Option<serde_json::Value>) -> Result<()> {
        let req = self.agent.post(self.url(path)).header("Authorization", &self.bearer);
        match body {
            Some(b) => req.send_json(b)?,
            None => req.send_empty()?,
        };
        Ok(())
    }

    fn patch(&self, path: &str, body: serde_json::Value) -> Result<()> {
        self.agent
            .patch(self.url(path))
            .header("Authorization", &self.bearer)
            .send_json(body)?;
        Ok(())
    }

    fn read(res: &mut ureq::http::Response<ureq::Body>) -> Result<serde_json::Value> {
        let s = res.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?;
        Ok(serde_json::from_str(&s)?)
    }
}

impl Backend for Ytm {
    fn state(&mut self) -> Result<State> {
        let mut res = self
            .agent
            .get(format!("{}/api/v1/song", self.base))
            .header("Authorization", &self.bearer)
            .call()?;

        // Nothing loaded: the plugin answers 204, or a body with no title.
        if res.status() == 204 {
            return Ok(State::Idle);
        }
        let v: serde_json::Value = res.body_mut().read_json()?;
        let title = v["title"].as_str().unwrap_or("").to_string();
        if title.is_empty() {
            return Ok(State::Idle);
        }
        Ok(State::Playing(Track {
            title,
            artist: v["artist"].as_str().unwrap_or("").to_string(),
            progress: v["elapsedSeconds"].as_f64().unwrap_or(0.0),
            duration: v["songDuration"].as_f64().unwrap_or(0.0),
            playing: !v["isPaused"].as_bool().unwrap_or(false),
        }))
    }

    fn command(&mut self, c: Cmd) -> Result<()> {
        use serde_json::json;
        match c {
            Cmd::PlayPause => self.post("/toggle-play", None),
            Cmd::Next => self.post("/next", None),
            Cmd::Prev => self.post("/previous", None),
            Cmd::Seek(s) if s >= 0 => self.post("/go-forward", Some(json!({ "seconds": s }))),
            Cmd::Seek(s) => self.post("/go-back", Some(json!({ "seconds": -s }))),
            Cmd::Shuffle => self.post("/shuffle", None),
            Cmd::Repeat => self.post("/switch-repeat", Some(json!({ "iteration": 1 }))),
            Cmd::Volume(d) => {
                // The client reports volume on a curved scale but takes it on
                // a straight one (pear-desktop #4458), so convert before nudging.
                let heard = Ytm::read(&mut self.get("/volume")?)?["state"].as_f64().unwrap_or(50.0);
                self.post("/volume", Some(json!({ "volume": (volume_to_set(heard) + d).clamp(0, 100) })))
            }
            Cmd::Like => self.post("/like", None),
            Cmd::JumpTo(i) => self.patch("/queue", json!({ "index": i })),
            Cmd::Enqueue { id, now } => {
                self.queue_next(&id)?;
                if now {
                    // The app adds it asynchronously. Wait until it lands just
                    // after the playing track, then jump to it.
                    for _ in 0..12 {
                        thread::sleep(Duration::from_millis(250));
                        let q = self.items()?;
                        let Some(cur) = q.iter().position(|it| it.current) else {
                            continue;
                        };
                        if let Some(next) = q.get(cur + 1).filter(|it| it.id == id) {
                            return self.patch("/queue", json!({ "index": next.pos }));
                        }
                    }
                }
                Ok(())
            }
            Cmd::FetchQueue | Cmd::Search(_) | Cmd::Playlists(_) | Cmd::Playlist { .. } => Ok(()),
        }
    }

    fn queue(&mut self) -> Result<Vec<Item>> {
        self.items()
    }

    fn search(&mut self, query: &str) -> Result<Vec<Item>> {
        let mut res = self
            .slow
            .post(self.url("/search"))
            .header("Authorization", &self.bearer)
            .send_json(serde_json::json!({ "query": query }))?;
        Ok(parse_search(&Ytm::read(&mut res)?))
    }

    fn playlists(&mut self, query: Option<&str>) -> Result<Vec<Item>> {
        // A lone space lists the whole library; YouTube won't take an empty query.
        let (query, params) = match query {
            Some(q) => (q, ALL_PLAYLISTS),
            None => (" ", LIBRARY_PLAYLISTS),
        };
        let mut res = self
            .slow
            .post(self.url("/search"))
            .header("Authorization", &self.bearer)
            .send_json(serde_json::json!({ "query": query, "params": params }))?;
        Ok(parse_playlists(&Ytm::read(&mut res)?))
    }

    fn load_playlist(&mut self, id: String, title: String, now: bool, tell: Sender<Update>) -> Result<()> {
        let ticket = self.loading.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        thread::spawn(move || {
            let note = |s: String| {
                let _ = tell.send(Update::Note(s));
            };
            match me.play_list(&id, &title, now, ticket, &note) {
                Ok(Some(q)) => {
                    let _ = tell.send(Update::Queue(q));
                }
                Ok(None) => {}
                Err(e) => note(e.to_string()),
            }
        });
        Ok(())
    }
}

/// Text of a YouTube `{ "runs": [{ "text": … }] }` block.
fn runs(v: &serde_json::Value) -> String {
    v["runs"]
        .as_array()
        .map(|a| a.iter().filter_map(|r| r["text"].as_str()).collect())
        .unwrap_or_default()
}

/// The first `videoId` anywhere under `v`.
fn first_video_id(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => m
            .get("videoId")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or_else(|| m.values().find_map(first_video_id)),
        serde_json::Value::Array(a) => a.iter().find_map(first_video_id),
        _ => None,
    }
}

/// The track in a queue entry, which comes plain or wrapped.
fn track_renderer(it: &serde_json::Value) -> Option<&serde_json::Value> {
    it.get("playlistPanelVideoRenderer")
        .or_else(|| it.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer"))
}

/// Playlists from a playlist search, in YouTube's order. `artist` is whose it
/// is and `length` the song count (or the views, on someone else's).
fn parse_playlists(v: &serde_json::Value) -> Vec<Item> {
    let none = Vec::new();
    let sections = v
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(|s| s.as_array())
        .unwrap_or(&none);
    let mut out: Vec<Item> = Vec::new();
    for s in sections {
        let rows = s
            .pointer("/musicShelfRenderer/contents")
            .or_else(|| s.pointer("/itemSectionRenderer/contents"));
        for row in rows.and_then(|r| r.as_array()).into_iter().flatten() {
            let Some(r) = row.get("musicResponsiveListItemRenderer") else {
                continue;
            };
            let id = r
                .pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchPlaylistEndpoint/playlistId")
                .and_then(|x| x.as_str())
                .or_else(|| {
                    r.pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(|x| x.as_str())
                        .and_then(|b| b.strip_prefix("VL"))
                });
            let Some(id) = id else {
                continue;
            };
            let cols: Vec<String> = r["flexColumns"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"]))
                .collect();
            // "Espershire • 84 songs", or "Playlist • Someone • 1.2M views".
            let parts: Vec<&str> = cols
                .get(1)
                .map_or("", String::as_str)
                .split('•')
                .map(str::trim)
                .filter(|p| !p.is_empty() && *p != "Playlist")
                .collect();
            out.push(Item {
                title: cols.first().cloned().unwrap_or_default(),
                artist: parts.first().copied().unwrap_or_default().to_string(),
                length: if parts.len() > 1 { parts[parts.len() - 1].to_string() } else { String::new() },
                id: id.to_string(),
                pos: out.len(),
                current: false,
                video: false,
            });
        }
    }
    out
}

/// The songs on a playlist page, in order. Unavailable ones carry no
/// `playlistItemData` and are skipped; repeats are kept.
fn collect_tracks(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(r) = m.get("musicResponsiveListItemRenderer") {
                if let Some(id) = r.pointer("/playlistItemData/videoId").and_then(|x| x.as_str()) {
                    out.push(id.to_string());
                }
                return;
            }
            m.values().for_each(|x| collect_tracks(x, out));
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_tracks(x, out)),
        _ => {}
    }
}

/// The token for a playlist's next page, if it has one.
fn continuation(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => m
            .get("continuationCommand")
            .and_then(|c| c["token"].as_str())
            .map(str::to_string)
            .or_else(|| m.values().find_map(continuation)),
        serde_json::Value::Array(a) => a.iter().find_map(continuation),
        _ => None,
    }
}

/// Every 25 songs, say how far a playlist has got.
fn progress(note: &dyn Fn(String), title: &str, k: usize, n: usize) {
    if k % 25 == 24 {
        note(format!("{title} · queued {} of {n}", k + 1));
    }
}

/// The moves that put `want` in order from position `start` of `queue`
/// (video ids, numbered as the client numbers them), and where that run
/// ends. Each move is (from, to) and counts on the ones before it having been
/// made. A song that never arrived is skipped rather than left as a gap.
fn reorder(queue: &[String], start: usize, want: &[String]) -> (Vec<(usize, usize)>, usize) {
    let mut q = queue.to_vec();
    let mut moves = Vec::new();
    let mut at = start;
    for id in want {
        let Some(j) = q.iter().skip(at).position(|x| x == id).map(|p| p + at) else {
            continue;
        };
        if j != at {
            let x = q.remove(j);
            q.insert(at, x);
            moves.push((j, at));
        }
        at += 1;
    }
    (moves, at.min(q.len()))
}

/// GET /volume answers on a curved scale and POST takes a straight one. This
/// maps the first onto the second: the conversion worked out in
/// pear-desktop #4458, exact at 0 and 100 and within a step between.
fn volume_to_set(heard: f64) -> i32 {
    (100.0 * (1.0 + 0.15 * heard.clamp(0.0, 100.0)).ln() / 16f64.ln()).round() as i32
}

/// The app's queue, as it sends it: renderer objects, some wrapped. Positions
/// are kept as the app numbers them, since that's what jumping takes.
fn parse_queue(v: &serde_json::Value) -> Vec<Item> {
    let Some(items) = v["items"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(pos, it)| {
            let r = track_renderer(it)?;
            Some(Item {
                title: runs(&r["title"]),
                artist: runs(&r["shortBylineText"]),
                length: runs(&r["lengthText"]),
                id: r["videoId"].as_str().unwrap_or_default().to_string(),
                pos,
                current: r["selected"].as_bool().unwrap_or(false),
                video: false,
            })
        })
        .collect()
}

/// Songs and videos from a search, top result first. Albums, playlists,
/// artists and podcasts are left out: the API can only queue single tracks.
fn parse_search(v: &serde_json::Value) -> Vec<Item> {
    let none = Vec::new();
    let sections = v
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(|s| s.as_array())
        .unwrap_or(&none);
    let mut out: Vec<Item> = Vec::new();
    for s in sections {
        if let Some(card) = s.get("musicCardShelfRenderer") {
            let top = playable(runs(&card["title"]), &runs(&card["subtitle"]));
            if let (Some(item), Some(id)) = (top, first_video_id(&card["buttons"])) {
                out.push(Item { id, ..item });
            }
            out.extend(card["contents"].as_array().into_iter().flatten().filter_map(list_item));
        }
        let rows = s
            .pointer("/itemSectionRenderer/contents")
            .or_else(|| s.pointer("/musicShelfRenderer/contents"));
        out.extend(rows.and_then(|r| r.as_array()).into_iter().flatten().filter_map(list_item));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|it| seen.insert(it.id.clone()));
    for (i, it) in out.iter_mut().enumerate() {
        it.pos = i;
    }
    out
}

fn list_item(it: &serde_json::Value) -> Option<Item> {
    let r = it.get("musicResponsiveListItemRenderer")?;
    let id = r.pointer("/playlistItemData/videoId")?.as_str()?.to_string();
    let cols: Vec<String> = r["flexColumns"]
        .as_array()?
        .iter()
        .map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"]))
        .collect();
    let item = playable(cols.first()?.clone(), cols.get(1).map_or("", String::as_str))?;
    Some(Item { id, ..item })
}

/// A title and a byline like "Song • Fleetwood Mac • 4:18", if it names a
/// song or a video. Untyped rows (on the top-result card) are videos.
fn playable(title: String, byline: &str) -> Option<Item> {
    let parts: Vec<&str> = byline.split('•').map(str::trim).filter(|p| !p.is_empty()).collect();
    let kind = *parts.first()?;
    let (video, artist, rest) = match kind {
        "Song" => (false, parts.get(1).copied(), 2),
        "Video" => (true, parts.get(1).copied(), 2),
        "Episode" | "Podcast" | "Album" | "Single" | "EP" | "Playlist" | "Artist" | "Profile" => {
            return None;
        }
        _ => (true, Some(kind), 1),
    };
    let is_length = |p: &&&str| {
        p.contains(':') && p.split(':').all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    Some(Item {
        title,
        artist: artist.unwrap_or_default().to_string(),
        length: parts.iter().skip(rest).find(is_length).map_or(String::new(), |s| s.to_string()),
        id: String::new(),
        pos: 0,
        current: false,
        video,
    })
}

#[cfg(test)]
mod ytm_tests {
    use super::*;
    use serde_json::json;

    fn row(title: &str, byline: &str, id: Option<&str>) -> serde_json::Value {
        let col = |t: &str| json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": t }] } } });
        let mut r = json!({ "flexColumns": [col(title), col(byline)] });
        if let Some(id) = id {
            r["playlistItemData"] = json!({ "videoId": id });
        }
        json!({ "musicResponsiveListItemRenderer": r })
    }

    #[test]
    fn search_keeps_songs_and_videos_and_drops_the_rest() {
        let section = |r| json!({ "itemSectionRenderer": { "contents": [r] } });
        let card = json!({ "musicCardShelfRenderer": {
            "title": { "runs": [{ "text": "Dreams (2004 Remaster)" }] },
            "subtitle": { "runs": [{ "text": "Song" }, { "text": " • " }, { "text": "Fleetwood Mac" }, { "text": " • " }, { "text": "4:18" }] },
            "buttons": [{ "buttonRenderer": { "command": { "watchEndpoint": { "videoId": "swJOIjjW69U" } } } }],
            "contents": [{ "messageRenderer": {} }, row("Dreams", "FLEETWOOD MAC • 92M views • 4:24", Some("Y3ywicffOj4"))]
        }});
        let v = json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": { "contents": [
            card,
            section(row("Dreams", "Song • Fleetwood Mac", Some("m8i5WiWCN-c"))),
            section(row("Rumours", "Album • Fleetwood Mac • 1977", None)),
            section(row("A talk about Dreams", "Episode • Jul 1, 2024", Some("BcuU763NTo4"))),
            section(row("Dreams", "Song • Fleetwood Mac", Some("m8i5WiWCN-c"))),
        ]}}}}]}}});

        let r = parse_search(&v);
        let ids: Vec<&str> = r.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["swJOIjjW69U", "Y3ywicffOj4", "m8i5WiWCN-c"], "top result first; no album, episode or repeat");
        assert_eq!((r[0].artist.as_str(), r[0].length.as_str()), ("Fleetwood Mac", "4:18"));
        assert!(!r[0].video && r[1].video, "the untyped card row is a video");
        assert_eq!((r[1].artist.as_str(), r[1].length.as_str()), ("FLEETWOOD MAC", "4:24"));
        assert_eq!(r[2].pos, 2);
    }

    #[test]
    fn queue_reads_both_renderer_shapes_and_marks_the_playing_track() {
        let track = |t: &str, sel: bool| json!({
            "title": { "runs": [{ "text": t }] },
            "shortBylineText": { "runs": [{ "text": "Fleetwood Mac" }] },
            "lengthText": { "runs": [{ "text": "3:44" }] },
            "videoId": format!("id-{t}"),
            "selected": sel,
        });
        let v = json!({ "items": [
            { "playlistPanelVideoRenderer": track("Dreams", false) },
            { "playlistPanelVideoWrapperRenderer": { "primaryRenderer": { "playlistPanelVideoRenderer": track("Go Your Own Way", true) } } },
            { "automixPreviewVideoRenderer": {} },
            { "playlistPanelVideoRenderer": track("Landslide", false) },
        ]});
        let q = parse_queue(&v);
        assert_eq!(q.len(), 3);
        assert_eq!(q[1].title, "Go Your Own Way");
        assert!(q[1].current && !q[0].current);
        assert_eq!(q[2].pos, 3, "positions stay the app's own, for jumping");
    }

    #[test]
    fn playlists_read_from_library_and_search_rows() {
        let col = |runs: serde_json::Value| json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } });
        let mine = json!({ "musicResponsiveListItemRenderer": {
            "flexColumns": [
                col(json!([{ "text": "Best of Fleetwood" }])),
                col(json!([{ "text": "Espershire" }, { "text": " • " }, { "text": "84 songs" }])),
            ],
            "overlay": { "musicItemThumbnailOverlayRenderer": { "content": { "musicPlayButtonRenderer": {
                "playNavigationEndpoint": { "watchPlaylistEndpoint": { "playlistId": "PLnlwnADdLfE7MRO1KS4tU4WyqviJ-wZLI" } }
            }}}},
        }});
        let theirs = json!({ "musicResponsiveListItemRenderer": {
            "flexColumns": [
                col(json!([{ "text": "Fleetwood Mac Radio" }])),
                col(json!([{ "text": "Playlist • Pramit Mohanty • 103K views" }])),
            ],
            "navigationEndpoint": { "browseEndpoint": { "browseId": "VLPLJyx7idLrmwMu4DOoL1ybLgJr3YWnpiGm" } },
        }});
        let unplayable = json!({ "musicResponsiveListItemRenderer": { "flexColumns": [col(json!([{ "text": "?" }]))] } });
        let v = json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": { "contents": [
            { "musicShelfRenderer": { "contents": [mine, unplayable, theirs] } },
        ]}}}}]}}});

        let p = parse_playlists(&v);
        assert_eq!(p.len(), 2, "a row with no playlist id is skipped");
        assert_eq!(
            (p[0].title.as_str(), p[0].artist.as_str(), p[0].length.as_str(), p[0].id.as_str()),
            ("Best of Fleetwood", "Espershire", "84 songs", "PLnlwnADdLfE7MRO1KS4tU4WyqviJ-wZLI")
        );
        assert_eq!(
            (p[1].artist.as_str(), p[1].length.as_str(), p[1].id.as_str()),
            ("Pramit Mohanty", "103K views", "PLJyx7idLrmwMu4DOoL1ybLgJr3YWnpiGm"),
            "the id falls back to the browse id, less its VL"
        );
        assert_eq!(p[1].pos, 1);
    }

    #[test]
    fn playlist_pages_give_songs_in_order_and_the_next_token() {
        let row = |id: Option<&str>| {
            let mut r = json!({ "flexColumns": [] });
            if let Some(id) = id {
                r["playlistItemData"] = json!({ "videoId": id });
            }
            json!({ "musicResponsiveListItemRenderer": r })
        };
        let page = json!({ "contents": { "twoColumnBrowseResultsRenderer": { "secondaryContents": { "sectionListRenderer": { "contents": [
            { "musicPlaylistShelfRenderer": { "contents": [
                row(Some("a")), row(None), row(Some("b")), row(Some("a")),
                { "continuationItemRenderer": { "continuationEndpoint": { "continuationCommand": { "token": "page-2" } } } },
            ]}},
        ]}}}}});
        let mut ids = Vec::new();
        collect_tracks(&page, &mut ids);
        assert_eq!(ids, ["a", "b", "a"], "unavailable songs skipped, repeats kept");
        assert_eq!(continuation(&page).as_deref(), Some("page-2"));
        assert_eq!(continuation(&json!({ "contents": {} })), None);
    }

    #[test]
    fn reorder_straightens_a_shuffled_run_and_finds_its_end() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // Playing "p". a–e landed out of turn, "r" is the radio refill, and
        // "x" never arrived at all.
        let queue = s(&["p", "c", "a", "b", "e", "d", "r", "r"]);
        let want = s(&["a", "b", "x", "c", "d", "e"]);
        let (moves, end) = reorder(&queue, 1, &want);
        let mut q = queue.clone();
        for (from, to) in &moves {
            let x = q.remove(*from);
            q.insert(*to, x);
        }
        assert_eq!(q, s(&["p", "a", "b", "c", "d", "e", "r", "r"]));
        assert_eq!(end, 6, "the refill starts where the playlist ends");
        assert!(reorder(&q, 1, &want).0.is_empty(), "an ordered queue needs no moves");
    }

    #[test]
    fn volume_converts_from_the_scale_it_reports() {
        assert_eq!(volume_to_set(0.0), 0);
        assert_eq!(volume_to_set(100.0), 100);
        assert_eq!(volume_to_set(15.0), 43, "#4458: setting 43 reads back as 15");
    }
}

// ── Spotify: drive the desktop app, no account setup at all ──────────
//
// Spotify's Web API would mean OAuth, a registered application, a redirect
// URL and a refresh-token dance — for the privilege of pressing pause. Both
// platforms already expose the running player locally: AppleScript on macOS,
// MPRIS over D-Bus on Linux. Nothing to sign in to, nothing to store.

struct Spotify;

impl Spotify {
    #[cfg(target_os = "macos")]
    fn read(&self) -> Result<State> {
        // One osascript call for the whole state — five would mean five
        // process spawns every poll.
        let script = r#"
            if application "Spotify" is running then
              tell application "Spotify"
                try
                  return (player state as string) & "|" & (name of current track) & "|" & ¬
                         (artist of current track) & "|" & ((duration of current track) / 1000) & "|" & (player position)
                on error
                  return "stopped|||0|0"
                end try
              end tell
            else
              return "notrunning"
            end if"#;
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()?;
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if raw == "notrunning" || raw.is_empty() {
            return Ok(State::Offline);
        }
        let f: Vec<&str> = raw.split('|').collect();
        if f.len() < 5 || f[1].is_empty() {
            return Ok(State::Idle);
        }
        Ok(State::Playing(Track {
            title: f[1].to_string(),
            artist: f[2].to_string(),
            duration: f[3].parse().unwrap_or(0.0),
            progress: f[4].parse().unwrap_or(0.0),
            playing: f[0] == "playing",
        }))
    }

    #[cfg(not(target_os = "macos"))]
    fn read(&self) -> Result<State> {
        // playerctl speaks MPRIS, which every Linux Spotify build exposes.
        let out = std::process::Command::new("playerctl")
            .args([
                "-p",
                "spotify",
                "metadata",
                "--format",
                "{{status}}|{{title}}|{{artist}}|{{mpris:length}}|{{position}}",
            ])
            .output()?;
        if !out.status.success() {
            return Ok(State::Offline);
        }
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let f: Vec<&str> = raw.split('|').collect();
        if f.len() < 5 || f[1].is_empty() {
            return Ok(State::Idle);
        }
        // MPRIS reports both of these in microseconds.
        let us = |s: &str| s.parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
        Ok(State::Playing(Track {
            title: f[1].to_string(),
            artist: f[2].to_string(),
            duration: us(f[3]),
            progress: us(f[4]),
            playing: f[0].eq_ignore_ascii_case("playing"),
        }))
    }

    #[cfg(target_os = "macos")]
    fn press(&self, c: Cmd) -> Result<()> {
        let verb = match c {
            Cmd::PlayPause => "playpause".to_string(),
            Cmd::Next => "next track".to_string(),
            Cmd::Prev => "previous track".to_string(),
            Cmd::Seek(s) => format!("set player position to (player position + {s})"),
            Cmd::Shuffle => "set shuffling to not shuffling".to_string(),
            Cmd::Repeat => "set repeating to not repeating".to_string(),
            Cmd::Volume(d) => format!("set sound volume to (sound volume + {d})"),
            _ => return Ok(()),
        };
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "if application \"Spotify\" is running then tell application \"Spotify\" to {verb}"
            ))
            .output()?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn press(&self, c: Cmd) -> Result<()> {
        let sign = |n: i32| if n >= 0 { "+" } else { "-" };
        let args: Vec<String> = match c {
            Cmd::PlayPause => vec!["play-pause".into()],
            Cmd::Next => vec!["next".into()],
            Cmd::Prev => vec!["previous".into()],
            Cmd::Seek(s) => vec!["position".into(), format!("{}{}", s.abs(), sign(s))],
            Cmd::Shuffle => vec!["shuffle".into(), "Toggle".into()],
            Cmd::Volume(d) => vec!["volume".into(), format!("{:.2}{}", d.abs() as f64 / 100.0, sign(d))],
            _ => return Ok(()),
        };
        let _ = std::process::Command::new("playerctl")
            .args(["-p", "spotify"])
            .args(&args)
            .output()?;
        Ok(())
    }
}

impl Backend for Spotify {
    fn state(&mut self) -> Result<State> {
        self.read()
    }
    fn command(&mut self, c: Cmd) -> Result<()> {
        self.press(c)
    }
}

// ── Jellyfin / Plex: your own server, played here ────────────────────

struct Local {
    player: Jukebox,
    /// A dead server shouldn't be hammered every 1.5s — but it also shouldn't
    /// stay dead forever once it comes back, so back off and retry.
    failed: Option<String>,
    retry_at: Option<std::time::Instant>,
}

const RETRY_AFTER: Duration = Duration::from_secs(30);

impl Local {
    fn new(catalog: Box<dyn library::Catalog>) -> Local {
        Local {
            player: Jukebox::new(catalog),
            failed: None,
            retry_at: None,
        }
    }
}

impl Backend for Local {
    fn state(&mut self) -> Result<State> {
        if let Some(why) = self.failed.clone() {
            match self.retry_at {
                Some(at) if std::time::Instant::now() >= at => {
                    // Time to try again. Clear the latch and fall through.
                    self.failed = None;
                    self.retry_at = None;
                }
                _ => return Err(anyhow!("{why}")),
            }
        }
        if self.player.queue_len() == 0 {
            if let Err(e) = self.player.ensure_queue() {
                self.failed = Some(e.to_string());
                self.retry_at = Some(std::time::Instant::now() + RETRY_AFTER);
                return Err(e);
            }
        }
        self.player.advance_if_finished();

        let Some(track) = self.player.current().cloned() else {
            return Ok(State::Idle);
        };
        if !self.player.started() {
            return Ok(State::Idle);
        }
        let (pos, playing, _) = self.player.progress();
        Ok(State::Playing(Track {
            title: track.title,
            artist: track.artist,
            progress: pos,
            duration: track.duration,
            playing,
        }))
    }

    fn command(&mut self, c: Cmd) -> Result<()> {
        match c {
            Cmd::PlayPause => self.player.toggle(),
            Cmd::Next => self.player.step(1),
            Cmd::Prev => self.player.step(-1),
            Cmd::JumpTo(i) => self.player.play_at(i),
            _ => Ok(()),
        }
    }

    fn queue(&mut self) -> Result<Vec<Item>> {
        let at = self.player.index();
        let started = self.player.started();
        Ok(self
            .player
            .tracks()
            .iter()
            .enumerate()
            .map(|(pos, t)| Item {
                title: t.title.clone(),
                artist: t.artist.clone(),
                length: if t.duration > 0.0 {
                    format!("{}:{:02}", t.duration as u64 / 60, t.duration as u64 % 60)
                } else {
                    String::new()
                },
                id: String::new(),
                pos,
                current: started && pos == at,
                video: false,
            })
            .collect())
    }
}

/// Pair with the desktop client. With the default AUTH_AT_FIRST strategy the
/// app shows a prompt you accept; the token it returns is stored locally.
pub fn authenticate(host: &str, port: u16) -> Result<()> {
    let base = format!("http://{host}:{port}");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .new_agent();

    println!("Contacting YouTube Music at {base} ...");
    println!("Accept the pairing prompt in the app if one appears.");

    let mut res = agent
        .post(format!("{base}/auth/{APP_ID}"))
        .send_empty()
        .map_err(|e| {
            anyhow!(
                "could not reach the API server ({e}).\n\n\
                 Install th-ch/youtube-music, then enable\n\
                 Plugins → API Server (default port {DEFAULT_PORT}).\n\n\
                   macOS  https://github.com/th-ch/youtube-music/releases (.dmg)\n\
                   Linux  the same page (.AppImage), or your package manager"
            )
        })?;

    let v: serde_json::Value = res.body_mut().read_json()?;
    let token = v["accessToken"]
        .as_str()
        .ok_or_else(|| anyhow!("no accessToken in response: {v}"))?;

    // Pairing is asking for music, so it switches music on.
    Config {
        enabled: true,
        source: Source::YouTubeMusic,
        host: host.to_string(),
        port,
        token: Some(token.to_string()),
        ..Config::load()
    }
    .save()?;

    println!();
    println!("Paired. Token saved to {}", Config::path().display());
    Ok(())
}

// ── guided setup ─────────────────────────────────────────────────────
//
// Doing this by hand means: find the right asset, fight Gatekeeper (the app
// is ad-hoc signed and never notarized, so macOS refuses it outright), find
// the API Server plugin, enable it, then pair. Five steps and two of them are
// non-obvious. `grimoire music-setup` does the lot.

const RELEASE_API: &str = "https://api.github.com/repos/th-ch/youtube-music/releases/latest";

/// The Windows client's process name, for tasklist and taskkill.
const WINDOWS_EXE: &str = "YouTube Music.exe";

fn app_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Applications/YouTube Music.app")
    } else if cfg!(windows) {
        // Where its per-user installer puts it — no admin prompt involved.
        windows_folder("LOCALAPPDATA", "AppData/Local")
            .join("Programs")
            .join("youtube-music")
            .join(WINDOWS_EXE)
    } else {
        grimoire_core::paths::home().join(".local/bin/youtube-music")
    }
}

fn client_config_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        grimoire_core::paths::home().join("Library/Application Support/YouTube Music/config.json")
    } else if cfg!(windows) {
        windows_folder("APPDATA", "AppData/Roaming")
            .join("YouTube Music")
            .join("config.json")
    } else {
        grimoire_core::paths::home().join(".config/YouTube Music/config.json")
    }
}

/// %LOCALAPPDATA% or %APPDATA%, falling back to where Windows keeps them.
fn windows_folder(var: &str, under_home: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| grimoire_core::paths::home().join(under_home))
}

fn installed() -> bool {
    app_path().exists() && !built_for_another_cpu(&app_path())
}

/// The CPU an ELF binary was built for, spelled the way
/// `std::env::consts::ARCH` spells it. `None` for anything that isn't ELF —
/// a macOS .app bundle, a script, a missing file. An AppImage's runtime is an
/// ordinary ELF header, so this reads the right answer for those too.
fn elf_arch(path: &std::path::Path) -> Option<&'static str> {
    use std::io::Read;
    let mut head = [0u8; 20];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..4] != b"\x7fELF" {
        return None;
    }
    let machine = if head[5] == 2 {
        u16::from_be_bytes([head[18], head[19]])
    } else {
        u16::from_le_bytes([head[18], head[19]])
    };
    Some(match machine {
        0x03 => "x86",
        0x28 => "arm",
        0x3E => "x86_64",
        0xB7 => "aarch64",
        _ => "unknown",
    })
}

fn built_for_another_cpu(path: &std::path::Path) -> bool {
    elf_arch(path).is_some_and(|a| a != machine_arch())
}

/// The CPU this machine actually has, spelled like `std::env::consts::ARCH`.
/// Asked of the OS rather than taken from the build target, because an Intel
/// build of Grimoire running under Rosetta on an Apple Silicon Mac would
/// otherwise report — and fetch — Intel.
fn machine_arch() -> &'static str {
    use std::process::Command;
    let run = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if cfg!(windows) {
        // PROCESSOR_ARCHITEW6432 is only set inside a 32-bit process on 64-bit
        // Windows, and then it's the one telling the truth.
        let cpu = std::env::var("PROCESSOR_ARCHITEW6432")
            .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
            .unwrap_or_default();
        return match cpu.to_uppercase().as_str() {
            "AMD64" => "x86_64",
            "ARM64" => "aarch64",
            "X86" => "x86",
            _ => std::env::consts::ARCH,
        };
    }
    if cfg!(target_os = "macos") {
        // Only Apple Silicon has this key; Intel Macs error out.
        return match run("sysctl", &["-n", "hw.optional.arm64"]).as_deref() {
            Some("1") => "aarch64",
            _ => std::env::consts::ARCH,
        };
    }
    match run("uname", &["-m"]).as_deref() {
        Some("x86_64" | "amd64") => "x86_64",
        Some("aarch64" | "arm64") => "aarch64",
        Some(m) if m.starts_with("armv") => "arm",
        _ => std::env::consts::ARCH,
    }
}

/// Ask which kind of computer this is, so the right build gets installed.
/// What the machine reports is the default, and Enter takes it — someone who
/// doesn't know the answer can't choose wrong by not choosing.
fn ask_arch(detected: &'static str) -> &'static str {
    use std::io::{Write, stdin, stdout};
    let (default, family) = if detected == "aarch64" || detected == "arm" {
        ("2", "ARM")
    } else {
        ("1", "x86")
    };
    println!("\n  Which kind of computer is this?");
    println!("    1) x86 — Intel or AMD: most Windows and Linux PCs, Intel Macs");
    println!("    2) ARM — Apple Silicon Macs (M1 and later), ARM Linux devices");
    println!("  This one reports {family}, so Enter picks {default}.");
    loop {
        print!("  1 or 2 [{default}]: ");
        let _ = stdout().flush();
        let mut s = String::new();
        if stdin().read_line(&mut s).unwrap_or(0) == 0 {
            return detected;
        }
        match parse_arch_answer(&s, detected) {
            Some(arch) => return arch,
            None => println!("  Type 1 for x86 or 2 for ARM, or just press Enter."),
        }
    }
}

fn parse_arch_answer(answer: &str, detected: &'static str) -> Option<&'static str> {
    match answer.trim().to_lowercase().as_str() {
        "" => Some(detected),
        "1" | "x86" | "x64" | "x86_64" | "amd64" | "intel" | "amd" => Some("x86_64"),
        // ARM means 64-bit unless this machine is itself 32-bit ARM.
        "2" | "arm" | "arm64" | "aarch64" | "apple" => {
            Some(if detected == "arm" { "arm" } else { "aarch64" })
        }
        _ => None,
    }
}

fn port_open(host: &str, port: u16) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(400)).is_ok())
}

fn prompt(label: &str, default: &str) -> String {
    use std::io::{Write, stdin, stdout};
    if default.is_empty() {
        print!("  {label}: ");
    } else {
        print!("  {label} [{default}]: ");
    }
    let _ = stdout().flush();
    let mut s = String::new();
    let _ = stdin().read_line(&mut s);
    let s = s.trim().to_string();
    if s.is_empty() { default.to_string() } else { s }
}

/// Read without echoing. Falls back to a visible prompt if the terminal
/// won't cooperate — better than refusing to run.
#[cfg(not(windows))]
fn prompt_secret(label: &str) -> String {
    use std::process::Command;
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("printf '  {label}: ' >&2; stty -echo 2>/dev/null; read -r v; stty echo 2>/dev/null; printf '\\n' >&2; printf '%s' \"$v\""))
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => prompt(label, ""),
    }
}

/// Windows has no `sh` or `stty`, so read the keys directly with the terminal
/// in raw mode. Piped input (no console to put in raw mode) gets the visible
/// prompt instead.
#[cfg(windows)]
fn prompt_secret(label: &str) -> String {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::crossterm::terminal;
    use std::io::{Write, stdout};

    if terminal::enable_raw_mode().is_err() {
        return prompt(label, "");
    }
    print!("  {label}: ");
    let _ = stdout().flush();
    let mut secret = String::new();
    while let Ok(ev) = event::read() {
        let Event::Key(k) = ev else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match k.code {
            KeyCode::Enter => break,
            KeyCode::Backspace => {
                secret.pop();
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let _ = terminal::disable_raw_mode();
                println!();
                std::process::exit(130);
            }
            KeyCode::Char(c) => secret.push(c),
            _ => {}
        }
    }
    let _ = terminal::disable_raw_mode();
    println!();
    secret
}

fn ask(prompt: &str) -> bool {
    use std::io::{Write, stdin, stdout};
    print!("{prompt} [y/N] ");
    let _ = stdout().flush();
    let mut s = String::new();
    let _ = stdin().read_line(&mut s);
    matches!(s.trim().to_lowercase().as_str(), "y" | "yes")
}

enum Plugin {
    AlreadyOn,
    TurnedOn,
    NoConfig,
}

/// Turn on the API Server plugin, touching nothing else in the client's config.
fn enable_api_plugin(port: u16) -> Result<Plugin> {
    let path = client_config_path();
    if !path.exists() {
        return Ok(Plugin::NoConfig);
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut cfg: serde_json::Value = serde_json::from_str(&raw)?;

    if cfg["plugins"]["api-server"]["enabled"].as_bool().unwrap_or(false) {
        return Ok(Plugin::AlreadyOn);
    }

    std::fs::write(path.with_extension("json.bak-grimoire"), &raw)?;
    if !cfg["plugins"].is_object() {
        cfg["plugins"] = serde_json::json!({});
    }
    cfg["plugins"]["api-server"] = serde_json::json!({
        "enabled": true,
        // Deliberately not 0.0.0.0 (the plugin default) — there is no reason
        // to expose someone's music controls to their whole network.
        "hostname": "127.0.0.1",
        "port": port,
        "authStrategy": "AUTH_AT_FIRST",
        "authorizedClients": [],
        "useHttps": false,
        "certPath": "",
        "keyPath": "",
    });
    std::fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
    Ok(Plugin::TurnedOn)
}

fn launch_client() {
    use std::process::{Command, Stdio};
    let _ = if cfg!(target_os = "macos") {
        Command::new("open").arg("-a").arg(app_path()).status()
    } else {
        Command::new(app_path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| Default::default())
    };
}

fn quit_client() {
    use std::process::Command;
    if cfg!(target_os = "macos") {
        let _ = Command::new("osascript")
            .args(["-e", "quit app \"YouTube Music\""])
            .status();
        thread::sleep(Duration::from_secs(3));
        return;
    }
    if cfg!(windows) {
        if !windows_client_running() {
            return;
        }
        // Without /F, taskkill asks the windows to close, and Electron quits
        // the way it would if you closed it yourself. /F only if that fails.
        let _ = Command::new("taskkill").args(["/IM", WINDOWS_EXE]).output();
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(500));
            if !windows_client_running() {
                return;
            }
        }
        let _ = Command::new("taskkill").args(["/F", "/T", "/IM", WINDOWS_EXE]).output();
        thread::sleep(Duration::from_secs(1));
        return;
    }

    // Signal Electron's main process only. It closes its helpers and the
    // AppImage runtime unmounts after it. Signalling the runtime as well (what
    // a plain pkill does) pulls the mount out from under an Electron that is
    // still shutting down, and it spins a full core forever.
    let main = client_pids(true);
    if main.is_empty() {
        return;
    }
    let _ = Command::new("kill").args(&main).status();
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(500));
        if client_pids(false).is_empty() {
            return;
        }
    }
    let _ = Command::new("kill").arg("-9").args(client_pids(false)).status();
}

/// Windows: whether any YouTube Music process is up. Checked by looking for
/// the image name in tasklist's output, not its "no tasks" message, which is
/// translated.
fn windows_client_running() -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {WINDOWS_EXE}"), "/NH"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(WINDOWS_EXE))
}

/// Linux: pids of the running AppImage client. With `main_only`, just
/// Electron's main process — the one inside the mount with no `--type=`.
///
/// Matched on "/youtube-music" as a whole path component: a bare
/// "youtube-music" also matches YTMDesktop (/app/lib/youtube-music-desktop-
/// app/…), a different player that may well be open.
fn client_pids(main_only: bool) -> Vec<String> {
    let Ok(out) = std::process::Command::new("pgrep")
        .args(["-af", "/youtube-music( |$)"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !main_only || (l.contains("/.mount_") && !l.contains("--type=")))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

fn download_and_install(arch: &str) -> Result<()> {
    use std::process::Command;

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .new_agent();

    println!("  Finding the latest release…");
    let mut res = agent
        .get(RELEASE_API)
        .header("User-Agent", APP_ID)
        .call()
        .context("querying the GitHub releases API")?;
    let rel: serde_json::Value = res.body_mut().read_json()?;
    let tag = rel["tag_name"].as_str().unwrap_or("?").to_string();

    let ext = if cfg!(target_os = "macos") {
        ".dmg"
    } else if cfg!(windows) {
        ".exe"
    } else {
        ".AppImage"
    };

    let assets = rel["assets"].as_array().cloned().unwrap_or_default();
    let pick = pick_asset(&assets, ext, arch)
        .ok_or_else(|| anyhow!("release {tag} has no {ext} built for {arch}"))?;

    let name = pick["name"].as_str().unwrap_or("download").to_string();
    let url = pick["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("asset has no download url"))?
        .to_string();
    let mb = pick["size"].as_u64().unwrap_or(0) / 1_048_576;

    let tmp = std::env::temp_dir().join(&name);
    let expect = pick["size"].as_u64().unwrap_or(0);
    let have = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if have > 0 && have == expect {
        println!("  Already downloaded {name} — reusing it.");
    } else {
        if mb > 0 {
            println!("  Downloading {name} ({mb} MB)…");
        } else {
            println!("  Downloading {name}…");
        }
        let mut res = agent.get(&url).header("User-Agent", APP_ID).call()?;
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut res.body_mut().as_reader(), &mut out)?;
        drop(out);
    }

    if cfg!(target_os = "macos") {
        println!("  Mounting…");
        // No -quiet here: attach prints the mount table on stdout and that is
        // the only reliable way to learn the volume path. It is captured, so
        // nothing leaks to the user.
        let mount = Command::new("hdiutil")
            .args(["attach", "-nobrowse"])
            .arg(&tmp)
            .output()?;
        if !mount.status.success() {
            anyhow::bail!("could not mount {}", tmp.display());
        }
        let out = String::from_utf8_lossy(&mount.stdout);
        let vol = out
            .lines()
            .filter_map(|l| l.split('\t').next_back())
            .map(str::trim)
            .find(|p| p.starts_with("/Volumes/"))
            .map(str::to_string)
            .ok_or_else(|| {
                anyhow!("mounted, but no /Volumes path in hdiutil output:\n{out}")
            })?;

        let src = PathBuf::from(&vol).join("YouTube Music.app");
        println!("  Copying to /Applications…");
        let _ = Command::new("rm").arg("-rf").arg(app_path()).status();
        Command::new("cp").arg("-R").arg(&src).arg("/Applications/").status()?;

        // The build is ad-hoc signed and unnotarized, so macOS quarantines it
        // and refuses to launch. Clearing the flag and re-signing ad-hoc is
        // what makes it runnable — this is the step people get stuck on.
        println!("  Clearing the macOS quarantine flag…");
        let _ = Command::new("xattr").args(["-cr"]).arg(app_path()).output();
        // .output() rather than .status(): codesign chatters on stderr and
        // there is nothing here the user needs to read.
        let _ = Command::new("codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(app_path())
            .output();
        let _ = Command::new("hdiutil").args(["detach", "-quiet", &vol]).status();
    } else if cfg!(windows) {
        // The web installer fetches the package for this PC's CPU itself, then
        // installs per-user. /S keeps it silent; it runs to completion before
        // returning, so there is nothing to poll.
        println!("  Installing — the installer downloads the app itself, so give it a minute…");
        let status = Command::new(&tmp)
            .arg("/S")
            .status()
            .with_context(|| format!("running {}", tmp.display()))?;
        if !status.success() || !app_path().exists() {
            anyhow::bail!(
                "the installer finished ({status}) but {} isn't there",
                app_path().display()
            );
        }
        println!("  Installed to {}", app_path().display());
    } else {
        let dest = app_path();
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(&tmp, &dest)?;
        let _ = Command::new("chmod").arg("+x").arg(&dest).status();
        println!("  Installed to {}", dest.display());
    }

    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// The release build that runs on `arch` (`std::env::consts::ARCH`).
///
/// th-ch tags every build with its CPU (`-arm64`, `-armv7l`) except x64,
/// which carries no tag at all — `YouTube-Music-3.12.0.AppImage`. So an
/// untagged name means x64. There is deliberately no "any .AppImage will do"
/// fallback: the armv7l build copies onto an x86_64 machine without complaint
/// and then simply never starts, which surfaced as "the API server never came
/// up" with nothing pointing at the real cause.
fn pick_asset<'a>(
    assets: &'a [serde_json::Value],
    ext: &str,
    arch: &str,
) -> Option<&'a serde_json::Value> {
    const TAGS: [(&str, &str); 7] = [
        ("arm64", "aarch64"),
        ("aarch64", "aarch64"),
        ("armv7l", "arm"),
        ("ia32", "x86"),
        ("x86_64", "x86_64"),
        ("x64", "x86_64"),
        ("amd64", "x86_64"),
    ];
    // Windows gets the web installer whatever the CPU: it carries no build of
    // its own and downloads the right one. The other .exe is the portable
    // build, which unpacks itself to a temp folder on every launch.
    if ext == ".exe" {
        return assets.iter().find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n.contains("Web-Setup") && n.ends_with(".exe"))
        });
    }
    assets.iter().find(|a| {
        let Some(name) = a["name"].as_str() else {
            return false;
        };
        if !name.ends_with(ext) {
            return false;
        }
        let name = name.to_lowercase();
        match TAGS.iter().find(|(tag, _)| name.contains(tag)) {
            Some((_, built_for)) => *built_for == arch,
            None => arch == "x86_64",
        }
    })
}

#[cfg(test)]
mod config_tests {
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

#[cfg(test)]
mod install_tests {
    use super::*;
    use serde_json::json;

    /// The real v3.12.0 asset list, in GitHub's order — the arm builds come
    /// before the untagged x64 one, which is how armv7l got picked.
    fn release() -> Vec<serde_json::Value> {
        [
            "YouTube-Music-3.12.0-arm64.AppImage",
            "YouTube-Music-3.12.0-arm64.dmg",
            "YouTube-Music-3.12.0-armv7l.AppImage",
            "YouTube-Music-3.12.0-x86_64.flatpak",
            "YouTube-Music-3.12.0.AppImage",
            "YouTube-Music-3.12.0.dmg",
            "YouTube-Music-3.12.0.exe",
            "YouTube-Music-Web-Setup-3.12.0.exe",
            "youtube-music-3.12.0-x64.nsis.7z",
        ]
        .iter()
        .map(|n| json!({ "name": n }))
        .collect()
    }

    fn picked(ext: &str, arch: &str) -> Option<String> {
        let assets = release();
        pick_asset(&assets, ext, arch).map(|a| a["name"].as_str().unwrap().to_string())
    }

    #[test]
    fn each_cpu_gets_its_own_build() {
        assert_eq!(picked(".AppImage", "x86_64").as_deref(), Some("YouTube-Music-3.12.0.AppImage"));
        assert_eq!(picked(".AppImage", "aarch64").as_deref(), Some("YouTube-Music-3.12.0-arm64.AppImage"));
        assert_eq!(picked(".AppImage", "arm").as_deref(), Some("YouTube-Music-3.12.0-armv7l.AppImage"));
        assert_eq!(picked(".dmg", "x86_64").as_deref(), Some("YouTube-Music-3.12.0.dmg"));
        assert_eq!(picked(".dmg", "aarch64").as_deref(), Some("YouTube-Music-3.12.0-arm64.dmg"));
    }

    #[test]
    fn the_cpu_answer_decides_the_build() {
        // Enter keeps what the machine reports.
        assert_eq!(parse_arch_answer("\n", "x86_64"), Some("x86_64"));
        assert_eq!(parse_arch_answer("", "aarch64"), Some("aarch64"));
        // An explicit answer overrides it, either way round.
        assert_eq!(parse_arch_answer("2\n", "x86_64"), Some("aarch64"));
        assert_eq!(parse_arch_answer(" 1 ", "aarch64"), Some("x86_64"));
        assert_eq!(parse_arch_answer("ARM", "x86_64"), Some("aarch64"));
        assert_eq!(parse_arch_answer("intel", "aarch64"), Some("x86_64"));
        // ARM on a 32-bit ARM board stays 32-bit.
        assert_eq!(parse_arch_answer("2", "arm"), Some("arm"));
        // Anything else is asked again, never guessed.
        assert_eq!(parse_arch_answer("3", "x86_64"), None);
        assert_eq!(parse_arch_answer("mac", "x86_64"), None);

        let installs = |answer: &str, detected| {
            picked(".AppImage", parse_arch_answer(answer, detected).unwrap())
        };
        assert_eq!(installs("", "x86_64").as_deref(), Some("YouTube-Music-3.12.0.AppImage"));
        assert_eq!(installs("2", "x86_64").as_deref(), Some("YouTube-Music-3.12.0-arm64.AppImage"));
    }

    #[test]
    fn windows_gets_the_web_installer_whatever_the_cpu() {
        for arch in ["x86_64", "aarch64", "x86"] {
            assert_eq!(
                picked(".exe", arch).as_deref(),
                Some("YouTube-Music-Web-Setup-3.12.0.exe"),
                "never the portable build, which unpacks itself on every launch"
            );
        }
    }

    #[test]
    fn a_native_build_detects_its_own_cpu() {
        assert_eq!(machine_arch(), std::env::consts::ARCH);
    }

    #[test]
    fn no_build_for_this_cpu_means_none_not_a_wrong_one() {
        assert_eq!(picked(".AppImage", "riscv64"), None);
        assert_eq!(picked(".dmg", "arm"), None);
    }

    #[test]
    fn reads_the_cpu_out_of_an_elf_header() {
        // The test binary itself is only ELF on Linux; Mach-O and PE get no opinion.
        let exe = std::env::current_exe().unwrap();
        if cfg!(target_os = "linux") {
            assert_eq!(elf_arch(&exe), Some(std::env::consts::ARCH));
        } else {
            assert_eq!(elf_arch(&exe), None);
        }
        assert!(!built_for_another_cpu(&exe));

        // First 20 bytes of the armv7l AppImage that was installed on bazzite.
        let dir = std::env::temp_dir().join(format!("grimoire-elf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let arm = dir.join("youtube-music");
        let head = [
            0x7f, b'E', b'L', b'F', 1, 1, 1, 0, b'A', b'I', 2, 0, 0, 0, 0, 0, 2, 0, 0x28, 0,
        ];
        std::fs::write(&arm, head).unwrap();
        assert_eq!(elf_arch(&arm), Some("arm"));
        assert!(built_for_another_cpu(&arm));

        std::fs::write(&arm, "#!/bin/sh\n").unwrap();
        assert_eq!(elf_arch(&arm), None, "not ELF: no opinion");
        let _ = std::fs::remove_dir_all(&dir);
    }
}


/// Set up whichever source you name. `grimoire music-setup jellyfin`, etc.
pub fn setup_for(source: Source) -> Result<()> {
    match source {
        Source::YouTubeMusic => setup(),
        Source::Spotify if cfg!(windows) => {
            println!("Spotify isn't supported on Windows yet — Grimoire drives it through");
            println!("AppleScript on macOS and MPRIS on Linux, and Windows has neither.");
            println!();
            println!("  YouTube Music, Jellyfin and Plex all work here:");
            println!("    grimoire music-setup youtube-music");
            Ok(())
        }
        Source::Spotify => {
            let mut cfg = Config::load();
            cfg.source = Source::Spotify;
            cfg.enabled = true;
            cfg.save()?;
            println!("Spotify needs no setup — Grimoire drives the desktop app directly.");
            println!();
            if cfg!(target_os = "macos") {
                println!("  Just have Spotify open. Nothing to sign into.");
            } else {
                println!("  Needs `playerctl` installed; Spotify exposes MPRIS through it.");
            }
            Ok(())
        }
        Source::Jellyfin => setup_jellyfin(),
        Source::Plex => setup_plex(),
    }
}

fn setup_jellyfin() -> Result<()> {
    let mut cfg = Config::load();
    println!("Connecting Grimoire to Jellyfin.\n");
    println!("  Grimoire plays these tracks itself — your files, your server,");
    println!("  nothing else needs to be running.\n");

    let server = prompt("Server URL", if cfg.server.is_empty() { "http://localhost:8096" } else { &cfg.server });
    let user = prompt("Username", "");
    let pass = prompt_secret("Password");

    println!("\n  Signing in…");
    let (token, uid) = crate::library::Jellyfin::login(&server, &user, &pass)?;

    cfg.source = Source::Jellyfin;
    cfg.enabled = true;
    cfg.server = server;
    cfg.api_key = token;
    cfg.user_id = uid;
    cfg.save()?;

    println!("  Signed in. Checking the library…");
    let cat = crate::library::Jellyfin {
        server: cfg.server.clone(),
        token: cfg.api_key.clone(),
        user_id: cfg.user_id.clone(),
    };
    use crate::library::Catalog;
    let tracks = cat.fetch(20)?;
    println!("  Found {} track(s).", tracks.len());
    if let Some(t) = tracks.first() {
        println!("  For example: {} — {}", t.title, t.artist);
    }
    println!("\nDone. F5 plays, F4/F6 move through the queue.");
    Ok(())
}

fn setup_plex() -> Result<()> {
    let mut cfg = Config::load();
    println!("Connecting Grimoire to Plex.\n");
    println!("  Grimoire plays these tracks itself — your files, your server.\n");
    println!("  Plex has no password login for apps. Get a token by opening any");
    println!("  item in the Plex web app, choosing Get Info → View XML, and");
    println!("  copying the X-Plex-Token value from the address bar.\n");

    let server = prompt("Server URL", if cfg.server.is_empty() { "http://localhost:32400" } else { &cfg.server });
    let token = prompt("X-Plex-Token", "");

    cfg.source = Source::Plex;
    cfg.enabled = true;
    cfg.server = server;
    cfg.api_key = token;
    cfg.user_id = String::new();
    cfg.save()?;

    println!("\n  Checking the library…");
    let cat = crate::library::Plex {
        server: cfg.server.clone(),
        token: cfg.api_key.clone(),
    };
    use crate::library::Catalog;
    let tracks = cat.fetch(20)?;
    println!("  Found {} track(s).", tracks.len());
    if let Some(t) = tracks.first() {
        println!("  For example: {} — {}", t.title, t.artist);
    }
    println!("\nDone. F5 plays, F4/F6 move through the queue.");
    Ok(())
}

/// One command instead of five. Installs the client if needed, enables the
/// API Server plugin, starts it, and pairs.
pub fn setup() -> Result<()> {
    let cfg = Config::load();
    let port = cfg.port;

    println!("Setting up music for Grimoire.\n");

    println!("Looking for YouTube Music…");
    if installed() {
        println!("  found {}", app_path().display());
    } else {
        if let Some(cpu) = elf_arch(&app_path()) {
            println!(
                "  found {}, but it's built for {cpu} and this machine is {}.",
                app_path().display(),
                machine_arch()
            );
            println!("  It can never start here — replacing it.\n");
        } else {
            println!("  not installed.\n");
        }
        println!("  Grimoire can fetch it from github.com/th-ch/youtube-music.");
        if cfg!(target_os = "macos") {
            println!("  That build is ad-hoc signed rather than notarised, so macOS");
            println!("  blocks it until the quarantine flag is cleared. Grimoire will");
            println!("  clear it and re-sign the app locally so it will launch.\n");
        }
        if !ask("  Download and install it now?") {
            println!("\nNothing installed. Re-run `grimoire music-setup` when you're ready.");
            return Ok(());
        }
        let arch = if cfg!(windows) {
            println!("\n  The Windows installer carries both the x86 and the ARM build and");
            println!("  installs whichever this PC needs, so there's nothing to choose.");
            machine_arch()
        } else {
            ask_arch(machine_arch())
        };
        download_and_install(arch)?;
        println!("  installed.");
    }

    println!("\nEnabling the API Server plugin…");
    if !client_config_path().exists() {
        println!("  no config yet — starting the app once to create it.");
        launch_client();
        // Up to a minute. A first launch is slow — Windows Defender scans a
        // freshly installed Electron app before it lets it start — and this
        // returns the moment the file appears, so fast machines never wait.
        for _ in 0..120 {
            if client_config_path().exists() {
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }
        quit_client();
    }
    match enable_api_plugin(port) {
        Ok(Plugin::AlreadyOn) => println!("  already on."),
        Ok(Plugin::TurnedOn) => {
            println!("  turned on, bound to 127.0.0.1:{port} (config backed up).")
        }
        Ok(Plugin::NoConfig) => println!("  no client config found — enable it by hand."),
        Err(e) => println!("  could not edit the config ({e}) — enable it by hand."),
    }

    println!("\nStarting YouTube Music…");
    if !port_open(&cfg.host, port) {
        quit_client();
        launch_client();
    }
    let mut up = false;
    for _ in 0..120 {
        if port_open(&cfg.host, port) {
            up = true;
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    if !up {
        anyhow::bail!(
            "the API server never came up on {}:{port}.\n\
             Open YouTube Music, then Plugins → API Server, and re-run this.",
            cfg.host
        );
    }
    println!("  API server responding.");

    println!("\nPairing…");
    authenticate(&cfg.host, port)?;

    println!();
    println!("Done. Sign in to YouTube Music and press play —");
    println!("Grimoire picks it up within a second or two. F4/F5/F6 control it.");
    Ok(())
}
