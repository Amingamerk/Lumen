//! What the player tells the play log. One play starts when audio starts, and
//! it counts only the time it actually sounds: a pause, a seek, a stream that
//! is being replaced or a failed load add no rows and no listened time.

use std::time::Duration;

use super::*;
use crate::catalog::{SourceKind, SourceRef, Track};
use crate::library::{EndReason, PlayId, PlayLog};

/// The play being listened to.
pub(super) struct Listen {
    /// The Queue entry or direct playback it belongs to.
    key: u32,
    play: PlayId,
    /// Time already counted.
    listened: Duration,
    /// When the sound last started, while it sounds.
    since: Option<Duration>,
}

impl<E: Engine> Player<E> {
    /// Where plays are written. Without one, nothing is.
    pub fn set_log(&mut self, log: Box<dyn PlayLog>) {
        self.log = log;
    }

    #[cfg(test)]
    pub fn with_log(mut self, log: Box<dyn PlayLog>) -> Self {
        self.set_log(log);
        self
    }

    /// Closes the play in progress because the app is closing. Nothing is
    /// logged after it.
    pub fn shutdown(&mut self) {
        self.end_listen(EndReason::Exit);
        self.log_closed = true;
    }

    /// Ends the play in progress, if there is one.
    pub(super) fn end_listen(&mut self, reason: EndReason) {
        let now = self.clock.monotonic();
        if let Some(listen) = self.listen.take() {
            let running = listen
                .since
                .map_or(Duration::ZERO, |s| now.saturating_sub(s));
            self.log
                .ended(listen.play, listen.listened + running, reason);
        }
    }

    /// Brings the play in progress in line with what the player now does. Run
    /// after every change.
    pub(super) fn track_listening(&mut self) {
        let now = self.clock.monotonic();
        let key = self.current_key();
        let reason = self.end_reason.take();
        if self.listen.as_ref().is_some_and(|l| Some(l.key) != key) {
            self.end_listen(reason.unwrap_or(EndReason::Replaced));
        }
        match self.phase {
            Phase::Ended => self.end_listen(EndReason::Finished),
            Phase::Failed(_) => self.end_listen(EndReason::Failed),
            _ => {}
        }
        let sounding = key.is_some() && self.phase == Phase::Open && self.playing;
        match (&mut self.listen, sounding) {
            (Some(listen), true) => {
                listen.since.get_or_insert(now);
            }
            (Some(listen), false) => {
                if let Some(since) = listen.since.take() {
                    listen.listened += now.saturating_sub(since);
                }
            }
            (None, true) => self.begin_listen(key),
            (None, false) => {}
        }
    }

    fn begin_listen(&mut self, key: Option<u32>) {
        let Some(key) = key.filter(|_| !self.log_closed) else {
            return;
        };
        let Some(track) = self.loggable() else {
            return;
        };
        let play = self.log.started(&track, self.clock.wall_ms());
        self.listen = Some(Listen {
            key,
            play,
            listened: Duration::ZERO,
            since: Some(self.clock.monotonic()),
        });
    }

    /// The Queue entry or direct playback that is current.
    fn current_key(&self) -> Option<u32> {
        match &self.direct {
            Some(direct) => Some(direct.key),
            None => self.queue.current().map(|entry| entry.key),
        }
    }

    /// The song that is current, if it is one the library can keep: not a
    /// local file.
    fn loggable(&self) -> Option<Track> {
        match &self.direct {
            Some(direct) => {
                let video = direct.video.clone()?;
                let row = &direct.row;
                if row.id.is_nil() {
                    return None;
                }
                Some(Track {
                    id: row.id,
                    title: row.title.clone(),
                    artists: row.artists.clone(),
                    album: row.album.clone(),
                    duration_ms: row.duration_ms,
                    artwork_url: None,
                    isrc: None,
                    sources: vec![SourceRef {
                        kind: SourceKind::YouTube,
                        id: video,
                    }],
                })
            }
            None => {
                let entry = self.queue.current()?;
                self.catalog.track(entry.track).cloned()
            }
        }
    }

    /// The current song as the library would keep it, for liking it.
    pub fn current_track(&self) -> Option<Track> {
        self.loggable()
    }
}

#[cfg(test)]
mod tests;
