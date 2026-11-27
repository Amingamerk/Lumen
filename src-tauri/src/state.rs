//! Navigation, search and catalog state. Rust owns it; the UI mirrors it
//! through IPC.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::{
    Catalog, CollectionDetail, Library, Playlist, SearchStatus, SearchView, Shelf, SourceFilter,
    SourceKind, Track, TrackRow, TrackSlice,
};
use crate::sources::youtube::{GateError, Parsed, ServiceError, search_key};

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
    #[error("playing YouTube Music songs is not available yet")]
    NotPlayableYet,
}

fn unknown_collection(id: &str) -> StateError {
    StateError::UnknownCollection { id: id.to_owned() }
}

/// A search that has to be asked of YouTube Music.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Only the newest ticket is still wanted.
    pub number: u64,
    pub query: String,
}

/// What a submit or a chip change leads to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Submitted {
    /// What to show now, if anything changed.
    pub view: Option<SearchView>,
    pub ticket: Option<Ticket>,
}

/// What was last submitted and chosen on the Search view, and what the live
/// search found.
#[derive(Debug, Default)]
struct SearchState {
    query: String,
    filter: SourceFilter,
    revision: u64,
    /// The number of the newest live search.
    newest: u64,
    /// The query in flight, as the cache keys it.
    pending: Option<String>,
    status: SearchStatus,
    /// The live songs shown, kept whole: their video ids stay here.
    live: Vec<Track>,
}

#[derive(Debug)]
pub struct AppState {
    navigation: NavigationState,
    /// Shared with the player, which has a lock of its own.
    catalog: Arc<Catalog>,
    search: SearchState,
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
            search: SearchState::default(),
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

    /// What playing a result of the remembered search queues: all its Tracks,
    /// and where `track_id` is among them.
    pub fn search_queue(&self, track_id: Uuid) -> Result<(Vec<Uuid>, usize), StateError> {
        if self.search.filter != SourceFilter::SoundCloud {
            return Err(StateError::NotPlayableYet);
        }
        let tracks: Vec<Uuid> = self.search_view().tracks.iter().map(|t| t.id).collect();
        let start = tracks
            .iter()
            .position(|id| *id == track_id)
            .ok_or(StateError::NotInSearch { id: track_id })?;
        Ok((tracks, start))
    }

    /// Enter on the Search view. A ticket means YouTube Music has to be asked.
    /// The SoundCloud chip searches the sample catalog instead.
    pub fn submit_search(&mut self, query: &str, filter: SourceFilter) -> Submitted {
        let live = filter != SourceFilter::SoundCloud;
        let key = search_key(query);
        let search = &mut self.search;
        search.filter = filter;
        // Enter again on the query in flight changes nothing.
        if live && search.pending.as_ref() == Some(&key) {
            return Submitted::default();
        }
        search.query = query.to_owned();
        search.newest += 1;
        search.live.clear();
        let asks = live && !key.is_empty();
        search.pending = asks.then_some(key);
        search.status = if asks {
            SearchStatus::Loading
        } else {
            SearchStatus::Idle
        };
        let ticket = asks.then(|| Ticket {
            number: search.newest,
            query: search.query.clone(),
        });
        Submitted {
            view: Some(self.next_search_view()),
            ticket,
        }
    }

    /// Whether the search with this number is still the newest.
    pub fn is_newest_search(&self, number: u64) -> bool {
        number == self.search.newest
    }

    /// Takes the answer to a ticket. Only the newest one is shown.
    pub fn finish_search(
        &mut self,
        number: u64,
        outcome: Result<Parsed, ServiceError>,
    ) -> Option<SearchView> {
        if !self.is_newest_search(number) {
            return None;
        }
        self.search.pending = None;
        let status = match outcome {
            Ok(found) => self.keep(found),
            Err(ServiceError::Offline) => SearchStatus::Offline,
            Err(ServiceError::Unexpected) => SearchStatus::Unexpected,
            Err(ServiceError::Gate(GateError::Superseded)) => SearchStatus::Idle,
            Err(
                ServiceError::Disconnected
                | ServiceError::Expired
                | ServiceError::Gate(GateError::CoolingDown | GateError::Paused)
                | ServiceError::Rejected
                | ServiceError::RateLimited
                | ServiceError::Blocked,
            ) => SearchStatus::Blocked,
        };
        self.search.status = status;
        // The sample chip does not show it; coming back reads the cache.
        (self.search.filter != SourceFilter::SoundCloud).then(|| self.next_search_view())
    }

    /// The query a chip change shows.
    pub fn submitted_query(&self) -> String {
        self.search.query.clone()
    }

    /// Switches the chip. `cached` is what the cache holds for `query`; nothing
    /// is ever sent.
    pub fn set_search_filter(
        &mut self,
        filter: SourceFilter,
        query: &str,
        cached: Result<Option<Parsed>, ServiceError>,
    ) -> SearchView {
        self.search.filter = filter;
        if filter != SourceFilter::SoundCloud {
            // Another search may have been submitted since the cache was read.
            let cached = if query == self.search.query {
                cached
            } else {
                Ok(None)
            };
            self.search.live.clear();
            self.search.status = match cached {
                Ok(Some(found)) => self.keep(found),
                Ok(None) if self.search.pending == Some(search_key(&self.search.query)) => {
                    SearchStatus::Loading
                }
                Ok(None) => SearchStatus::Idle,
                Err(_) => SearchStatus::Blocked,
            };
        }
        self.next_search_view()
    }

    /// The account changed, so what its searches found is gone.
    pub fn forget_live_search(&mut self) -> Option<SearchView> {
        self.search.newest += 1;
        self.search.pending = None;
        self.search.status = SearchStatus::Idle;
        self.search.live.clear();
        (self.search.filter != SourceFilter::SoundCloud).then(|| self.next_search_view())
    }

    /// The submitted search as the chosen chip shows it.
    pub fn search_view(&self) -> SearchView {
        let search = &self.search;
        if search.filter == SourceFilter::SoundCloud {
            return SearchView {
                revision: search.revision,
                ..self.catalog.search(&search.query, search.filter)
            };
        }
        SearchView {
            revision: search.revision,
            query: search.query.clone(),
            filter: search.filter,
            status: search.status,
            tracks: search.live.iter().map(TrackRow::from).collect(),
            collections: Vec::new(),
        }
    }

    fn next_search_view(&mut self) -> SearchView {
        self.search.revision += 1;
        self.search_view()
    }

    fn keep(&mut self, found: Parsed) -> SearchStatus {
        match found {
            Parsed::Songs(tracks) => {
                self.search.live = tracks;
                SearchStatus::Results
            }
            Parsed::NoResults => {
                self.search.live.clear();
                SearchStatus::NoResults
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::catalog::{LONG_PLAYLIST_ID, SourceRef};

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
        assert_eq!(view.status, SearchStatus::Idle);
        assert!(view.tracks.is_empty());
    }

    #[test]
    fn remembers_the_last_search() {
        let mut state = AppState::default();

        let submitted = state.submit_search("night", SourceFilter::SoundCloud);

        assert_eq!(Some(state.search_view()), submitted.view);
        assert_eq!(state.search_view().query, "night");
    }

    #[test]
    fn searching_does_not_navigate() {
        let mut state = AppState::default();

        state.submit_search("night", SourceFilter::All);

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
        state.submit_search("", SourceFilter::SoundCloud);
        let found: Vec<Uuid> = state.search_view().tracks.iter().map(|t| t.id).collect();
        assert!(found.len() > 1);

        assert_eq!(state.search_queue(found[1]), Ok((found.clone(), 1)));
    }

    #[test]
    fn a_track_outside_the_search_is_rejected() {
        let mut state = AppState::default();
        state.submit_search("night", SourceFilter::SoundCloud);
        let id = Uuid::from_u128(999_999);

        assert_eq!(state.search_queue(id), Err(StateError::NotInSearch { id }));
    }

    fn song(title: &str) -> Track {
        Track {
            id: Uuid::from_u128(7),
            title: title.to_owned(),
            artists: vec!["Ada Brook".to_owned()],
            album: None,
            duration_ms: 1_000,
            artwork_url: Some("https://art.invalid/a.jpg".to_owned()),
            isrc: None,
            sources: vec![SourceRef {
                kind: SourceKind::YouTube,
                id: "fakeVideo01".to_owned(),
            }],
        }
    }

    fn songs(title: &str) -> Result<Parsed, ServiceError> {
        Ok(Parsed::Songs(vec![song(title)]))
    }

    fn submit(state: &mut AppState, query: &str) -> Ticket {
        let submitted = state.submit_search(query, SourceFilter::All);
        submitted.ticket.expect("a ticket")
    }

    fn shown(view: &SearchView) -> (SearchStatus, Vec<&str>) {
        let titles = view.tracks.iter().map(|t| t.title.as_str()).collect();
        (view.status, titles)
    }

    #[test]
    fn a_live_search_loads_and_then_shows_its_songs_with_a_rising_revision() {
        let mut state = AppState::default();
        assert_eq!(shown(&state.search_view()), (SearchStatus::Idle, vec![]));

        let submitted = state.submit_search(" Paper ", SourceFilter::YouTube);

        let loading = submitted.view.expect("a view");
        assert_eq!(shown(&loading), (SearchStatus::Loading, vec![]));
        assert_eq!(loading.query, " Paper ");
        let ticket = submitted.ticket.expect("a ticket");
        assert_eq!(ticket.query, " Paper ");
        assert!(state.is_newest_search(ticket.number));

        let done = state
            .finish_search(ticket.number, songs("Lanterns"))
            .unwrap();

        assert_eq!(shown(&done), (SearchStatus::Results, vec!["Lanterns"]));
        assert!(done.collections.is_empty());
        assert!(done.revision > loading.revision);
        assert_eq!(state.search_view(), done);
    }

    #[test]
    fn a_blank_submit_is_idle_asks_nothing_and_supersedes() {
        let mut state = AppState::default();
        let ticket = submit(&mut state, "paper");

        let blank = state.submit_search("  ", SourceFilter::All);

        assert_eq!(blank.ticket, None);
        assert_eq!(blank.view.unwrap().status, SearchStatus::Idle);
        assert!(!state.is_newest_search(ticket.number));
        assert_eq!(state.finish_search(ticket.number, songs("Late")), None);
        assert_eq!(shown(&state.search_view()), (SearchStatus::Idle, vec![]));
    }

    #[test]
    fn the_query_already_pending_is_not_asked_again() {
        let mut state = AppState::default();
        let ticket = submit(&mut state, "paper");

        let again = state.submit_search(" PAPER ", SourceFilter::All);

        assert_eq!(again.ticket, None);
        assert!(state.is_newest_search(ticket.number));
        // Once it is answered, Enter asks again.
        state.finish_search(ticket.number, Err(ServiceError::Offline));
        submit(&mut state, "paper");
    }

    #[test]
    fn a_late_answer_or_error_does_not_replace_a_newer_one() {
        for late in [songs("Old"), Err(ServiceError::Offline)] {
            let mut state = AppState::default();
            let old = submit(&mut state, "old");
            let new = submit(&mut state, "new");
            state.finish_search(new.number, songs("New")).unwrap();

            assert_eq!(state.finish_search(old.number, late), None);

            assert_eq!(
                shown(&state.search_view()),
                (SearchStatus::Results, vec!["New"])
            );
        }
    }

    #[test]
    fn each_outcome_has_its_status() {
        use SearchStatus::{Blocked, Idle, NoResults, Offline, Unexpected};
        use ServiceError as E;

        for (outcome, status) in [
            (Ok(Parsed::NoResults), NoResults),
            (Err(E::Offline), Offline),
            (Err(E::Unexpected), Unexpected),
            (Err(E::Disconnected), Blocked),
            (Err(E::Expired), Blocked),
            (Err(E::Rejected), Blocked),
            (Err(E::RateLimited), Blocked),
            (Err(E::Blocked), Blocked),
            (Err(E::Gate(GateError::Paused)), Blocked),
            (Err(E::Gate(GateError::CoolingDown)), Blocked),
            (Err(E::Gate(GateError::Superseded)), Idle),
        ] {
            let mut state = AppState::default();
            let ticket = submit(&mut state, "paper");

            let view = state.finish_search(ticket.number, outcome).unwrap();

            assert_eq!(view.status, status);
        }
    }

    #[test]
    fn a_chip_change_shows_only_what_is_cached() {
        let mut state = AppState::default();
        let ticket = submit(&mut state, "night");
        state.finish_search(ticket.number, songs("Live")).unwrap();

        let sample = state.set_search_filter(SourceFilter::SoundCloud, "night", Ok(None));
        assert!(
            sample
                .tracks
                .iter()
                .all(|t| t.sources.iter().any(|s| s.badge == "SC"))
        );
        assert!(!sample.tracks.is_empty());

        let cached =
            state.set_search_filter(SourceFilter::YouTube, "night", songs("Kept").map(Some));
        assert_eq!(shown(&cached), (SearchStatus::Results, vec!["Kept"]));
        assert!(cached.revision > sample.revision);

        for (cached, status) in [
            (Ok(None), SearchStatus::Idle),
            (Err(ServiceError::Expired), SearchStatus::Blocked),
        ] {
            let view = state.set_search_filter(SourceFilter::All, "night", cached);
            assert_eq!(shown(&view), (status, vec![]));
        }
        // An answer for a query that is no longer the submitted one is ignored.
        let stale = state.set_search_filter(SourceFilter::All, "old", songs("Old").map(Some));
        assert_eq!(shown(&stale), (SearchStatus::Idle, vec![]));
    }

    #[test]
    fn a_search_keeps_loading_across_chip_changes() {
        let mut state = AppState::default();
        let ticket = submit(&mut state, "night");

        state.set_search_filter(SourceFilter::SoundCloud, "night", Ok(None));
        // Its answer is not for this chip.
        let mut elsewhere = AppState::default();
        let other = submit(&mut elsewhere, "night");
        elsewhere.set_search_filter(SourceFilter::SoundCloud, "night", Ok(None));
        assert_eq!(elsewhere.finish_search(other.number, songs("Live")), None);

        let back = state.set_search_filter(SourceFilter::YouTube, "night", Ok(None));

        assert_eq!(back.status, SearchStatus::Loading);
        assert!(state.is_newest_search(ticket.number));
    }

    #[test]
    fn the_soundcloud_chip_searches_the_sample_catalog_and_asks_nothing() {
        let mut state = AppState::default();
        let live = submit(&mut state, "paper");

        let submitted = state.submit_search("", SourceFilter::SoundCloud);

        assert_eq!(submitted.ticket, None);
        let view = submitted.view.unwrap();
        assert!(!view.tracks.is_empty());
        assert!(!state.is_newest_search(live.number));
        let first = view.tracks[0].id;
        assert!(state.search_queue(first).is_ok());
    }

    #[test]
    fn a_live_result_cannot_be_played_yet() {
        for filter in [SourceFilter::All, SourceFilter::YouTube] {
            let mut state = AppState::default();
            let ticket = state.submit_search("paper", filter).ticket.unwrap();
            state.finish_search(ticket.number, songs("Live")).unwrap();

            assert_eq!(
                state.search_queue(song("Live").id),
                Err(StateError::NotPlayableYet)
            );
        }
    }

    #[test]
    fn an_account_change_clears_the_results_and_supersedes() {
        let mut state = AppState::default();
        let ticket = submit(&mut state, "paper");
        state.finish_search(ticket.number, songs("Live")).unwrap();

        let cleared = state.forget_live_search().unwrap();

        assert_eq!(shown(&cleared), (SearchStatus::Idle, vec![]));
        assert_eq!(cleared.query, "paper");

        let waiting = submit(&mut state, "paper");
        state.forget_live_search();
        assert_eq!(state.finish_search(waiting.number, songs("Late")), None);
        // The same query may be asked of the new account.
        submit(&mut state, "paper");
    }

    #[test]
    fn live_songs_reach_the_ui_without_an_address() {
        let mut state = AppState::default();
        let ticket = submit(&mut state, "paper");
        let view = state.finish_search(ticket.number, songs("Live")).unwrap();

        let wire = serde_json::to_string(&view).unwrap();

        assert!(wire.contains(r#""status":"results""#), "{wire}");
        assert!(!wire.contains("http"), "{wire}");
        assert!(!wire.contains("fakeVideo01"), "{wire}");
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
