//! Playing from your own server.
//!
//! Jellyfin and Plex are the one case where Grimoire can legitimately *play*
//! rather than remote-control. These are your files on your hardware — no
//! terms of service to violate, no subscription to side-step. So unlike the
//! YouTube Music and Spotify sources, nothing else has to be running.
//!
//! Both servers differ only in how you ask them for a track list and a stream
//! URL, so that is all `Catalog` abstracts. Everything after — queue, decode,
//! transport, position — is shared.

use anyhow::{Context, Result, anyhow};
use std::time::Duration;

const AGENT_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT: &str = "Grimoire";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Enough to play a track and say what it is.
#[derive(Debug, Clone)]
pub struct TrackRef {
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub url: String,
}

/// Where a track list comes from. The only thing the two servers disagree on.
pub trait Catalog: Send {
    fn fetch(&self, limit: usize) -> Result<Vec<TrackRef>>;
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(AGENT_TIMEOUT))
        .build()
        .new_agent()
}

// ── Jellyfin ─────────────────────────────────────────────────────────

pub struct Jellyfin {
    pub server: String,
    pub token: String,
    pub user_id: String,
}

impl Jellyfin {
    /// Log in with the credentials you already use for the web UI, rather than
    /// making someone hunt through the dashboard for an API key.
    pub fn login(server: &str, user: &str, pass: &str) -> Result<(String, String)> {
        let server = server.trim_end_matches('/');
        let auth = format!(
            "MediaBrowser Client=\"{CLIENT}\", Device=\"{CLIENT}\", DeviceId=\"grimoire-tui\", Version=\"{VERSION}\""
        );
        let mut res = agent()
            .post(format!("{server}/Users/AuthenticateByName"))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .send_json(serde_json::json!({ "Username": user, "Pw": pass }))
            .map_err(|e| anyhow!("Jellyfin rejected the login ({e})"))?;

        let v: serde_json::Value = res.body_mut().read_json()?;
        let token = v["AccessToken"]
            .as_str()
            .ok_or_else(|| anyhow!("no AccessToken in the reply"))?
            .to_string();
        let uid = v["User"]["Id"]
            .as_str()
            .ok_or_else(|| anyhow!("no user id in the reply"))?
            .to_string();
        Ok((token, uid))
    }
}

impl Catalog for Jellyfin {
    fn fetch(&self, limit: usize) -> Result<Vec<TrackRef>> {
        let server = self.server.trim_end_matches('/');
        let url = format!(
            "{server}/Items?userId={}&IncludeItemTypes=Audio&Recursive=true&SortBy=Random&Limit={limit}&Fields=RunTimeTicks",
            self.user_id
        );
        let mut res = agent()
            .get(&url)
            .header("X-Emby-Token", &self.token)
            .call()
            .context("asking Jellyfin for tracks")?;
        let v: serde_json::Value = res.body_mut().read_json()?;

        let items = v["Items"].as_array().cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|it| {
                let id = it["Id"].as_str()?;
                Some(TrackRef {
                    title: it["Name"].as_str().unwrap_or("untitled").to_string(),
                    artist: it["Artists"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|a| a.as_str())
                        .or_else(|| it["AlbumArtist"].as_str())
                        .unwrap_or("")
                        .to_string(),
                    // Jellyfin counts in 100-nanosecond ticks.
                    duration: it["RunTimeTicks"].as_f64().unwrap_or(0.0) / 10_000_000.0,
                    url: format!(
                        "{server}/Audio/{id}/stream?static=true&api_key={}",
                        self.token
                    ),
                })
            })
            .collect())
    }
}

// ── Plex ─────────────────────────────────────────────────────────────

pub struct Plex {
    pub server: String,
    pub token: String,
}

impl Plex {
    /// Find the first music library on the server.
    fn music_section(&self) -> Result<String> {
        let server = self.server.trim_end_matches('/');
        let mut res = agent()
            .get(format!("{server}/library/sections?X-Plex-Token={}", self.token))
            .header("Accept", "application/json")
            .call()
            .context("asking Plex for its libraries")?;
        let v: serde_json::Value = res.body_mut().read_json()?;
        v["MediaContainer"]["Directory"]
            .as_array()
            .and_then(|d| {
                d.iter()
                    .find(|s| s["type"].as_str() == Some("artist"))
                    .and_then(|s| s["key"].as_str())
                    .map(str::to_string)
            })
            .ok_or_else(|| anyhow!("no music library found on this Plex server"))
    }
}

impl Catalog for Plex {
    fn fetch(&self, limit: usize) -> Result<Vec<TrackRef>> {
        let server = self.server.trim_end_matches('/');
        let section = self.music_section()?;
        // type=10 is a track.
        let url = format!(
            "{server}/library/sections/{section}/all?type=10&X-Plex-Token={}&X-Plex-Container-Start=0&X-Plex-Container-Size={limit}",
            self.token
        );
        let mut res = agent()
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .context("asking Plex for tracks")?;
        let v: serde_json::Value = res.body_mut().read_json()?;

        let items = v["MediaContainer"]["Metadata"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|it| {
                let part = it["Media"]
                    .as_array()?
                    .first()?["Part"]
                    .as_array()?
                    .first()?["key"]
                    .as_str()?;
                Some(TrackRef {
                    title: it["title"].as_str().unwrap_or("untitled").to_string(),
                    artist: it["grandparentTitle"]
                        .as_str()
                        .or_else(|| it["originalTitle"].as_str())
                        .unwrap_or("")
                        .to_string(),
                    duration: it["duration"].as_f64().unwrap_or(0.0) / 1000.0,
                    url: format!("{server}{part}?X-Plex-Token={}", self.token),
                })
            })
            .collect())
    }
}

// ── the player ───────────────────────────────────────────────────────

/// Owns the queue and, when built with the `audio` feature, the output device.
/// Named to avoid colliding with `rodio::Player`, which is rodio's sink.
pub struct Jukebox {
    catalog: Box<dyn Catalog>,
    queue: Vec<TrackRef>,
    idx: usize,
    started: bool,
    #[cfg(feature = "audio")]
    audio: Option<Audio>,
}

#[cfg(feature = "audio")]
struct Audio {
    // Held only to keep the device open; dropping it silences everything.
    _device: rodio::MixerDeviceSink,
    sink: rodio::Player,
}

impl Jukebox {
    pub fn new(catalog: Box<dyn Catalog>) -> Jukebox {
        Jukebox {
            catalog,
            queue: Vec::new(),
            idx: 0,
            started: false,
            #[cfg(feature = "audio")]
            audio: None,
        }
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn current(&self) -> Option<&TrackRef> {
        self.queue.get(self.idx)
    }

    /// Load a queue if we don't have one. Cheap to call repeatedly.
    pub fn ensure_queue(&mut self) -> Result<()> {
        if self.queue.is_empty() {
            self.queue = self.catalog.fetch(300)?;
            self.idx = 0;
        }
        Ok(())
    }

    #[cfg(feature = "audio")]
    fn device(&mut self) -> Result<&mut Audio> {
        if self.audio.is_none() {
            let device = rodio::DeviceSinkBuilder::open_default_sink()
                .map_err(|e| anyhow!("no audio output available ({e})"))?;
            let sink = rodio::Player::connect_new(device.mixer());
            self.audio = Some(Audio {
                _device: device,
                sink,
            });
        }
        Ok(self.audio.as_mut().expect("just created"))
    }

    /// Fetch to a temp file, then decode. Streaming decode would need a
    /// Read+Seek source, and a few megabytes on disk is a fair trade for
    /// formats that seek reliably.
    #[cfg(feature = "audio")]
    fn load(&mut self, i: usize) -> Result<()> {
        let Some(track) = self.queue.get(i).cloned() else {
            return Ok(());
        };
        let mut res = agent().get(&track.url).call().context("fetching the track")?;
        let tmp = std::env::temp_dir().join("grimoire-now-playing.audio");
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut res.body_mut().as_reader(), &mut out)?;
        drop(out);

        let file = std::io::BufReader::new(std::fs::File::open(&tmp)?);
        let decoded = rodio::Decoder::new(file).context("decoding the track")?;

        let audio = self.device()?;
        audio.sink.clear();
        audio.sink.append(decoded);
        audio.sink.play();
        self.idx = i;
        self.started = true;
        Ok(())
    }

    #[cfg(not(feature = "audio"))]
    fn load(&mut self, _i: usize) -> Result<()> {
        Err(anyhow!(
            "this build has no audio support — rebuild with the `audio` feature"
        ))
    }

    #[cfg(feature = "audio")]
    pub fn toggle(&mut self) -> Result<()> {
        self.ensure_queue()?;
        if !self.started {
            let i = self.idx;
            return self.load(i);
        }
        let audio = self.device()?;
        if audio.sink.is_paused() {
            audio.sink.play();
        } else {
            audio.sink.pause();
        }
        Ok(())
    }

    #[cfg(not(feature = "audio"))]
    pub fn toggle(&mut self) -> Result<()> {
        self.load(0)
    }

    pub fn step(&mut self, delta: isize) -> Result<()> {
        self.ensure_queue()?;
        if self.queue.is_empty() {
            return Ok(());
        }
        let n = self.queue.len() as isize;
        let next = ((self.idx as isize + delta) % n + n) % n;
        // Move the cursor first: a track that fails to fetch or decode should
        // not strand you on it.
        self.idx = next as usize;
        self.load(self.idx)
    }

    /// Position in seconds, whether it's paused, and whether the track ended.
    #[cfg(feature = "audio")]
    pub fn progress(&mut self) -> (f64, bool, bool) {
        match &self.audio {
            Some(a) => (
                a.sink.get_pos().as_secs_f64(),
                !a.sink.is_paused(),
                a.sink.empty(),
            ),
            None => (0.0, false, false),
        }
    }

    #[cfg(not(feature = "audio"))]
    pub fn progress(&mut self) -> (f64, bool, bool) {
        (0.0, false, false)
    }

    pub fn started(&self) -> bool {
        self.started
    }

    /// Roll onto the next track when the current one runs out.
    pub fn advance_if_finished(&mut self) {
        let (_, _, ended) = self.progress();
        if self.started && ended {
            let _ = self.step(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<TrackRef>);
    impl Catalog for Fake {
        fn fetch(&self, _l: usize) -> Result<Vec<TrackRef>> {
            Ok(self.0.clone())
        }
    }

    fn t(name: &str) -> TrackRef {
        TrackRef {
            title: name.into(),
            artist: "A".into(),
            duration: 100.0,
            url: String::new(),
        }
    }

    #[test]
    fn the_queue_loads_once_and_starts_at_the_top() {
        let mut p = Jukebox::new(Box::new(Fake(vec![t("one"), t("two"), t("three")])));
        assert_eq!(p.queue_len(), 0);
        p.ensure_queue().unwrap();
        assert_eq!(p.queue_len(), 3);
        assert_eq!(p.current().unwrap().title, "one");
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut p = Jukebox::new(Box::new(Fake(vec![t("one"), t("two"), t("three")])));
        p.ensure_queue().unwrap();
        // load() fails without a real device or URL; index movement is what matters.
        let _ = p.step(-1);
        assert_eq!(p.idx, 2, "stepping back from the first track wraps to the last");
        let _ = p.step(1);
        assert_eq!(p.idx, 0, "and forward from the last wraps to the first");
    }

    #[test]
    fn an_empty_library_does_not_panic() {
        let mut p = Jukebox::new(Box::new(Fake(vec![])));
        assert!(p.ensure_queue().is_ok());
        assert!(p.step(1).is_ok());
        assert!(p.current().is_none());
    }
}
