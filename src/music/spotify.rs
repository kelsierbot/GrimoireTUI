//! Spotify: drive the desktop app, no account setup at all.
//!
//! Spotify's Web API would mean OAuth, a registered application, a redirect
//! URL and a refresh-token dance — for the privilege of pressing pause. Both
//! platforms already expose the running player locally: AppleScript on macOS,
//! MPRIS over D-Bus on Linux. Nothing to sign in to, nothing to store.

use super::{Backend, Cmd, State, Track};
use anyhow::Result;

pub(super) struct Spotify;

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
            Cmd::Volume(d) => vec![
                "volume".into(),
                format!("{:.2}{}", d.abs() as f64 / 100.0, sign(d)),
            ],
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
