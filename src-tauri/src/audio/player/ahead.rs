//! Resolving ahead: while a queued song plays, the streams of the next entries
//! are asked for, one at a time, so that moving on needs no run. Only their
//! videos are remembered here. The resolver keeps the streams, and no audio is
//! fetched. As with recovery, the player only decides.

use super::entries::youtube_video;
use super::*;
use crate::resolver::REUSE_MS;

/// How many entries after the current one are resolved ahead.
const AHEAD: usize = 2;

/// What became of an entry that was asked for ahead.
#[derive(Debug, Clone, Copy)]
enum Settled {
    /// Its stream can be used without a run until then (Unix milliseconds).
    Ready { until_ms: u64 },
    /// It is left until it is current.
    Failed,
}

#[derive(Debug, Default)]
pub(super) struct Ahead {
    /// The resolve in progress.
    running: Option<Fetch>,
    /// By video, for the current entry and the ones resolved ahead only.
    settled: Vec<(String, Settled)>,
    /// The key of the song during which a resolve failed: no more are asked
    /// for while it is current.
    failed_during: Option<u32>,
}

impl Ahead {
    fn settled(&self, video: &str) -> Option<Settled> {
        let found = self.settled.iter().find(|(known, _)| known == video);
        found.map(|&(_, settled)| settled)
    }

    /// Whether the stream of `video` is known to need no run.
    fn ready(&self, video: &str, now_ms: u64) -> bool {
        matches!(self.settled(video), Some(Settled::Ready { until_ms }) if now_ms < until_ms)
    }

    /// Whether asking for `video` ahead now would be of use.
    fn unsettled(&self, video: &str, now_ms: u64) -> bool {
        match self.settled(video) {
            None => true,
            // A stream that has grown too old is resolved again.
            Some(Settled::Ready { until_ms }) => now_ms >= until_ms,
            Some(Settled::Failed) => false,
        }
    }

    /// Drops what is known of `video`, as when its stream turned out bad.
    pub(super) fn forget(&mut self, video: &str) {
        self.settled.retain(|(known, _)| known != video);
    }

    /// The video being resolved.
    pub(super) fn resolving(&self) -> Option<&str> {
        self.running.as_ref().map(|fetch| fetch.video.as_str())
    }
}

impl<E: Engine> Player<E> {
    /// Gives up a resolve ahead that is no longer the one to run, and forgets
    /// what has had its turn or left the window. Called after every change.
    pub fn review_ahead(&mut self) {
        let current = self.direct.as_ref().and_then(|d| d.video.clone());
        let coming = self.coming_videos();
        // A failure is forgotten once its song is current: the usual rules
        // look after it from there.
        self.ahead.settled.retain(|(video, settled)| {
            if current.as_ref() == Some(video) {
                matches!(settled, Settled::Ready { .. })
            } else {
                coming.contains(video)
            }
        });
        let key = self.direct.as_ref().map(|direct| direct.key);
        if self.ahead.failed_during != key {
            self.ahead.failed_during = None;
        }
        let Some(running) = self.ahead.resolving() else {
            return;
        };
        let now_ms = self.clock.wall_ms();
        let wanted = match &current {
            // The song moved on to it: its answer is what the song waits for.
            Some(current) if current == running => true,
            current => {
                // A current song that needs a run of its own goes first.
                let in_the_way = self.fetch_pending()
                    && current
                        .as_ref()
                        .is_some_and(|video| !self.ahead.ready(video, now_ms));
                let next = coming.iter().find(|video| {
                    current.as_ref() != Some(*video) && self.ahead.unsettled(video, now_ms)
                });
                !in_the_way && next.is_some_and(|video| video == running)
            }
        };
        if !wanted {
            self.ahead.running = None;
        }
    }

    /// Gives up resolving ahead and forgets what it found, as while nothing may
    /// be asked of YouTube.
    pub fn stop_ahead(&mut self) {
        self.ahead = Ahead::default();
    }

    /// The next entry to resolve ahead, if one is due: only while a queued song
    /// plays, and only one at a time.
    pub fn due_ahead(&mut self) -> Option<Fetch> {
        let direct = self.direct.as_ref().filter(|direct| direct.queued)?;
        if self.ahead.running.is_some()
            || !self.playing
            || self.phase != Phase::Open
            || self.ahead.failed_during == Some(direct.key)
        {
            return None;
        }
        let now_ms = self.clock.wall_ms();
        let video = self.coming_videos().into_iter().find(|video| {
            direct.video.as_ref() != Some(video) && self.ahead.unsettled(video, now_ms)
        })?;
        self.tickets = self.tickets.wrapping_add(1);
        let fetch = Fetch {
            ticket: self.tickets,
            video,
        };
        self.ahead.running = Some(fetch.clone());
        Some(fetch)
    }

    /// Whether the resolve ahead `ticket` is still wanted.
    pub fn awaits_ahead(&self, ticket: u32) -> bool {
        self.ahead
            .running
            .as_ref()
            .is_some_and(|fetch| fetch.ticket == ticket)
    }

    /// Takes the outcome of the resolve ahead `ticket`: when the stream found
    /// expires (Unix seconds), or `None` without one. A stale one changes
    /// nothing.
    pub fn finish_ahead(&mut self, ticket: u32, expires_at: Option<u64>) {
        if !self.awaits_ahead(ticket) {
            return;
        }
        let Some(Fetch { video, .. }) = self.ahead.running.take() else {
            return;
        };
        let until_ms = expires_at.map_or(0, |at| at.saturating_mul(1_000).saturating_sub(REUSE_MS));
        // One that is too old already would only be asked for over and over.
        let settled = if until_ms > self.clock.wall_ms() {
            Settled::Ready { until_ms }
        } else {
            self.ahead.failed_during = self.direct.as_ref().map(|direct| direct.key);
            Settled::Failed
        };
        self.ahead.forget(&video);
        self.ahead.settled.push((video, settled));
    }

    /// The videos of the entries that play next, nearest first. A sample or a
    /// song without a YouTube source has none.
    fn coming_videos(&self) -> Vec<String> {
        self.queue
            .coming(AHEAD)
            .iter()
            .filter(|entry| self.catalog.track(entry.track).is_none())
            .filter_map(|entry| youtube_video(&self.tracks.track(entry.track)?))
            .collect()
    }
}
