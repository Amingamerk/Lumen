//! Tracks as the library keeps them: one row per song, however many services
//! know it, found again by any of its `SourceRef`s.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::LibraryError;
use crate::catalog::{SourceKind, SourceRef, Track};

pub(super) fn kind_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::YouTube => "youtube",
        SourceKind::SoundCloud => "soundcloud",
        SourceKind::Spotify => "spotify",
    }
}

pub(super) fn kind_of(name: &str) -> Option<SourceKind> {
    match name {
        "youtube" => Some(SourceKind::YouTube),
        "soundcloud" => Some(SourceKind::SoundCloud),
        "spotify" => Some(SourceKind::Spotify),
        _ => None,
    }
}

/// Stores `track` and its refs, and gives the id it is kept under. A song
/// already known by one of its refs keeps the id it had, so there is one Track
/// per ref.
pub(super) fn ensure(conn: &Connection, track: &Track) -> Result<Uuid, LibraryError> {
    let mut id = track.id;
    for reference in &track.sources {
        let known: Option<String> = conn
            .query_row(
                "SELECT track_id FROM track_sources WHERE kind = ?1 AND source_id = ?2",
                params![kind_name(reference.kind), reference.id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(known) = known.and_then(|text| Uuid::parse_str(&text).ok()) {
            id = known;
            break;
        }
    }
    let artists = serde_json::to_string(&track.artists).unwrap_or_else(|_| "[]".to_owned());
    // What a later sighting leaves out does not wipe what was learned before.
    conn.execute(
        "INSERT INTO tracks (id, title, artists, album, duration_ms, artwork_url, isrc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            artists = CASE WHEN excluded.artists = '[]' THEN artists ELSE excluded.artists END,
            album = COALESCE(excluded.album, album),
            duration_ms = CASE WHEN excluded.duration_ms = 0 THEN duration_ms ELSE excluded.duration_ms END,
            artwork_url = COALESCE(excluded.artwork_url, artwork_url),
            isrc = COALESCE(excluded.isrc, isrc)",
        params![
            id.to_string(),
            track.title,
            artists,
            track.album,
            track.duration_ms,
            track.artwork_url,
            track.isrc
        ],
    )?;
    for reference in &track.sources {
        conn.execute(
            "INSERT OR IGNORE INTO track_sources (kind, source_id, track_id) VALUES (?1, ?2, ?3)",
            params![kind_name(reference.kind), reference.id, id.to_string()],
        )?;
    }
    Ok(id)
}

/// Where the picture of the Track under `id` is fetched from, if it has one.
pub(super) fn artwork_url(conn: &Connection, id: Uuid) -> Result<Option<String>, LibraryError> {
    let url = conn
        .query_row(
            "SELECT artwork_url FROM tracks WHERE id = ?1",
            [id.to_string()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(url.flatten())
}

/// The Track kept under `id`.
pub(super) fn find(conn: &Connection, id: Uuid) -> Result<Option<Track>, LibraryError> {
    let row = conn
        .query_row(
            "SELECT title, artists, album, duration_ms, artwork_url, isrc FROM tracks WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((title, artists, album, duration_ms, artwork_url, isrc)) = row else {
        return Ok(None);
    };
    let mut refs = conn.prepare_cached(
        "SELECT kind, source_id FROM track_sources WHERE track_id = ?1 ORDER BY rowid",
    )?;
    let sources = refs
        .query_map([id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(Result::ok)
        .filter_map(|(kind, id)| {
            Some(SourceRef {
                kind: kind_of(&kind)?,
                id,
            })
        })
        .collect();
    Ok(Some(Track {
        id,
        title,
        artists: serde_json::from_str(&artists).unwrap_or_default(),
        album,
        duration_ms,
        artwork_url,
        isrc,
        sources,
    }))
}
