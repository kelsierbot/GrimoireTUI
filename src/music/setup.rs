//! Setting a source up: pairing with YouTube Music (installing its client
//! first if need be), and signing in to Jellyfin or Plex.

use super::{APP_ID, Config, DEFAULT_PORT, Source};
use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

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

    if cfg["plugins"]["api-server"]["enabled"]
        .as_bool()
        .unwrap_or(false)
    {
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
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/IM", WINDOWS_EXE])
            .output();
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
    let _ = Command::new("kill")
        .arg("-9")
        .args(client_pids(false))
        .status();
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
            .ok_or_else(|| anyhow!("mounted, but no /Volumes path in hdiutil output:\n{out}"))?;

        let src = PathBuf::from(&vol).join("YouTube Music.app");
        println!("  Copying to /Applications…");
        let _ = Command::new("rm").arg("-rf").arg(app_path()).status();
        Command::new("cp")
            .arg("-R")
            .arg(&src)
            .arg("/Applications/")
            .status()?;

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
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet", &vol])
            .status();
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

    let server = prompt(
        "Server URL",
        if cfg.server.is_empty() {
            "http://localhost:8096"
        } else {
            &cfg.server
        },
    );
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

    let server = prompt(
        "Server URL",
        if cfg.server.is_empty() {
            "http://localhost:32400"
        } else {
            &cfg.server
        },
    );
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
        assert_eq!(
            picked(".AppImage", "x86_64").as_deref(),
            Some("YouTube-Music-3.12.0.AppImage")
        );
        assert_eq!(
            picked(".AppImage", "aarch64").as_deref(),
            Some("YouTube-Music-3.12.0-arm64.AppImage")
        );
        assert_eq!(
            picked(".AppImage", "arm").as_deref(),
            Some("YouTube-Music-3.12.0-armv7l.AppImage")
        );
        assert_eq!(
            picked(".dmg", "x86_64").as_deref(),
            Some("YouTube-Music-3.12.0.dmg")
        );
        assert_eq!(
            picked(".dmg", "aarch64").as_deref(),
            Some("YouTube-Music-3.12.0-arm64.dmg")
        );
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
        assert_eq!(
            installs("", "x86_64").as_deref(),
            Some("YouTube-Music-3.12.0.AppImage")
        );
        assert_eq!(
            installs("2", "x86_64").as_deref(),
            Some("YouTube-Music-3.12.0-arm64.AppImage")
        );
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
