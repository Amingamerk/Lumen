//! The SQLite library: Tracks Lumen has met, liked songs and play history. It
//! stays on this machine and holds no secrets, only what a row shows.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::Connection;
use uuid::Uuid;

use crate::catalog::Track;
use crate::sources::youtube::GateClock;

mod history;
mod likes;
mod matches;
mod playlists;
mod schema;
mod streams;
mod tracks;
mod writer;

pub use history::{EndReason, HistoryChange, NoLog, PlayId, PlayLog};
pub use likes::LikeChange;
pub use playlists::PlaylistSummary;
pub use streams::StoredStream;
pub use writer::HistoryWriter;

/// Why the library could not be used.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    /// Opening it as an older Lumen would change a file a newer Lumen made.
    #[error(
        "the library was made by a newer Lumen (schema {found}, this Lumen reads up to {supported}); it was left as it is"
    )]
    Newer { found: i64, supported: i64 },
    #[error("the library file is damaged; it was left as it is ({0})")]
    Corrupt(String),
    #[error("the library failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("the library folder could not be made: {0}")]
    Folder(#[from] std::io::Error),
    #[error("a playlist needs a name")]
    BlankName,
    #[error("there is no playlist with the id “{id}”")]
    NoPlaylist { id: String },
}

/// The time rows are stamped with. A seam so tests move it by hand.
pub trait WallClock: Send + Sync {
    /// Unix milliseconds.
    fn wall_ms(&self) -> u64;
}

impl<C: GateClock> WallClock for C {
    fn wall_ms(&self) -> u64 {
        GateClock::wall_ms(self)
    }
}

/// SQLite integers are signed; nothing stored here comes near the limit.
fn sql_int(n: impl TryInto<i64>) -> i64 {
    n.try_into().unwrap_or(i64::MAX)
}

/// A file that is not a database, or is broken, is damaged rather than a
/// failure of this call.
fn damaged(error: rusqlite::Error) -> LibraryError {
    use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
    match error.sqlite_error_code() {
        Some(NotADatabase | DatabaseCorrupt) => LibraryError::Corrupt(error.to_string()),
        _ => LibraryError::Database(error),
    }
}

/// The library over one SQLite file.
pub struct Library {
    conn: Mutex<Connection>,
    clock: Box<dyn WallClock>,
}

impl Library {
    /// Opens the file, making it if there is none, and brings it to the current
    /// schema. A newer or damaged file is reported and never changed.
    pub fn open(path: &Path, clock: impl WallClock + 'static) -> Result<Self, LibraryError> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let mut conn = Connection::open(path).map_err(damaged)?;
        // Reading first: a file that is not a database says so here, before
        // any setting below would write to it.
        let found: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(damaged)?;
        if found > schema::CURRENT {
            return Err(LibraryError::Newer {
                found,
                supported: schema::CURRENT,
            });
        }
        let check: String = conn
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(damaged)?;
        if check != "ok" {
            return Err(LibraryError::Corrupt(check));
        }
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;",
        )?;
        schema::migrate(&mut conn)?;
        let library = Self {
            conn: Mutex::new(conn),
            clock: Box::new(clock),
        };
        library.close_interrupted()?;
        Ok(library)
    }

    #[cfg(test)]
    pub fn schema_version(&self) -> i64 {
        self.conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// The Track kept under `id`, if the library has met it.
    pub fn track(&self, id: Uuid) -> Result<Option<Track>, LibraryError> {
        tracks::find(&self.conn(), id)
    }

    /// Stores a song without liking it, so the Queue can find it by id. Gives
    /// the id it is kept under.
    pub fn keep(&self, track: &Track) -> Result<Uuid, LibraryError> {
        tracks::ensure(&self.conn(), track)
    }

    /// Where the picture of the Track under `id` comes from. The webview never
    /// sees it; the art cache fetches it.
    pub fn artwork_url(&self, id: Uuid) -> Result<Option<String>, LibraryError> {
        tracks::artwork_url(&self.conn(), id)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Stand-ins for the tests of this module and of the ones that use the library.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::Path;

    use uuid::Uuid;

    use super::*;
    use crate::catalog::{SourceKind, SourceRef};
    use crate::sources::youtube::testing::FakeClock;

    pub const START_MS: u64 = 1_700_000_000_000;

    /// A song with its own id and one YouTube ref.
    pub fn song(n: u128) -> Track {
        Track {
            id: Uuid::from_u128(n),
            title: format!("Song {n}"),
            artists: vec!["Mira Sol".to_owned()],
            album: None,
            duration_ms: 200_000,
            artwork_url: None,
            isrc: None,
            sources: vec![SourceRef {
                kind: SourceKind::YouTube,
                id: format!("video{n:06}"),
            }],
        }
    }

    pub fn library_at(path: &Path) -> (Library, FakeClock) {
        let clock = FakeClock::at(START_MS);
        (Library::open(path, clock.clone()).unwrap(), clock)
    }

    pub fn reopen(path: &Path) -> Library {
        library_at(path).0
    }

    /// A library in a file that goes away with the test thread's temp folder.
    pub fn memory() -> (Library, FakeClock) {
        let dir = tempfile::tempdir().unwrap().keep();
        library_at(&dir.join("lumen.db"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::youtube::testing::FakeClock;
    use std::fs;

    fn clock() -> FakeClock {
        FakeClock::at(1_700_000_000_000)
    }

    #[test]
    fn an_empty_database_is_brought_to_the_current_schema() {
        let dir = tempfile::tempdir().unwrap();

        let library = Library::open(&dir.path().join("lumen.db"), clock()).unwrap();

        assert_eq!(library.schema_version(), schema::CURRENT);
    }

    #[test]
    fn a_newer_database_is_reported_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let newer = Connection::open(&path).unwrap();
        newer
            .execute_batch("CREATE TABLE future (x); PRAGMA user_version = 999;")
            .unwrap();
        drop(newer);
        let before = fs::read(&path).unwrap();

        let error = Library::open(&path, clock()).err().expect("refused");

        assert!(matches!(error, LibraryError::Newer { found: 999, .. }));
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_corrupt_file_is_reported_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        fs::write(
            &path,
            b"this is not a database, but it is long enough to be read as one.....",
        )
        .unwrap();
        let before = fs::read(&path).unwrap();

        let error = Library::open(&path, clock()).err().expect("refused");

        assert!(matches!(error, LibraryError::Corrupt(_)), "{error}");
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn the_library_gives_the_artwork_address_of_a_kept_track() {
        let (library, _) = testing::memory();
        let mut with = testing::song(1);
        with.artwork_url = Some("https://i.ytimg.com/vi/a/hq720.jpg".to_owned());
        library.like(&with).unwrap();
        library.like(&testing::song(2)).unwrap();

        assert_eq!(
            library.artwork_url(with.id).unwrap().as_deref(),
            with.artwork_url.as_deref()
        );
        assert_eq!(library.artwork_url(Uuid::from_u128(2)).unwrap(), None);
        assert_eq!(library.artwork_url(Uuid::from_u128(3)).unwrap(), None);
    }

    #[test]
    fn keeping_a_song_makes_it_known_under_the_id_it_already_had() {
        let (library, _) = testing::memory();
        let first = testing::song(1);
        library.like(&first).unwrap();
        let mut again = testing::song(2);
        again.sources = first.sources.clone();

        assert_eq!(library.keep(&again).unwrap(), first.id);
        assert_eq!(library.keep(&testing::song(3)).unwrap(), Uuid::from_u128(3));

        assert_eq!(
            library.track(Uuid::from_u128(3)).unwrap().unwrap().title,
            "Song 3"
        );
        assert_eq!(library.liked_count().unwrap(), 1);
    }
}
