//! Likes and history: commands in, events out. The rules live in `library`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use uuid::Uuid;

use super::playback::play_stream;
use super::player::PlayerHandle;
use super::{EventSink, IpcError, SharedState, lock};
use crate::audio::PlayerError;
use crate::catalog::{Catalog, LONG_PLAYLIST_ID, SourceKind, Track, TrackRow, TrackSlice};
use crate::library::{HistoryChange, HistoryWriter, Library, LibraryError, LikeChange};
use crate::sources::youtube::SystemClock;

/// Events sent to the UI. Mirrored in `src/lib/ipc.ts`.
pub const LIKES_CHANGED: &str = "likes-changed";
pub const HISTORY_CHANGED: &str = "history-changed";

/// The file in the app's local data folder.
const FILE_NAME: &str = "library.db";

/// The folder in the system's temp folder that a `--list` run uses instead.
const THROWAWAY_FOLDER: &str = "lumen-list";

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

/// The library as the app holds it: open, or why it is not. Clones share
/// everything, so one can move to a blocking thread.
#[derive(Clone)]
pub struct LibraryHandle {
    library: Result<Arc<Library>, String>,
    writer: Option<Arc<HistoryWriter>>,
    revision: Arc<AtomicU64>,
}

impl LibraryHandle {
    pub fn available(library: Arc<Library>, writer: Option<Arc<HistoryWriter>>) -> Self {
        Self {
            library: Ok(library),
            writer,
            revision: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn unavailable(problem: String) -> Self {
        Self {
            library: Err(problem),
            writer: None,
            revision: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) fn library(&self) -> Result<&Arc<Library>, IpcError> {
        self.library
            .as_ref()
            .map_err(|problem| IpcError::LibraryUnavailable(problem.clone()))
    }

    pub(super) fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The revision of the newest library event sent so far.
    pub(super) fn current_revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
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

/// Where the library lives. A `--list` run gets a file of its own in the temp
/// folder, so it never reads or changes the user's library.
fn library_path(list: bool, data_dir: &Path, temp_dir: &Path) -> PathBuf {
    if list {
        temp_dir.join(THROWAWAY_FOLDER).join(FILE_NAME)
    } else {
        data_dir.join(FILE_NAME)
    }
}

/// Opens the library for this run. A `--list` run starts from an empty
/// throwaway library holding the sample long playlist.
fn open_library(list: bool, data_dir: &Path, temp_dir: &Path) -> Result<Library, LibraryError> {
    let path = library_path(list, data_dir, temp_dir);
    if list {
        // An earlier run's leftovers: a missing folder is as good as an empty one.
        let _ = std::fs::remove_dir_all(temp_dir.join(THROWAWAY_FOLDER));
    }
    let library = Library::open(&path, SystemClock::default())?;
    if list {
        let samples = Catalog::default();
        let tracks: Vec<Track> = samples
            .listed(LONG_PLAYLIST_ID)
            .unwrap_or_default()
            .iter()
            .filter_map(|id| samples.track(*id).cloned())
            .collect();
        let name = samples
            .collection(LONG_PLAYLIST_ID)
            .map_or_else(|| LONG_PLAYLIST_ID.to_owned(), |c| c.title);
        library.create_playlist_with(LONG_PLAYLIST_ID, &name, &tracks)?;
    }
    Ok(library)
}

/// Opens the library and starts writing history to it. A file that cannot be
/// used is reported through the commands and left as it is.
pub fn start<R: Runtime>(app: &AppHandle<R>, list: bool) -> anyhow::Result<()> {
    let data_dir = app.path().app_local_data_dir()?;
    let handle = match open_library(list, &data_dir, &std::env::temp_dir()) {
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
        && !writer.finish()
    {
        eprintln!("lumen: the history writer did not finish in time; a play may be missing");
    }
}

/// What a play log should write through, if the library is open.
pub fn play_log<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<HistoryWriter>> {
    app.try_state::<LibraryHandle>()?.writer.clone()
}

/// The open library, for what is kept besides likes and history. `None` when it
/// could not be opened.
pub fn open_shared<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<Library>> {
    let handle = app.try_state::<LibraryHandle>()?;
    handle.library().ok().cloned()
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

/// Runs database work on a blocking thread. Commands that wait for the library
/// must never wait on the thread that serves the window.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| IpcError::LibraryUnavailable("the library work was cut short".to_owned()))?
}

/// The song a command is about: the one the caller already holds, or the one
/// the library keeps under that id.
pub(super) fn find_track(
    handle: &LibraryHandle,
    known: Option<Track>,
    id: Uuid,
) -> Result<Track, IpcError> {
    match known {
        Some(track) => Ok(track),
        None => Ok(handle
            .library()?
            .track(id)?
            .ok_or(PlayerError::UnknownTrack { id })?),
    }
}

#[tauri::command]
pub async fn get_library_summary(
    library: State<'_, LibraryHandle>,
) -> Result<LibrarySummary, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.summary())).await
}

/// Likes a song from the results on show, the catalog, what plays now, or what
/// the library already holds.
#[tauri::command]
pub async fn like_track<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    player: State<'_, PlayerHandle>,
    library: State<'_, LibraryHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    // The app's lock is released before the player's is taken.
    let known = lock(&state).known_track(track_id);
    let known = known.or_else(|| player.current_track().filter(|track| track.id == track_id));
    let handle = library.inner().clone();
    blocking(move || {
        let track = find_track(&handle, known, track_id)?;
        like_and_notify(&handle, &track, &app)
    })
    .await
}

#[tauri::command]
pub async fn unlike_track<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    let handle = library.inner().clone();
    blocking(move || unlike_and_notify(&handle, track_id, &app)).await
}

/// Which of the songs shown are liked.
#[tauri::command]
pub async fn get_liked_ids(
    library: State<'_, LibraryHandle>,
    track_ids: Vec<Uuid>,
) -> Result<Vec<Uuid>, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.library()?.liked_among(&track_ids)?)).await
}

/// The visible window of the liked songs. The rest stay in the database.
#[tauri::command]
pub async fn get_liked_slice(
    library: State<'_, LibraryHandle>,
    offset: usize,
    limit: usize,
) -> Result<TrackSlice, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.library()?.liked_slice(offset, limit)?)).await
}

/// The visible window of the songs played, the latest first.
#[tauri::command]
pub async fn get_recent_slice(
    library: State<'_, LibraryHandle>,
    offset: usize,
    limit: usize,
) -> Result<TrackSlice, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.library()?.recent_slice(offset, limit)?)).await
}

/// Plays a song of the library by itself.
#[tauri::command]
pub async fn play_library_track<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    player: State<'_, PlayerHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    let handle = library.inner().clone();
    let track = blocking(move || {
        Ok(handle
            .library()?
            .track(track_id)?
            .ok_or(PlayerError::UnknownTrack { id: track_id })?)
    })
    .await?;
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
    fn a_list_run_opens_a_throwaway_library_and_leaves_the_users_alone() {
        let data = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user = data.path().join(FILE_NAME);
        let (mine, _) = library_at(&user);
        mine.like(&song(1)).unwrap();
        mine.create_playlist("Mine").unwrap();
        drop(mine);
        let before = std::fs::read(&user).unwrap();
        // A throwaway left by an earlier run does not carry over.
        let leftover = library_path(true, data.path(), temp.path());
        let (stale, _) = library_at(&leftover);
        stale.create_playlist("Stale").unwrap();
        drop(stale);

        let library = open_library(true, data.path(), temp.path()).unwrap();

        assert!(leftover.starts_with(temp.path()));
        assert_ne!(leftover, user);
        let names: Vec<String> = library
            .playlists()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["The long haul"]);
        let long = library.playlist(LONG_PLAYLIST_ID).unwrap();
        assert_eq!(long.track_count, 5_000);
        drop(library);
        assert_eq!(std::fs::read(&user).unwrap(), before);
        assert_eq!(library_path(false, data.path(), temp.path()), user);
    }

    #[test]
    fn a_normal_run_opens_the_users_library_with_no_sample_playlist() {
        let data = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        let library = open_library(false, data.path(), temp.path()).unwrap();

        assert!(library.playlists().unwrap().is_empty());
        assert!(data.path().join(FILE_NAME).exists());
        assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
    }

    #[test]
    fn library_commands_wait_on_a_thread_of_their_own() {
        let here = std::thread::current().id();

        let there = tauri::async_runtime::block_on(blocking(|| Ok(std::thread::current().id())));

        assert_ne!(there.unwrap(), here);
    }

    #[test]
    fn a_failure_on_the_blocking_thread_reaches_the_caller() {
        let result: Result<(), IpcError> =
            tauri::async_runtime::block_on(blocking(|| Err(IpcError::NotPlayable)));

        assert!(matches!(result, Err(IpcError::NotPlayable)));
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
