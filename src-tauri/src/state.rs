//! Navigation and sidebar state. Rust owns it; the UI mirrors it through IPC.

use serde::{Deserialize, Serialize};

/// The service a sidebar playlist comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    YouTube,
    SoundCloud,
    Spotify,
}

impl SourceKind {
    /// The short label shown next to a playlist.
    pub fn badge(self) -> &'static str {
        match self {
            Self::YouTube => "YT",
            Self::SoundCloud => "SC",
            Self::Spotify => "SP",
        }
    }
}

/// A playlist as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub source: SourceKind,
    pub badge: &'static str,
}

impl PlaylistSummary {
    fn new(id: &str, name: &str, source: SourceKind) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            source,
            badge: source.badge(),
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

/// The current route plus a counter that grows with every change, so the UI can
/// drop a snapshot that arrives after a newer event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NavigationState {
    pub route: Route,
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
    #[error("no playlist has the id “{id}”")]
    UnknownPlaylist { id: String },
}

#[derive(Debug)]
pub struct AppState {
    navigation: NavigationState,
    playlists: Vec<PlaylistSummary>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            navigation: NavigationState {
                route: Route::Home,
                revision: 0,
            },
            playlists: placeholder_playlists(),
        }
    }
}

/// Fixed sidebar entries until the SQLite library lands (Milestone 3).
fn placeholder_playlists() -> Vec<PlaylistSummary> {
    use SourceKind::{SoundCloud, Spotify, YouTube};

    vec![
        PlaylistSummary::new("night-shift", "Night shift", YouTube),
        PlaylistSummary::new("tunnel-mixes", "Tunnel mixes", SoundCloud),
        PlaylistSummary::new("liked-on-spotify", "Liked on Spotify", Spotify),
        PlaylistSummary::new("sunday-cooking", "Sunday cooking", YouTube),
        PlaylistSummary::new("field-recordings", "Field recordings", SoundCloud),
        PlaylistSummary::new(
            "long-drive-home",
            "Songs for the long drive home after a late shift",
            Spotify,
        ),
    ]
}

impl AppState {
    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            navigation: self.navigation.clone(),
            playlists: self.playlists.clone(),
        }
    }

    /// Switches the main frame to `route`. Returns the new navigation state, or
    /// `None` when that route is already showing.
    pub fn navigate(&mut self, route: Route) -> Result<Option<NavigationState>, StateError> {
        if let Route::Playlist { id } = &route
            && !self.playlists.iter().any(|playlist| playlist.id == *id)
        {
            return Err(StateError::UnknownPlaylist { id: id.clone() });
        }
        if route == self.navigation.route {
            return Ok(None);
        }
        self.navigation = NavigationState {
            route,
            revision: self.navigation.revision + 1,
        };
        Ok(Some(self.navigation.clone()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
    fn each_source_has_its_badge() {
        assert_eq!(SourceKind::YouTube.badge(), "YT");
        assert_eq!(SourceKind::SoundCloud.badge(), "SC");
        assert_eq!(SourceKind::Spotify.badge(), "SP");
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
                revision: 1
            })
        );
        assert_eq!(
            second,
            Some(NavigationState {
                route: Route::Library,
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
    fn rejects_an_unknown_playlist_and_stays_put() {
        let mut state = AppState::default();
        state.navigate(Route::Library).unwrap();

        let result = state.navigate(playlist_route("nope"));

        assert_eq!(
            result,
            Err(StateError::UnknownPlaylist {
                id: "nope".to_owned()
            })
        );
        assert_eq!(state.snapshot().navigation.route, Route::Library);
        assert_eq!(state.snapshot().navigation.revision, 1);
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
            json!({ "route": { "view": "home" }, "revision": 0 })
        );
        assert_eq!(
            wire["playlists"].as_array().map(Vec::len),
            Some(snapshot.playlists.len())
        );
    }
}
