//! The seam to whatever makes the sound.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// What an engine is asked to play. Resolving happens before this.
#[derive(Clone, PartialEq, Eq)]
pub enum Media {
    /// A catalog Track with no audio behind it.
    Sample { duration: Duration },
    /// A file on this computer.
    File(PathBuf),
    /// A Stream URL and the headers it must be fetched with.
    Stream {
        url: String,
        headers: Vec<(String, String)>,
        /// Unix seconds after which the URL stops working, if it says.
        expires_at: Option<u64>,
    },
}

/// Never shows a Stream URL or its headers, so neither can end up in a log.
impl fmt::Debug for Media {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sample { duration } => f.debug_tuple("Sample").field(duration).finish(),
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
            Self::Stream { .. } => f.write_str("Stream(..)"),
        }
    }
}

/// Why media could not be loaded. It carries nothing from the engine, whose own
/// messages can hold a Stream URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("the file was not found")]
    NotFound,
    #[error("this audio cannot be played")]
    Unplayable,
}

/// What actually makes sound: libmpv in the app, stand-ins in tests.
pub trait Engine: Send {
    /// Whether catalog sample Tracks can be loaded.
    fn plays_samples(&self) -> bool;
    /// Takes new media in place of the old, paused at `start`. On an error
    /// nothing is left loaded.
    fn load(&mut self, media: &Media, start: Duration) -> Result<(), EngineError>;
    /// Lets go of the media. Nothing is loaded afterwards.
    fn stop(&mut self);
    fn set_playing(&mut self, playing: bool);
    /// The caller clamps to the duration.
    fn seek(&mut self, position: Duration);
    /// From 0 to 100.
    fn set_volume(&mut self, volume: u8);
    fn position(&self) -> Duration;
    /// Zero while unknown.
    fn duration(&self) -> Duration;
}

impl<E: Engine + ?Sized> Engine for Box<E> {
    fn plays_samples(&self) -> bool {
        (**self).plays_samples()
    }

    fn load(&mut self, media: &Media, start: Duration) -> Result<(), EngineError> {
        (**self).load(media, start)
    }

    fn stop(&mut self) {
        (**self).stop();
    }

    fn set_playing(&mut self, playing: bool) {
        (**self).set_playing(playing);
    }

    fn seek(&mut self, position: Duration) {
        (**self).seek(position);
    }

    fn set_volume(&mut self, volume: u8) {
        (**self).set_volume(volume);
    }

    fn position(&self) -> Duration {
        (**self).position()
    }

    fn duration(&self) -> Duration {
        (**self).duration()
    }
}

/// The only way progress, the end and failures reach the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineEvent {
    /// The media is open and can play.
    Loaded,
    /// The length became known or changed.
    Duration,
    Progress,
    Ended,
    /// Loading or playing went wrong.
    Failed,
}
