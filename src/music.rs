//! Music, by remote control.
//!
//! There is no official YouTube Music API, and anything that extracts stream
//! URLs both violates the terms and side-steps the subscription you already
//! pay for. So Grimoire never plays audio. It drives a desktop client that is
//! already signed into your real account — your playlists, your Premium, your
//! playback. We just press the buttons.
//!
//! The backend is th-ch/youtube-music's API Server plugin, chosen because it
//! ships a signed .dmg *and* an .AppImage, so the same setup works on macOS
//! and Linux. (YTMDesktop was the original target; its Homebrew cask was
//! disabled on 2026-09-01 for failing Apple's Gatekeeper check.)
//!
//!   https://github.com/th-ch/youtube-music

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

pub const APP_ID: &str = "grimoiretui";
pub const DEFAULT_PORT: u16 = 26538;

const POLL: Duration = Duration::from_millis(1500);
const HTTP_TIMEOUT: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: DEFAULT_PORT,
            token: None,
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
                "token" if !v.is_empty() => cfg.token = Some(v),
                "host" if !v.is_empty() => cfg.host = v,
                "port" => {
                    if let Ok(p) = v.parse() {
                        cfg.port = p;
                    }
                }
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
            "host = \"{}\"\nport = {}\ntoken = \"{}\"\n",
            self.host,
            self.port,
            self.token.as_deref().unwrap_or("")
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

#[derive(Debug, Clone, Copy)]
pub enum Cmd {
    PlayPause,
    Next,
    Prev,
}

impl Cmd {
    fn path(self) -> &'static str {
        match self {
            Cmd::PlayPause => "/api/v1/toggle-play",
            Cmd::Next => "/api/v1/next",
            Cmd::Prev => "/api/v1/previous",
        }
    }
}

pub struct Music {
    pub state: State,
    rx: Option<Receiver<State>>,
    tx: Option<Sender<Cmd>>,
}

impl Music {
    /// Spawns the poller. With no token we stay inert and never touch the network.
    pub fn spawn(cfg: Config) -> Music {
        let Some(token) = cfg.token.clone() else {
            return Music {
                state: State::NoToken,
                rx: None,
                tx: None,
            };
        };

        let (state_tx, state_rx) = mpsc::channel::<State>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let base = cfg.base();

        thread::spawn(move || {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(HTTP_TIMEOUT))
                .build()
                .new_agent();
            let bearer = format!("Bearer {token}");

            loop {
                // Commands first, so a keypress feels immediate.
                loop {
                    match cmd_rx.try_recv() {
                        Ok(c) => {
                            let _ = agent
                                .post(format!("{base}{}", c.path()))
                                .header("Authorization", &bearer)
                                .send_empty();
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                let next = fetch_state(&agent, &base, &bearer).unwrap_or(State::Offline);
                if state_tx.send(next).is_err() {
                    return;
                }
                thread::sleep(POLL);
            }
        });

        Music {
            state: State::Offline,
            rx: Some(state_rx),
            tx: Some(cmd_tx),
        }
    }

    /// Drain whatever the poller has sent since the last frame.
    pub fn drain(&mut self) {
        if let Some(rx) = &self.rx {
            while let Ok(s) = rx.try_recv() {
                self.state = s;
            }
        }
    }

    pub fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }
}

fn fetch_state(agent: &ureq::Agent, base: &str, bearer: &str) -> Result<State> {
    let mut res = agent
        .get(format!("{base}/api/v1/song"))
        .header("Authorization", bearer)
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
        // isPaused is optional; absent means playing.
        playing: !v["isPaused"].as_bool().unwrap_or(false),
    }))
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
        host: host.to_string(),
        port,
        token: Some(token.to_string()),
    }
    .save()?;

    println!();
    println!("Paired. Token saved to {}", Config::path().display());
    Ok(())
}
