//! What Home shows: a short row of the latest plays, the latest likes and the
//! playlists. Each is bounded, so Home stays cheap however large the library is.

use serde::Serialize;

use super::{Library, LibraryError, PlaylistSummary};
use crate::catalog::TrackRow;

/// The most cards one Home shelf holds.
pub const HOME_CARDS: usize = 8;

/// The three Home shelves. A shelf with nothing in it is empty, not missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HomeView {
    /// Songs played, the latest first, each once.
    pub recent: Vec<TrackRow>,
    /// Liked songs, the newest like first.
    pub liked: Vec<TrackRow>,
    pub playlists: Vec<PlaylistSummary>,
}

impl Library {
    pub fn home(&self) -> Result<HomeView, LibraryError> {
        Ok(HomeView {
            recent: self.recent_slice(0, HOME_CARDS)?.tracks,
            liked: self.liked_slice(0, HOME_CARDS)?.tracks,
            playlists: self.first_playlists(HOME_CARDS)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testing::{START_MS, memory, song};

    fn ids(rows: &[TrackRow]) -> Vec<uuid::Uuid> {
        rows.iter().map(|row| row.id).collect()
    }

    #[test]
    fn recently_played_comes_newest_first_with_each_song_once() {
        let (library, _) = memory();
        for (play, n) in [(1, 1), (2, 2), (3, 1)] {
            library
                .begin_play(play, &song(n), START_MS + play * 1_000)
                .unwrap();
        }

        let home = library.home().unwrap();

        assert_eq!(ids(&home.recent), [song(1).id, song(2).id]);
    }

    #[test]
    fn an_empty_library_has_no_cards() {
        let (library, _) = memory();

        let home = library.home().unwrap();

        assert!(home.recent.is_empty());
        assert!(home.liked.is_empty());
        assert!(home.playlists.is_empty());
    }

    #[test]
    fn a_shelf_holds_at_most_one_row_of_cards() {
        let (library, _) = memory();
        for n in 1..=(HOME_CARDS as u128 + 5) {
            library.like(&song(n)).unwrap();
            library.create_playlist(&format!("List {n}")).unwrap();
        }

        let home = library.home().unwrap();

        assert_eq!(home.liked.len(), HOME_CARDS);
        assert_eq!(home.playlists.len(), HOME_CARDS);
        assert_eq!(home.liked[0].id, song(HOME_CARDS as u128 + 5).id);
    }

    #[test]
    fn a_playlist_card_carries_the_art_of_its_first_song_with_a_picture() {
        let (library, _) = memory();
        let list = library.create_playlist("Mix").unwrap();
        let mut bare = song(1);
        bare.artwork_url = None;
        let mut covered = song(2);
        covered.artwork_url = Some("https://example.test/2.jpg".to_owned());
        library.add_to_playlist(&list.id, &bare).unwrap();
        library.add_to_playlist(&list.id, &covered).unwrap();

        let home = library.home().unwrap();

        assert_eq!(home.playlists[0].track_count, 2);
        assert_eq!(home.playlists[0].cover_track, Some(covered.id));
    }
}
