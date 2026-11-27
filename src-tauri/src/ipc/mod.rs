//! Tauri commands in, events out. No business logic lives here.

use std::sync::{Mutex, MutexGuard, PoisonError};

pub mod account;
#[cfg(debug_assertions)]
mod break_url;
pub mod playback;
pub mod player;
mod recovery;
pub mod search;
pub mod youtube;

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::audio::{PlayerError, PlayerPosition, PlayerState};
use crate::catalog::{CollectionDetail, Library, SearchView, Shelf, TrackSlice};
use crate::secrets::SecretError;
use crate::state::{AppSnapshot, AppState, NavigationState, Route, StateError};

/// The label of the app's own window, set in `tauri.conf.json`.
pub const MAIN_WINDOW: &str = "main";

/// Event sent after the route changes. Mirrored in `src/lib/ipc.ts`.
pub const NAVIGATION_CHANGED: &str = "navigation-changed";

pub type SharedState = Mutex<AppState>;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Player(#[from] PlayerError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("the window failed: {0}")]
    Window(#[from] tauri::Error),
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
    fn player_changed(&self, state: &PlayerState) -> Result<(), IpcError>;
    fn player_position(&self, position: &PlayerPosition) -> Result<(), IpcError>;
    fn search_changed(&self, view: &SearchView) -> Result<(), IpcError>;
}

impl<R: Runtime> EventSink for AppHandle<R> {
    fn navigation_changed(&self, navigation: &NavigationState) -> Result<(), IpcError> {
        Ok(self.emit(NAVIGATION_CHANGED, navigation)?)
    }

    fn player_changed(&self, state: &PlayerState) -> Result<(), IpcError> {
        Ok(self.emit(player::PLAYER_CHANGED, state)?)
    }

    fn player_position(&self, position: &PlayerPosition) -> Result<(), IpcError> {
        Ok(self.emit(player::PLAYER_POSITION, position)?)
    }

    fn search_changed(&self, view: &SearchView) -> Result<(), IpcError> {
        Ok(self.emit_to(MAIN_WINDOW, search::SEARCH_CHANGED, view)?)
    }
}

/// Brings the app's window to the front, as a second launch asks for.
pub fn focus_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        // Best effort: the second launch has already exited either way.
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// A panic elsewhere must not take the whole UI down with a poisoned lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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

fn toggle_queue_and_notify(state: &SharedState, sink: &impl EventSink) -> Result<(), IpcError> {
    let navigation = lock(state).toggle_queue();
    sink.navigation_changed(&navigation)
}

#[tauri::command]
pub fn toggle_queue<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
) -> Result<(), IpcError> {
    toggle_queue_and_notify(&state, &app)
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::json;

    use super::*;

    /// Keeps what would have gone to the window.
    #[derive(Default)]
    pub(super) struct RecordingSink {
        pub(super) navigations: RefCell<Vec<NavigationState>>,
        pub(super) states: RefCell<Vec<PlayerState>>,
        pub(super) positions: RefCell<Vec<PlayerPosition>>,
        pub(super) searches: RefCell<Vec<SearchView>>,
    }

    impl EventSink for RecordingSink {
        fn navigation_changed(&self, navigation: &NavigationState) -> Result<(), IpcError> {
            self.navigations.borrow_mut().push(navigation.clone());
            Ok(())
        }

        fn player_changed(&self, state: &PlayerState) -> Result<(), IpcError> {
            self.states.borrow_mut().push(state.clone());
            Ok(())
        }

        fn player_position(&self, position: &PlayerPosition) -> Result<(), IpcError> {
            self.positions.borrow_mut().push(*position);
            Ok(())
        }

        fn search_changed(&self, view: &SearchView) -> Result<(), IpcError> {
            self.searches.borrow_mut().push(view.clone());
            Ok(())
        }
    }

    #[test]
    fn announces_a_route_change_once_with_the_new_state() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        navigate_and_notify(&state, Route::Search, &sink).unwrap();

        assert_eq!(
            *sink.navigations.borrow(),
            [NavigationState {
                route: Route::Search,
                queue_open: false,
                revision: 1
            }]
        );
    }

    #[test]
    fn stays_quiet_when_the_route_does_not_change() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        navigate_and_notify(&state, Route::Home, &sink).unwrap();

        assert!(sink.navigations.borrow().is_empty());
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
        assert!(sink.navigations.borrow().is_empty());
    }

    #[test]
    fn toggling_the_queue_is_announced_as_navigation() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        toggle_queue_and_notify(&state, &sink).unwrap();

        assert_eq!(
            *sink.navigations.borrow(),
            [NavigationState {
                route: Route::Home,
                queue_open: true,
                revision: 1
            }]
        );
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
            queue_open: true,
            revision: 3,
        };

        assert_eq!(
            serde_json::to_value(&navigation).unwrap(),
            json!({
                "route": { "view": "playlist", "id": "p1" },
                "queueOpen": true,
                "revision": 3
            })
        );
    }
}
