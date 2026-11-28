//! Stream URLs kept for a replay after a restart. A row belongs to one login and
//! is useless to another. The URL is stored as it is, and it holds no cookie:
//! the headers kept beside it are the ones mpv needs, minus anything secret.

use std::fmt;

use rusqlite::{OptionalExtension, params};

use super::{Library, LibraryError, sql_int, tracks};
use crate::catalog::SourceRef;

/// One resolved stream as the library keeps it.
#[derive(Clone, PartialEq)]
pub struct StoredStream {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Unix seconds before which the URL does not work.
    pub available_at: Option<u64>,
    /// Unix seconds after which it stops working.
    pub expires_at: u64,
}

/// Never shows the URL, which holds the client IP, or a header.
impl fmt::Debug for StoredStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredStream")
            .field("available_at", &self.available_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Headers that carry a login. They are never written.
fn is_secret(name: &str) -> bool {
    [
        "cookie",
        "set-cookie",
        "authorization",
        "proxy-authorization",
    ]
    .iter()
    .any(|secret| name.eq_ignore_ascii_case(secret))
}

impl Library {
    /// Keeps `stream` as the one of `reference` for `login`, and drops the ones
    /// that have expired.
    pub fn keep_stream(
        &self,
        login: &str,
        reference: &SourceRef,
        stream: &StoredStream,
    ) -> Result<(), LibraryError> {
        let headers: Vec<&(String, String)> = stream
            .headers
            .iter()
            .filter(|(name, _)| !is_secret(name))
            .collect();
        let headers = serde_json::to_string(&headers).unwrap_or_else(|_| "[]".to_owned());
        let now_s = sql_int(self.clock.wall_ms() / 1_000);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM streams WHERE expires_at <= ?1", [now_s])?;
        tx.execute(
            "INSERT OR REPLACE INTO streams
                (kind, source_id, login, url, headers, available_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                tracks::kind_name(reference.kind),
                reference.id,
                login,
                stream.url,
                headers,
                stream.available_at.map(sql_int),
                sql_int(stream.expires_at),
            ],
        )?;
        Ok(tx.commit()?)
    }

    /// The stream kept for `reference` by `login`, if it still works at
    /// `needed_ms` (Unix milliseconds). Another login's stream is not given.
    pub fn stream(
        &self,
        login: &str,
        reference: &SourceRef,
        needed_ms: u64,
    ) -> Result<Option<StoredStream>, LibraryError> {
        let row = self
            .conn()
            .query_row(
                "SELECT url, headers, available_at, expires_at FROM streams
                 WHERE kind = ?1 AND source_id = ?2 AND login = ?3 AND expires_at * 1000 >= ?4",
                params![
                    tracks::kind_name(reference.kind),
                    reference.id,
                    login,
                    sql_int(needed_ms)
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(
            row.map(|(url, headers, available_at, expires_at)| StoredStream {
                url,
                headers: serde_json::from_str(&headers).unwrap_or_default(),
                available_at: available_at.and_then(|at| u64::try_from(at).ok()),
                expires_at: u64::try_from(expires_at).unwrap_or(0),
            }),
        )
    }

    /// Drops what is kept for `reference`, whoever it belongs to.
    pub fn forget_stream(&self, reference: &SourceRef) -> Result<(), LibraryError> {
        self.conn().execute(
            "DELETE FROM streams WHERE kind = ?1 AND source_id = ?2",
            params![tracks::kind_name(reference.kind), reference.id],
        )?;
        Ok(())
    }

    /// Drops every kept stream.
    pub fn forget_streams(&self) -> Result<(), LibraryError> {
        self.conn().execute("DELETE FROM streams", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::testing::{START_MS, memory};
    use super::*;
    use crate::catalog::SourceKind;

    const EXPIRES_S: u64 = START_MS / 1_000 + 21_600;

    fn youtube(id: &str) -> SourceRef {
        SourceRef {
            kind: SourceKind::YouTube,
            id: id.to_owned(),
        }
    }

    fn stream() -> StoredStream {
        StoredStream {
            url: "https://rr1.googlevideo.com/videoplayback?expire=1".to_owned(),
            headers: vec![
                ("User-Agent".to_owned(), "Agent/1.0".to_owned()),
                ("Cookie".to_owned(), "SAPISID=secret".to_owned()),
                ("authorization".to_owned(), "secret".to_owned()),
            ],
            available_at: Some(START_MS / 1_000 + 2),
            expires_at: EXPIRES_S,
        }
    }

    #[test]
    fn a_stream_comes_back_for_its_login_without_secret_headers() {
        let (library, _) = memory();
        library
            .keep_stream("login-a", &youtube("a"), &stream())
            .unwrap();

        let back = library.stream("login-a", &youtube("a"), START_MS).unwrap();

        let mut expected = stream();
        expected.headers.truncate(1);
        assert_eq!(back, Some(expected));
        assert_eq!(
            library.stream("login-b", &youtube("a"), START_MS).unwrap(),
            None
        );
        assert_eq!(
            library.stream("login-a", &youtube("b"), START_MS).unwrap(),
            None
        );
    }

    #[test]
    fn a_stream_with_too_little_left_is_not_given() {
        let (library, _) = memory();
        library
            .keep_stream("login-a", &youtube("a"), &stream())
            .unwrap();

        let exactly = EXPIRES_S * 1_000;
        assert!(
            library
                .stream("login-a", &youtube("a"), exactly)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            library
                .stream("login-a", &youtube("a"), exactly + 1)
                .unwrap(),
            None
        );
    }

    #[test]
    fn keeping_a_stream_drops_the_expired_ones() {
        let (library, clock) = memory();
        library
            .keep_stream("login-a", &youtube("old"), &stream())
            .unwrap();
        clock.advance(Duration::from_secs(21_600));

        library
            .keep_stream(
                "login-a",
                &youtube("new"),
                &StoredStream {
                    expires_at: EXPIRES_S + 21_600,
                    ..stream()
                },
            )
            .unwrap();

        assert_eq!(library.stream("login-a", &youtube("old"), 0).unwrap(), None);
        assert!(
            library
                .stream("login-a", &youtube("new"), 0)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn forgetting_one_or_all_streams() {
        let (library, _) = memory();
        library
            .keep_stream("login-a", &youtube("a"), &stream())
            .unwrap();
        library
            .keep_stream("login-a", &youtube("b"), &stream())
            .unwrap();

        library.forget_stream(&youtube("a")).unwrap();
        assert_eq!(library.stream("login-a", &youtube("a"), 0).unwrap(), None);
        assert!(
            library
                .stream("login-a", &youtube("b"), 0)
                .unwrap()
                .is_some()
        );

        library.forget_streams().unwrap();
        assert_eq!(library.stream("login-a", &youtube("b"), 0).unwrap(), None);
    }
}
