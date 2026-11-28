//! Versioned schema steps. A step runs once, in a transaction that also moves
//! `user_version`, so a crash leaves the old version or the new one.

use rusqlite::Connection;

use super::LibraryError;

/// Step `n` takes a database from version `n - 1` to `n`. Add steps at the end;
/// never edit one that has shipped.
const STEPS: &[&str] = &[
    // 1: Tracks, the services' ids for them, likes and play history.
    "
    CREATE TABLE tracks (
        id TEXT PRIMARY KEY NOT NULL,
        title TEXT NOT NULL,
        artists TEXT NOT NULL,
        album TEXT,
        duration_ms INTEGER NOT NULL,
        artwork_url TEXT,
        isrc TEXT
    ) STRICT;
    CREATE TABLE track_sources (
        kind TEXT NOT NULL,
        source_id TEXT NOT NULL,
        track_id TEXT NOT NULL REFERENCES tracks(id),
        PRIMARY KEY (kind, source_id)
    ) STRICT;
    CREATE INDEX track_sources_by_track ON track_sources(track_id);
    CREATE TABLE likes (
        track_id TEXT PRIMARY KEY NOT NULL REFERENCES tracks(id),
        liked_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX likes_by_time ON likes(liked_at DESC);
    CREATE TABLE history (
        id INTEGER PRIMARY KEY NOT NULL,
        track_id TEXT NOT NULL REFERENCES tracks(id),
        started_at INTEGER NOT NULL,
        listened_ms INTEGER NOT NULL DEFAULT 0,
        end_reason TEXT
    ) STRICT;
    CREATE INDEX history_by_track ON history(track_id, started_at DESC);
    ",
    // 2: Local playlists. An entry is a place in a playlist, so one Track can
    // hold several. Removing a playlist takes its entries and nothing else.
    "
    CREATE TABLE playlists (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL
    ) STRICT;
    CREATE TABLE playlist_entries (
        id INTEGER PRIMARY KEY NOT NULL,
        playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        track_id TEXT NOT NULL REFERENCES tracks(id)
    ) STRICT;
    CREATE INDEX playlist_entries_by_position ON playlist_entries(playlist_id, position);
    ",
];

/// The version a fresh database ends at.
pub const CURRENT: i64 = STEPS.len() as i64;

/// Brings the database to [`CURRENT`]. A newer one is refused before anything
/// is written.
pub fn migrate(conn: &mut Connection) -> Result<(), LibraryError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > CURRENT {
        return Err(LibraryError::Newer {
            found,
            supported: CURRENT,
        });
    }
    for (index, step) in STEPS
        .iter()
        .enumerate()
        .skip(usize::try_from(found).unwrap_or(0))
    {
        let version = index + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(step)?;
        tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        tx.commit()?;
    }
    Ok(())
}
