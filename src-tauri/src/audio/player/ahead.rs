//! Resolving ahead: while a queued song plays, the streams of the next entries
//! are asked for, one at a time, so that moving on needs no run. Only their
//! videos are remembered here. The resolver keeps the streams, and no audio is
//! fetched. As with recovery, the player only decides: whoever watches it reads
//! the library and resolves, and hands the outcome back.

use std::fmt;

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

/// The video of each entry that was looked up, by the entry's key; `None` for
/// one without.
pub type Found = Vec<(u32, Option<String>)>;

/// The library reads the entries to come still need, to be run away from the
/// player.
pub struct Lookup {
    tracks: Arc<dyn TrackLookup>,
    /// Entry key and Track id.
    entries: Vec<(u32, Uuid)>,
}

impl fmt::Debug for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lookup")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl Lookup {
    /// Reads the library, so it belongs on a blocking thread.
    pub fn run(&self) -> Found {
        let video = |id| youtube_video(&self.tracks.track(id)?);
        let entries = self.entries.iter();
        entries.map(|&(key, id)| (key, video(id))).collect()
    }

    /// What to take if the lookup is lost: none of them has a video.
    pub fn unanswered(&self) -> Found {
        self.entries.iter().map(|&(key, _)| (key, None)).collect()
    }
}

#[derive(Debug, Default)]
pub(super) struct Ahead {
    /// What the library said of the entries to come. An entry's key is never
    /// used again, so this stays true.
    videos: Found,
    /// Whether a lookup is out.
    looking: bool,
    /// The resolve in progress.
    running: Option<Fetch>,
    /// By video, for the current entry and the ones resolved ahead only.
    settled: Vec<(String, Settled)>,
    /// The key of the song during which a resolve failed: no more are asked
    /// for while it is current.
    failed_during: Option<u32>,
}

impl Ahead {
    /// The video of the entry `key`, once it has been looked up.
    fn video(&self, key: u32) -> Option<&Option<String>> {
        let found = self.videos.iter().find(|(known, _)| *known == key);
        found.map(|(_, video)| video)
    }

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
        let keys: Vec<u32> = self.queue.coming(AHEAD).iter().map(|e| e.key).collect();
        self.ahead.videos.retain(|(key, _)| keys.contains(key));
        let current = self.direct.as_ref().and_then(|d| d.video.clone());
        let (coming, all_known) = self.coming_videos();
        // A failure is forgotten once its song is current: the usual rules
        // look after it from there.
        self.ahead.settled.retain(|(video, settled)| {
            if current.as_ref() == Some(video) {
                matches!(settled, Settled::Ready { .. })
            } else {
                !all_known || coming.contains(video)
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
                // Until the library has been read, it may still be the one.
                !in_the_way && next.map_or(!all_known, |video| video == running)
            }
        };
        if !wanted {
            self.ahead.running = None;
        }
    }

    /// Gives up resolving ahead and forgets what it found, as while nothing may
    /// be asked of YouTube.
    pub fn stop_ahead(&mut self) {
        self.ahead.running = None;
        self.ahead.settled.clear();
        self.ahead.failed_during = None;
    }

    /// The library reads to make before more can be resolved ahead, if any are
    /// needed and none are out.
    pub fn due_lookup(&mut self) -> Option<Lookup> {
        if self.ahead.looking {
            return None;
        }
        let entries: Vec<(u32, Uuid)> = self
            .queue
            .coming(AHEAD)
            .iter()
            .filter(|entry| self.catalog.track(entry.track).is_none())
            .filter(|entry| self.ahead.video(entry.key).is_none())
            .map(|entry| (entry.key, entry.track))
            .collect();
        if entries.is_empty() {
            return None;
        }
        self.ahead.looking = true;
        Some(Lookup {
            tracks: Arc::clone(&self.tracks),
            entries,
        })
    }

    /// Takes what a lookup found.
    pub fn learn(&mut self, found: Found) {
        self.ahead.looking = false;
        self.ahead.videos.extend(found);
    }

    #[cfg(test)]
    pub fn looking_up(&self) -> bool {
        self.ahead.looking
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
        let (coming, _) = self.coming_videos();
        let video = coming.into_iter().find(|video| {
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

    /// Asked when the run of `ticket` could start, also after it has waited its
    /// turn: whether it still may. None starts while the song is paused or
    /// loading, unless the song is waiting for this very run. One held back is
    /// given up, not failed, and is asked for again later.
    pub fn may_run_ahead(&mut self, ticket: u32) -> bool {
        if !self.awaits_ahead(ticket) {
            return false;
        }
        let may = self.resolving_current() || (self.playing && self.phase == Phase::Open);
        if !may {
            self.ahead.running = None;
        }
        may
    }

    /// Takes the outcome of the resolve ahead `ticket`: when the stream found
    /// expires (Unix seconds), or why there is none. A stale one changes
    /// nothing. If the song waits for it, a failure is that song's answer too,
    /// so that it is not asked for a second time.
    pub fn finish_ahead(
        &mut self,
        ticket: u32,
        outcome: Result<Option<u64>, NoStream>,
    ) -> Option<PlayerState> {
        if !self.awaits_ahead(ticket) {
            return None;
        }
        let awaited = self.resolving_current();
        let Fetch { video, .. } = self.ahead.running.take()?;
        let expires_at = outcome.as_ref().ok().copied().flatten();
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
        let no = outcome.err().filter(|_| awaited)?;
        let fetch = self.due_fetch()?;
        self.finish_fetch(fetch.ticket, Err(no))
    }

    /// The videos of the entries that play next, nearest first, as far as the
    /// library has been read, and whether that is all of them. A sample or a
    /// song without a YouTube source has none.
    fn coming_videos(&self) -> (Vec<String>, bool) {
        let mut videos = Vec::new();
        for entry in self.queue.coming(AHEAD) {
            if self.catalog.track(entry.track).is_some() {
                continue;
            }
            let Some(video) = self.ahead.video(entry.key) else {
                return (videos, false);
            };
            videos.extend(video.clone());
        }
        (videos, true)
    }
}
