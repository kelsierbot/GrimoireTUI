//! YouTube Music, through th-ch/youtube-music's API Server plugin — and
//! YouTube Music's public web endpoint for what's in a playlist.

use super::{Backend, Cmd, Config, HTTP_TIMEOUT, Item, Modes, Repeat, State, Track, Update};
use anyhow::{Result, anyhow};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Ytm {
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
    pub(super) fn new(cfg: &Config, token: String) -> Ytm {
        let agent = |t| {
            ureq::Agent::config_builder()
                .timeout_global(Some(t))
                .build()
                .new_agent()
        };
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
        self.agent
            .delete(self.url(path))
            .header("Authorization", &self.bearer)
            .call()?;
        Ok(())
    }

    /// Put a song straight after the one playing.
    fn queue_next(&self, id: &str) -> Result<()> {
        self.post(
            "/queue",
            Some(serde_json::json!({ "videoId": id, "insertPosition": AFTER_CURRENT })),
        )
    }

    fn fetch_queue(&self) -> Result<serde_json::Value> {
        let mut res = self
            .slow
            .get(self.url("/queue"))
            .header("Authorization", &self.bearer)
            .call()?;
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
            ids.push(
                r.and_then(|r| r["videoId"].as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
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
    fn play_list(
        &self,
        id: &str,
        title: &str,
        now: bool,
        ticket: u64,
        note: &dyn Fn(String),
    ) -> Result<Option<Vec<Item>>> {
        use serde_json::json;
        let current = || self.loading.load(Ordering::SeqCst) == ticket;
        let (ids, more) = self.tracks(id)?;
        if ids.is_empty() {
            return Err(anyhow!(
                "“{title}” is private. Make it unlisted in YouTube Music to play it here"
            ));
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
        let cut = if more {
            format!(", the first {PLAYLIST_MAX}")
        } else {
            String::new()
        };
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
        Ok(self
            .agent
            .get(self.url(path))
            .header("Authorization", &self.bearer)
            .call()?)
    }

    fn post(&self, path: &str, body: Option<serde_json::Value>) -> Result<()> {
        let req = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.bearer);
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
        let s = res
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()?;
        Ok(serde_json::from_str(&s)?)
    }
}

impl Ytm {
    fn read_modes(&mut self) -> Result<Modes> {
        let repeat = match Ytm::read(&mut self.get("/repeat-mode")?)?["mode"].as_str() {
            Some("ALL") => Some(Repeat::All),
            Some("ONE") => Some(Repeat::One),
            Some("NONE") => Some(Repeat::Off),
            _ => None,
        };
        let shuffle = Ytm::read(&mut self.get("/shuffle")?)?["state"].as_bool();
        let vol = Ytm::read(&mut self.get("/volume")?)?;
        let liked = Ytm::read(&mut self.get("/like-state")?)?["state"]
            .as_str()
            .map(|s| s == "LIKE");
        Ok(Modes {
            repeat,
            shuffle,
            volume: vol["state"]
                .as_f64()
                .map(|v| v.round().clamp(0.0, 100.0) as u8),
            muted: vol["isMuted"].as_bool().unwrap_or(false),
            liked,
        })
    }
}

impl Backend for Ytm {
    fn modes(&mut self) -> Result<Modes> {
        self.read_modes()
    }

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
                // a straight one (pear-desktop #4458). Step on the scale it
                // reports — the one shown — so each press moves it by `d`
                // there (100 → 90 → 80), then convert what to set. Stepping
                // on the straight scale went 100 → 74 → 55.
                let heard = Ytm::read(&mut self.get("/volume")?)?["state"]
                    .as_f64()
                    .unwrap_or(50.0);
                let target = (heard + d as f64).clamp(0.0, 100.0);
                self.post("/volume", Some(json!({ "volume": volume_to_set(target) })))
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

    fn load_playlist(
        &mut self,
        id: String,
        title: String,
        now: bool,
        tell: Sender<Update>,
    ) -> Result<()> {
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
    it.get("playlistPanelVideoRenderer").or_else(|| {
        it.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer")
    })
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
                length: if parts.len() > 1 {
                    parts[parts.len() - 1].to_string()
                } else {
                    String::new()
                },
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
                if let Some(id) = r
                    .pointer("/playlistItemData/videoId")
                    .and_then(|x| x.as_str())
                {
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
            out.extend(
                card["contents"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(list_item),
            );
        }
        let rows = s
            .pointer("/itemSectionRenderer/contents")
            .or_else(|| s.pointer("/musicShelfRenderer/contents"));
        out.extend(
            rows.and_then(|r| r.as_array())
                .into_iter()
                .flatten()
                .filter_map(list_item),
        );
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
    let id = r
        .pointer("/playlistItemData/videoId")?
        .as_str()?
        .to_string();
    let cols: Vec<String> = r["flexColumns"]
        .as_array()?
        .iter()
        .map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"]))
        .collect();
    let item = playable(
        cols.first()?.clone(),
        cols.get(1).map_or("", String::as_str),
    )?;
    Some(Item { id, ..item })
}

/// A title and a byline like "Song • Fleetwood Mac • 4:18", if it names a
/// song or a video. Untyped rows (on the top-result card) are videos.
fn playable(title: String, byline: &str) -> Option<Item> {
    let parts: Vec<&str> = byline
        .split('•')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
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
        p.contains(':')
            && p.split(':')
                .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    Some(Item {
        title,
        artist: artist.unwrap_or_default().to_string(),
        length: parts
            .iter()
            .skip(rest)
            .find(is_length)
            .map_or(String::new(), |s| s.to_string()),
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
        assert_eq!(
            ids,
            ["swJOIjjW69U", "Y3ywicffOj4", "m8i5WiWCN-c"],
            "top result first; no album, episode or repeat"
        );
        assert_eq!(
            (r[0].artist.as_str(), r[0].length.as_str()),
            ("Fleetwood Mac", "4:18")
        );
        assert!(!r[0].video && r[1].video, "the untyped card row is a video");
        assert_eq!(
            (r[1].artist.as_str(), r[1].length.as_str()),
            ("FLEETWOOD MAC", "4:24")
        );
        assert_eq!(r[2].pos, 2);
    }

    #[test]
    fn queue_reads_both_renderer_shapes_and_marks_the_playing_track() {
        let track = |t: &str, sel: bool| {
            json!({
                "title": { "runs": [{ "text": t }] },
                "shortBylineText": { "runs": [{ "text": "Fleetwood Mac" }] },
                "lengthText": { "runs": [{ "text": "3:44" }] },
                "videoId": format!("id-{t}"),
                "selected": sel,
            })
        };
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
            (
                p[0].title.as_str(),
                p[0].artist.as_str(),
                p[0].length.as_str(),
                p[0].id.as_str()
            ),
            (
                "Best of Fleetwood",
                "Espershire",
                "84 songs",
                "PLnlwnADdLfE7MRO1KS4tU4WyqviJ-wZLI"
            )
        );
        assert_eq!(
            (p[1].artist.as_str(), p[1].length.as_str(), p[1].id.as_str()),
            (
                "Pramit Mohanty",
                "103K views",
                "PLJyx7idLrmwMu4DOoL1ybLgJr3YWnpiGm"
            ),
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
        assert_eq!(
            ids,
            ["a", "b", "a"],
            "unavailable songs skipped, repeats kept"
        );
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
        assert!(
            reorder(&q, 1, &want).0.is_empty(),
            "an ordered queue needs no moves"
        );
    }

    #[test]
    fn volume_converts_from_the_scale_it_reports() {
        assert_eq!(volume_to_set(0.0), 0);
        assert_eq!(volume_to_set(100.0), 100);
        assert_eq!(
            volume_to_set(15.0),
            43,
            "#4458: setting 43 reads back as 15"
        );
    }
}

#[cfg(test)]
mod volume_steps {
    use super::volume_to_set;

    /// What the client reports back for a value set on the straight scale:
    /// the inverse of `volume_to_set`.
    fn heard(set: i32) -> f64 {
        (16f64.powf(set as f64 / 100.0) - 1.0) / 0.15
    }

    #[test]
    fn each_press_moves_the_shown_volume_by_about_ten() {
        let mut v = 100.0;
        for want in [90.0, 80.0, 70.0, 60.0, 50.0] {
            v = heard(volume_to_set((v - 10.0f64).clamp(0.0, 100.0)));
            assert!(
                (v - want).abs() <= 2.0,
                "stepped to {v}, wanted about {want}"
            );
        }
    }
}
