//! Small choices kept between launches, each a value under a key.

use rusqlite::OptionalExtension;

use super::{Library, LibraryError};

impl Library {
    /// The value kept under `key`, if there is one.
    pub fn setting(&self, key: &str) -> Result<Option<String>, LibraryError> {
        let value = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value)
    }

    /// Keeps `value` under `key`, replacing what was there.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), LibraryError> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::library::testing::{library_at, reopen, song};

    #[test]
    fn a_setting_is_kept_between_launches_and_replaced_by_a_newer_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let (library, _) = library_at(&path);
        assert_eq!(library.setting("repeat").unwrap(), None);

        library.set_setting("repeat", "all").unwrap();
        library.set_setting("shuffle", "on").unwrap();
        library.set_setting("repeat", "one").unwrap();
        drop(library);

        let library = reopen(&path);
        assert_eq!(library.setting("repeat").unwrap().as_deref(), Some("one"));
        assert_eq!(library.setting("shuffle").unwrap().as_deref(), Some("on"));
    }

    #[test]
    fn a_library_from_before_settings_gains_them_and_keeps_its_likes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let (library, _) = library_at(&path);
        library.like(&song(1)).unwrap();
        drop(library);
        // Back to the shape version 3 left.
        Connection::open(&path)
            .unwrap()
            .execute_batch("DROP TABLE IF EXISTS settings; PRAGMA user_version = 3;")
            .unwrap();

        let library = reopen(&path);

        library.set_setting("repeat", "all").unwrap();
        assert_eq!(library.setting("repeat").unwrap().as_deref(), Some("all"));
        assert_eq!(library.liked_count().unwrap(), 1);
    }
}
