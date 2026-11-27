//! What the UI is told about playback. Mirrored in `src/lib/ipc.ts`. Nothing
//! here grows with the length of the Queue, except a slice, which is capped.

use serde::Serialize;

use crate::catalog::TrackRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    Idle,
    Playing,
    Paused,
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
    /// How many entries come after the current one.
    pub upcoming: usize,
    /// Grows when the Queue changes.
    pub queue_revision: u64,
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
