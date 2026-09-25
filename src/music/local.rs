//! Jellyfin and Plex: your own server, played here.

use super::{Backend, Cmd, Item, State, Track};
use crate::library::{self, Jukebox};
use anyhow::{Result, anyhow};
use std::time::Duration;

pub(super) struct Local {
    player: Jukebox,
    /// A dead server shouldn't be hammered every 1.5s — but it also shouldn't
    /// stay dead forever once it comes back, so back off and retry.
    failed: Option<String>,
    retry_at: Option<std::time::Instant>,
}

const RETRY_AFTER: Duration = Duration::from_secs(30);

impl Local {
    pub(super) fn new(catalog: Box<dyn library::Catalog>) -> Local {
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
        if self.player.queue_len() == 0
            && let Err(e) = self.player.ensure_queue()
        {
            self.failed = Some(e.to_string());
            self.retry_at = Some(std::time::Instant::now() + RETRY_AFTER);
            return Err(e);
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
