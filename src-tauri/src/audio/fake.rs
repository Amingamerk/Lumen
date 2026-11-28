//! An engine tests drive by hand: it records what it is asked, and the test says
//! what it found out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use uuid::Uuid;

use super::{Engine, EngineError, Media, TrackLookup};
use crate::catalog::{SourceKind, SourceRef, Track};

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
    pub stops: usize,
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

    fn stop(&mut self) {
        let mut fake = self.fake();
        fake.stops += 1;
        fake.playing = false;
        fake.position = Duration::ZERO;
        fake.duration = Duration::ZERO;
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

/// Library Tracks held in memory. Clones share them, so a test can add more after
/// the player took one.
#[derive(Debug, Clone, Default)]
pub struct FakeTracks(Arc<Mutex<HashMap<Uuid, Track>>>);

impl FakeTracks {
    /// Tracks 1 to `count`, each of 200 seconds and playing from the YouTube
    /// video `v<n>`.
    pub fn of(count: u128) -> Self {
        let tracks = Self::default();
        for n in 1..=count {
            tracks.add(Self::track(n));
        }
        tracks
    }

    pub fn track(n: u128) -> Track {
        Track {
            id: Uuid::from_u128(n),
            title: format!("Track {n}"),
            artists: vec!["Mira Sol".to_owned()],
            album: None,
            duration_ms: 200_000,
            artwork_url: None,
            isrc: None,
            sources: vec![SourceRef {
                kind: SourceKind::YouTube,
                id: format!("v{n}"),
            }],
        }
    }

    pub fn add(&self, track: Track) {
        self.0.lock().unwrap().insert(track.id, track);
    }

    pub fn ids(count: u128) -> Vec<Uuid> {
        (1..=count).map(Uuid::from_u128).collect()
    }
}

impl TrackLookup for FakeTracks {
    fn track(&self, id: Uuid) -> Option<Track> {
        self.0.lock().unwrap().get(&id).cloned()
    }
}
