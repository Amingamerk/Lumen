//! Navigation, search and catalog state. Rust owns it; the UI mirrors it
//! through IPC.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::{
    Catalog, CollectionDetail, Library, Playlist, SearchView, Shelf, SourceFilter, SourceKind,
    TrackSlice,
};

/// A playlist as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub source: SourceKind,
    pub badge: &'static str,
}

impl From<&Playlist> for PlaylistSummary {
    fn from(playlist: &Playlist) -> Self {
        Self {
            id: playlist.id.clone(),
            name: playlist.name.clone(),
            source: playlist.source,
            badge: playlist.source.badge(),
        }
    }
}

/// The view shown in the main frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "lowercase")]
pub enum Route {
    Home,
    Search,
    Library,
    Playlist { id: String },
}

impl Route {
    /// Picks the opening route from command-line arguments. `--list` opens
    /// the 5,000-track playlist directly so RAM checks need no clicking.
    pub fn from_args<I>(args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        if args.into_iter().any(|arg| arg.as_ref() == "--list") {
            Self::Playlist {
                id: crate::catalog::LONG_PLAYLIST_ID.to_owned(),
            }
        } else {
            Self::Home
        }
    }
}

/// The current route plus a counter that grows with every change, so the UI can
/// drop a snapshot that arrives after a newer event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationState {
    pub route: Route,
    /// Whether the queue panel is showing.
    pub queue_open: bool,
    pub revision: u64,
}

/// Everything the UI needs to draw the shell on start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppSnapshot {
    pub navigation: NavigationState,
    pub playlists: Vec<PlaylistSummary>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    #[error("no playlist or album has the id “{id}”")]
    UnknownCollection { id: String },
    #[error("the last search did not find the track {id}")]
    NotInSearch { id: Uuid },
}

fn unknown_collection(id: &str) -> StateError {
    StateError::UnknownCollection { id: id.to_owned() }
}

/// What was last typed and chosen on the Search view.
#[derive(Debug, Default)]
struct SearchRequest {
    query: String,
    filter: SourceFilter,
}

#[derive(Debug)]
pub struct AppState {
    navigation: NavigationState,
    /// Shared with the player, which has a lock of its own.
    catalog: Arc<Catalog>,
    search: SearchRequest,
}

impl Default for AppState {
    fn default() -> Self {
        Self::from_args(std::iter::empty::<&str>())
    }
}

impl AppState {
    /// Builds the starting state for the given command-line arguments.
    pub fn from_args<I>(args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        Self {
            navigation: NavigationState {
                route: Route::from_args(args),
                queue_open: false,
                revision: 0,
            },
            catalog: Arc::default(),
            search: SearchRequest::default(),
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            navigation: self.navigation.clone(),
            playlists: self
                .catalog
                .playlists()
                .iter()
                .map(PlaylistSummary::from)
                .collect(),
        }
    }

    /// Switches the main frame to `route`. Returns the new navigation state, or
    /// `None` when that route is already showing.
    pub fn navigate(&mut self, route: Route) -> Result<Option<NavigationState>, StateError> {
        // The playlist route shows albums too.
        if let Route::Playlist { id } = &route
            && self.catalog.collection(id).is_none()
        {
            return Err(unknown_collection(id));
        }
        if route == self.navigation.route {
            return Ok(None);
        }
        self.navigation = NavigationState {
            route,
            queue_open: self.navigation.queue_open,
            revision: self.navigation.revision + 1,
        };
        Ok(Some(self.navigation.clone()))
    }

    /// Shows or hides the queue panel.
    pub fn toggle_queue(&mut self) -> NavigationState {
        self.navigation.queue_open = !self.navigation.queue_open;
        self.navigation.revision += 1;
        self.navigation.clone()
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog)
    }

    pub fn home(&self) -> Vec<Shelf> {
        self.catalog.home()
    }

    pub fn library(&self) -> Library {
        self.catalog.library()
    }

    /// The playlist or album a playlist route points at, with its totals.
    pub fn collection(&self, id: &str) -> Result<CollectionDetail, StateError> {
        self.catalog
            .collection_detail(id)
            .ok_or_else(|| unknown_collection(id))
    }

    /// The rows of that Collection the UI is about to show.
    pub fn track_slice(
        &self,
        playlist_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<TrackSlice, StateError> {
        self.catalog
            .track_slice(playlist_id, offset, limit)
            .ok_or_else(|| unknown_collection(playlist_id))
    }

    /// Runs a search and remembers it for the next visit to the Search view.
    pub fn search(&mut self, query: &str, filter: SourceFilter) -> SearchView {
        self.search = SearchRequest {
            query: query.to_owned(),
            filter,
        };
        self.search_view()
    }

    /// What playing a result of the remembered search queues: all its Tracks,
    /// and where `track_id` is among them.
    pub fn search_queue(&self, track_id: Uuid) -> Result<(Vec<Uuid>, usize), StateError> {
        let tracks: Vec<Uuid> = self.search_view().tracks.iter().map(|t| t.id).collect();
        let start = tracks
            .iter()
            .position(|id| *id == track_id)
            .ok_or(StateError::NotInSearch { id: track_id })?;
        Ok((tracks, start))
    }

    /// The remembered search with its results.
    pub fn search_view(&self) -> SearchView {
        self.catalog.search(&self.search.query, self.search.filter)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::catalog::LONG_PLAYLIST_ID;

    fn playlist_route(id: &str) -> Route {
        Route::Playlist { id: id.to_owned() }
    }

    #[test]
    fn starts_on_home_at_revision_zero() {
        let navigation = AppState::default().snapshot().navigation;

        assert_eq!(navigation.route, Route::Home);
        assert_eq!(navigation.revision, 0);
    }

    #[test]
    fn route_defaults_to_home_without_list_flag() {
        assert_eq!(Route::from_args(std::iter::empty::<&str>()), Route::Home);
        assert_eq!(Route::from_args(["lumen.exe"]), Route::Home);
        assert_eq!(Route::from_args(["lumen.exe", "--listing"]), Route::Home);
    }

    #[test]
    fn list_flag_selects_the_long_playlist_route() {
        let expected = playlist_route(LONG_PLAYLIST_ID);
        assert_eq!(Route::from_args(["--list"]), expected);
        assert_eq!(Route::from_args(["lumen.exe", "--list"]), expected);
    }

    #[test]
    fn app_state_from_args_with_list_starts_on_long_playlist_at_revision_zero() {
        let state = AppState::from_args(["--list"]);
        let navigation = state.snapshot().navigation;

        assert_eq!(navigation.route, playlist_route(LONG_PLAYLIST_ID));
        assert_eq!(navigation.revision, 0);
        assert!(!navigation.queue_open);
        assert_eq!(
            state.collection(LONG_PLAYLIST_ID).map(|c| c.track_count),
            Ok(5_000)
        );
    }

    #[test]
    fn starts_with_sidebar_playlists_from_all_three_sources() {
        let playlists = AppState::default().snapshot().playlists;

        for source in [
            SourceKind::YouTube,
            SourceKind::SoundCloud,
            SourceKind::Spotify,
        ] {
            assert!(
                playlists.iter().any(|p| p.source == source),
                "no playlist from {source:?}"
            );
        }
        assert!(playlists.iter().all(|p| !p.name.trim().is_empty()));
    }

    #[test]
    fn playlist_ids_are_unique() {
        let playlists = AppState::default().snapshot().playlists;

        let mut ids: Vec<&str> = playlists.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();

        assert_eq!(ids.len(), playlists.len());
    }

    #[test]
    fn playlist_badges_match_their_source() {
        for playlist in AppState::default().snapshot().playlists {
            assert_eq!(playlist.badge, playlist.source.badge(), "{}", playlist.name);
        }
    }

    #[test]
    fn navigating_switches_the_route_and_bumps_the_revision() {
        let mut state = AppState::default();

        let first = state.navigate(Route::Search).unwrap();
        let second = state.navigate(Route::Library).unwrap();

        assert_eq!(
            first,
            Some(NavigationState {
                route: Route::Search,
                queue_open: false,
                revision: 1
            })
        );
        assert_eq!(
            second,
            Some(NavigationState {
                route: Route::Library,
                queue_open: false,
                revision: 2
            })
        );
        assert_eq!(state.snapshot().navigation, second.unwrap());
    }

    #[test]
    fn navigating_to_the_current_route_changes_nothing() {
        let mut state = AppState::default();

        assert_eq!(state.navigate(Route::Home), Ok(None));
        assert_eq!(state.snapshot().navigation.revision, 0);
    }

    #[test]
    fn navigates_to_a_sidebar_playlist() {
        let mut state = AppState::default();
        let id = state.snapshot().playlists[1].id.clone();

        let changed = state.navigate(playlist_route(&id)).unwrap();

        assert_eq!(changed.map(|n| n.route), Some(playlist_route(&id)));
    }

    #[test]
    fn sidebar_playlists_are_the_catalog_playlists() {
        let state = AppState::default();

        let sidebar: Vec<(String, String)> = state
            .snapshot()
            .playlists
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        let library: Vec<(String, String)> = state
            .library()
            .playlists
            .into_iter()
            .map(|c| (c.id, c.title))
            .collect();

        assert!(!library.is_empty());
        assert_eq!(sidebar, library);
    }

    #[test]
    fn navigates_to_every_collection_in_the_library_and_on_home() {
        let mut state = AppState::default();
        let library = state.library();
        let home = state.home();
        let ids: Vec<String> = library
            .playlists
            .iter()
            .chain(&library.albums)
            .chain(home.iter().flat_map(|shelf| &shelf.items))
            .map(|c| c.id.clone())
            .collect();
        assert!(!ids.is_empty());

        for id in ids {
            state.navigate(playlist_route(&id)).unwrap();

            assert_eq!(state.snapshot().navigation.route, playlist_route(&id));
            assert_eq!(state.collection(&id).map(|c| c.summary.id), Ok(id));
        }
    }

    #[test]
    fn navigates_to_an_album() {
        let mut state = AppState::default();
        let albums = state.library().albums;
        let album = &albums.first().expect("the library has an album").id;

        let changed = state.navigate(playlist_route(album)).unwrap();

        assert_eq!(changed.map(|n| n.route), Some(playlist_route(album)));
    }

    #[test]
    fn rejects_an_unknown_collection_lookup() {
        assert_eq!(
            AppState::default().collection("nope"),
            Err(StateError::UnknownCollection {
                id: "nope".to_owned()
            })
        );
    }

    #[test]
    fn slices_the_tracks_of_a_collection() {
        let state = AppState::default();

        let slice = state.track_slice("night-shift", 1, 2).unwrap();

        assert_eq!((slice.total, slice.offset, slice.tracks.len()), (5, 1, 2));
        assert_eq!(
            state.collection("night-shift").map(|c| c.track_count),
            Ok(slice.total)
        );
    }

    #[test]
    fn rejects_a_slice_of_an_unknown_collection() {
        assert_eq!(
            AppState::default().track_slice("nope", 0, 50),
            Err(StateError::UnknownCollection {
                id: "nope".to_owned()
            })
        );
    }

    #[test]
    fn opens_the_long_playlist() {
        let mut state = AppState::default();

        let changed = state.navigate(playlist_route(LONG_PLAYLIST_ID)).unwrap();

        assert!(changed.is_some());
        assert_eq!(
            state.collection(LONG_PLAYLIST_ID).map(|c| c.track_count),
            Ok(5_000)
        );
    }

    #[test]
    fn starts_with_a_blank_search_of_every_source() {
        let view = AppState::default().search_view();

        assert_eq!(view.query, "");
        assert_eq!(view.filter, SourceFilter::All);
        assert!(!view.tracks.is_empty());
    }

    #[test]
    fn remembers_the_last_search() {
        let mut state = AppState::default();

        let found = state.search("night", SourceFilter::YouTube);

        assert_eq!(found.query, "night");
        assert_eq!(state.search_view(), found);
    }

    #[test]
    fn searching_does_not_navigate() {
        let mut state = AppState::default();

        state.search("night", SourceFilter::All);

        assert_eq!(state.snapshot().navigation.revision, 0);
    }

    #[test]
    fn rejects_an_unknown_playlist_and_stays_put() {
        let mut state = AppState::default();
        state.navigate(Route::Library).unwrap();

        let result = state.navigate(playlist_route("nope"));

        assert_eq!(
            result,
            Err(StateError::UnknownCollection {
                id: "nope".to_owned()
            })
        );
        assert_eq!(state.snapshot().navigation.route, Route::Library);
        assert_eq!(state.snapshot().navigation.revision, 1);
    }

    #[test]
    fn toggling_the_queue_flips_it_and_bumps_the_revision() {
        let mut state = AppState::default();

        let opened = state.toggle_queue();
        let closed = state.toggle_queue();

        assert_eq!((opened.queue_open, opened.revision), (true, 1));
        assert_eq!((closed.queue_open, closed.revision), (false, 2));
        assert_eq!(state.snapshot().navigation, closed);
    }

    #[test]
    fn navigating_keeps_the_queue_open() {
        let mut state = AppState::default();
        state.toggle_queue();

        let changed = state.navigate(Route::Library).unwrap();

        assert_eq!(changed.map(|n| n.queue_open), Some(true));
    }

    #[test]
    fn a_search_result_queues_the_remembered_results() {
        let mut state = AppState::default();
        let found: Vec<Uuid> = state
            .search("night", SourceFilter::All)
            .tracks
            .iter()
            .map(|t| t.id)
            .collect();
        assert!(found.len() > 1);

        assert_eq!(state.search_queue(found[1]), Ok((found.clone(), 1)));
    }

    #[test]
    fn a_track_outside_the_search_is_rejected() {
        let mut state = AppState::default();
        state.search("night", SourceFilter::All);
        let id = Uuid::from_u128(999_999);

        assert_eq!(state.search_queue(id), Err(StateError::NotInSearch { id }));
    }

    #[test]
    fn shares_one_catalog_with_the_player() {
        let state = AppState::default();

        assert!(Arc::ptr_eq(&state.catalog(), &state.catalog()));
    }

    #[test]
    fn routes_serialize_with_a_view_tag() {
        let cases = [
            (Route::Home, json!({ "view": "home" })),
            (Route::Search, json!({ "view": "search" })),
            (Route::Library, json!({ "view": "library" })),
            (
                playlist_route("abc"),
                json!({ "view": "playlist", "id": "abc" }),
            ),
        ];

        for (route, wire) in cases {
            assert_eq!(serde_json::to_value(&route).unwrap(), wire);
            assert_eq!(serde_json::from_value::<Route>(wire).unwrap(), route);
        }
    }

    #[test]
    fn routes_reject_unknown_views_and_missing_ids() {
        for wire in [
            json!({ "view": "settings" }),
            json!({ "view": "playlist" }),
            json!("home"),
        ] {
            assert!(
                serde_json::from_value::<Route>(wire.clone()).is_err(),
                "accepted {wire}"
            );
        }
    }

    #[test]
    fn playlist_summary_serializes_source_and_badge() {
        let cases = [
            (SourceKind::YouTube, "youtube", "YT"),
            (SourceKind::SoundCloud, "soundcloud", "SC"),
            (SourceKind::Spotify, "spotify", "SP"),
        ];

        for (source, wire_source, badge) in cases {
            let playlist = PlaylistSummary {
                id: "p1".to_owned(),
                name: "Night shift".to_owned(),
                source,
                badge: source.badge(),
            };

            assert_eq!(
                serde_json::to_value(&playlist).unwrap(),
                json!({ "id": "p1", "name": "Night shift", "source": wire_source, "badge": badge })
            );
        }
    }

    #[test]
    fn snapshot_serializes_navigation_and_playlists() {
        let snapshot = AppState::default().snapshot();

        let wire = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(
            wire["navigation"],
            json!({ "route": { "view": "home" }, "queueOpen": false, "revision": 0 })
        );
        assert_eq!(
            wire["playlists"].as_array().map(Vec::len),
            Some(snapshot.playlists.len())
        );
    }
}
