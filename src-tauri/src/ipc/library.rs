//! Likes and history: commands in, events out. The rules live in `library`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use uuid::Uuid;

use super::playback::play_stream;
use super::player::PlayerHandle;
use super::{EventSink, IpcError, SharedState, lock};
use crate::audio::PlayerError;
use crate::catalog::{SourceKind, Track, TrackRow, TrackSlice};
use crate::library::{HistoryChange, HistoryWriter, Library, LikeChange};
use crate::sources::youtube::SystemClock;

/// Events sent to the UI. Mirrored in `src/lib/ipc.ts`.
pub const LIKES_CHANGED: &str = "likes-changed";
pub const HISTORY_CHANGED: &str = "history-changed";

/// The file in the app's local data folder.
const FILE_NAME: &str = "library.db";

/// A song was liked or unliked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LikesChanged {
    /// Rises with every library event, so an older one can be dropped.
    pub revision: u64,
    pub track_id: Uuid,
    pub liked: bool,
    pub liked_count: usize,
}

/// A play started, so the songs played changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryChanged {
    pub revision: u64,
    pub played_count: usize,
}

/// What the Library view starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    /// The revision of the newest event this snapshot already includes.
    pub revision: u64,
    pub liked_count: usize,
    pub played_count: usize,
    /// Why the library cannot be used, in words. The file is left as it is.
    pub problem: Option<String>,
}

/// The library as the app holds it: open, or why it is not.
pub struct LibraryHandle {
    library: Result<Arc<Library>, String>,
    writer: Option<Arc<HistoryWriter>>,
    revision: AtomicU64,
}

impl LibraryHandle {
    pub fn available(library: Arc<Library>, writer: Option<Arc<HistoryWriter>>) -> Self {
        Self {
            library: Ok(library),
            writer,
            revision: AtomicU64::new(0),
        }
    }

    pub fn unavailable(problem: String) -> Self {
        Self {
            library: Err(problem),
            writer: None,
            revision: AtomicU64::new(0),
        }
    }

    fn library(&self) -> Result<&Arc<Library>, IpcError> {
        self.library
            .as_ref()
            .map_err(|problem| IpcError::LibraryUnavailable(problem.clone()))
    }

    fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn summary(&self) -> LibrarySummary {
        let revision = self.revision.load(Ordering::SeqCst);
        match &self.library {
            Ok(library) => LibrarySummary {
                revision,
                liked_count: library.liked_count().unwrap_or(0),
                played_count: library.played_count().unwrap_or(0),
                problem: None,
            },
            Err(problem) => LibrarySummary {
                revision,
                liked_count: 0,
                played_count: 0,
                problem: Some(problem.clone()),
            },
        }
    }
}

/// Opens the library and starts writing history to it. A file that cannot be
/// used is reported through the commands and left as it is.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let path = app.path().app_local_data_dir()?.join(FILE_NAME);
    let handle = match Library::open(&path, SystemClock::default()) {
        Ok(library) => {
            let library = Arc::new(library);
            let sink = app.clone();
            let writer = HistoryWriter::start(Arc::clone(&library), move |change| {
                let handle = sink.state::<LibraryHandle>();
                // A failure means the window is gone: nobody is left to tell.
                let _ = announce_history(&handle, change, &sink);
            });
            match writer {
                Ok(writer) => LibraryHandle::available(library, Some(Arc::new(writer))),
                Err(error) => LibraryHandle::unavailable(error.to_string()),
            }
        }
        Err(error) => LibraryHandle::unavailable(error.to_string()),
    };
    app.manage(handle);
    Ok(())
}

/// Writes the plays still waiting, so none is lost when the app closes.
pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    if let Some(handle) = app.try_state::<LibraryHandle>()
        && let Some(writer) = &handle.writer
    {
        writer.finish();
    }
}

/// What a play log should write through, if the library is open.
pub fn play_log<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<HistoryWriter>> {
    app.try_state::<LibraryHandle>()?.writer.clone()
}

fn announce_history(
    handle: &LibraryHandle,
    change: HistoryChange,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    sink.history_changed(&HistoryChanged {
        revision: handle.next_revision(),
        played_count: change.played_count,
    })
}

fn announce_like(
    handle: &LibraryHandle,
    change: LikeChange,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    if !change.changed {
        return Ok(());
    }
    sink.likes_changed(&LikesChanged {
        revision: handle.next_revision(),
        track_id: change.track_id,
        liked: change.liked,
        liked_count: change.liked_count,
    })
}

fn like_and_notify(
    handle: &LibraryHandle,
    track: &Track,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    let change = handle.library()?.like(track)?;
    announce_like(handle, change, sink)
}

fn unlike_and_notify(
    handle: &LibraryHandle,
    id: Uuid,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    let change = handle.library()?.unlike(id)?;
    announce_like(handle, change, sink)
}

/// The YouTube video a song plays from.
fn video_of(track: &Track) -> Result<String, IpcError> {
    track
        .sources
        .iter()
        .find(|source| source.kind == SourceKind::YouTube)
        .map(|source| source.id.clone())
        .ok_or(IpcError::NotPlayable)
}

#[tauri::command]
pub fn get_library_summary(library: State<'_, LibraryHandle>) -> LibrarySummary {
    library.summary()
}

/// Likes a song from the results on show, the catalog, what plays now, or what
/// the library already holds.
#[tauri::command]
pub fn like_track<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    player: State<'_, PlayerHandle>,
    library: State<'_, LibraryHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    // The app's lock is released before the player's is taken.
    let known = lock(&state).known_track(track_id);
    let track = match known {
        Some(track) => track,
        None => match player.current_track().filter(|track| track.id == track_id) {
            Some(track) => track,
            None => library
                .library()?
                .track(track_id)?
                .ok_or(PlayerError::UnknownTrack { id: track_id })?,
        },
    };
    like_and_notify(&library, &track, &app)
}

#[tauri::command]
pub fn unlike_track<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    unlike_and_notify(&library, track_id, &app)
}

/// Which of the songs shown are liked.
#[tauri::command]
pub fn get_liked_ids(
    library: State<'_, LibraryHandle>,
    track_ids: Vec<Uuid>,
) -> Result<Vec<Uuid>, IpcError> {
    Ok(library.library()?.liked_among(&track_ids)?)
}

/// The visible window of the liked songs. The rest stay in the database.
#[tauri::command]
pub fn get_liked_slice(
    library: State<'_, LibraryHandle>,
    offset: usize,
    limit: usize,
) -> Result<TrackSlice, IpcError> {
    Ok(library.library()?.liked_slice(offset, limit)?)
}

/// The visible window of the songs played, the latest first.
#[tauri::command]
pub fn get_recent_slice(
    library: State<'_, LibraryHandle>,
    offset: usize,
    limit: usize,
) -> Result<TrackSlice, IpcError> {
    Ok(library.library()?.recent_slice(offset, limit)?)
}

/// Plays a song of the library by itself.
#[tauri::command]
pub fn play_library_track<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    player: State<'_, PlayerHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    let track = library
        .library()?
        .track(track_id)?
        .ok_or(PlayerError::UnknownTrack { id: track_id })?;
    let video = video_of(&track)?;
    play_stream(&app, &player, video, TrackRow::from(&track))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::RecordingSink;
    use super::*;
    use crate::library::testing::{library_at, song};

    fn handle() -> LibraryHandle {
        let dir = tempfile::tempdir().unwrap().keep();
        let (library, _) = library_at(&dir.join("lumen.db"));
        LibraryHandle::available(Arc::new(library), None)
    }

    #[test]
    fn a_like_is_announced_once_with_the_new_count() {
        let handle = handle();
        let sink = RecordingSink::default();

        like_and_notify(&handle, &song(1), &sink).unwrap();
        like_and_notify(&handle, &song(1), &sink).unwrap();

        assert_eq!(
            *sink.likes.borrow(),
            [LikesChanged {
                revision: 1,
                track_id: song(1).id,
                liked: true,
                liked_count: 1
            }]
        );
    }

    #[test]
    fn an_unlike_is_announced_and_unliking_what_is_not_liked_is_quiet() {
        let handle = handle();
        let sink = RecordingSink::default();
        like_and_notify(&handle, &song(1), &sink).unwrap();

        unlike_and_notify(&handle, song(1).id, &sink).unwrap();
        unlike_and_notify(&handle, song(1).id, &sink).unwrap();
        unlike_and_notify(&handle, song(2).id, &sink).unwrap();

        let likes = sink.likes.borrow();
        assert_eq!(likes.len(), 2);
        assert_eq!(
            likes[1],
            LikesChanged {
                revision: 2,
                track_id: song(1).id,
                liked: false,
                liked_count: 0
            }
        );
    }

    #[test]
    fn a_new_play_is_announced_with_how_many_songs_were_played() {
        let handle = handle();
        let sink = RecordingSink::default();

        announce_history(&handle, HistoryChange { played_count: 4 }, &sink).unwrap();

        assert_eq!(
            *sink.histories.borrow(),
            [HistoryChanged {
                revision: 1,
                played_count: 4
            }]
        );
    }

    #[test]
    fn a_library_that_cannot_be_used_says_why_and_changes_nothing() {
        let handle = LibraryHandle::unavailable("the library was made by a newer Lumen".to_owned());
        let sink = RecordingSink::default();

        let error = like_and_notify(&handle, &song(1), &sink).expect_err("no library");

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!("the library was made by a newer Lumen")
        );
        assert!(sink.likes.borrow().is_empty());
        assert_eq!(
            handle.summary().problem.as_deref(),
            Some("the library was made by a newer Lumen")
        );
    }

    #[test]
    fn the_summary_counts_what_is_kept() {
        let handle = handle();
        let sink = RecordingSink::default();
        like_and_notify(&handle, &song(1), &sink).unwrap();

        assert_eq!(
            handle.summary(),
            LibrarySummary {
                revision: 1,
                liked_count: 1,
                played_count: 0,
                problem: None
            }
        );
    }

    #[test]
    fn a_song_plays_from_its_youtube_ref_only() {
        assert_eq!(video_of(&song(3)).unwrap(), "video000003");
        let mut elsewhere = song(3);
        elsewhere.sources[0].kind = SourceKind::Spotify;

        assert!(matches!(video_of(&elsewhere), Err(IpcError::NotPlayable)));
    }

    #[test]
    fn like_events_serialize_for_the_ui() {
        let event = LikesChanged {
            revision: 2,
            track_id: Uuid::from_u128(7),
            liked: true,
            liked_count: 3,
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "revision": 2,
                "trackId": "00000000-0000-0000-0000-000000000007",
                "liked": true,
                "likedCount": 3
            })
        );
        assert_eq!(
            serde_json::to_value(HistoryChanged {
                revision: 5,
                played_count: 9
            })
            .unwrap(),
            json!({ "revision": 5, "playedCount": 9 })
        );
    }
}
