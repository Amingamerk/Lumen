//! Liked songs.

use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use super::{Library, LibraryError, tracks};
use crate::catalog::{MAX_SLICE, Track, TrackRow, TrackSlice};

/// What a like or unlike came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LikeChange {
    /// The id the song is kept under.
    pub track_id: Uuid,
    pub liked: bool,
    /// How many songs are liked now.
    pub liked_count: usize,
    /// Whether this call changed anything.
    #[serde(skip)]
    pub changed: bool,
}

impl Library {
    /// Likes `track`, storing it first. Liking a liked song changes nothing.
    pub fn like(&self, track: &Track) -> Result<LikeChange, LibraryError> {
        let now = super::sql_int(self.clock.wall_ms());
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let id = tracks::ensure(&tx, track)?;
        let added = tx.execute(
            "INSERT OR IGNORE INTO likes (track_id, liked_at) VALUES (?1, ?2)",
            params![id.to_string(), now],
        )?;
        let liked_count = count_likes(&tx)?;
        tx.commit()?;
        Ok(LikeChange {
            track_id: id,
            liked: true,
            liked_count,
            changed: added > 0,
        })
    }

    /// Takes the like away. The song stays known, for its history.
    pub fn unlike(&self, id: Uuid) -> Result<LikeChange, LibraryError> {
        let conn = self.conn();
        let removed = conn.execute("DELETE FROM likes WHERE track_id = ?1", [id.to_string()])?;
        Ok(LikeChange {
            track_id: id,
            liked: false,
            liked_count: count_likes(&conn)?,
            changed: removed > 0,
        })
    }

    pub fn liked_count(&self) -> Result<usize, LibraryError> {
        count_likes(&self.conn())
    }

    /// Which of `ids` are liked.
    pub fn liked_among(&self, ids: &[Uuid]) -> Result<Vec<Uuid>, LibraryError> {
        let conn = self.conn();
        let mut ask = conn.prepare_cached("SELECT 1 FROM likes WHERE track_id = ?1")?;
        let mut liked = Vec::new();
        for id in ids {
            if ask.exists([id.to_string()])? {
                liked.push(*id);
            }
        }
        Ok(liked)
    }

    /// Every liked song by id, the newest like first.
    pub fn liked_list(&self) -> Result<Vec<Uuid>, LibraryError> {
        let conn = self.conn();
        let mut ask =
            conn.prepare_cached("SELECT track_id FROM likes ORDER BY liked_at DESC, rowid DESC")?;
        let ids = ask
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect())
    }

    /// Up to `limit` liked songs from `offset`, the newest like first.
    pub fn liked_slice(&self, offset: usize, limit: usize) -> Result<TrackSlice, LibraryError> {
        let conn = self.conn();
        let mut ask = conn.prepare_cached(
            "SELECT track_id FROM likes ORDER BY liked_at DESC, rowid DESC LIMIT ?1 OFFSET ?2",
        )?;
        let ids: Vec<String> = ask
            .query_map(
                params![super::sql_int(limit.min(MAX_SLICE)), super::sql_int(offset)],
                |row| row.get(0),
            )?
            .collect::<Result<_, _>>()?;
        let rows = ids
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .filter_map(|id| tracks::find(&conn, id).transpose())
            .map(|track| track.map(|track| TrackRow::from(&track)))
            .collect::<Result<_, _>>()?;
        Ok(TrackSlice {
            total: count_likes(&conn)?,
            offset,
            tracks: rows,
        })
    }
}

fn count_likes(conn: &rusqlite::Connection) -> Result<usize, LibraryError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM likes", [], |row| row.get(0))?;
    Ok(usize::try_from(count).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use crate::library::testing::{library_at, reopen, song};

    #[test]
    fn a_like_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let (library, _) = library_at(&path);
        let change = library.like(&song(1)).unwrap();
        assert!(change.liked && change.changed);
        drop(library);

        let library = reopen(&path);

        assert_eq!(
            library.liked_among(&[song(1).id, song(2).id]).unwrap(),
            [song(1).id]
        );
        assert_eq!(library.liked_count().unwrap(), 1);
        assert_eq!(library.track(song(1).id).unwrap(), Some(song(1)));
    }

    #[test]
    fn liking_twice_changes_nothing_the_second_time() {
        let (library, _) = crate::library::testing::memory();

        assert!(library.like(&song(1)).unwrap().changed);
        let again = library.like(&song(1)).unwrap();

        assert!(!again.changed);
        assert_eq!(again.liked_count, 1);
    }

    #[test]
    fn unliking_removes_the_like_but_keeps_the_song() {
        let (library, _) = crate::library::testing::memory();
        library.like(&song(1)).unwrap();

        let gone = library.unlike(song(1).id).unwrap();

        assert!(!gone.liked && gone.changed);
        assert_eq!(gone.liked_count, 0);
        assert!(library.liked_among(&[song(1).id]).unwrap().is_empty());
        assert!(library.track(song(1).id).unwrap().is_some());
        assert!(!library.unlike(song(1).id).unwrap().changed);
    }

    #[test]
    fn one_song_per_source_ref_whatever_id_it_arrives_with() {
        let (library, _) = crate::library::testing::memory();
        let first = library.like(&song(1)).unwrap();
        let mut again = song(1);
        again.id = uuid::Uuid::from_u128(999);

        let second = library.like(&again).unwrap();

        assert_eq!(second.track_id, first.track_id);
        assert!(!second.changed);
        assert_eq!(library.liked_count().unwrap(), 1);
    }

    #[test]
    fn a_later_sighting_keeps_what_was_known() {
        let (library, _) = crate::library::testing::memory();
        let mut full = song(1);
        full.artwork_url = Some("https://i.ytimg.com/a.jpg".to_owned());
        full.isrc = Some("USRC17607839".to_owned());
        library.like(&full).unwrap();
        let mut bare = song(1);
        bare.title = "Renamed".to_owned();

        library.like(&bare).unwrap();

        let kept = library.track(full.id).unwrap().unwrap();
        assert_eq!(kept.title, "Renamed");
        assert_eq!(kept.artwork_url, full.artwork_url);
        assert_eq!(kept.isrc, full.isrc);
    }

    #[test]
    fn the_liked_slice_is_newest_first_and_capped() {
        let (library, clock) = crate::library::testing::memory();
        for n in 1..=5 {
            library.like(&song(n)).unwrap();
            clock.advance(std::time::Duration::from_secs(1));
        }

        let slice = library.liked_slice(1, 2).unwrap();

        assert_eq!((slice.total, slice.offset), (5, 1));
        let titles: Vec<&str> = slice.tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Song 4", "Song 3"]);
        assert!(library.liked_slice(9, 5).unwrap().tracks.is_empty());
        assert_eq!(library.liked_slice(0, 1_000_000).unwrap().tracks.len(), 5);
    }

    #[test]
    fn the_liked_list_is_every_song_newest_like_first() {
        let (library, clock) = crate::library::testing::memory();
        for n in [2, 1, 3] {
            library.like(&song(n)).unwrap();
            clock.advance(std::time::Duration::from_secs(1));
        }

        assert_eq!(
            library.liked_list().unwrap(),
            [song(3).id, song(1).id, song(2).id]
        );
    }
}
