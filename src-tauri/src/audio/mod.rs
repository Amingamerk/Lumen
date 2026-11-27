//! Playback: the player's state and rules, over an engine that makes the sound.

mod engine;
mod player;
mod simulated;
mod state;

pub use engine::{Engine, EngineEvent};
pub use player::{Player, PlayerError};
pub use simulated::{Clock, SimulatedEngine, SystemClock};
pub use state::{
    NowPlaying, PlaybackStatus, PlayerEvent, PlayerPosition, PlayerState, QueueItem, QueueSlice,
};
