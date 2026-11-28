//! Queued playback of library Tracks: each entry that becomes current is
//! resolved on its own turn, and one that cannot be played is passed over.

use super::*;
use crate::catalog::{SourceKind, Track};

/// Where the Queue's Tracks are found by id: the library, in the app.
pub trait TrackLookup: Send {
    fn track(&self, id: Uuid) -> Option<Track>;
}

/// Knows no Track.
pub struct NoTracks;

impl TrackLookup for NoTracks {
    fn track(&self, _: Uuid) -> Option<Track> {
        None
    }
}

/// Where a queued song goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    /// Right after the current song.
    Next,
    /// After what the user queued, ahead of the Recommender's picks.
    Last,
}

/// Songs passed over one after the other before playback stops, so a list of
/// songs that all fail does not send a request for each.
const MAX_SKIPS: usize = 5;

impl<E: Engine> Player<E> {
    pub fn set_tracks(&mut self, tracks: Box<dyn TrackLookup>) {
        self.tracks = tracks;
    }

    /// Queues the Tracks by id and plays the one at `start`. Nothing is looked
    /// up but that one.
    pub fn play_list(
        &mut self,
        tracks: Vec<Uuid>,
        start: usize,
    ) -> Result<PlayerState, PlayerError> {
        self.queue.replace(tracks, start)?;
        self.skipped = None;
        self.skips = 0;
        self.start_entry();
        Ok(self.commit())
    }

    /// Adds a song the library knows to the Queue. The current song is left
    /// alone: it is neither restarted nor reloaded. With nothing in the Queue
    /// the song plays at once, unless a stand-alone song is playing: that one
    /// becomes the current entry and the new song follows it.
    pub fn enqueue(
        &mut self,
        id: Uuid,
        placement: Placement,
    ) -> Result<Option<PlayerState>, PlayerError> {
        if self.find(id).is_none() {
            return Err(PlayerError::UnknownTrack { id });
        }
        if self.queue.current().is_none() {
            let standalone = self
                .direct
                .as_ref()
                .filter(|_| !matches!(self.phase, Phase::Failed(_)));
            let Some(direct) = standalone else {
                return self.play_list(vec![id], 0).map(Some);
            };
            self.queue.adopt(direct.key, direct.row.id);
            if let Some(direct) = &mut self.direct {
                direct.queued = true;
            }
        }
        match placement {
            Placement::Next => self.queue.insert_next(id),
            Placement::Last => self.queue.insert_last(id),
        };
        Ok(Some(self.commit()))
    }

    /// Takes one upcoming entry out. The current one is refused.
    pub fn remove_queue_entry(&mut self, key: u32) -> Result<PlayerState, PlayerError> {
        self.queue.remove(key)?;
        Ok(self.commit())
    }

    /// Drops every upcoming entry, if there are any.
    pub fn clear_queue(&mut self) -> Option<PlayerState> {
        self.queue.clear_upcoming().then(|| self.commit())
    }

    /// Up to `limit` entries before the current one, the latest first.
    pub fn history_slice(&self, offset: usize, limit: usize) -> QueueSlice {
        let history = self.queue.history();
        QueueSlice {
            queue_revision: self.queue.revision(),
            offset,
            total: history.len(),
            items: history
                .iter()
                .rev()
                .skip(offset)
                .take(limit.min(MAX_SLICE))
                .filter_map(|entry| {
                    let (key, track, autoplay) = self.row(entry)?;
                    Some(QueueItem {
                        key,
                        track,
                        autoplay,
                    })
                })
                .collect(),
        }
    }

    /// The Track of an entry: a catalog sample, or a Track of the library.
    pub(super) fn find(&self, id: Uuid) -> Option<Track> {
        self.catalog
            .track(id)
            .cloned()
            .or_else(|| self.tracks.track(id))
    }

    /// Starts the current entry. A sample plays at once. A library Track shows
    /// as loading and waits for its stream, which the watcher resolves; one that
    /// has no source to play from is passed over.
    pub(super) fn start_entry(&mut self) {
        let Some(entry) = self.queue.current().copied() else {
            return;
        };
        if self.catalog.track(entry.track).is_some() {
            self.direct = None;
            self.play_current();
            return;
        }
        let track = self.tracks.track(entry.track);
        let video = track.as_ref().and_then(youtube_video);
        let row = track.as_ref().map_or_else(
            || TrackRow::untagged("A missing song".to_owned()),
            TrackRow::from,
        );
        self.engine.stop();
        self.direct = Some(Direct {
            key: entry.key,
            queued: true,
            media: None,
            video: video.clone(),
            row,
            recovery: Recovery::default(),
        });
        self.phase = Phase::Loading;
        self.set_playing(true);
        match (track, video) {
            (_, Some(_)) => self.plan_replacement(),
            (Some(_), None) => self.skip_entry("it has no YouTube Music source"),
            (None, None) => self.skip_entry("it is not in the library any more"),
        }
    }

    /// The current queued song cannot be played. Says so and goes on with the
    /// next entry; with none to go on to, or too many passed over, it stops.
    pub(super) fn skip_entry(&mut self, reason: &str) {
        let Some(direct) = &self.direct else {
            return;
        };
        let skipped = Box::new(Skipped {
            key: direct.key,
            message: format!("Lumen skipped “{}”: {reason}.", direct.row.title),
        });
        // A repeating Queue does not come round to a song it just passed over.
        let entries = self.queue.history().len() + 1 + self.queue.upcoming().len();
        let round = self.queue.repeat() == Repeat::All && self.skips + 1 >= entries;
        if self.skips < MAX_SKIPS && !round && self.advance() {
            self.skips += 1;
            self.skipped = Some(skipped);
            self.start_entry();
        } else {
            self.stop_recovering(Some(reason));
        }
    }
}

/// The YouTube video a Track plays from.
pub(super) fn youtube_video(track: &Track) -> Option<String> {
    track
        .sources
        .iter()
        .find(|source| source.kind == SourceKind::YouTube)
        .map(|source| source.id.clone())
}

#[cfg(test)]
mod tests;
