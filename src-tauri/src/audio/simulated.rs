//! An engine that only keeps time, so the Queue can be tested without audio.

use std::time::{Duration, Instant};

use super::{Engine, EngineError, EngineEvent, Media};

/// Time since some fixed moment.
pub trait Clock: Send {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Position is worked out from the clock when asked, so nothing drifts.
#[derive(Debug)]
pub struct SimulatedEngine<C = SystemClock> {
    clock: C,
    duration: Duration,
    /// The position when playing last started, or where it is paused.
    base: Duration,
    /// When playing last started. `None` while paused.
    since: Option<Duration>,
    /// Whether the end was already reported.
    ended: bool,
}

impl<C: Clock> SimulatedEngine<C> {
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            duration: Duration::ZERO,
            base: Duration::ZERO,
            since: None,
            ended: false,
        }
    }

    /// How long until `poll` has something new: the next whole second of the
    /// track or its end. `None` while nothing moves.
    pub fn next_wake(&self) -> Option<Duration> {
        if self.since.is_none() || self.ended {
            return None;
        }
        let position = self.position();
        let next_second = Duration::from_secs(position.as_secs() + 1);
        Some(next_second.min(self.duration) - position)
    }

    /// What happened since the last look. The end is reported once.
    pub fn poll(&mut self) -> Option<EngineEvent> {
        if self.since.is_none() || self.ended {
            return None;
        }
        if self.position() < self.duration {
            return Some(EngineEvent::Progress);
        }
        self.ended = true;
        Some(EngineEvent::Ended)
    }
}

impl<C: Clock> Engine for SimulatedEngine<C> {
    fn plays_samples(&self) -> bool {
        true
    }

    /// Only samples: there is nothing here to open a file or stream with.
    fn load(&mut self, media: &Media, start: Duration) -> Result<(), EngineError> {
        let sample = match media {
            Media::Sample { duration } => Some(*duration),
            Media::File(_) | Media::Stream { .. } => None,
        };
        self.duration = sample.unwrap_or_default();
        self.base = start.min(self.duration);
        self.since = None;
        self.ended = false;
        sample.map(|_| ()).ok_or(EngineError::Unplayable)
    }

    fn stop(&mut self) {
        self.duration = Duration::ZERO;
        self.base = Duration::ZERO;
        self.since = None;
        self.ended = false;
    }

    fn set_playing(&mut self, playing: bool) {
        self.base = self.position();
        self.since = playing.then(|| self.clock.now());
    }

    fn seek(&mut self, position: Duration) {
        self.base = position.min(self.duration);
        self.since = self.since.map(|_| self.clock.now());
        self.ended = false;
    }

    /// Nothing makes sound, so there is nothing to turn.
    fn set_volume(&mut self, _volume: u8) {}

    fn position(&self) -> Duration {
        let played = self.since.map_or(Duration::ZERO, |since| {
            self.clock.now().saturating_sub(since)
        });
        self.base.saturating_add(played).min(self.duration)
    }

    fn duration(&self) -> Duration {
        self.duration
    }
}

/// A clock tests move by hand. Clones share the time.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct ManualClock(std::sync::Arc<std::sync::atomic::AtomicU64>);

#[cfg(test)]
impl ManualClock {
    pub fn advance(&self, by: Duration) {
        let nanos = u64::try_from(by.as_nanos()).unwrap();
        self.0
            .fetch_add(nanos, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LENGTH: Duration = Duration::from_secs(200);

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn millis(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn load(engine: &mut SimulatedEngine<ManualClock>) {
        let sample = Media::Sample { duration: LENGTH };
        engine.load(&sample, Duration::ZERO).unwrap();
    }

    fn loaded() -> (SimulatedEngine<ManualClock>, ManualClock) {
        let clock = ManualClock::default();
        let mut engine = SimulatedEngine::new(clock.clone());
        load(&mut engine);
        (engine, clock)
    }

    fn playing() -> (SimulatedEngine<ManualClock>, ManualClock) {
        let (mut engine, clock) = loaded();
        engine.set_playing(true);
        (engine, clock)
    }

    #[test]
    fn a_loaded_track_waits_paused_at_zero() {
        let (mut engine, clock) = loaded();
        clock.advance(secs(5));

        assert_eq!(engine.position(), Duration::ZERO);
        assert_eq!(engine.duration(), LENGTH);
        assert_eq!(engine.next_wake(), None);
        assert_eq!(engine.poll(), None);
    }

    #[test]
    fn position_follows_the_clock_only_while_playing() {
        let (mut engine, clock) = playing();

        clock.advance(secs(2));
        assert_eq!(engine.position(), secs(2));
        assert_eq!(engine.poll(), Some(EngineEvent::Progress));

        engine.set_playing(false);
        clock.advance(secs(10));
        assert_eq!(engine.position(), secs(2));

        engine.set_playing(true);
        engine.set_playing(true);
        clock.advance(secs(1));
        assert_eq!(engine.position(), secs(3));
    }

    #[test]
    fn position_stops_at_the_duration() {
        let (engine, clock) = playing();

        clock.advance(secs(3_600));

        assert_eq!(engine.position(), LENGTH);
    }

    #[test]
    fn seeking_continues_from_the_new_place_and_clamps() {
        let (mut engine, clock) = playing();

        engine.seek(secs(10));
        clock.advance(secs(1));
        assert_eq!(engine.position(), secs(11));

        engine.seek(secs(999));
        assert_eq!(engine.position(), LENGTH);
    }

    #[test]
    fn next_wake_reaches_the_next_second_or_the_end() {
        let (mut engine, clock) = playing();

        clock.advance(millis(1_400));
        assert_eq!(engine.next_wake(), Some(millis(600)));

        engine.seek(LENGTH - millis(200));
        assert_eq!(engine.next_wake(), Some(millis(200)));

        engine.seek(LENGTH);
        assert_eq!(engine.next_wake(), Some(Duration::ZERO));
    }

    #[test]
    fn reports_the_end_once() {
        let (mut engine, clock) = playing();
        clock.advance(LENGTH);

        assert_eq!(engine.poll(), Some(EngineEvent::Ended));
        assert_eq!(engine.poll(), None);
        assert_eq!(engine.next_wake(), None);
    }

    #[test]
    fn refuses_real_audio() {
        let (mut engine, _) = loaded();

        let refused = engine.load(&Media::File("tone.wav".into()), Duration::ZERO);

        assert_eq!(refused, Err(EngineError::Unplayable));
        assert_eq!(engine.duration(), Duration::ZERO);
    }

    #[test]
    fn a_new_track_or_a_seek_can_end_again() {
        let (mut engine, clock) = playing();
        clock.advance(LENGTH);
        engine.poll();

        engine.seek(secs(1));
        clock.advance(LENGTH);
        assert_eq!(engine.poll(), Some(EngineEvent::Ended));

        load(&mut engine);
        assert_eq!(engine.position(), Duration::ZERO);
        assert_eq!(engine.next_wake(), None);
        engine.set_playing(true);
        clock.advance(LENGTH);
        assert_eq!(engine.poll(), Some(EngineEvent::Ended));
    }
}
