//! What the UI is told about playback. Mirrored in `src/lib/ipc.ts`. Nothing
//! here grows with the length of the Queue, except a slice, which is capped.

use serde::Serialize;

use crate::catalog::TrackRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    Idle,
    /// Real audio is being opened, and plays once it is.
    Loading,
    Playing,
    Paused,
    /// A stream was lost and a new one is on its way.
    Reconnecting,
    /// Real audio played to its end.
    Ended,
    Error,
}

/// The current Queue entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub key: u32,
    pub track: TrackRow,
    /// Whether the Recommender picked it.
    pub autoplay: bool,
}

/// Everything about playback but the Queue's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    /// Grows with every change except the passing of time.
    pub revision: u64,
    pub status: PlaybackStatus,
    pub current: Option<NowPlaying>,
    pub position_ms: u32,
    /// 0 while idle.
    pub duration_ms: u32,
    /// From 0 to 100.
    pub volume: u8,
    /// Whether there is an entry to go to, queued or recommendable.
    pub can_next: bool,
    /// Whether there is a Queue to go back in or restart.
    pub can_previous: bool,
    /// How many entries come after the current one.
    pub upcoming: usize,
    /// How many entries came before it.
    pub history: usize,
    /// Grows when the Queue changes.
    pub queue_revision: u64,
    /// Why playback failed, in words for the user. Set in the error state only.
    pub error: Option<String>,
    /// The latest queued song that could not be played and was passed over.
    pub skipped: Option<Box<Skipped>>,
}

/// A queued song that was skipped because it could not be played.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    /// The Queue entry that was passed over, so the UI says it once.
    pub key: u32,
    /// In words for the user: the title, never where the audio came from.
    pub message: String,
}

/// The clock moving on. It belongs to the state with the same revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPosition {
    pub revision: u64,
    pub position_ms: u32,
}

/// One upcoming Queue entry as a row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub key: u32,
    pub track: TrackRow,
    /// Whether the Recommender picked it.
    pub autoplay: bool,
}

/// Some consecutive upcoming entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSlice {
    pub queue_revision: u64,
    pub offset: usize,
    /// How many upcoming entries there are in all.
    pub total: usize,
    pub items: Vec<QueueItem>,
}

/// Something to announce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    Changed(PlayerState),
    Position(PlayerPosition),
}
