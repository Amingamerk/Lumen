//! Local playlists: commands in, events out. The rules live in `library`.

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use uuid::Uuid;

use super::library::{LibraryHandle, blocking, find_track};
use super::player::PlayerHandle;
use super::{EventSink, IpcError, SharedState, lock};
use crate::catalog::{Track, TrackSlice};
use crate::library::PlaylistSummary;
use crate::state::Route;

/// Event sent after a playlist is made, renamed, removed or changed. Mirrored
/// in `src/lib/ipc.ts`.
pub const PLAYLISTS_CHANGED: &str = "playlists-changed";

/// Every playlist as it stands, and which one just changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistsChanged {
    /// Rises with every library event, so an older one can be dropped.
    pub revision: u64,
    pub playlists: Vec<PlaylistSummary>,
    /// The playlist whose name or entries changed, if one did.
    pub changed: Option<String>,
}

impl LibraryHandle {
    /// The playlists now, or none if the library cannot be used. The reason is
    /// told by the library summary.
    fn playlists_view(&self, changed: Option<String>) -> PlaylistsChanged {
        let playlists = self
            .library()
            .and_then(|library| Ok(library.playlists()?))
            .unwrap_or_default();
        PlaylistsChanged {
            revision: self.current_revision(),
            playlists,
            changed,
        }
    }
}

fn announce(handle: &LibraryHandle, changed: &str, sink: &impl EventSink) -> Result<(), IpcError> {
    let revision = handle.next_revision();
    let view = PlaylistsChanged {
        revision,
        ..handle.playlists_view(Some(changed.to_owned()))
    };
    sink.playlists_changed(&view)
}

fn create_and_notify(
    handle: &LibraryHandle,
    name: &str,
    sink: &impl EventSink,
) -> Result<PlaylistSummary, IpcError> {
    let made = handle.library()?.create_playlist(name)?;
    announce(handle, &made.id, sink)?;
    Ok(made)
}

fn rename_and_notify(
    handle: &LibraryHandle,
    id: &str,
    name: &str,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    handle.library()?.rename_playlist(id, name)?;
    announce(handle, id, sink)
}

/// Removes a playlist. If its page is open, the app moves to the Library.
fn delete_and_notify(
    handle: &LibraryHandle,
    state: &SharedState,
    id: &str,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    handle.library()?.delete_playlist(id)?;
    announce(handle, id, sink)?;
    let left = lock(state).leave_playlist(id);
    if let Some(navigation) = left {
        sink.navigation_changed(&navigation)?;
    }
    Ok(())
}

fn add_and_notify(
    handle: &LibraryHandle,
    id: &str,
    track: &Track,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    handle.library()?.add_to_playlist(id, track)?;
    announce(handle, id, sink)
}

/// A removal the list has moved on from changes nothing and says nothing.
fn remove_and_notify(
    handle: &LibraryHandle,
    id: &str,
    position: usize,
    track_id: Uuid,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    if handle
        .library()?
        .remove_from_playlist(id, position, track_id)?
    {
        announce(handle, id, sink)?;
    }
    Ok(())
}

/// Opens a playlist page. The library says whether the playlist exists.
pub(super) fn open_and_notify(
    handle: &LibraryHandle,
    state: &SharedState,
    id: &str,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    if !handle.library()?.has_playlist(id)? {
        return Err(crate::library::LibraryError::NoPlaylist { id: id.to_owned() }.into());
    }
    let changed = lock(state).navigate_known(Route::Playlist { id: id.to_owned() });
    if let Some(navigation) = changed {
        sink.navigation_changed(&navigation)?;
    }
    Ok(())
}

/// Every playlist, to start from.
#[tauri::command]
pub async fn get_playlists(
    library: State<'_, LibraryHandle>,
) -> Result<PlaylistsChanged, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.playlists_view(None))).await
}

#[tauri::command]
pub async fn get_playlist(
    library: State<'_, LibraryHandle>,
    id: String,
) -> Result<PlaylistSummary, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.library()?.playlist(&id)?)).await
}

/// The visible window of a playlist. The rest stays in the database.
#[tauri::command]
pub async fn get_playlist_slice(
    library: State<'_, LibraryHandle>,
    id: String,
    offset: usize,
    limit: usize,
) -> Result<TrackSlice, IpcError> {
    let handle = library.inner().clone();
    blocking(move || Ok(handle.library()?.playlist_slice(&id, offset, limit)?)).await
}

#[tauri::command]
pub async fn create_playlist<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    name: String,
) -> Result<PlaylistSummary, IpcError> {
    let handle = library.inner().clone();
    blocking(move || create_and_notify(&handle, &name, &app)).await
}

#[tauri::command]
pub async fn rename_playlist<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    id: String,
    name: String,
) -> Result<(), IpcError> {
    let handle = library.inner().clone();
    blocking(move || rename_and_notify(&handle, &id, &name, &app)).await
}

#[tauri::command]
pub async fn delete_playlist<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    id: String,
) -> Result<(), IpcError> {
    let handle = library.inner().clone();
    blocking(move || {
        let state = app.state::<SharedState>();
        delete_and_notify(&handle, &state, &id, &app)
    })
    .await
}

/// Puts a song from the results on show, the library or what plays now at the
/// end of a playlist. Adding it again makes a second entry.
#[tauri::command]
pub async fn add_to_playlist<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    player: State<'_, PlayerHandle>,
    library: State<'_, LibraryHandle>,
    id: String,
    track_id: Uuid,
) -> Result<(), IpcError> {
    // The app's lock is released before the player's is taken.
    let known = lock(&state).known_track(track_id);
    let known = known.or_else(|| player.current_track().filter(|track| track.id == track_id));
    let handle = library.inner().clone();
    blocking(move || {
        let track = find_track(&handle, known, track_id)?;
        add_and_notify(&handle, &id, &track, &app)
    })
    .await
}

/// Takes the entry at `position` out, if it still holds `track_id`.
#[tauri::command]
pub async fn remove_from_playlist<R: Runtime>(
    app: AppHandle<R>,
    library: State<'_, LibraryHandle>,
    id: String,
    position: usize,
    track_id: Uuid,
) -> Result<(), IpcError> {
    let handle = library.inner().clone();
    blocking(move || remove_and_notify(&handle, &id, position, track_id, &app)).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::super::tests::RecordingSink;
    use super::*;
    use crate::library::testing::{library_at, song};

    fn handle() -> LibraryHandle {
        let dir = tempfile::tempdir().unwrap().keep();
        let (library, _) = library_at(&dir.join("lumen.db"));
        LibraryHandle::available(Arc::new(library), None)
    }

    fn names(change: &PlaylistsChanged) -> Vec<(&str, usize)> {
        change
            .playlists
            .iter()
            .map(|p| (p.name.as_str(), p.track_count))
            .collect()
    }

    #[test]
    fn every_change_announces_the_playlists_as_they_now_stand() {
        let handle = handle();
        let sink = RecordingSink::default();

        let made = create_and_notify(&handle, " Night shift ", &sink).unwrap();
        add_and_notify(&handle, &made.id, &song(1), &sink).unwrap();
        add_and_notify(&handle, &made.id, &song(1), &sink).unwrap();
        rename_and_notify(&handle, &made.id, "Late", &sink).unwrap();
        remove_and_notify(&handle, &made.id, 0, song(1).id, &sink).unwrap();

        let events = sink.playlists.borrow();
        let revisions: Vec<u64> = events.iter().map(|e| e.revision).collect();
        assert_eq!(revisions, [1, 2, 3, 4, 5]);
        assert!(
            events
                .iter()
                .all(|e| e.changed.as_deref() == Some(made.id.as_str()))
        );
        assert_eq!(names(&events[0]), [("Night shift", 0)]);
        assert_eq!(names(&events[2]), [("Night shift", 2)]);
        assert_eq!(names(&events[4]), [("Late", 1)]);
    }

    #[test]
    fn a_removal_the_list_moved_on_from_says_nothing() {
        let handle = handle();
        let sink = RecordingSink::default();
        let made = create_and_notify(&handle, "Mix", &sink).unwrap();
        add_and_notify(&handle, &made.id, &song(1), &sink).unwrap();

        remove_and_notify(&handle, &made.id, 0, song(2).id, &sink).unwrap();
        remove_and_notify(&handle, &made.id, 9, song(1).id, &sink).unwrap();

        assert_eq!(sink.playlists.borrow().len(), 2);
        assert_eq!(
            handle
                .library()
                .unwrap()
                .playlist(&made.id)
                .unwrap()
                .track_count,
            1
        );
    }

    #[test]
    fn a_failed_change_announces_nothing_and_says_why() {
        let handle = handle();
        let sink = RecordingSink::default();

        let blank = create_and_notify(&handle, "  ", &sink).expect_err("blank");
        let missing = add_and_notify(&handle, "nope", &song(1), &sink).expect_err("missing");

        assert_eq!(
            serde_json::to_value(&blank).unwrap(),
            json!("a playlist needs a name")
        );
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            json!("there is no playlist with the id “nope”")
        );
        assert!(sink.playlists.borrow().is_empty());
    }

    #[test]
    fn removing_the_open_playlist_moves_to_the_library() {
        let handle = handle();
        let sink = RecordingSink::default();
        let state = SharedState::default();
        let made = create_and_notify(&handle, "Mix", &sink).unwrap();
        open_and_notify(&handle, &state, &made.id, &sink).unwrap();
        let other = create_and_notify(&handle, "Other", &sink).unwrap();

        delete_and_notify(&handle, &state, &other.id, &sink).unwrap();
        assert_eq!(sink.navigations.borrow().len(), 1);
        delete_and_notify(&handle, &state, &made.id, &sink).unwrap();

        let navigations = sink.navigations.borrow();
        assert_eq!(navigations.len(), 2);
        assert_eq!(navigations[1].route, Route::Library);
        assert!(sink.playlists.borrow().last().unwrap().playlists.is_empty());
    }

    #[test]
    fn only_a_playlist_the_library_holds_can_be_opened() {
        let handle = handle();
        let sink = RecordingSink::default();
        let state = SharedState::default();
        let made = create_and_notify(&handle, "Mix", &sink).unwrap();

        let missing = open_and_notify(&handle, &state, "nope", &sink);
        open_and_notify(&handle, &state, &made.id, &sink).unwrap();

        assert!(missing.is_err());
        let navigations = sink.navigations.borrow();
        assert_eq!(navigations.len(), 1);
        assert_eq!(
            navigations[0].route,
            Route::Playlist {
                id: made.id.clone()
            }
        );
    }

    #[test]
    fn a_library_that_cannot_be_used_shows_no_playlists() {
        let handle = LibraryHandle::unavailable("the library was made by a newer Lumen".to_owned());

        let view = handle.playlists_view(None);

        assert!(view.playlists.is_empty());
        assert!(create_and_notify(&handle, "Mix", &RecordingSink::default()).is_err());
    }

    #[test]
    fn the_event_serializes_for_the_ui() {
        let event = PlaylistsChanged {
            revision: 4,
            playlists: vec![PlaylistSummary {
                id: "p1".to_owned(),
                name: "Mix".to_owned(),
                track_count: 2,
                duration_ms: 400_000,
                cover_track: None,
            }],
            changed: Some("p1".to_owned()),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "revision": 4,
                "playlists": [{ "id": "p1", "name": "Mix", "trackCount": 2, "durationMs": 400000, "coverTrack": null }],
                "changed": "p1"
            })
        );
    }
}
