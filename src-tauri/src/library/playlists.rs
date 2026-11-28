//! Local playlists: named lists of Tracks the user puts together. An entry is a
//! place in a playlist, so the same Track can be there more than once.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use uuid::Uuid;

use super::{Library, LibraryError, tracks};
use crate::catalog::{MAX_SLICE, Track, TrackRow, TrackSlice};

/// A playlist as the sidebar and its page show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub track_count: usize,
    /// The length of all its entries together.
    pub duration_ms: u64,
    /// The first of its songs that has a picture, which the playlist's cover is
    /// drawn from.
    pub cover_track: Option<Uuid>,
}

/// A name as it is kept: trimmed, and never empty.
fn clean(name: &str) -> Result<String, LibraryError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(LibraryError::BlankName);
    }
    Ok(name.to_owned())
}

fn exists(conn: &Connection, id: &str) -> Result<bool, LibraryError> {
    Ok(conn
        .query_row("SELECT 1 FROM playlists WHERE id = ?1", [id], |_| Ok(()))
        .optional()?
        .is_some())
}

fn require(conn: &Connection, id: &str) -> Result<(), LibraryError> {
    if exists(conn, id)? {
        Ok(())
    } else {
        Err(LibraryError::NoPlaylist { id: id.to_owned() })
    }
}

fn count_entries(conn: &Connection, id: &str) -> Result<usize, LibraryError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM playlist_entries WHERE playlist_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count).unwrap_or(0))
}

/// Puts the Track at the end of the playlist and gives its position.
fn append(tx: &Transaction<'_>, id: &str, track: &Track) -> Result<usize, LibraryError> {
    let track_id = tracks::ensure(tx, track)?;
    let position = count_entries(tx, id)?;
    tx.execute(
        "INSERT INTO playlist_entries (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
        params![id, super::sql_int(position), track_id.to_string()],
    )?;
    Ok(position)
}

const SUMMARY: &str = "SELECT p.id, p.name, COUNT(e.id), COALESCE(SUM(t.duration_ms), 0),
            (SELECT c.id FROM playlist_entries ce JOIN tracks c ON c.id = ce.track_id
              WHERE ce.playlist_id = p.id AND c.artwork_url IS NOT NULL
              ORDER BY ce.position LIMIT 1)
     FROM playlists p
     LEFT JOIN playlist_entries e ON e.playlist_id = p.id
     LEFT JOIN tracks t ON t.id = e.track_id";

fn summary_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlaylistSummary> {
    Ok(PlaylistSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        track_count: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
        duration_ms: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
        cover_track: row
            .get::<_, Option<String>>(4)?
            .and_then(|id| Uuid::parse_str(&id).ok()),
    })
}

fn summary(conn: &Connection, id: &str) -> Result<PlaylistSummary, LibraryError> {
    conn.query_row(
        &format!("{SUMMARY} WHERE p.id = ?1 GROUP BY p.id"),
        [id],
        summary_of,
    )
    .optional()?
    .ok_or_else(|| LibraryError::NoPlaylist { id: id.to_owned() })
}

impl Library {
    /// Makes an empty playlist after the others.
    pub fn create_playlist(&self, name: &str) -> Result<PlaylistSummary, LibraryError> {
        let name = clean(name)?;
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO playlists (id, name) VALUES (?1, ?2)",
            params![id, name],
        )?;
        summary(&conn, &id)
    }

    /// Makes a playlist under an id of its own, holding `tracks`. Used to fill a
    /// throwaway library with sample data.
    pub fn create_playlist_with(
        &self,
        id: &str,
        name: &str,
        tracks: &[Track],
    ) -> Result<PlaylistSummary, LibraryError> {
        let name = clean(name)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO playlists (id, name) VALUES (?1, ?2)",
            params![id, name],
        )?;
        for track in tracks {
            append(&tx, id, track)?;
        }
        tx.commit()?;
        summary(&conn, id)
    }

    /// Renames a playlist. Giving it the name it has changes nothing.
    pub fn rename_playlist(&self, id: &str, name: &str) -> Result<PlaylistSummary, LibraryError> {
        let name = clean(name)?;
        let conn = self.conn();
        require(&conn, id)?;
        conn.execute(
            "UPDATE playlists SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        summary(&conn, id)
    }

    /// Removes the playlist and its entries. The Tracks, likes and history stay.
    pub fn delete_playlist(&self, id: &str) -> Result<(), LibraryError> {
        let conn = self.conn();
        require(&conn, id)?;
        conn.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn has_playlist(&self, id: &str) -> Result<bool, LibraryError> {
        exists(&self.conn(), id)
    }

    /// Every playlist, in the order they were made.
    pub fn playlists(&self) -> Result<Vec<PlaylistSummary>, LibraryError> {
        let conn = self.conn();
        let mut ask = conn.prepare(&format!("{SUMMARY} GROUP BY p.id ORDER BY p.rowid"))?;
        let all = ask.query_map([], summary_of)?.collect::<Result<_, _>>()?;
        Ok(all)
    }

    /// The first `limit` playlists, in the order they were made.
    pub fn first_playlists(&self, limit: usize) -> Result<Vec<PlaylistSummary>, LibraryError> {
        let conn = self.conn();
        let mut ask = conn.prepare_cached(&format!(
            "{SUMMARY} GROUP BY p.id ORDER BY p.rowid LIMIT ?1"
        ))?;
        let found = ask
            .query_map([super::sql_int(limit)], summary_of)?
            .collect::<Result<_, _>>()?;
        Ok(found)
    }

    pub fn playlist(&self, id: &str) -> Result<PlaylistSummary, LibraryError> {
        summary(&self.conn(), id)
    }

    /// Puts `track` at the end of the playlist, storing it first. The same
    /// Track added again is a second entry. Gives its position.
    pub fn add_to_playlist(&self, id: &str, track: &Track) -> Result<usize, LibraryError> {
        let mut conn = self.conn();
        require(&conn, id)?;
        let tx = conn.transaction()?;
        let position = append(&tx, id, track)?;
        tx.commit()?;
        Ok(position)
    }

    /// Takes out the entry at `position`, if it still holds `track_id`: the
    /// list may have changed since the caller looked. The entries after it move
    /// up. Whether anything was removed.
    pub fn remove_from_playlist(
        &self,
        id: &str,
        position: usize,
        track_id: Uuid,
    ) -> Result<bool, LibraryError> {
        let mut conn = self.conn();
        require(&conn, id)?;
        let tx = conn.transaction()?;
        let at = super::sql_int(position);
        let removed = tx.execute(
            "DELETE FROM playlist_entries
             WHERE playlist_id = ?1 AND position = ?2 AND track_id = ?3",
            params![id, at, track_id.to_string()],
        )?;
        if removed > 0 {
            tx.execute(
                "UPDATE playlist_entries SET position = position - 1
                 WHERE playlist_id = ?1 AND position > ?2",
                params![id, at],
            )?;
        }
        tx.commit()?;
        Ok(removed > 0)
    }

    /// Every Track of the playlist by id, in its order, repeats included.
    pub fn playlist_track_ids(&self, id: &str) -> Result<Vec<Uuid>, LibraryError> {
        let conn = self.conn();
        require(&conn, id)?;
        let mut ask = conn.prepare_cached(
            "SELECT track_id FROM playlist_entries WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let ids = ask
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect())
    }

    /// Up to `limit` entries from `offset`, in the playlist's order.
    pub fn playlist_slice(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<TrackSlice, LibraryError> {
        let conn = self.conn();
        require(&conn, id)?;
        let mut ask = conn.prepare_cached(
            "SELECT track_id FROM playlist_entries WHERE playlist_id = ?1
             ORDER BY position LIMIT ?2 OFFSET ?3",
        )?;
        let ids: Vec<String> = ask
            .query_map(
                params![
                    id,
                    super::sql_int(limit.min(MAX_SLICE)),
                    super::sql_int(offset)
                ],
                |row| row.get(0),
            )?
            .collect::<Result<_, _>>()?;
        let rows = ids
            .iter()
            .filter_map(|track| Uuid::parse_str(track).ok())
            .filter_map(|track| tracks::find(&conn, track).transpose())
            .map(|track| track.map(|track| TrackRow::from(&track)))
            .collect::<Result<_, _>>()?;
        Ok(TrackSlice {
            total: count_entries(&conn, id)?,
            offset,
            tracks: rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::library::testing::{library_at, memory, reopen, song};
    use crate::library::{EndReason, LibraryError};

    #[test]
    fn a_playlist_is_covered_by_its_first_song_with_a_picture() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Covers").unwrap();
        assert_eq!(playlist.cover_track, None);
        library.add_to_playlist(&playlist.id, &song(1)).unwrap();
        let mut drawn = song(2);
        drawn.artwork_url = Some("https://i.ytimg.com/vi/b/hq720.jpg".to_owned());
        library.add_to_playlist(&playlist.id, &drawn).unwrap();
        let mut later = song(3);
        later.artwork_url = Some("https://i.ytimg.com/vi/c/hq720.jpg".to_owned());
        library.add_to_playlist(&playlist.id, &later).unwrap();

        let summary = library.playlists().unwrap();

        assert_eq!(summary[0].cover_track, Some(drawn.id));
    }

    #[test]
    fn positions_stay_in_order_after_a_removal() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Night shift").unwrap();
        for n in 1..=4 {
            library.add_to_playlist(&playlist.id, &song(n)).unwrap();
        }

        let removed = library
            .remove_from_playlist(&playlist.id, 1, song(2).id)
            .unwrap();

        assert!(removed);
        let slice = library.playlist_slice(&playlist.id, 0, 10).unwrap();
        let titles: Vec<&str> = slice.tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Song 1", "Song 3", "Song 4"]);
        assert_eq!(slice.total, 3);
        library.add_to_playlist(&playlist.id, &song(5)).unwrap();
        let again = library.playlist_slice(&playlist.id, 2, 10).unwrap();
        let titles: Vec<&str> = again.tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Song 4", "Song 5"]);
    }

    #[test]
    fn the_same_track_added_twice_is_two_entries() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Loop").unwrap();

        assert_eq!(library.add_to_playlist(&playlist.id, &song(1)).unwrap(), 0);
        assert_eq!(library.add_to_playlist(&playlist.id, &song(1)).unwrap(), 1);

        let summary = library.playlist(&playlist.id).unwrap();
        assert_eq!(summary.track_count, 2);
        assert_eq!(summary.duration_ms, 400_000);
        // Taking one out leaves the other.
        assert!(
            library
                .remove_from_playlist(&playlist.id, 0, song(1).id)
                .unwrap()
        );
        let slice = library.playlist_slice(&playlist.id, 0, 10).unwrap();
        assert_eq!(slice.total, 1);
        assert_eq!(slice.tracks[0].id, song(1).id);
    }

    #[test]
    fn a_stale_removal_changes_nothing() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Mix").unwrap();
        library.add_to_playlist(&playlist.id, &song(1)).unwrap();
        library.add_to_playlist(&playlist.id, &song(2)).unwrap();

        // Position 0 holds song 1, not song 2; and position 5 holds nothing.
        assert!(
            !library
                .remove_from_playlist(&playlist.id, 0, song(2).id)
                .unwrap()
        );
        assert!(
            !library
                .remove_from_playlist(&playlist.id, 5, song(2).id)
                .unwrap()
        );

        assert_eq!(library.playlist(&playlist.id).unwrap().track_count, 2);
    }

    #[test]
    fn playlists_and_their_order_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let (library, _) = library_at(&path);
        let first = library.create_playlist("First").unwrap();
        let second = library.create_playlist("Second").unwrap();
        library.add_to_playlist(&second.id, &song(1)).unwrap();
        library.add_to_playlist(&second.id, &song(2)).unwrap();
        library.rename_playlist(&first.id, "  Renamed ").unwrap();
        drop(library);

        let library = reopen(&path);

        let kept = library.playlists().unwrap();
        let names: Vec<(&str, usize)> = kept
            .iter()
            .map(|p| (p.name.as_str(), p.track_count))
            .collect();
        assert_eq!(names, [("Renamed", 0), ("Second", 2)]);
        let slice = library.playlist_slice(&second.id, 0, 10).unwrap();
        assert_eq!(slice.tracks[1].title, "Song 2");
    }

    #[test]
    fn deleting_a_playlist_keeps_tracks_likes_and_history() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Gone soon").unwrap();
        library.add_to_playlist(&playlist.id, &song(1)).unwrap();
        library.like(&song(1)).unwrap();
        let play = library.next_play_id().unwrap();
        library.begin_play(play, &song(1), 1_000).unwrap();
        library
            .end_play(play, std::time::Duration::from_secs(5), EndReason::Finished)
            .unwrap();

        library.delete_playlist(&playlist.id).unwrap();

        assert!(library.playlists().unwrap().is_empty());
        assert!(!library.has_playlist(&playlist.id).unwrap());
        assert_eq!(library.track(song(1).id).unwrap(), Some(song(1)));
        assert_eq!(library.liked_count().unwrap(), 1);
        assert_eq!(library.played_count().unwrap(), 1);
        assert!(matches!(
            library.playlist_slice(&playlist.id, 0, 10),
            Err(LibraryError::NoPlaylist { .. })
        ));
    }

    #[test]
    fn a_playlist_needs_a_name_and_must_exist() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Real").unwrap();

        assert!(matches!(
            library.create_playlist("   "),
            Err(LibraryError::BlankName)
        ));
        assert!(matches!(
            library.rename_playlist(&playlist.id, ""),
            Err(LibraryError::BlankName)
        ));
        assert!(matches!(
            library.add_to_playlist("nope", &song(1)),
            Err(LibraryError::NoPlaylist { .. })
        ));
        assert!(matches!(
            library.delete_playlist("nope"),
            Err(LibraryError::NoPlaylist { .. })
        ));
        assert_eq!(library.playlist(&playlist.id).unwrap().name, "Real");
    }

    #[test]
    fn a_playlist_can_be_made_whole_under_an_id_of_its_own() {
        let (library, _) = memory();
        let tracks: Vec<_> = (1..=3).map(song).collect();

        let made = library
            .create_playlist_with("long-haul", "The long haul", &tracks)
            .unwrap();

        assert_eq!((made.id.as_str(), made.track_count), ("long-haul", 3));
        assert!(library.has_playlist("long-haul").unwrap());
    }

    #[test]
    fn a_playlist_lists_every_track_in_order_repeats_included() {
        let (library, _) = memory();
        let playlist = library.create_playlist("Mix").unwrap();
        for n in [3, 1, 3, 2] {
            library.add_to_playlist(&playlist.id, &song(n)).unwrap();
        }

        let ids = library.playlist_track_ids(&playlist.id).unwrap();

        assert_eq!(ids, [song(3).id, song(1).id, song(3).id, song(2).id]);
        assert!(matches!(
            library.playlist_track_ids("nope"),
            Err(LibraryError::NoPlaylist { .. })
        ));
    }
}
