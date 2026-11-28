//! A window onto a Collection's Tracks, and the totals its page header shows. The
//! whole list stays in Rust; the UI gets only the rows it is about to show.

use serde::Serialize;

use super::{Catalog, CollectionSummary, TrackRow};

/// The most rows one slice carries, whatever is asked for. A list never shows
/// more than a screenful, so this only stops a whole long playlist going over IPC.
pub const MAX_SLICE: usize = 200;

/// Some consecutive rows of a Collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackSlice {
    /// How many Tracks the whole Collection has.
    pub total: usize,
    /// The position of the first row that was asked for, counted from 0.
    pub offset: usize,
    pub tracks: Vec<TrackRow>,
}

/// A Collection as its page header shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDetail {
    #[serde(flatten)]
    pub summary: CollectionSummary,
    pub track_count: usize,
    /// The length of all its Tracks together.
    pub duration_ms: u64,
}

impl Catalog {
    /// The playlist or album with this id, with its totals.
    pub fn collection_detail(&self, id: &str) -> Option<CollectionDetail> {
        let listed = self.listed(id)?;
        Some(CollectionDetail {
            summary: self.collection(id)?,
            track_count: listed.len(),
            duration_ms: listed
                .iter()
                .filter_map(|track| self.track(*track))
                .map(|track| u64::from(track.duration_ms))
                .sum(),
        })
    }

    /// Up to `limit` rows (never more than [`MAX_SLICE`]) of the playlist or
    /// album `playlist_id`, starting at `offset`. An offset past the end gives
    /// no rows. `None` when there is no such Collection.
    pub fn track_slice(
        &self,
        playlist_id: &str,
        offset: usize,
        limit: usize,
    ) -> Option<TrackSlice> {
        let listed = self.listed(playlist_id)?;
        Some(TrackSlice {
            total: listed.len(),
            offset,
            tracks: listed
                .iter()
                .skip(offset)
                .take(limit.min(MAX_SLICE))
                .filter_map(|track| self.track(*track))
                .map(TrackRow::from)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::catalog::{Album, CollectionKind, Playlist, SourceKind, SourceRef, Track};

    fn track(n: u128) -> Track {
        Track {
            id: Uuid::from_u128(n),
            title: format!("Track {n}"),
            artists: vec!["Mira Sol".to_owned()],
            album: None,
            duration_ms: u32::try_from(n).unwrap() * 1000,
            artwork_url: None,
            isrc: None,
            sources: vec![SourceRef {
                kind: SourceKind::YouTube,
                id: format!("ref-{n}"),
            }],
        }
    }

    fn playlist(id: &str, numbers: &[u128]) -> Playlist {
        Playlist {
            id: id.to_owned(),
            name: id.to_owned(),
            description: String::new(),
            source: SourceKind::YouTube,
            tracks: numbers.iter().copied().map(Uuid::from_u128).collect(),
        }
    }

    /// Five Tracks; a playlist that orders them its own way, an empty playlist
    /// and an album.
    fn catalog() -> Catalog {
        Catalog::new(
            (1..=5).map(track).collect(),
            vec![playlist("mix", &[3, 1, 5, 2, 4]), playlist("empty", &[])],
            vec![Album {
                id: "album".to_owned(),
                title: "Album".to_owned(),
                artists: vec!["Mira Sol".to_owned()],
                source: SourceKind::YouTube,
                tracks: vec![Uuid::from_u128(2), Uuid::from_u128(4)],
            }],
        )
    }

    fn titles(slice: &TrackSlice) -> Vec<&str> {
        slice.tracks.iter().map(|t| t.title.as_str()).collect()
    }

    fn mix(offset: usize, limit: usize) -> TrackSlice {
        catalog().track_slice("mix", offset, limit).unwrap()
    }

    #[test]
    fn returns_the_window_asked_for_in_playlist_order() {
        let slice = mix(1, 3);

        assert_eq!(titles(&slice), ["Track 1", "Track 5", "Track 2"]);
        assert_eq!((slice.total, slice.offset), (5, 1));
    }

    #[test]
    fn a_slice_at_the_start_begins_with_the_first_track() {
        assert_eq!(titles(&mix(0, 2)), ["Track 3", "Track 1"]);
    }

    #[test]
    fn a_slice_over_the_end_stops_at_the_last_track() {
        let slice = mix(3, 10);

        assert_eq!(titles(&slice), ["Track 2", "Track 4"]);
        assert_eq!(slice.total, 5);
    }

    #[test]
    fn the_last_position_gives_the_last_track_alone() {
        assert_eq!(titles(&mix(4, 10)), ["Track 4"]);
    }

    #[test]
    fn an_offset_out_of_range_gives_no_rows_but_the_total() {
        for offset in [5, 6, 10_000, usize::MAX] {
            let slice = mix(offset, 10);

            assert!(slice.tracks.is_empty(), "offset {offset}");
            assert_eq!((slice.total, slice.offset), (5, offset));
        }
    }

    #[test]
    fn a_huge_limit_does_not_overflow() {
        assert_eq!(mix(2, usize::MAX).tracks.len(), 3);
        assert!(mix(usize::MAX, usize::MAX).tracks.is_empty());
    }

    #[test]
    fn a_zero_limit_gives_only_the_total() {
        let slice = mix(0, 0);

        assert!(slice.tracks.is_empty());
        assert_eq!(slice.total, 5);
    }

    #[test]
    fn an_empty_playlist_has_no_rows_at_any_offset() {
        for offset in [0, 1, 50] {
            assert_eq!(
                catalog().track_slice("empty", offset, 50),
                Some(TrackSlice {
                    total: 0,
                    offset,
                    tracks: Vec::new()
                })
            );
        }
    }

    #[test]
    fn slices_an_album_too() {
        let slice = catalog().track_slice("album", 1, 5).unwrap();

        assert_eq!(titles(&slice), ["Track 4"]);
        assert_eq!(slice.total, 2);
    }

    #[test]
    fn knows_no_slice_or_detail_of_an_unknown_id() {
        assert_eq!(catalog().track_slice("nope", 0, 10), None);
        assert_eq!(catalog().collection_detail("nope"), None);
    }

    #[test]
    fn counts_and_adds_up_the_tracks_of_a_collection() {
        let catalog = catalog();

        let mix = catalog.collection_detail("mix").unwrap();
        let album = catalog.collection_detail("album").unwrap();

        assert_eq!(mix.summary, catalog.collection("mix").unwrap());
        assert_eq!((mix.track_count, mix.duration_ms), (5, 15_000));
        assert_eq!(album.summary.kind, CollectionKind::Album);
        assert_eq!((album.track_count, album.duration_ms), (2, 6_000));
    }

    #[test]
    fn an_empty_playlist_lasts_no_time() {
        let empty = catalog().collection_detail("empty").unwrap();

        assert_eq!((empty.track_count, empty.duration_ms), (0, 0));
    }

    mod long_playlist {
        use std::collections::HashSet;

        use super::*;
        use crate::catalog::LONG_PLAYLIST_ID;

        fn slice(offset: usize, limit: usize) -> TrackSlice {
            Catalog::default()
                .track_slice(LONG_PLAYLIST_ID, offset, limit)
                .expect("the long playlist exists")
        }

        #[test]
        fn counts_five_thousand_tracks() {
            let slice = slice(0, 50);

            assert_eq!(slice.total, 5_000);
            assert_eq!(slice.tracks.len(), 50);
        }

        #[test]
        fn never_sends_more_than_the_cap() {
            assert_eq!(slice(0, 5_000).tracks.len(), MAX_SLICE);
            assert_eq!(slice(100, usize::MAX).tracks.len(), MAX_SLICE);
        }

        #[test]
        fn ends_at_track_five_thousand() {
            let catalog = Catalog::default();
            let last = *catalog
                .playlists()
                .iter()
                .find(|p| p.id == LONG_PLAYLIST_ID)
                .and_then(|p| p.tracks.last())
                .unwrap();

            let end = slice(4_990, 50);

            assert_eq!(end.tracks.len(), 10);
            assert_eq!(end.tracks.last().map(|t| t.id), Some(last));
            assert!(slice(5_000, 50).tracks.is_empty());
        }

        #[test]
        fn consecutive_slices_cover_every_track_once() {
            let catalog = Catalog::default();
            let expected = &catalog
                .playlists()
                .iter()
                .find(|p| p.id == LONG_PLAYLIST_ID)
                .unwrap()
                .tracks;

            let seen: Vec<Uuid> = (0..5_000)
                .step_by(MAX_SLICE)
                .flat_map(|offset| {
                    catalog
                        .track_slice(LONG_PLAYLIST_ID, offset, MAX_SLICE)
                        .unwrap()
                        .tracks
                })
                .map(|row| row.id)
                .collect();

            assert_eq!(&seen, expected);
            assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 5_000);
        }

        #[test]
        fn adds_up_the_length_of_all_of_them() {
            let catalog = Catalog::default();

            let detail = catalog.collection_detail(LONG_PLAYLIST_ID).unwrap();
            let rows: u64 = (0..5_000)
                .step_by(MAX_SLICE)
                .flat_map(|offset| slice(offset, MAX_SLICE).tracks)
                .map(|row| u64::from(row.duration_ms))
                .sum();

            assert_eq!(detail.track_count, 5_000);
            assert_eq!(detail.duration_ms, rows);
        }
    }

    #[test]
    fn slices_serialize_for_the_ui() {
        let wire = serde_json::to_value(mix(4, 1)).unwrap();

        assert_eq!(
            wire,
            json!({
                "total": 5,
                "offset": 4,
                "tracks": [{
                    "id": "00000000-0000-0000-0000-000000000004",
                    "title": "Track 4",
                    "artists": ["Mira Sol"],
                    "album": null,
                    "durationMs": 4000,
                    "hasArt": false,
                    "sources": [{ "source": "youtube", "badge": "YT" }]
                }]
            })
        );
    }

    #[test]
    fn details_serialize_as_one_flat_object() {
        let detail = catalog().collection_detail("album").unwrap();

        assert_eq!(
            serde_json::to_value(&detail).unwrap(),
            json!({
                "id": "album",
                "kind": "album",
                "title": "Album",
                "subtitle": "Mira Sol",
                "source": "youtube",
                "badge": "YT",
                "trackCount": 2,
                "durationMs": 6000
            })
        );
    }
}
