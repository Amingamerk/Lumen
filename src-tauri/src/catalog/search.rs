//! Searching the catalog. All matching and filtering happens here, not in the UI.

use serde::{Deserialize, Serialize};

use super::{Catalog, CollectionSummary, HitRow, SearchKind, SourceKind, TopView, TrackRow};

/// The source chip chosen on the Search view. Spotify has no chip of its own:
/// it shows up under `All`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFilter {
    #[default]
    All,
    YouTube,
    SoundCloud,
}

impl SourceFilter {
    fn accepts(self, kind: SourceKind) -> bool {
        match self {
            Self::All => true,
            Self::YouTube => kind == SourceKind::YouTube,
            Self::SoundCloud => kind == SourceKind::SoundCloud,
        }
    }
}

/// Where a search stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchStatus {
    /// Nothing has been asked.
    #[default]
    Idle,
    Loading,
    Results,
    NoResults,
    Offline,
    Unexpected,
    /// The account or the gate lets nothing through; their stores say why.
    Blocked,
}

/// A search and what it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchView {
    /// Grows with every change, so the UI can drop a snapshot that arrives
    /// after a newer event.
    pub revision: u64,
    /// The query exactly as it was typed.
    pub query: String,
    pub filter: SourceFilter,
    /// The type chip. Only live results are narrowed by it.
    pub kind: SearchKind,
    pub status: SearchStatus,
    /// The best match and its songs, when the service named one.
    pub top: Option<TopView>,
    pub tracks: Vec<TrackRow>,
    pub albums: Vec<HitRow>,
    pub artists: Vec<HitRow>,
    pub playlists: Vec<HitRow>,
    pub collections: Vec<CollectionSummary>,
}

impl Catalog {
    /// Tracks, playlists and albums matching every word of `query` within the
    /// chosen source. A blank query matches everything.
    pub fn search(&self, query: &str, filter: SourceFilter) -> SearchView {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();

        let tracks: Vec<TrackRow> = self.tracks[..self.searchable]
            .iter()
            .filter(|track| track.sources.iter().any(|r| filter.accepts(r.kind)))
            .filter(|track| {
                let artists = track.artists.iter().map(String::as_str);
                let fields = [track.title.as_str()]
                    .into_iter()
                    .chain(artists)
                    .chain(track.album.as_deref());
                matches(&words, fields)
            })
            .map(TrackRow::from)
            .collect();

        let playlists = self
            .playlists
            .iter()
            .filter(|p| filter.accepts(p.source) && matches(&words, [p.name.as_str()]))
            .map(CollectionSummary::from);
        let albums = self
            .albums
            .iter()
            .filter(|album| {
                let artists = album.artists.iter().map(String::as_str);
                filter.accepts(album.source)
                    && matches(&words, [album.title.as_str()].into_iter().chain(artists))
            })
            .map(CollectionSummary::from);
        let collections: Vec<CollectionSummary> = playlists.chain(albums).collect();
        let status = if tracks.is_empty() && collections.is_empty() {
            SearchStatus::NoResults
        } else {
            SearchStatus::Results
        };

        SearchView {
            revision: 0,
            query: query.to_owned(),
            filter,
            kind: SearchKind::All,
            status,
            top: None,
            tracks,
            albums: Vec::new(),
            artists: Vec::new(),
            playlists: Vec::new(),
            collections,
        }
    }
}

/// Whether each of the lowercased `words` appears in one of the `fields`. No
/// words means everything matches.
fn matches<'a>(words: &[String], fields: impl IntoIterator<Item = &'a str>) -> bool {
    let fields: Vec<String> = fields.into_iter().map(str::to_lowercase).collect();
    words
        .iter()
        .all(|word| fields.iter().any(|field| field.contains(word.as_str())))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::catalog::{Album, Playlist, SourceRef, Track};

    fn track(n: u128, title: &str, artist: &str, album: &str, kinds: &[SourceKind]) -> Track {
        Track {
            id: Uuid::from_u128(n),
            title: title.to_owned(),
            artists: vec![artist.to_owned()],
            album: Some(album.to_owned()),
            duration_ms: 180_000,
            artwork_url: None,
            isrc: None,
            sources: kinds
                .iter()
                .map(|&kind| SourceRef {
                    kind,
                    id: format!("ref-{n}"),
                })
                .collect(),
        }
    }

    /// A small catalog of its own, so these tests do not depend on the fixtures.
    fn catalog() -> Catalog {
        use SourceKind::{SoundCloud, Spotify, YouTube};

        let mut duet = track(
            4,
            "Glasshouse",
            "Søren Vale",
            "Greenwood",
            &[Spotify, YouTube],
        );
        duet.artists.push("Mira Sol".to_owned());

        Catalog::new(
            vec![
                track(1, "Harbor Lights", "Mira Sol", "Low Tide", &[YouTube]),
                track(
                    2,
                    "Night Ferry",
                    "The Quiet Pier",
                    "Crossings",
                    &[SoundCloud],
                ),
                track(3, "Lamplighter", "Juno Park", "Low Tide", &[Spotify]),
                duet,
            ],
            vec![Playlist {
                id: "harbor-walks".to_owned(),
                name: "Harbor walks".to_owned(),
                description: "For the quay".to_owned(),
                source: SoundCloud,
                tracks: vec![Uuid::from_u128(2)],
            }],
            vec![Album {
                id: "low-tide".to_owned(),
                title: "Low Tide".to_owned(),
                artists: vec!["Mira Sol".to_owned()],
                source: YouTube,
                tracks: vec![Uuid::from_u128(1)],
            }],
            &[],
        )
    }

    fn titles(view: &SearchView) -> Vec<&str> {
        view.tracks.iter().map(|t| t.title.as_str()).collect()
    }

    fn collection_ids(view: &SearchView) -> Vec<&str> {
        view.collections.iter().map(|c| c.id.as_str()).collect()
    }

    fn search(query: &str) -> SearchView {
        catalog().search(query, SourceFilter::All)
    }

    #[test]
    fn matches_part_of_a_track_title() {
        assert_eq!(titles(&search("ferr")), ["Night Ferry"]);
    }

    #[test]
    fn matches_an_artist() {
        assert_eq!(titles(&search("juno")), ["Lamplighter"]);
    }

    #[test]
    fn matches_any_of_the_artists() {
        assert_eq!(titles(&search("mira")), ["Harbor Lights", "Glasshouse"]);
    }

    #[test]
    fn matches_an_album_name() {
        assert_eq!(titles(&search("crossings")), ["Night Ferry"]);
    }

    #[test]
    fn ignores_case_beyond_ascii() {
        assert_eq!(titles(&search("SØREN")), ["Glasshouse"]);
        assert_eq!(titles(&search("hARBOR liGHts")), ["Harbor Lights"]);
    }

    #[test]
    fn needs_every_word_but_allows_them_in_different_fields() {
        assert_eq!(titles(&search("mira harbor")), ["Harbor Lights"]);
        assert_eq!(titles(&search("juno harbor")), Vec::<&str>::new());
    }

    #[test]
    fn does_not_match_a_word_across_two_fields() {
        // "Lights" ends a title and "Mira" starts an artist.
        assert_eq!(titles(&search("lightsmira")), Vec::<&str>::new());
    }

    #[test]
    fn ignores_extra_spaces() {
        assert_eq!(titles(&search("  night    ferry ")), ["Night Ferry"]);
    }

    #[test]
    fn finds_playlists_by_name_and_albums_by_title_or_artist() {
        assert_eq!(collection_ids(&search("harbor")), ["harbor-walks"]);
        assert_eq!(collection_ids(&search("low tide")), ["low-tide"]);
        assert_eq!(collection_ids(&search("mira")), ["low-tide"]);
    }

    #[test]
    fn reports_nothing_for_a_query_without_matches() {
        let view = search("zzz");

        assert!(view.tracks.is_empty());
        assert!(view.collections.is_empty());
    }

    #[test]
    fn an_empty_query_lists_the_whole_catalog_in_order() {
        let view = search("");

        assert_eq!(
            titles(&view),
            ["Harbor Lights", "Night Ferry", "Lamplighter", "Glasshouse"]
        );
        assert_eq!(collection_ids(&view), ["harbor-walks", "low-tide"]);
    }

    #[test]
    fn a_blank_query_counts_as_empty() {
        assert_eq!(search("   ").tracks, search("").tracks);
        assert_eq!(search("   ").collections, search("").collections);
    }

    #[test]
    fn the_youtube_chip_keeps_everything_available_on_youtube() {
        let view = catalog().search("", SourceFilter::YouTube);

        assert_eq!(titles(&view), ["Harbor Lights", "Glasshouse"]);
        assert_eq!(collection_ids(&view), ["low-tide"]);
    }

    #[test]
    fn the_soundcloud_chip_keeps_only_soundcloud() {
        let view = catalog().search("", SourceFilter::SoundCloud);

        assert_eq!(titles(&view), ["Night Ferry"]);
        assert_eq!(collection_ids(&view), ["harbor-walks"]);
    }

    #[test]
    fn only_the_all_chip_shows_spotify_only_tracks() {
        let has_lamplighter = |filter| titles(&catalog().search("lamp", filter)) == ["Lamplighter"];

        assert!(has_lamplighter(SourceFilter::All));
        assert!(!has_lamplighter(SourceFilter::YouTube));
        assert!(!has_lamplighter(SourceFilter::SoundCloud));
    }

    #[test]
    fn query_and_chip_narrow_together() {
        let view = catalog().search("harbor", SourceFilter::SoundCloud);

        assert!(view.tracks.is_empty());
        assert_eq!(collection_ids(&view), ["harbor-walks"]);
    }

    #[test]
    fn a_filtered_track_keeps_all_of_its_badges() {
        let view = catalog().search("glasshouse", SourceFilter::YouTube);

        let badges: Vec<&str> = view.tracks[0].sources.iter().map(|s| s.badge).collect();

        assert_eq!(badges, ["SP", "YT"]);
    }

    #[test]
    fn echoes_the_query_as_typed_and_the_chip() {
        let view = catalog().search(" Night ", SourceFilter::SoundCloud);

        assert_eq!(view.query, " Night ");
        assert_eq!(view.filter, SourceFilter::SoundCloud);
    }

    #[test]
    fn filters_use_lowercase_names_on_the_wire() {
        let cases = [
            (SourceFilter::All, "all"),
            (SourceFilter::YouTube, "youtube"),
            (SourceFilter::SoundCloud, "soundcloud"),
        ];

        for (filter, wire) in cases {
            assert_eq!(serde_json::to_value(filter).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<SourceFilter>(json!(wire)).unwrap(),
                filter
            );
        }
        assert!(serde_json::from_value::<SourceFilter>(json!("spotify")).is_err());
    }
}
