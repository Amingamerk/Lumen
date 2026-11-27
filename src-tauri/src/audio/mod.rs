//! Playback: the player's state and rules, over an engine that makes the sound.

mod engine;
mod player;
mod state;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod real_audio_tests;
#[cfg(test)]
mod simulated;

pub use engine::{Engine, EngineError, EngineEvent, Media};
pub use player::{Player, PlayerError};
#[cfg(test)]
pub use simulated::{SimulatedEngine, SystemClock};
pub use state::{
    NowPlaying, PlaybackStatus, PlayerEvent, PlayerPosition, PlayerState, QueueItem, QueueSlice,
};
