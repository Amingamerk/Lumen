//! Play history: one row per play, written when audio starts and closed when
//! it stops.

use std::time::Duration;

use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use super::{Library, LibraryError, sql_int, tracks};
use crate::catalog::{MAX_SLICE, Track, TrackRow, TrackSlice};

/// Names one play. Handed out by the [`PlayLog`] that starts it.
pub type PlayId = u64;

/// Why a play stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The track played to its end.
    Finished,
    /// Next or Previous moved on from it.
    Skipped,
    /// Another song was chosen.
    Replaced,
    /// Playback failed and gave up.
    Failed,
    /// The app closed.
    Exit,
}

impl EndReason {
    pub fn name(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Skipped => "skipped",
            Self::Replaced => "replaced",
            Self::Failed => "failed",
            Self::Exit => "exit",
        }
    }
}

/// Where the player writes what was played. It never waits for a database.
pub trait PlayLog: Send {
    /// Audio of `track` has started. Gives the number the play is closed by.
    fn started(&self, track: &Track, started_at_ms: u64) -> PlayId;
    /// The play stopped, after `listened` of actual playing.
    fn ended(&self, play: PlayId, listened: Duration, reason: EndReason);
}

/// A log that keeps nothing.
pub struct NoLog;

impl PlayLog for NoLog {
    fn started(&self, _: &Track, _: u64) -> PlayId {
        0
    }

    fn ended(&self, _: PlayId, _: Duration, _: EndReason) {}
}

/// What the UI is told when the history changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryChange {
    /// How many different songs have been played.
    pub played_count: usize,
}

/// One row of the history table, as tests read it back.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayRecord {
    pub track_id: Uuid,
    pub started_at_ms: u64,
    pub listened_ms: u64,
    pub end_reason: Option<String>,
}

impl Library {
    /// The number the next play gets.
    pub fn next_play_id(&self) -> Result<PlayId, LibraryError> {
        let last: i64 =
            self.conn()
                .query_row("SELECT COALESCE(MAX(id), 0) FROM history", [], |row| {
                    row.get(0)
                })?;
        Ok(u64::try_from(last).unwrap_or(0) + 1)
    }

    /// Records that audio of `track` started as play `id`.
    pub fn begin_play(
        &self,
        id: PlayId,
        track: &Track,
        started_at_ms: u64,
    ) -> Result<HistoryChange, LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let track = tracks::ensure(&tx, track)?;
        tx.execute(
            "INSERT INTO history (id, track_id, started_at) VALUES (?1, ?2, ?3)",
            params![sql_int(id), track.to_string(), sql_int(started_at_ms)],
        )?;
        let played_count = played_count(&tx)?;
        tx.commit()?;
        Ok(HistoryChange { played_count })
    }

    /// Closes play `id`. A play that is closed stays as it was.
    pub fn end_play(
        &self,
        id: PlayId,
        listened: Duration,
        reason: EndReason,
    ) -> Result<(), LibraryError> {
        self.conn().execute(
            "UPDATE history SET listened_ms = ?2, end_reason = ?3
             WHERE id = ?1 AND end_reason IS NULL",
            params![sql_int(id), sql_int(listened.as_millis()), reason.name()],
        )?;
        Ok(())
    }

    /// Plays a crash cut short are closed, with the time they had no word of.
    pub(super) fn close_interrupted(&self) -> Result<(), LibraryError> {
        self.conn().execute(
            "UPDATE history SET end_reason = 'interrupted' WHERE end_reason IS NULL",
            [],
        )?;
        Ok(())
    }

    /// How many different songs have been played.
    pub fn played_count(&self) -> Result<usize, LibraryError> {
        played_count(&self.conn())
    }

    /// Every song played by id, the one played last first, each once.
    pub fn recent_list(&self) -> Result<Vec<Uuid>, LibraryError> {
        let conn = self.conn();
        let mut ask = conn.prepare_cached(
            "SELECT track_id FROM history GROUP BY track_id
             ORDER BY MAX(started_at) DESC, MAX(id) DESC",
        )?;
        let ids = ask
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect())
    }

    /// Up to `limit` songs from `offset`, the one played last first. A song
    /// played often is listed once.
    pub fn recent_slice(&self, offset: usize, limit: usize) -> Result<TrackSlice, LibraryError> {
        let conn = self.conn();
        let mut ask = conn.prepare_cached(
            "SELECT track_id FROM history GROUP BY track_id
             ORDER BY MAX(started_at) DESC, MAX(id) DESC LIMIT ?1 OFFSET ?2",
        )?;
        let ids: Vec<String> = ask
            .query_map(
                params![sql_int(limit.min(MAX_SLICE)), sql_int(offset)],
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
            total: played_count(&conn)?,
            offset,
            tracks: rows,
        })
    }

    /// Every play, oldest first.
    #[cfg(test)]
    pub fn plays(&self) -> Vec<PlayRecord> {
        let conn = self.conn();
        let mut ask = conn
            .prepare(
                "SELECT track_id, started_at, listened_ms, end_reason FROM history ORDER BY id",
            )
            .unwrap();
        ask.query_map([], |row| {
            Ok(PlayRecord {
                track_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                started_at_ms: row.get::<_, i64>(1)?.try_into().unwrap(),
                listened_ms: row.get::<_, i64>(2)?.try_into().unwrap(),
                end_reason: row.get(3)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }
}

fn played_count(conn: &rusqlite::Connection) -> Result<usize, LibraryError> {
    let count: i64 = conn.query_row("SELECT COUNT(DISTINCT track_id) FROM history", [], |row| {
        row.get(0)
    })?;
    Ok(usize::try_from(count).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::library::testing::{START_MS, library_at, memory, reopen, song};

    #[test]
    fn a_play_keeps_when_it_started_how_long_it_ran_and_why_it_stopped() {
        let (library, _) = memory();
        let id = library.next_play_id().unwrap();

        let change = library.begin_play(id, &song(1), START_MS).unwrap();
        library
            .end_play(id, Duration::from_millis(61_500), EndReason::Skipped)
            .unwrap();

        assert_eq!(change.played_count, 1);
        assert_eq!(
            library.plays(),
            [PlayRecord {
                track_id: song(1).id,
                started_at_ms: START_MS,
                listened_ms: 61_500,
                end_reason: Some("skipped".to_owned()),
            }]
        );
        assert_eq!(library.next_play_id().unwrap(), id + 1);
    }

    #[test]
    fn a_closed_play_is_not_closed_again() {
        let (library, _) = memory();
        library.begin_play(1, &song(1), START_MS).unwrap();
        library
            .end_play(1, Duration::from_secs(5), EndReason::Finished)
            .unwrap();

        library
            .end_play(1, Duration::from_secs(99), EndReason::Exit)
            .unwrap();

        assert_eq!(library.plays()[0].listened_ms, 5_000);
        assert_eq!(library.plays()[0].end_reason.as_deref(), Some("finished"));
    }

    #[test]
    fn a_play_a_crash_cut_short_is_closed_as_interrupted_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let (library, _) = library_at(&path);
        library.begin_play(1, &song(1), START_MS).unwrap();
        drop(library);

        let library = reopen(&path);

        assert_eq!(
            library.plays()[0].end_reason.as_deref(),
            Some("interrupted")
        );
        assert_eq!(library.next_play_id().unwrap(), 2);
    }

    #[test]
    fn recent_songs_come_once_each_with_the_last_played_first() {
        let (library, _) = memory();
        for (id, n, at) in [(1, 1, 10), (2, 2, 20), (3, 1, 30), (4, 3, 5)] {
            library.begin_play(id, &song(n), START_MS + at).unwrap();
        }

        let slice = library.recent_slice(0, 10).unwrap();

        let titles: Vec<&str> = slice.tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Song 1", "Song 2", "Song 3"]);
        assert_eq!(slice.total, 3);
        assert_eq!(
            library.recent_list().unwrap(),
            [song(1).id, song(2).id, song(3).id]
        );
        let tail = library.recent_slice(2, 10).unwrap();
        assert_eq!(tail.tracks.len(), 1);
    }
}
