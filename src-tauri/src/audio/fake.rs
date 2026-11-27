//! An engine tests drive by hand: it records what it is asked, and the test says
//! what it found out.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{Engine, EngineError, Media};

#[derive(Debug, Default)]
pub struct Fake {
    pub loads: Vec<(Media, Duration)>,
    pub playing: bool,
    pub volume: u8,
    pub position: Duration,
    pub duration: Duration,
    /// What the next loads answer instead of succeeding.
    pub refuse: Option<EngineError>,
    pub plays_samples: bool,
}

/// Clones share the state, so a test keeps one after the player takes another.
#[derive(Debug, Clone, Default)]
pub struct FakeEngine(Arc<Mutex<Fake>>);

impl FakeEngine {
    pub fn fake(&self) -> MutexGuard<'_, Fake> {
        self.0.lock().unwrap()
    }
}

impl Engine for FakeEngine {
    fn plays_samples(&self) -> bool {
        self.fake().plays_samples
    }

    fn load(&mut self, media: &Media, start: Duration) -> Result<(), EngineError> {
        let mut fake = self.fake();
        fake.loads.push((media.clone(), start));
        fake.playing = false;
        fake.position = Duration::ZERO;
        fake.duration = Duration::ZERO;
        if let Some(error) = fake.refuse {
            return Err(error);
        }
        fake.position = start;
        Ok(())
    }

    fn set_playing(&mut self, playing: bool) {
        self.fake().playing = playing;
    }

    fn seek(&mut self, position: Duration) {
        self.fake().position = position;
    }

    fn set_volume(&mut self, volume: u8) {
        self.fake().volume = volume;
    }

    fn position(&self) -> Duration {
        self.fake().position
    }

    fn duration(&self) -> Duration {
        self.fake().duration
    }
}
