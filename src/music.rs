//! YouTube Music, by remote control.
//!
//! There is no official YouTube Music API, and anything that extracts stream
//! URLs both violates the terms and side-steps the subscription you already
//! pay for. So Grimoire never plays audio. It drives YTMDesktop's Companion
//! Server, which is already signed into your real account — your playlists,
//! your Premium, your playback. We just press the buttons.
//!
//!   https://github.com/ytmdesktop/ytmdesktop/wiki/v2-%E2%80%90-Companion-Server-API-v1

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

pub const APP_ID: &str = "grimoiretui";
pub const APP_NAME: &str = "GrimoireTUI";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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
            port: 9863,
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
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
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
    /// No token configured — the pane invites you to run the auth command.
    NoToken,
    /// Configured, but YTMDesktop isn't reachable.
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
    fn name(self) -> &'static str {
        match self {
            Cmd::PlayPause => "playPause",
            Cmd::Next => "next",
            Cmd::Prev => "previous",
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

            loop {
                // Commands first, so a keypress feels immediate.
                loop {
                    match cmd_rx.try_recv() {
                        Ok(c) => {
                            let _ = agent
                                .post(format!("{base}/api/v1/command"))
                                .header("Authorization", &token)
                                .send_json(serde_json::json!({ "command": c.name() }));
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                let next = match fetch_state(&agent, &base, &token) {
                    Ok(s) => s,
                    Err(_) => State::Offline,
                };
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

    pub fn enabled(&self) -> bool {
        self.tx.is_some()
    }
}

fn fetch_state(agent: &ureq::Agent, base: &str, token: &str) -> Result<State> {
    let mut res = agent
        .get(format!("{base}/api/v1/state"))
        .header("Authorization", token)
        .call()?;
    let v: serde_json::Value = res.body_mut().read_json()?;

    let title = v["video"]["title"].as_str().unwrap_or("").to_string();
    if title.is_empty() {
        return Ok(State::Idle);
    }

    // trackState: -1 unknown, 0 paused, 1 playing, 2 buffering.
    let track_state = v["player"]["trackState"].as_i64().unwrap_or(-1);

    Ok(State::Playing(Track {
        title,
        artist: v["video"]["author"].as_str().unwrap_or("").to_string(),
        progress: v["player"]["videoProgress"].as_f64().unwrap_or(0.0),
        duration: v["video"]["durationSeconds"].as_f64().unwrap_or(0.0),
        playing: track_state == 1,
    }))
}

/// Interactive pairing. Prints a code you approve inside YTMDesktop.
pub fn authenticate(host: &str, port: u16) -> Result<()> {
    let base = format!("http://{host}:{port}");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(35)))
        .build()
        .new_agent();

    println!("Contacting YTMDesktop at {base} ...");

    let mut res = agent
        .post(format!("{base}/auth/requestcode"))
        .send_json(serde_json::json!({
            "appId": APP_ID,
            "appName": APP_NAME,
            "appVersion": APP_VERSION,
        }))
        .map_err(|e| {
            anyhow!(
                "could not reach the companion server ({e}).\n\
                 Is YTMDesktop running, with Settings → Integrations → \
                 Companion Server enabled?"
            )
        })?;

    let v: serde_json::Value = res.body_mut().read_json()?;
    let code = v["code"]
        .as_str()
        .ok_or_else(|| anyhow!("no code in response: {v}"))?;

    println!();
    println!("   Approve this code in YTMDesktop:   {code}");
    println!();
    println!("   (Settings → Integrations → Companion Server → allow)");
    println!("   Waiting up to 30 seconds ...");

    let mut res = agent
        .post(format!("{base}/auth/request"))
        .send_json(serde_json::json!({ "appId": APP_ID, "code": code }))
        .map_err(|e| anyhow!("pairing was not approved in time ({e})"))?;

    let v: serde_json::Value = res.body_mut().read_json()?;
    let token = v["token"]
        .as_str()
        .ok_or_else(|| anyhow!("no token in response: {v}"))?;

    let cfg = Config {
        host: host.to_string(),
        port,
        token: Some(token.to_string()),
    };
    cfg.save()?;

    println!();
    println!("Paired. Token saved to {}", Config::path().display());
    Ok(())
}
