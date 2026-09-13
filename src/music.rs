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
