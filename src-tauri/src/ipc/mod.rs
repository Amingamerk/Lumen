//! Tauri commands in, events out. No business logic lives here.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::catalog::{CollectionDetail, Library, SearchView, Shelf, SourceFilter, TrackSlice};
use crate::state::{AppSnapshot, AppState, NavigationState, Route, StateError};

/// Event sent after the route changes. Mirrored in `src/lib/ipc.ts`.
pub const NAVIGATION_CHANGED: &str = "navigation-changed";

pub type SharedState = Mutex<AppState>;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error("could not notify the window: {0}")]
    Emit(#[from] tauri::Error),
}

/// The webview receives the message as a plain string.
impl Serialize for IpcError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Where state changes are announced. A seam so tests need no running window.
pub trait EventSink {
    fn navigation_changed(&self, navigation: &NavigationState) -> Result<(), IpcError>;
}

impl<R: Runtime> EventSink for AppHandle<R> {
    fn navigation_changed(&self, navigation: &NavigationState) -> Result<(), IpcError> {
        Ok(self.emit(NAVIGATION_CHANGED, navigation)?)
    }
}

/// A panic elsewhere must not take the whole UI down with a poisoned lock.
fn lock(state: &SharedState) -> MutexGuard<'_, AppState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Applies a route change and announces it if anything changed.
fn navigate_and_notify(
    state: &SharedState,
    route: Route,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    // The lock is released before the event goes out. The revision keeps order.
    let changed = lock(state).navigate(route)?;
    if let Some(navigation) = changed {
        sink.navigation_changed(&navigation)?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_app_state(state: State<'_, SharedState>) -> AppSnapshot {
    lock(&state).snapshot()
}

#[tauri::command]
pub fn navigate<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    route: Route,
) -> Result<(), IpcError> {
    navigate_and_notify(&state, route, &app)
}

#[tauri::command]
pub fn get_home(state: State<'_, SharedState>) -> Vec<Shelf> {
    lock(&state).home()
}

#[tauri::command]
pub fn get_library(state: State<'_, SharedState>) -> Library {
    lock(&state).library()
}

#[tauri::command]
pub fn get_collection(
    state: State<'_, SharedState>,
    id: String,
) -> Result<CollectionDetail, IpcError> {
    Ok(lock(&state).collection(&id)?)
}

/// The visible window of a long list. The rest of its rows stay in Rust.
#[tauri::command]
pub fn get_track_slice(
    state: State<'_, SharedState>,
    playlist_id: String,
    offset: usize,
    limit: usize,
) -> Result<TrackSlice, IpcError> {
    Ok(lock(&state).track_slice(&playlist_id, offset, limit)?)
}

#[tauri::command]
pub fn get_search(state: State<'_, SharedState>) -> SearchView {
    lock(&state).search_view()
}

#[tauri::command]
pub fn search_catalog(
    state: State<'_, SharedState>,
    query: String,
    filter: SourceFilter,
) -> SearchView {
    lock(&state).search(&query, filter)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct RecordingSink(RefCell<Vec<NavigationState>>);

    impl EventSink for RecordingSink {
        fn navigation_changed(&self, navigation: &NavigationState) -> Result<(), IpcError> {
            self.0.borrow_mut().push(navigation.clone());
            Ok(())
        }
    }

    #[test]
    fn announces_a_route_change_once_with_the_new_state() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        navigate_and_notify(&state, Route::Search, &sink).unwrap();

        assert_eq!(
            *sink.0.borrow(),
            [NavigationState {
                route: Route::Search,
                revision: 1
            }]
        );
    }

    #[test]
    fn stays_quiet_when_the_route_does_not_change() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        navigate_and_notify(&state, Route::Home, &sink).unwrap();

        assert!(sink.0.borrow().is_empty());
    }

    #[test]
    fn reports_an_unknown_playlist_without_announcing() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        let result = navigate_and_notify(
            &state,
            Route::Playlist {
                id: "nope".to_owned(),
            },
            &sink,
        );

        assert!(matches!(result, Err(IpcError::State(_))));
        assert!(sink.0.borrow().is_empty());
    }

    #[test]
    fn errors_serialize_as_their_message() {
        let error = IpcError::from(StateError::UnknownCollection {
            id: "nope".to_owned(),
        });

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!("no playlist or album has the id “nope”")
        );
    }

    #[test]
    fn navigation_event_payload_has_route_and_revision() {
        let navigation = NavigationState {
            route: Route::Playlist {
                id: "p1".to_owned(),
            },
            revision: 3,
        };

        assert_eq!(
            serde_json::to_value(&navigation).unwrap(),
            json!({ "route": { "view": "playlist", "id": "p1" }, "revision": 3 })
        );
    }
}
