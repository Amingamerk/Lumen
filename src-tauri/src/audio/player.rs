//! Transport rules. The Queue says what comes next, the engine keeps time.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::{
    Engine, EngineError, EngineEvent, Media, NowPlaying, PlaybackStatus, PlayerEvent,
    PlayerPosition, PlayerState, QueueItem, QueueSlice,
};
use crate::catalog::{Catalog, MAX_SLICE, TrackRow};
use crate::queue::{Entry, Queue, QueueError};
use crate::recs::{RECENT_HISTORY, Recommender};
use crate::sources::youtube::SystemClock;

mod recovery;

use recovery::PlayerClock;
use recovery::Recovery;
pub use recovery::{Fetch, NoStream};

pub const DEFAULT_VOLUME: u8 = 80;
/// Previous restarts the current track once it is further in than this.
pub const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// How many Recommender picks wait in the Queue once autoplay has started.
pub const AUTOPLAY_AHEAD: usize = 5;

#[cfg(test)]
use super::SimulatedEngine;
#[cfg(test)]
use super::simulated::Clock;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlayerError {
    #[error("no playlist or album has the id “{id}”")]
    UnknownCollection { id: String },
    #[error("no track has the id {id}")]
    UnknownTrack { id: Uuid },
    #[error(transparent)]
    Queue(#[from] QueueError),
    #[error("sample track, cannot be played")]
    SampleTrack,
}

/// How far the current media has come.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Loading,
    Open,
    Ended,
    /// A live stream was let go of: a fresh URL is awaited, or a resume.
    Broken,
    /// With the words the user is told.
    Failed(String),
}

/// Real audio, which stands outside the Queue until Milestone 4.
struct Direct {
    key: u32,
    /// `None` while its stream is being resolved, or if that failed.
    media: Option<Media>,
    /// The YouTube video a resolved stream is of.
    video: Option<String>,
    row: TrackRow,
    recovery: Recovery,
}

pub struct Player<E: Engine> {
    engine: E,
    catalog: Arc<Catalog>,
    recommender: Box<dyn Recommender>,
    queue: Queue,
    /// Set while real audio is loaded. The Queue is empty then.
    direct: Option<Direct>,
    phase: Phase,
    /// Whether it should be playing, also while still loading.
    playing: bool,
    volume: u8,
    revision: u64,
    /// The whole second of the track the UI last heard about.
    reported_second: u64,
    /// The duration the UI last heard about.
    reported_duration_ms: u32,
    clock: Box<dyn PlayerClock>,
    /// Told of every change.
    wake: Option<Box<dyn Fn() + Send>>,
    /// The last fetch number given out.
    tickets: u32,
    /// Both recovery attempts have just been used up.
    gave_up: bool,
    /// The video whose stream was just let go of.
    dropped: Option<String>,
}

/// Commands that may have nothing to change return `None` when they do not.
impl<E: Engine> Player<E> {
    pub fn new(mut engine: E, catalog: Arc<Catalog>, recommender: Box<dyn Recommender>) -> Self {
        engine.set_volume(DEFAULT_VOLUME);
        Self {
            engine,
            catalog,
            recommender,
            queue: Queue::default(),
            direct: None,
            phase: Phase::Open,
            playing: false,
            volume: DEFAULT_VOLUME,
            revision: 0,
            reported_second: 0,
            reported_duration_ms: 0,
            clock: Box::new(SystemClock::default()),
            wake: None,
            tickets: 0,
            gave_up: false,
            dropped: None,
        }
    }

    pub fn state(&self) -> PlayerState {
        let queued = self.queue.current().and_then(|entry| {
            let (key, track, autoplay) = self.row(entry)?;
            Some(NowPlaying {
                key,
                track,
                autoplay,
            })
        });
        let duration_ms = millis(self.duration());
        let current = match &self.direct {
            Some(direct) => {
                let mut track = direct.row.clone();
                if duration_ms > 0 {
                    track.duration_ms = duration_ms;
                }
                Some(NowPlaying {
                    key: direct.key,
                    track,
                    autoplay: false,
                })
            }
            None => queued,
        };
        let status = match &self.phase {
            _ if current.is_none() => PlaybackStatus::Idle,
            Phase::Failed(_) => PlaybackStatus::Error,
            Phase::Ended => PlaybackStatus::Ended,
            _ if !self.playing => PlaybackStatus::Paused,
            _ if self
                .direct
                .as_ref()
                .is_some_and(|d| d.recovery.reconnecting) =>
            {
                PlaybackStatus::Reconnecting
            }
            Phase::Loading | Phase::Broken => PlaybackStatus::Loading,
            Phase::Open => PlaybackStatus::Playing,
        };
        let queued = self.queue.current().is_some();
        let upcoming = self.queue.upcoming().len();
        PlayerState {
            revision: self.revision,
            status,
            // The engine's last word can fall just short of the end.
            position_ms: match self.phase {
                Phase::Ended => duration_ms,
                _ => millis(self.position()),
            },
            duration_ms,
            volume: self.volume,
            can_next: queued && (upcoming > 0 || self.recommendation().is_some()),
            can_previous: queued,
            upcoming,
            queue_revision: self.queue.revision(),
            error: match &self.phase {
                Phase::Failed(message) => Some(message.clone()),
                _ => None,
            },
            current,
        }
    }

    /// Plays a file or stream by itself: the Queue is emptied, so nothing comes
    /// before or after it. A failure to load it shows in the state.
    pub fn play_media(&mut self, media: Media, row: TrackRow) -> PlayerState {
        let key = self.queue.clear();
        self.direct = Some(Direct {
            key,
            media: Some(media),
            video: None,
            row,
            recovery: Recovery::default(),
        });
        self.load(Duration::ZERO, true);
        self.commit()
    }

    /// Shows a song as loading while its stream is resolved: what played before
    /// is let go of, and the Queue is emptied. The key names this attempt.
    pub fn begin_stream(&mut self, video: String, row: TrackRow) -> (u32, PlayerState) {
        let key = self.queue.clear();
        self.engine.stop();
        self.direct = Some(Direct {
            key,
            media: None,
            video: Some(video),
            row,
            recovery: Recovery::default(),
        });
        self.phase = Phase::Loading;
        self.set_playing(true);
        (key, self.commit())
    }

    /// Whether the stream of the attempt `key` is still wanted.
    pub fn awaits(&self, key: u32) -> bool {
        self.phase == Phase::Loading
            && self
                .direct
                .as_ref()
                .is_some_and(|direct| direct.key == key && direct.media.is_none())
    }

    /// Takes the outcome of the attempt `key`: loads the stream, or says why
    /// there is none. A stale outcome changes nothing.
    pub fn finish_stream(
        &mut self,
        key: u32,
        outcome: Result<Media, String>,
    ) -> Option<PlayerState> {
        if !self.awaits(key) {
            return None;
        }
        let direct = self.direct.as_mut()?;
        match outcome {
            Ok(media) => {
                direct.media = Some(media);
                // It may have been paused while loading.
                let (start, playing) = (direct.recovery.resume_at, self.playing);
                self.load(start, playing);
            }
            Err(reason) => {
                let message = format!("Lumen could not play “{}”: {reason}.", direct.row.title);
                self.set_playing(false);
                self.phase = Phase::Failed(message);
            }
        }
        Some(self.commit())
    }

    /// The stream that failed to resolve or to play, to ask for again.
    pub fn failed_stream(&self) -> Option<(String, TrackRow)> {
        let direct = self.direct.as_ref()?;
        match self.phase {
            Phase::Failed(_) => Some((direct.video.clone()?, direct.row.clone())),
            _ => None,
        }
    }

    /// Queues the whole playlist or album and plays the Track at `index`.
    pub fn play_collection(&mut self, id: &str, index: usize) -> Result<PlayerState, PlayerError> {
        let tracks = self
            .catalog
            .listed(id)
            .ok_or_else(|| PlayerError::UnknownCollection { id: id.to_owned() })?
            .to_vec();
        self.play_tracks(tracks, index)
    }

    /// Queues `tracks` and plays the one at `start`.
    pub fn play_tracks(
        &mut self,
        tracks: Vec<Uuid>,
        start: usize,
    ) -> Result<PlayerState, PlayerError> {
        if let Some(&id) = tracks.iter().find(|id| self.catalog.track(**id).is_none()) {
            return Err(PlayerError::UnknownTrack { id });
        }
        if !self.engine.plays_samples() {
            return Err(PlayerError::SampleTrack);
        }
        self.queue.replace(tracks, start)?;
        self.direct = None;
        self.play_current();
        Ok(self.commit())
    }

    /// After the end or a failure, playing starts the media over.
    pub fn toggle_play(&mut self) -> Option<PlayerState> {
        // A stream that never arrived has nothing to play: it is asked for again.
        let unresolved = self.direct.as_ref().is_some_and(|d| d.media.is_none());
        if !self.has_current() || (unresolved && matches!(self.phase, Phase::Failed(_))) {
            return None;
        }
        let restarts = matches!(self.phase, Phase::Ended | Phase::Failed(_));
        if self.phase == Phase::Broken {
            self.toggle_without_stream();
        } else if (restarts || !self.playing) && self.expired() {
            let start = if restarts {
                Duration::ZERO
            } else {
                self.reached()
            };
            self.set_playing(true);
            self.replace_expired(start);
        } else if restarts {
            self.load(Duration::ZERO, true);
        } else {
            self.set_playing(!self.playing);
        }
        Some(self.commit())
    }

    /// Plays the following entry, asking the Recommender for one if the Queue
    /// has run out.
    pub fn next_track(&mut self) -> Option<PlayerState> {
        if !self.advance() {
            return None;
        }
        self.play_current();
        Some(self.commit())
    }

    pub fn previous_track(&mut self) -> Option<PlayerState> {
        self.queue.current()?;
        // On the first entry there is nowhere back to go, so it restarts.
        if self.engine.position() <= RESTART_THRESHOLD {
            self.queue.retreat();
        }
        self.play_current();
        Some(self.commit())
    }

    pub fn seek(&mut self, position_ms: i64) -> Option<PlayerState> {
        // Media that is still opening, or never did, has nowhere to go.
        if !self.has_current() || matches!(self.phase, Phase::Loading | Phase::Failed(_)) {
            return None;
        }
        let wanted = Duration::from_millis(u64::try_from(position_ms).unwrap_or(0));
        if self.phase == Phase::Broken {
            return self.seek_without_stream(wanted).then(|| self.commit());
        }
        let target = wanted.min(self.engine.duration());
        let unchanged = if self.phase == Phase::Ended {
            target == self.engine.duration()
        } else {
            millis(target) == millis(self.engine.position())
        };
        if unchanged {
            return None;
        }
        if self.expired() {
            if self.phase == Phase::Ended {
                self.set_playing(false);
            }
            self.replace_expired(target);
        } else if self.phase == Phase::Ended {
            // The engine let go of it at the end: open it again there.
            self.load(target, false);
        } else {
            self.engine.seek(target);
            self.watch_from_now();
        }
        Some(self.commit())
    }

    pub fn seek_by(&mut self, delta_ms: i64) -> Option<PlayerState> {
        let position = i64::from(millis(self.position()));
        self.seek(position.saturating_add(delta_ms))
    }

    pub fn set_volume(&mut self, volume: i64) -> Option<PlayerState> {
        // The clamp makes the cast exact.
        let volume = volume.clamp(0, 100) as u8;
        if volume == self.volume {
            return None;
        }
        self.volume = volume;
        self.engine.set_volume(volume);
        Some(self.commit())
    }

    pub fn change_volume(&mut self, delta: i64) -> Option<PlayerState> {
        self.set_volume(i64::from(self.volume).saturating_add(delta))
    }

    /// See [`Queue::move_before`].
    pub fn move_queue_entry(
        &mut self,
        key: u32,
        before: Option<u32>,
    ) -> Result<Option<PlayerState>, PlayerError> {
        let moved = self.queue.move_before(key, before)?;
        Ok(moved.then(|| self.commit()))
    }

    /// Up to `limit` upcoming entries (never more than [`MAX_SLICE`]).
    pub fn queue_slice(&self, offset: usize, limit: usize) -> QueueSlice {
        let upcoming = self.queue.upcoming();
        QueueSlice {
            queue_revision: self.queue.revision(),
            offset,
            total: upcoming.len(),
            items: upcoming
                .iter()
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

    pub fn on_engine_event(&mut self, event: EngineEvent) -> Option<PlayerEvent> {
        // While a stream is resolved, nothing the engine says is about it.
        let unresolved = self.direct.as_ref().is_some_and(|d| d.media.is_none());
        if !self.has_current() || unresolved || matches!(self.phase, Phase::Failed(_)) {
            return None;
        }
        match event {
            EngineEvent::Loaded => {
                if self.phase != Phase::Loading {
                    return None;
                }
                self.phase = Phase::Open;
                if let Some(direct) = &mut self.direct {
                    direct.recovery.reconnecting = false;
                }
                self.watch_from_now();
                Some(PlayerEvent::Changed(self.commit()))
            }
            EngineEvent::Duration => {
                if self.phase == Phase::Ended
                    || millis(self.engine.duration()) == self.reported_duration_ms
                {
                    return None;
                }
                Some(PlayerEvent::Changed(self.commit()))
            }
            EngineEvent::Progress => {
                self.watch_progress();
                let position = self.engine.position();
                if self.phase != Phase::Open
                    || !self.playing
                    || position.as_secs() == self.reported_second
                {
                    return None;
                }
                self.reported_second = position.as_secs();
                Some(PlayerEvent::Position(PlayerPosition {
                    revision: self.revision,
                    position_ms: millis(position),
                }))
            }
            EngineEvent::Ended => {
                if self.phase == Phase::Ended {
                    return None;
                }
                if self.recoverable() && self.ended_early() {
                    self.drop_failed(self.reached());
                } else if self.direct.is_some() {
                    // Real audio stops here. Nothing advances by itself.
                    self.set_playing(false);
                    self.phase = Phase::Ended;
                } else if self.advance() {
                    self.play_current();
                } else {
                    // Nothing to go on with: stay here, ready to play it again.
                    self.load(Duration::ZERO, false);
                }
                Some(PlayerEvent::Changed(self.commit()))
            }
            EngineEvent::Failed => {
                // A stream that has ended needs nothing more.
                if self.phase == Phase::Ended && self.recoverable() {
                    return None;
                }
                if self.recoverable() {
                    self.drop_failed(self.reached());
                } else {
                    self.set_playing(false);
                    self.phase = self.failure(EngineError::Unplayable);
                }
                Some(PlayerEvent::Changed(self.commit()))
            }
        }
    }
}

impl<E: Engine> Player<E> {
    /// Marks a change and describes the result.
    fn commit(&mut self) -> PlayerState {
        self.revision += 1;
        self.reported_second = self.position().as_secs();
        self.reported_duration_ms = millis(self.duration());
        if let Some(wake) = &self.wake {
            wake();
        }
        self.state()
    }

    fn has_current(&self) -> bool {
        self.direct.is_some() || self.queue.current().is_some()
    }

    fn set_playing(&mut self, playing: bool) {
        // A resumed stream gets its full time to move on.
        if playing && !self.playing {
            self.watch_from_now();
        }
        self.playing = playing;
        self.engine.set_playing(playing);
    }

    /// Where playback is, also while there is no stream to ask.
    fn position(&self) -> Duration {
        match &self.direct {
            Some(direct) if direct.media.is_none() => direct.recovery.resume_at,
            _ => self.engine.position(),
        }
    }

    /// The length, as last known while the engine does not say.
    fn duration(&self) -> Duration {
        let known = self.engine.duration();
        match &self.direct {
            Some(direct) if known.is_zero() => direct.recovery.duration,
            _ => known,
        }
    }

    /// The real audio, or the current Queue entry's sample.
    fn current_media(&self) -> Media {
        if let Some(media) = self.direct.as_ref().and_then(|d| d.media.clone()) {
            return media;
        }
        let duration = self
            .queue
            .current()
            .and_then(|entry| self.catalog.track(entry.track))
            .map_or(Duration::ZERO, |track| {
                Duration::from_millis(track.duration_ms.into())
            });
        Media::Sample { duration }
    }

    /// Gives the engine the current media at `start`.
    fn load(&mut self, start: Duration, playing: bool) {
        let media = self.current_media();
        if let Some(direct) = &mut self.direct {
            // Where it goes on from if it never opens.
            direct.recovery.resume_at = start;
        }
        match self.engine.load(&media, start) {
            Ok(()) => {
                self.watch_from_now();
                // A sample has nothing to open.
                self.phase = match media {
                    Media::Sample { .. } => Phase::Open,
                    Media::File(_) | Media::Stream { .. } => Phase::Loading,
                };
                self.set_playing(playing);
            }
            Err(_) if self.recoverable() => {
                self.set_playing(playing);
                self.drop_failed(start);
            }
            Err(error) => {
                self.set_playing(false);
                self.phase = self.failure(error);
            }
        }
    }

    /// What the user is told: the title, never where the audio came from.
    fn failure(&self, error: EngineError) -> Phase {
        let title = match &self.direct {
            Some(direct) => Some(direct.row.title.as_str()),
            None => self
                .queue
                .current()
                .and_then(|entry| self.catalog.track(entry.track))
                .map(|track| track.title.as_str()),
        };
        let verb = match error {
            EngineError::NotFound => "find",
            EngineError::Unplayable => "play",
        };
        Phase::Failed(format!(
            "Lumen could not {verb} “{}”.",
            title.unwrap_or("this track")
        ))
    }

    fn play_current(&mut self) {
        self.load(Duration::ZERO, true);
    }

    /// Makes the following entry current, adding a pick first if there is none.
    /// While a pick is current, more are kept waiting behind it.
    fn advance(&mut self) -> bool {
        if self.queue.upcoming().is_empty() && !self.append_pick() {
            return false;
        }
        self.queue.advance();
        if self.queue.current().is_some_and(|entry| entry.autoplay) {
            while self.waiting_picks() < AUTOPLAY_AHEAD && self.append_pick() {}
        }
        true
    }

    fn waiting_picks(&self) -> usize {
        self.queue.upcoming().iter().filter(|e| e.autoplay).count()
    }

    fn append_pick(&mut self) -> bool {
        let pick = self.recommendation();
        if let Some(track) = pick {
            self.queue.append_autoplay(track);
        }
        pick.is_some()
    }

    /// What the Recommender would put at the end of the Queue.
    fn recommendation(&self) -> Option<Uuid> {
        let current = self.queue.current()?;
        let history = self.queue.history();
        let recent: Vec<Uuid> = history[history.len().saturating_sub(RECENT_HISTORY)..]
            .iter()
            .map(|entry| entry.track)
            .collect();
        let upcoming: Vec<Uuid> = self.queue.upcoming().iter().map(|e| e.track).collect();
        self.recommender
            .recommend(&recent, current.track, &upcoming)
            .filter(|id| self.catalog.track(*id).is_some())
    }

    /// An entry's key, row and autoplay mark.
    fn row(&self, entry: &Entry) -> Option<(u32, TrackRow, bool)> {
        let track = self.catalog.track(entry.track)?;
        Some((entry.key, TrackRow::from(track), entry.autoplay))
    }
}

/// Track times fit: the longest is far below `u32::MAX` milliseconds.
fn millis(time: Duration) -> u32 {
    u32::try_from(time.as_millis()).unwrap_or(u32::MAX)
}

/// How the Queue tests move the simulated clock along.
#[cfg(test)]
impl<C: Clock> Player<SimulatedEngine<C>> {
    pub fn next_wake(&self) -> Option<Duration> {
        self.engine.next_wake()
    }

    /// Looks at the simulated engine and handles what it reports.
    pub fn tick(&mut self) -> Option<PlayerEvent> {
        let event = self.engine.poll()?;
        self.on_engine_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FakeEngine;
    use crate::audio::simulated::ManualClock;
    use crate::catalog::{LONG_PLAYLIST_ID, SourceKind, SourceRef, Track};
    use crate::recs::StaticRecommender;

    type TestPlayer = Player<SimulatedEngine<ManualClock>>;

    /// A Recommender with nothing to offer, so the Queue can run out.
    struct Silent;

    impl Recommender for Silent {
        fn recommend(&self, _: &[Uuid], _: Uuid, _: &[Uuid]) -> Option<Uuid> {
            None
        }
    }

    fn build(
        catalog: Catalog,
        recommender: impl FnOnce(Arc<Catalog>) -> Box<dyn Recommender>,
    ) -> (TestPlayer, ManualClock) {
        let catalog = Arc::new(catalog);
        let clock = ManualClock::default();
        let engine = SimulatedEngine::new(clock.clone());
        let recommender = recommender(Arc::clone(&catalog));
        (Player::new(engine, catalog, recommender), clock)
    }

    fn idle() -> (TestPlayer, ManualClock) {
        build(Catalog::default(), |c| Box::new(StaticRecommender::new(c)))
    }

    /// `night-shift` is tracks 24, 25, 26, 1 and 3; track 24 lasts 375 s.
    fn playing(row: usize) -> (TestPlayer, ManualClock) {
        let (mut player, clock) = idle();
        player.play_collection("night-shift", row).unwrap();
        (player, clock)
    }

    /// The same, with nothing to play once the Queue runs out.
    fn finite(row: usize) -> (TestPlayer, ManualClock) {
        let (mut player, clock) = build(Catalog::default(), |_| Box::new(Silent));
        player.play_collection("night-shift", row).unwrap();
        (player, clock)
    }

    /// A player over a catalog of `count` Tracks, ten seconds each.
    fn small(count: u128) -> (TestPlayer, ManualClock) {
        let tracks = (1..=count)
            .map(|n| Track {
                id: Uuid::from_u128(n),
                title: format!("Track {n}"),
                artists: vec!["Mira Sol".to_owned()],
                album: None,
                duration_ms: 10_000,
                artwork_url: None,
                isrc: None,
                sources: vec![SourceRef {
                    kind: SourceKind::YouTube,
                    id: format!("ref-{n}"),
                }],
            })
            .collect();
        build(Catalog::of(tracks), |c| Box::new(StaticRecommender::new(c)))
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn millis(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn track_of(state: &PlayerState) -> u128 {
        state.current.as_ref().unwrap().track.id.as_u128()
    }

    fn key_of(state: &PlayerState) -> u32 {
        state.current.as_ref().unwrap().key
    }

    fn autoplaying(state: &PlayerState) -> bool {
        state.current.as_ref().unwrap().autoplay
    }

    /// Track number and autoplay mark of what is waiting.
    fn upcoming(player: &TestPlayer) -> Vec<(u128, bool)> {
        player
            .queue_slice(0, MAX_SLICE)
            .items
            .iter()
            .map(|item| (item.track.id.as_u128(), item.autoplay))
            .collect()
    }

    fn changed(event: Option<PlayerEvent>) -> PlayerState {
        match event {
            Some(PlayerEvent::Changed(state)) => state,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    #[test]
    fn starts_idle() {
        let state = idle().0.state();

        assert_eq!(state.status, PlaybackStatus::Idle);
        assert_eq!(state.current, None);
        assert_eq!(state.volume, 80);
        assert_eq!((state.position_ms, state.duration_ms), (0, 0));
        assert_eq!((state.upcoming, state.can_next), (0, false));
    }

    #[test]
    fn playing_a_row_queues_the_collection() {
        let (mut player, _) = idle();

        let state = player.play_collection("night-shift", 1).unwrap();

        assert_eq!(track_of(&state), 25);
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!((state.position_ms, state.duration_ms), (0, 248_000));
        assert_eq!(state.upcoming, 3);
        assert!(!autoplaying(&state));
        assert_eq!(upcoming(&player), [(26, false), (1, false), (3, false)]);
        assert_eq!(player.state(), state);
    }

    #[test]
    fn clicking_the_playing_row_restarts_it() {
        let (mut player, clock) = playing(0);
        clock.advance(secs(10));

        let state = player.play_collection("night-shift", 0).unwrap();

        assert_eq!((track_of(&state), state.position_ms), (24, 0));
    }

    #[test]
    fn a_bad_collection_index_or_track_changes_nothing() {
        let (mut player, _) = playing(1);
        let before = player.state();

        assert_eq!(
            player.play_collection("nope", 0),
            Err(PlayerError::UnknownCollection {
                id: "nope".to_owned()
            })
        );
        assert_eq!(
            player.play_collection("night-shift", 5),
            Err(PlayerError::Queue(QueueError::OutOfRange {
                index: 5,
                len: 5
            }))
        );
        let unknown = Uuid::from_u128(999);
        assert_eq!(
            player.play_tracks(vec![Uuid::from_u128(1), unknown], 0),
            Err(PlayerError::UnknownTrack { id: unknown })
        );

        assert_eq!(player.state(), before);
    }

    #[test]
    fn toggle_pauses_and_resumes_in_place() {
        let (mut player, clock) = playing(0);
        clock.advance(secs(5));

        let paused = player.toggle_play().unwrap();
        clock.advance(secs(10));
        let resumed = player.toggle_play().unwrap();

        assert_eq!(
            (paused.status, paused.position_ms),
            (PlaybackStatus::Paused, 5_000)
        );
        assert_eq!(
            (resumed.status, resumed.position_ms),
            (PlaybackStatus::Playing, 5_000)
        );
        assert!(resumed.revision > paused.revision);
    }

    #[test]
    fn idle_transport_does_nothing() {
        let (mut player, _) = idle();

        assert_eq!(player.toggle_play(), None);
        assert_eq!(player.next_track(), None);
        assert_eq!(player.previous_track(), None);
        assert_eq!(player.seek(1_000), None);
        assert_eq!(player.seek_by(1_000), None);
        assert_eq!(player.on_engine_event(EngineEvent::Ended), None);
        assert_eq!(player.state().revision, 0);
    }

    #[test]
    fn seek_clamps_to_the_track() {
        let (mut player, _) = playing(0);

        assert_eq!(player.seek(10_000).unwrap().position_ms, 10_000);
        assert_eq!(player.seek(-5).unwrap().position_ms, 0);
        assert_eq!(player.seek(999_999_999).unwrap().position_ms, 375_000);
    }

    #[test]
    fn seeking_to_where_it_is_changes_nothing() {
        let (mut player, _) = playing(0);

        assert_eq!(player.seek(0), None);
        assert_eq!(player.seek_by(-5_000), None);
    }

    #[test]
    fn seek_by_uses_the_true_position() {
        let (mut player, clock) = playing(0);
        clock.advance(millis(10_700));

        assert_eq!(player.seek_by(5_000).unwrap().position_ms, 15_700);
        assert_eq!(player.seek_by(i64::MIN).unwrap().position_ms, 0);
        assert_eq!(player.seek_by(i64::MAX).unwrap().position_ms, 375_000);
    }

    #[test]
    fn seeking_to_the_end_ends_the_track_once_it_plays() {
        let (mut player, _) = playing(0);
        player.toggle_play();
        player.seek(375_000);
        assert_eq!((player.next_wake(), player.tick()), (None, None));

        player.toggle_play();

        assert_eq!(player.next_wake(), Some(Duration::ZERO));
        assert_eq!(track_of(&changed(player.tick())), 25);
    }

    #[test]
    fn volume_clamps_and_repeats_stay_quiet() {
        let (mut player, _) = idle();

        assert_eq!(player.set_volume(-1).unwrap().volume, 0);
        assert_eq!(player.set_volume(300).unwrap().volume, 100);
        assert_eq!(player.set_volume(100), None);
        assert_eq!(player.change_volume(-5).unwrap().volume, 95);
        assert_eq!(player.change_volume(i64::MAX).unwrap().volume, 100);
        assert_eq!(player.change_volume(5), None);
        assert_eq!(player.change_volume(i64::MIN).unwrap().volume, 0);
        assert_eq!(player.state().status, PlaybackStatus::Idle);
    }

    #[test]
    fn next_plays_the_following_entry_even_when_paused() {
        let (mut player, clock) = playing(1);
        clock.advance(secs(5));
        player.toggle_play();

        let state = player.next_track().unwrap();

        assert_eq!(track_of(&state), 26);
        assert_eq!(
            (state.status, state.position_ms),
            (PlaybackStatus::Playing, 0)
        );
    }

    #[test]
    fn next_on_the_last_entry_does_nothing_without_a_recommendation() {
        let (mut player, clock) = finite(4);
        clock.advance(secs(5));
        let before = player.state();

        assert_eq!(player.next_track(), None);

        assert_eq!(player.state(), before);
        assert!(!before.can_next);
    }

    #[test]
    fn previous_after_three_seconds_restarts() {
        let (mut player, clock) = playing(1);
        let key = key_of(&player.state());
        clock.advance(millis(3_001));

        let state = player.previous_track().unwrap();

        assert_eq!((key_of(&state), state.position_ms), (key, 0));
    }

    #[test]
    fn previous_at_three_seconds_goes_back() {
        let (mut player, clock) = playing(1);
        clock.advance(millis(3_000));
        player.toggle_play();

        let state = player.previous_track().unwrap();

        assert_eq!(track_of(&state), 24);
        assert_eq!(
            (state.status, state.position_ms),
            (PlaybackStatus::Playing, 0)
        );
    }

    #[test]
    fn previous_on_the_first_entry_restarts_it() {
        let (mut player, clock) = playing(0);
        let key = key_of(&player.state());
        clock.advance(secs(1));

        let state = player.previous_track().unwrap();

        assert_eq!((key_of(&state), state.position_ms), (key, 0));
    }

    #[test]
    fn a_finished_track_advances_and_keeps_playing() {
        let (mut player, clock) = playing(0);
        clock.advance(secs(375));

        let state = changed(player.tick());

        assert_eq!(track_of(&state), 25);
        assert_eq!(
            (state.status, state.position_ms),
            (PlaybackStatus::Playing, 0)
        );
    }

    #[test]
    fn the_last_track_ends_paused_at_zero_and_can_replay_without_a_recommendation() {
        let (mut player, clock) = finite(4);
        let key = key_of(&player.state());
        clock.advance(secs(1_000));

        let ended = changed(player.tick());

        assert_eq!(
            (ended.status, ended.position_ms),
            (PlaybackStatus::Paused, 0)
        );
        assert_eq!(key_of(&ended), key);
        assert_eq!(player.next_wake(), None);
        assert_eq!(
            player.toggle_play().map(|s| s.status),
            Some(PlaybackStatus::Playing)
        );
    }

    #[test]
    fn a_late_wake_advances_one_track_only() {
        let (mut player, clock) = playing(0);
        clock.advance(secs(3_600));

        let state = changed(player.tick());

        assert_eq!((track_of(&state), state.position_ms), (25, 0));
        assert_eq!(player.tick(), None);
    }

    #[test]
    fn a_tick_reports_each_second_once() {
        let (mut player, clock) = playing(0);
        let revision = player.state().revision;
        assert_eq!(player.tick(), None);

        clock.advance(millis(1_000));
        assert_eq!(
            player.tick(),
            Some(PlayerEvent::Position(PlayerPosition {
                revision,
                position_ms: 1_000
            }))
        );
        clock.advance(millis(100));
        assert_eq!(player.tick(), None);
        assert_eq!(player.state().revision, revision);
    }

    #[test]
    fn a_tick_skips_the_second_a_command_just_sent() {
        let (mut player, _) = playing(0);

        player.seek(5_000);

        assert_eq!(player.tick(), None);
    }

    #[test]
    fn a_paused_player_never_asks_to_be_woken() {
        let (mut player, _) = idle();
        assert_eq!(player.next_wake(), None);

        player.play_collection("night-shift", 0).unwrap();
        assert_eq!(player.next_wake(), Some(secs(1)));
        player.toggle_play();

        assert_eq!(player.next_wake(), None);
    }

    #[test]
    fn moving_an_entry_changes_what_plays_next() {
        let (mut player, _) = playing(0);
        let before = player.state();
        let items = player.queue_slice(0, 10).items;

        let moved = player
            .move_queue_entry(items[3].key, Some(items[0].key))
            .unwrap()
            .unwrap();

        assert!(moved.queue_revision > before.queue_revision);
        assert!(moved.revision > before.revision);
        assert_eq!(track_of(&player.next_track().unwrap()), 3);
    }

    #[test]
    fn a_move_that_changes_nothing_or_names_no_entry() {
        let (mut player, _) = playing(0);
        let last = player.queue_slice(0, 10).items[3].key;

        assert_eq!(player.move_queue_entry(last, None), Ok(None));
        assert_eq!(
            player.move_queue_entry(999, None),
            Err(PlayerError::Queue(QueueError::UnknownEntry { key: 999 }))
        );
    }

    #[test]
    fn a_queue_slice_is_only_the_window() {
        let (mut player, _) = idle();
        player.play_collection(LONG_PLAYLIST_ID, 0).unwrap();

        let slice = player.queue_slice(100, 30);

        assert_eq!(
            (slice.offset, slice.total, slice.items.len()),
            (100, 4_999, 30)
        );
        assert_eq!(slice.items[0].track.id.as_u128(), 10_101);
        assert_eq!(slice.queue_revision, player.state().queue_revision);
    }

    #[test]
    fn a_queue_slice_is_capped_and_safe_past_the_end() {
        let (mut player, _) = idle();
        assert_eq!(player.queue_slice(0, 10).total, 0);
        player.play_collection(LONG_PLAYLIST_ID, 0).unwrap();

        assert_eq!(player.queue_slice(0, usize::MAX).items.len(), 200);
        for offset in [9_999, usize::MAX] {
            let slice = player.queue_slice(offset, 10);
            assert_eq!((slice.items.len(), slice.total), (0, 4_999));
        }
    }

    #[test]
    fn state_stays_small_for_a_huge_queue() {
        let (mut player, _) = idle();

        let state = player.play_collection(LONG_PLAYLIST_ID, 0).unwrap();

        assert_eq!(state.upcoming, 4_999);
        assert!(serde_json::to_vec(&state).unwrap().len() < 1_024);
    }

    // Autoplay.

    #[test]
    fn the_last_entry_can_go_next_when_there_is_a_recommendation() {
        let (player, _) = playing(4);

        let state = player.state();

        assert!(state.can_next);
        assert_eq!(state.upcoming, 0);
    }

    #[test]
    fn next_on_the_last_entry_continues_with_a_recommendation() {
        let (mut player, _) = playing(4);
        player.toggle_play();
        let before = player.state();

        let state = player.next_track().unwrap();

        assert!(autoplaying(&state));
        assert_eq!(
            (state.status, state.position_ms),
            (PlaybackStatus::Playing, 0)
        );
        // Track 3 is by Mira Sol, like track 1, which was heard just before.
        // Track 2 is the first of that artist's that was not.
        assert_eq!(track_of(&state), 2);
        assert_eq!((state.upcoming, state.can_next), (AUTOPLAY_AHEAD, true));
        assert!(state.queue_revision > before.queue_revision);
        assert!(upcoming(&player).iter().all(|(_, autoplay)| *autoplay));
    }

    #[test]
    fn the_last_track_ending_continues_with_a_recommendation() {
        let (mut player, clock) = playing(4);
        clock.advance(secs(1_000));

        let state = changed(player.tick());

        assert!(autoplaying(&state));
        assert_eq!(
            (state.status, state.position_ms),
            (PlaybackStatus::Playing, 0)
        );
        assert_eq!(state.upcoming, 5);
    }

    #[test]
    fn autoplay_keeps_five_picks_waiting_without_repeats() {
        let (mut player, _) = playing(4);
        let mut heard: Vec<u128> = vec![24, 25, 26, 1, 3];

        for _ in 0..100 {
            let state = player.next_track().unwrap();
            let current = track_of(&state);
            let waiting = upcoming(&player);

            let mut recent = heard.iter().rev().take(RECENT_HISTORY);
            assert!(!recent.any(|heard| *heard == current));
            assert_eq!(waiting.len(), 5);
            assert!(
                waiting
                    .iter()
                    .all(|&(track, autoplay)| { autoplay && track != current && track < 10_000 })
            );
            let mut distinct: Vec<u128> = waiting.iter().map(|w| w.0).collect();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), 5);
            heard.push(current);
        }
    }

    #[test]
    fn the_next_pick_is_the_one_that_was_waiting() {
        let (mut player, _) = playing(4);
        player.next_track();
        let waiting = upcoming(&player)[0].0;

        assert_eq!(track_of(&player.next_track().unwrap()), waiting);
    }

    #[test]
    fn autoplay_picks_the_same_way_every_time() {
        let run = || {
            let (mut player, _) = playing(4);
            (0..10)
                .map(|_| track_of(&player.next_track().unwrap()))
                .collect::<Vec<_>>()
        };

        assert_eq!(run(), run());
    }

    #[test]
    fn going_back_keeps_the_picks_and_adds_none() {
        let (mut player, _) = playing(4);
        let pick = track_of(&player.next_track().unwrap());
        let waiting = upcoming(&player);

        let back = player.previous_track().unwrap();

        assert_eq!((track_of(&back), autoplaying(&back)), (3, false));
        assert_eq!(back.upcoming, 6);
        assert_eq!(track_of(&player.next_track().unwrap()), pick);
        assert_eq!(upcoming(&player), waiting);
    }

    #[test]
    fn a_new_collection_drops_the_picks() {
        let (mut player, _) = playing(4);
        player.next_track();

        let state = player.play_collection("night-shift", 0).unwrap();

        assert_eq!(state.upcoming, 4);
        assert!(upcoming(&player).iter().all(|(_, autoplay)| !autoplay));
    }

    #[test]
    fn a_two_track_catalog_never_stops_or_repeats_at_once() {
        let (mut player, _) = small(2);
        player.play_tracks(vec![Uuid::from_u128(1)], 0).unwrap();

        let heard: Vec<u128> = (0..6)
            .map(|_| track_of(&player.next_track().unwrap()))
            .collect();

        assert_eq!(heard, [2, 1, 2, 1, 2, 1]);
    }

    #[test]
    fn a_one_track_catalog_stops_paused_at_zero() {
        let (mut player, clock) = small(1);
        let started = player.play_tracks(vec![Uuid::from_u128(1)], 0).unwrap();
        assert!(!started.can_next);
        assert_eq!(player.next_track(), None);
        clock.advance(secs(10));

        let ended = changed(player.tick());

        assert_eq!(
            (ended.status, ended.position_ms),
            (PlaybackStatus::Paused, 0)
        );
        assert_eq!((ended.upcoming, ended.can_next), (0, false));
    }

    #[test]
    fn an_empty_catalog_stays_idle() {
        let (mut player, _) = small(0);

        assert_eq!(
            player.play_tracks(Vec::new(), 0),
            Err(PlayerError::Queue(QueueError::OutOfRange {
                index: 0,
                len: 0
            }))
        );
        assert_eq!(player.next_track(), None);
        assert_eq!(player.state().status, PlaybackStatus::Idle);
        assert!(!player.state().can_next);
    }

    #[test]
    fn a_pick_the_catalog_does_not_know_is_ignored() {
        struct Stranger;
        impl Recommender for Stranger {
            fn recommend(&self, _: &[Uuid], _: Uuid, _: &[Uuid]) -> Option<Uuid> {
                Some(Uuid::from_u128(999_999))
            }
        }
        let (mut player, _) = build(Catalog::default(), |_| Box::new(Stranger));
        let state = player.play_collection("night-shift", 4).unwrap();

        assert!(!state.can_next);
        assert_eq!(player.next_track(), None);
    }

    /// A player over an engine the test drives, with a song being resolved.
    fn resolving() -> (Player<FakeEngine>, FakeEngine, u32) {
        let engine = FakeEngine::default();
        let catalog = Arc::new(Catalog::default());
        let mut player = Player::new(engine.clone(), catalog, Box::new(Silent));
        let row = TrackRow::untagged("Paper".to_owned());
        let (key, _) = player.begin_stream("dQw4w9WgXcQ".to_owned(), row);
        (player, engine, key)
    }

    fn stream() -> Media {
        Media::Stream {
            url: "https://a.googlevideo.com/x".to_owned(),
            headers: Vec::new(),
            expires_at: None,
        }
    }

    #[test]
    fn a_chosen_stream_shows_as_loading_and_loads_nothing_yet() {
        let (mut player, engine, key) = resolving();

        let state = player.state();

        assert_eq!(state.status, PlaybackStatus::Loading);
        assert_eq!(state.current.unwrap().track.title, "Paper");
        assert!(!state.can_next && !state.can_previous);
        assert!(player.awaits(key));
        assert_eq!(engine.fake().stops, 1);
        assert!(engine.fake().loads.is_empty());
        // The old media's last events are not about this song.
        assert_eq!(player.on_engine_event(EngineEvent::Failed), None);
        assert_eq!(player.on_engine_event(EngineEvent::Ended), None);
        assert_eq!(player.seek(1_000), None);
    }

    #[test]
    fn a_resolved_stream_is_loaded_and_plays() {
        let (mut player, engine, key) = resolving();

        let state = player.finish_stream(key, Ok(stream())).unwrap();

        assert_eq!(state.status, PlaybackStatus::Loading);
        assert_eq!(engine.fake().loads, [(stream(), Duration::ZERO)]);
        assert!(engine.fake().playing);
        let loaded = changed(player.on_engine_event(EngineEvent::Loaded));
        assert_eq!(loaded.status, PlaybackStatus::Playing);
    }

    #[test]
    fn a_pause_while_loading_is_kept() {
        let (mut player, engine, key) = resolving();
        player.toggle_play().unwrap();

        let state = player.finish_stream(key, Ok(stream())).unwrap();

        assert_eq!(state.status, PlaybackStatus::Paused);
        assert!(!engine.fake().playing);
    }

    #[test]
    fn only_the_newest_choice_is_loaded() {
        let (mut player, engine, old) = resolving();
        let row = TrackRow::untagged("Newer".to_owned());
        let (new, _) = player.begin_stream("aaaaaaaaaaa".to_owned(), row);

        assert!(!player.awaits(old));
        assert_eq!(player.finish_stream(old, Ok(stream())), None);
        assert_eq!(player.finish_stream(old, Err("no".to_owned())), None);
        assert!(engine.fake().loads.is_empty());
        assert!(player.finish_stream(new, Ok(stream())).is_some());
    }

    #[test]
    fn a_failed_resolve_says_why_and_can_be_asked_for_again() {
        let (mut player, engine, key) = resolving();
        assert_eq!(player.failed_stream(), None);

        let reason = "this song is unavailable".to_owned();
        let state = player.finish_stream(key, Err(reason)).unwrap();

        assert_eq!(state.status, PlaybackStatus::Error);
        assert_eq!(
            state.error.as_deref(),
            Some("Lumen could not play “Paper”: this song is unavailable.")
        );
        assert_eq!(player.failed_stream().unwrap().0, "dQw4w9WgXcQ");
        assert_eq!(player.toggle_play(), None);
        assert!(engine.fake().loads.is_empty());
    }
}
