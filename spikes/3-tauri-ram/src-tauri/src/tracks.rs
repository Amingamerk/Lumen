use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
}

/// One window of the library, plus the full size so the UI can size its scrollbar.
#[derive(Debug, Serialize)]
pub struct TrackSlice {
    pub total: usize,
    pub tracks: Vec<Track>,
}

/// All tracks stay here in Rust; the webview only ever receives a slice.
pub struct Library {
    tracks: Vec<Track>,
}

impl Library {
    pub fn generate(count: u32) -> Self {
        let tracks = (1..=count)
            .map(|id| Track {
                id,
                title: format!("Track {id:04}"),
                artist: format!("Artist {:03}", id % 250 + 1),
                album: format!("Album {:03}", id % 400 + 1),
                duration_ms: 150_000 + (id * 7_919) % 180_000,
            })
            .collect();
        Self { tracks }
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Up to `limit` tracks from `offset`. Windows outside the library are cut, never an error.
    pub fn slice(&self, offset: usize, limit: usize) -> TrackSlice {
        let start = offset.min(self.len());
        let end = offset.saturating_add(limit).min(self.len());
        TrackSlice {
            total: self.len(),
            tracks: self.tracks[start..end].to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_holds_the_requested_number_of_numbered_tracks() {
        let library = Library::generate(5_000);

        assert_eq!(library.len(), 5_000);
        let slice = library.slice(0, 5_000);
        assert_eq!(slice.tracks[0].id, 1);
        assert_eq!(slice.tracks[0].title, "Track 0001");
        assert_eq!(slice.tracks[4_999].id, 5_000);
        assert_eq!(slice.tracks[4_999].title, "Track 5000");
    }

    fn ids(slice: &TrackSlice) -> Vec<u32> {
        slice.tracks.iter().map(|t| t.id).collect()
    }

    #[test]
    fn middle_slice_returns_only_the_window_and_the_full_total() {
        let slice = Library::generate(5_000).slice(100, 3);

        assert_eq!(slice.total, 5_000);
        assert_eq!(ids(&slice), [101, 102, 103]);
    }

    #[test]
    fn window_running_past_the_end_is_cut_at_the_last_track() {
        let slice = Library::generate(5_000).slice(4_998, 10);

        assert_eq!(slice.total, 5_000);
        assert_eq!(ids(&slice), [4_999, 5_000]);
    }

    #[test]
    fn offset_past_the_end_gives_an_empty_window() {
        let library = Library::generate(5_000);

        assert!(ids(&library.slice(5_000, 10)).is_empty());
        assert!(ids(&library.slice(9_999, 10)).is_empty());
        assert_eq!(library.slice(9_999, 10).total, 5_000);
    }

    #[test]
    fn zero_limit_gives_an_empty_window() {
        let slice = Library::generate(5_000).slice(100, 0);

        assert_eq!(slice.total, 5_000);
        assert!(ids(&slice).is_empty());
    }

    #[test]
    fn empty_library_gives_an_empty_window() {
        let slice = Library::generate(0).slice(0, 10);

        assert_eq!(slice.total, 0);
        assert!(ids(&slice).is_empty());
    }

    #[test]
    fn huge_offset_or_limit_does_not_overflow() {
        let library = Library::generate(3);

        assert_eq!(ids(&library.slice(1, usize::MAX)), [2, 3]);
        assert!(ids(&library.slice(usize::MAX, usize::MAX)).is_empty());
    }
}
