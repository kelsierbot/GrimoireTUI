//! Music, by remote control.
//!
//! There is no official YouTube Music API, and anything that extracts stream
//! URLs both violates the terms and side-steps the subscription you already
//! pay for. So Grimoire never plays audio. It drives a desktop client that is
//! already signed into your real account — your playlists, your Premium, your
//! playback. We just press the buttons.
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
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".config/grimoire/music.toml")
    }

    /// Missing or unreadable config just means "music off".
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Ok(s) = std::fs::read_to_string(Config::path()) else {
            return cfg;
        };
        for line in s.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
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
            "source = \"{}\"\nhost = \"{}\"\nport = {}\ntoken = \"{}\"\n\
             server = \"{}\"\napi_key = \"{}\"\nuser_id = \"{}\"\n",
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
    /// Queue a track by id: straight after this one and play it (`now`), or at the end.
    Enqueue { id: String, now: bool },
    /// Ask for the queue; answered by a fresh `Music::queue`.
    FetchQueue,
    /// Search; answered by `Music::results`.
    Search(String),
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
    Note(String),
}

pub struct Music {
    pub state: State,
    pub source: Source,
    /// The last queue fetched, and the last search's results.
    pub queue: Vec<Item>,
    pub results: Vec<Item>,
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
}

impl Music {
    /// Spawns the poller. Unconfigured sources stay inert and never touch the
    /// network or shell out.
    pub fn spawn(cfg: Config) -> Music {
        let source = cfg.source;
        let backend: Option<Box<dyn Backend>> = match source {
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
        };

        let Some(mut backend) = backend else {
            return Music {
                state: State::NoToken,
                source,
                queue: Vec::new(),
                results: Vec::new(),
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
                    let repoll = !matches!(c, Cmd::FetchQueue | Cmd::Search(_));
                    let reply = match c {
                        Cmd::FetchQueue => Some(backend.queue().map(Update::Queue)),
                        Cmd::Search(q) => Some(backend.search(&q).map(Update::Results)),
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
            state: State::Offline,
            source,
            queue: Vec::new(),
            results: Vec::new(),
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

struct Ytm {
    agent: ureq::Agent,
    /// Search goes out to YouTube from inside the app, so it gets longer.
    slow: ureq::Agent,
    base: String,
    bearer: String,
}

/// Large queues run to megabytes of renderer JSON.
const BODY_LIMIT: u64 = 64 << 20;

impl Ytm {
    fn new(cfg: &Config, token: String) -> Ytm {
        let agent = |t| ureq::Agent::config_builder().timeout_global(Some(t)).build().new_agent();
        Ytm {
            agent: agent(HTTP_TIMEOUT),
            slow: agent(Duration::from_secs(12)),
            base: cfg.base(),
            bearer: format!("Bearer {token}"),
        }
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
                let now = Ytm::read(&mut self.get("/volume")?)?["state"].as_f64().unwrap_or(50.0) as i32;
                self.post("/volume", Some(json!({ "volume": (now + d).clamp(0, 100) })))
            }
            Cmd::Like => self.post("/like", None),
            Cmd::JumpTo(i) => self.patch("/queue", json!({ "index": i })),
            Cmd::Enqueue { id, now } => {
                let at = if now { "INSERT_AFTER_CURRENT_VIDEO" } else { "INSERT_AT_END" };
                self.post("/queue", Some(json!({ "videoId": id, "insertPosition": at })))?;
                if now {
                    // The app adds it asynchronously. Wait until it lands just
                    // after the playing track, then jump to it.
                    for _ in 0..12 {
                        thread::sleep(Duration::from_millis(250));
                        let q = self.queue()?;
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
            Cmd::FetchQueue | Cmd::Search(_) => Ok(()),
        }
    }

    fn queue(&mut self) -> Result<Vec<Item>> {
        let mut res = self.get("/queue")?;
        if res.status() == 204 {
            return Ok(Vec::new());
        }
        Ok(parse_queue(&Ytm::read(&mut res)?))
    }

    fn search(&mut self, query: &str) -> Result<Vec<Item>> {
        let mut res = self
            .slow
            .post(self.url("/search"))
            .header("Authorization", &self.bearer)
            .send_json(serde_json::json!({ "query": query }))?;
        Ok(parse_search(&Ytm::read(&mut res)?))
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
            let r = it.get("playlistPanelVideoRenderer").or_else(|| {
                it.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer")
            })?;
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

    Config {
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

fn app_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Applications/YouTube Music.app")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".local/bin/youtube-music")
    }
}

fn client_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/YouTube Music/config.json")
    } else {
        PathBuf::from(home).join(".config/YouTube Music/config.json")
    }
}

fn installed() -> bool {
    app_path().exists()
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
    } else {
        let _ = Command::new("pkill").arg("-f").arg("youtube-music").status();
    }
    thread::sleep(Duration::from_secs(3));
}

fn download_and_install() -> Result<()> {
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

    let want_arm = cfg!(target_arch = "aarch64");
    let ext = if cfg!(target_os = "macos") { ".dmg" } else { ".AppImage" };

    let assets = rel["assets"].as_array().cloned().unwrap_or_default();
    let pick = assets
        .iter()
        .filter(|a| a["name"].as_str().is_some_and(|n| n.ends_with(ext)))
        .find(|a| {
            let n = a["name"].as_str().unwrap_or("");
            n.contains("arm64") == want_arm
        })
        .or_else(|| {
            assets
                .iter()
                .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(ext)))
        })
        .ok_or_else(|| anyhow!("no {ext} asset in release {tag}"))?;

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
        println!("  Downloading {name} ({mb} MB)…");
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


/// Set up whichever source you name. `grimoire music-setup jellyfin`, etc.
pub fn setup_for(source: Source) -> Result<()> {
    match source {
        Source::YouTubeMusic => setup(),
        Source::Spotify => {
            let mut cfg = Config::load();
            cfg.source = Source::Spotify;
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
        println!("  not installed.\n");
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
        download_and_install()?;
        println!("  installed.");
    }

    println!("\nEnabling the API Server plugin…");
    if !client_config_path().exists() {
        println!("  no config yet — starting the app once to create it.");
        launch_client();
        for _ in 0..30 {
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
    for _ in 0..40 {
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
