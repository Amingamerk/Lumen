//! Playback: the player's state and rules, over an engine that makes the sound.

mod engine;
mod local;
mod mpv;
mod mpv_events;
mod player;
mod state;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod mpv_events_tests;
#[cfg(test)]
mod mpv_tests;
#[cfg(test)]
mod real_audio_tests;
#[cfg(test)]
mod simulated;

pub use engine::{Engine, EngineError, EngineEvent, Media};
#[cfg(test)]
pub use fake::{FakeEngine, FakeTracks};
pub use local::{file_media, play_file_arg};
pub use mpv::{MpvEngine, MpvEvents, MpvShutdown};
pub use player::{Fetch, Lookup, NoStream, Placement, Player, PlayerError, TrackLookup};
#[cfg(test)]
pub use simulated::{SimulatedEngine, SystemClock};
pub use state::{
    NowPlaying, PlaybackStatus, PlayerEvent, PlayerPosition, PlayerState, QueueItem, QueueSlice,
    Skipped,
};
