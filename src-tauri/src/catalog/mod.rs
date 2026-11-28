//! Lumen's domain shapes and the views the UI draws from them. The data is a
//! static fixture until the SQLite library replaces it (Milestone 3).

use std::collections::HashMap;

use serde::Serialize;
use uuid::Uuid;

mod fixtures;
mod found;
mod search;
mod slice;

pub use fixtures::LONG_PLAYLIST_ID;
pub use found::{Hit, HitKind, HitRow, QUICK_SONGS, Results, SearchKind, Top, TopView};
pub use search::{SearchStatus, SearchView, SourceFilter};
pub use slice::{CollectionDetail, MAX_SLICE, TrackSlice};

/// The service a Track or Collection comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    YouTube,
    SoundCloud,
    Spotify,
}

impl SourceKind {
    /// The short label shown next to a Track or Collection.
    pub fn badge(self) -> &'static str {
        match self {
            Self::YouTube => "YT",
            Self::SoundCloud => "SC",
            Self::Spotify => "SP",
        }
    }
}

/// A pointer to one item on one service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRef {
    pub kind: SourceKind,
    pub id: String,
}

/// One song as Lumen knows it. It holds no Stream URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Track {
    pub id: Uuid,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u32,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
    pub sources: Vec<SourceRef>,
}

/// An ordered list of Tracks that someone put together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: SourceKind,
    pub tracks: Vec<Uuid>,
}

/// A release by its artists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub source: SourceKind,
    pub tracks: Vec<Uuid>,
}

/// Which kind of Collection a card or page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionKind {
    Playlist,
    Album,
}

/// A playlist or album as a card or a page header shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollectionSummary {
    pub id: String,
    pub kind: CollectionKind,
    pub title: String,
    /// A playlist's description, or an album's artists.
    pub subtitle: String,
    pub source: SourceKind,
    pub badge: &'static str,
}

/// One source of a Track as the UI labels it. The service's own id stays in Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceTag {
    pub source: SourceKind,
    pub badge: &'static str,
}

/// A Track as a list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRow {
    pub id: Uuid,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u32,
    /// Whether the picture can be asked for at `lumen://art/<size>/<id>`. The
    /// address it comes from stays in Rust.
    pub has_art: bool,
    pub sources: Vec<SourceTag>,
}

/// One horizontal row of cards on Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shelf {
    pub id: &'static str,
    pub title: &'static str,
    pub items: Vec<CollectionSummary>,
}

/// Everything saved: playlists and albums.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Library {
    pub playlists: Vec<CollectionSummary>,
    pub albums: Vec<CollectionSummary>,
}

impl From<&Playlist> for CollectionSummary {
    fn from(playlist: &Playlist) -> Self {
        Self {
            id: playlist.id.clone(),
            kind: CollectionKind::Playlist,
            title: playlist.name.clone(),
            subtitle: playlist.description.clone(),
            source: playlist.source,
            badge: playlist.source.badge(),
        }
    }
}

impl From<&Album> for CollectionSummary {
    fn from(album: &Album) -> Self {
        Self {
            id: album.id.clone(),
            kind: CollectionKind::Album,
            title: album.title.clone(),
            subtitle: album.artists.join(", "),
            source: album.source,
            badge: album.source.badge(),
        }
    }
}

impl TrackRow {
    /// A row for audio that is not in the catalog: a title and nothing else.
    pub fn untagged(title: String) -> Self {
        Self {
            id: Uuid::nil(),
            title,
            artists: Vec::new(),
            album: None,
            duration_ms: 0,
            has_art: false,
            sources: Vec::new(),
        }
    }
}

impl From<&Track> for TrackRow {
    fn from(track: &Track) -> Self {
        let mut sources: Vec<SourceTag> = Vec::new();
        for reference in &track.sources {
            if !sources.iter().any(|tag| tag.source == reference.kind) {
                sources.push(SourceTag {
                    source: reference.kind,
                    badge: reference.kind.badge(),
                });
            }
        }
        Self {
            id: track.id,
            title: track.title.clone(),
            artists: track.artists.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
            has_art: track.artwork_url.is_some(),
            sources,
        }
    }
}

/// Which Collections a Home shelf holds, by id.
#[derive(Debug, Clone, Copy)]
struct ShelfPlan {
    id: &'static str,
    title: &'static str,
    collections: &'static [&'static str],
}

/// Every Track, playlist and album Lumen knows about.
#[derive(Debug)]
pub struct Catalog {
    tracks: Vec<Track>,
    /// How many of `tracks`, from the front, Search looks through. The generated
    /// Tracks after them are only listed in their playlist: Search answers with
    /// every match at once, which has to stay small.
    searchable: usize,
    /// Where each Track sits in `tracks`, so a slice does not scan for its rows.
    positions: HashMap<Uuid, usize>,
    playlists: Vec<Playlist>,
    albums: Vec<Album>,
    shelves: &'static [ShelfPlan],
}

/// The static fixtures.
impl Default for Catalog {
    fn default() -> Self {
        fixtures::catalog()
    }
}

impl Catalog {
    fn new(
        tracks: Vec<Track>,
        playlists: Vec<Playlist>,
        albums: Vec<Album>,
        shelves: &'static [ShelfPlan],
    ) -> Self {
        let positions = tracks
            .iter()
            .enumerate()
            .map(|(at, t)| (t.id, at))
            .collect();
        Self {
            searchable: tracks.len(),
            tracks,
            positions,
            playlists,
            albums,
            shelves,
        }
    }

    /// A catalog with nothing in it.
    pub fn empty() -> Self {
        Self::new(Vec::new(), Vec::new(), Vec::new(), &[])
    }

    /// Adds a playlist with Tracks of its own that Search leaves out.
    fn with_unsearched(mut self, playlist: Playlist, tracks: Vec<Track>) -> Self {
        for track in tracks {
            self.positions.insert(track.id, self.tracks.len());
            self.tracks.push(track);
        }
        self.playlists.push(playlist);
        self
    }

    /// A catalog holding only these Tracks.
    #[cfg(test)]
    pub fn of(tracks: Vec<Track>) -> Self {
        Self::new(tracks, Vec::new(), Vec::new(), &[])
    }

    /// The Tracks a Recommender may pick from: the ones Search looks through.
    pub fn recommendable(&self) -> &[Track] {
        &self.tracks[..self.searchable]
    }

    pub fn track(&self, id: Uuid) -> Option<&Track> {
        self.positions.get(&id).map(|&at| &self.tracks[at])
    }

    /// The Tracks of a playlist or album, in its order.
    pub fn listed(&self, id: &str) -> Option<&[Uuid]> {
        let playlist = self.playlists.iter().find(|p| p.id == id);
        playlist.map(|p| p.tracks.as_slice()).or_else(|| {
            self.albums
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.tracks.as_slice())
        })
    }

    #[cfg(test)]
    pub fn playlists(&self) -> &[Playlist] {
        &self.playlists
    }

    /// The playlist or album with this id.
    pub fn collection(&self, id: &str) -> Option<CollectionSummary> {
        let playlist = self.playlists.iter().find(|p| p.id == id);
        playlist.map(CollectionSummary::from).or_else(|| {
            self.albums
                .iter()
                .find(|a| a.id == id)
                .map(CollectionSummary::from)
        })
    }

    pub fn home(&self) -> Vec<Shelf> {
        self.shelves
            .iter()
            .map(|plan| Shelf {
                id: plan.id,
                title: plan.title,
                items: plan
                    .collections
                    .iter()
                    .filter_map(|id| self.collection(id))
                    .collect(),
            })
            .collect()
    }

    pub fn library(&self) -> Library {
        Library {
            playlists: self.playlists.iter().map(CollectionSummary::from).collect(),
            albums: self.albums.iter().map(CollectionSummary::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;

    const ALL_SOURCES: [SourceKind; 3] = [
        SourceKind::YouTube,
        SourceKind::SoundCloud,
        SourceKind::Spotify,
    ];

    #[test]
    fn each_source_has_its_badge() {
        assert_eq!(SourceKind::YouTube.badge(), "YT");
        assert_eq!(SourceKind::SoundCloud.badge(), "SC");
        assert_eq!(SourceKind::Spotify.badge(), "SP");
    }

    #[test]
    fn fixtures_cover_all_three_sources() {
        let catalog = Catalog::default();

        for source in ALL_SOURCES {
            assert!(
                catalog
                    .tracks
                    .iter()
                    .any(|t| t.sources.iter().any(|r| r.kind == source)),
                "no track from {source:?}"
            );
            assert!(
                catalog.playlists.iter().any(|p| p.source == source),
                "no playlist from {source:?}"
            );
        }
        assert!(!catalog.albums.is_empty());
    }

    #[test]
    fn every_track_is_complete() {
        let catalog = Catalog::default();

        assert!(!catalog.tracks.is_empty());
        for track in &catalog.tracks {
            assert!(!track.title.trim().is_empty());
            assert!(!track.artists.is_empty(), "{}", track.title);
            assert!(track.duration_ms > 0, "{}", track.title);
            assert!(!track.sources.is_empty(), "{}", track.title);
        }
    }

    fn long_playlist(catalog: &Catalog) -> &Playlist {
        catalog
            .playlists
            .iter()
            .find(|p| p.id == LONG_PLAYLIST_ID)
            .expect("the long playlist exists")
    }

    #[test]
    fn holds_a_five_thousand_track_playlist_beside_the_others() {
        let catalog = Catalog::default();

        let long = long_playlist(&catalog);

        assert_eq!(long.tracks.len(), 5_000);
        assert!(catalog.playlists.len() > 6);
        assert!(
            catalog
                .library()
                .playlists
                .iter()
                .any(|c| c.id == LONG_PLAYLIST_ID)
        );
    }

    #[test]
    fn builds_the_same_long_playlist_every_time() {
        let (first, second) = (Catalog::default(), Catalog::default());

        assert_eq!(long_playlist(&first), long_playlist(&second));
        assert_eq!(first.tracks, second.tracks);
    }

    #[test]
    fn the_long_playlist_has_matched_and_unmatched_spotify_tracks() {
        let catalog = Catalog::default();
        let listed: HashSet<Uuid> = long_playlist(&catalog).tracks.iter().copied().collect();

        let mut mixes: HashSet<Vec<&str>> = HashSet::new();
        for track in catalog.tracks.iter().filter(|t| listed.contains(&t.id)) {
            mixes.insert(
                TrackRow::from(track)
                    .sources
                    .iter()
                    .map(|s| s.badge)
                    .collect(),
            );
        }

        assert_eq!(
            mixes,
            HashSet::from([vec!["SP", "YT"], vec!["SP", "SC"], vec!["SP"]])
        );
    }

    #[test]
    fn search_lists_the_long_playlist_but_not_its_generated_tracks() {
        let catalog = Catalog::default();
        let listed: HashSet<Uuid> = long_playlist(&catalog).tracks.iter().copied().collect();

        let view = catalog.search("", SourceFilter::All);

        assert!(view.tracks.iter().all(|t| !listed.contains(&t.id)));
        assert_eq!(view.tracks.len(), catalog.tracks.len() - listed.len());
        assert!(view.collections.iter().any(|c| c.id == LONG_PLAYLIST_ID));
    }

    #[test]
    fn a_row_says_it_has_art_but_never_carries_the_address() {
        let mut track = Catalog::default().tracks[0].clone();
        track.artwork_url = Some("https://i.ytimg.com/vi/a/hq720.jpg".to_owned());

        let with = serde_json::to_string(&TrackRow::from(&track)).unwrap();
        track.artwork_url = None;
        let without = serde_json::to_string(&TrackRow::from(&track)).unwrap();

        assert!(with.contains(r#""hasArt":true"#), "{with}");
        assert!(!with.contains("ytimg"), "{with}");
        assert!(without.contains(r#""hasArt":false"#), "{without}");
    }

    #[test]
    fn fixtures_link_no_remote_artwork() {
        assert!(
            Catalog::default()
                .tracks
                .iter()
                .all(|t| t.artwork_url.is_none())
        );
    }

    #[test]
    fn track_ids_are_unique() {
        let catalog = Catalog::default();

        let ids: HashSet<Uuid> = catalog.tracks.iter().map(|t| t.id).collect();

        assert_eq!(ids.len(), catalog.tracks.len());
    }

    #[test]
    fn collection_ids_are_unique_across_playlists_and_albums() {
        let catalog = Catalog::default();
        let ids: Vec<&str> = catalog
            .playlists
            .iter()
            .map(|p| p.id.as_str())
            .chain(catalog.albums.iter().map(|a| a.id.as_str()))
            .collect();

        let unique: HashSet<&str> = ids.iter().copied().collect();

        assert!(!ids.is_empty());
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn collections_only_refer_to_known_tracks() {
        let catalog = Catalog::default();
        let known: HashSet<Uuid> = catalog.tracks.iter().map(|t| t.id).collect();
        let lists = catalog
            .playlists
            .iter()
            .map(|p| (&p.id, &p.tracks))
            .chain(catalog.albums.iter().map(|a| (&a.id, &a.tracks)));

        for (id, tracks) in lists {
            assert!(!tracks.is_empty(), "{id} is empty");
            assert!(tracks.iter().all(|t| known.contains(t)), "{id}");
        }
    }

    #[test]
    fn finds_a_playlist_by_id() {
        let summary = Catalog::default().collection("night-shift").unwrap();

        assert_eq!(summary.kind, CollectionKind::Playlist);
        assert_eq!(summary.title, "Night shift");
        assert_eq!(summary.source, SourceKind::YouTube);
        assert_eq!(summary.badge, "YT");
        assert!(!summary.subtitle.is_empty());
    }

    #[test]
    fn finds_an_album_by_id_and_names_its_artists() {
        let catalog = Catalog::default();
        let album = &catalog.albums[0];

        let summary = catalog.collection(&album.id).unwrap();

        assert_eq!(summary.kind, CollectionKind::Album);
        assert_eq!(summary.title, album.title);
        assert_eq!(summary.subtitle, album.artists.join(", "));
        assert_eq!(summary.badge, album.source.badge());
    }

    #[test]
    fn knows_nothing_about_an_unknown_id() {
        assert_eq!(Catalog::default().collection("nope"), None);
    }

    #[test]
    fn home_has_titled_shelves_with_every_planned_card() {
        let catalog = Catalog::default();

        let shelves = catalog.home();

        assert!(shelves.len() >= 2);
        assert_eq!(shelves.len(), catalog.shelves.len());
        for (shelf, plan) in shelves.iter().zip(catalog.shelves) {
            assert!(!shelf.title.is_empty());
            let ids: Vec<&str> = shelf.items.iter().map(|c| c.id.as_str()).collect();
            assert_eq!(ids, plan.collections, "{}", shelf.title);
        }
    }

    #[test]
    fn library_lists_every_playlist_and_album_in_order() {
        let catalog = Catalog::default();

        let library = catalog.library();

        assert!(!library.playlists.is_empty());
        assert!(!library.albums.is_empty());
        assert!(
            library.playlists.iter().map(|c| (&c.id, c.kind)).eq(catalog
                .playlists
                .iter()
                .map(|p| (&p.id, CollectionKind::Playlist)))
        );
        assert!(
            library.albums.iter().map(|c| (&c.id, c.kind)).eq(catalog
                .albums
                .iter()
                .map(|a| (&a.id, CollectionKind::Album)))
        );
    }

    #[test]
    fn a_track_row_lists_each_source_once() {
        let mut track = Catalog::default()
            .tracks
            .first()
            .expect("the catalog has a track")
            .clone();
        track.sources = [
            SourceKind::Spotify,
            SourceKind::YouTube,
            SourceKind::Spotify,
        ]
        .into_iter()
        .enumerate()
        .map(|(n, kind)| SourceRef {
            kind,
            id: format!("ref-{n}"),
        })
        .collect();

        let badges: Vec<&str> = TrackRow::from(&track)
            .sources
            .iter()
            .map(|s| s.badge)
            .collect();

        assert_eq!(badges, ["SP", "YT"]);
    }

    #[test]
    fn collection_summaries_serialize_for_the_ui() {
        let summary = CollectionSummary {
            id: "a1".to_owned(),
            kind: CollectionKind::Album,
            title: "Low Tide".to_owned(),
            subtitle: "Mira Sol".to_owned(),
            source: SourceKind::SoundCloud,
            badge: "SC",
        };

        assert_eq!(
            serde_json::to_value(&summary).unwrap(),
            json!({
                "id": "a1",
                "kind": "album",
                "title": "Low Tide",
                "subtitle": "Mira Sol",
                "source": "soundcloud",
                "badge": "SC"
            })
        );
    }

    #[test]
    fn track_rows_serialize_for_the_ui_without_service_ids() {
        let row = TrackRow {
            id: Uuid::from_u128(7),
            title: "Harbor Lights".to_owned(),
            artists: vec!["Mira Sol".to_owned()],
            album: None,
            duration_ms: 201_000,
            has_art: true,
            sources: vec![SourceTag {
                source: SourceKind::YouTube,
                badge: "YT",
            }],
        };

        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            json!({
                "id": "00000000-0000-0000-0000-000000000007",
                "title": "Harbor Lights",
                "artists": ["Mira Sol"],
                "album": null,
                "durationMs": 201_000,
                "hasArt": true,
                "sources": [{ "source": "youtube", "badge": "YT" }]
            })
        );
    }
}
