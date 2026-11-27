//! The seam to whatever makes the sound.

use std::time::Duration;

/// What actually makes sound. Simulated until Milestone 2 puts libmpv behind it.
pub trait Engine: Send {
    /// Takes a new track: paused at position 0.
    fn load(&mut self, duration: Duration);
    fn set_playing(&mut self, playing: bool);
    /// Clamps to the duration.
    fn seek(&mut self, position: Duration);
    /// From 0 to 100.
    fn set_volume(&mut self, volume: u8);
    fn position(&self) -> Duration;
    fn duration(&self) -> Duration;
}

/// The only way progress and the end of a track reach the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineEvent {
    Progress,
    Ended,
}
