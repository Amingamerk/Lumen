//! The one captured YouTube Music session and its storage in Windows Credential Manager.

use serde::{Deserialize, Serialize};

use crate::{
    auth::sapisid,
    cookies::{Cookie, matching, youtube_only},
};

const HOST: &str = "music.youtube.com";
const MAX_USER_AGENT_BYTES: usize = 512;

/// Windows rejects a generic credential blob above this size (`CRED_MAX_CREDENTIAL_BLOB_SIZE`).
pub const MAX_SECRET_BYTES: usize = 2560;
/// Upper bound on the entries one session may use.
const MAX_CHUNKS: usize = 16;
const MANIFEST_KEY: &str = "session-manifest";

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SessionError {
    #[error(
        "no SAPISID or __Secure-3PAPISID cookie for music.youtube.com: not a signed-in session"
    )]
    NotSignedIn,
    #[error("the User-Agent is empty, too long or contains control characters")]
    BadUserAgent,
    #[error("the session is too large to store")]
    TooLarge,
    #[error("the stored session is incomplete or unreadable: clear it and log in again")]
    Corrupt,
    #[error("credential store: {0}")]
    Store(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CaptureMethod {
    LoginWindow,
    Import,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub cookies: Vec<Cookie>,
    pub user_agent: String,
    pub method: CaptureMethod,
    /// Unix seconds.
    pub captured_at: u64,
    /// Set on 401/403. An expired session is never sent again.
    pub expired: bool,
}

/// What the UI may know about a session. Holds no cookie values.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionStatus {
    pub method: CaptureMethod,
    pub captured_at: u64,
    pub cookie_count: usize,
    pub has_login_info: bool,
    pub expired: bool,
    pub user_agent: String,
}

impl Session {
    pub fn new(
        cookies: Vec<Cookie>,
        user_agent: &str,
        method: CaptureMethod,
        now: u64,
    ) -> Result<Self, SessionError> {
        let cookies = youtube_only(cookies);
        if sapisid(&matching(&cookies, HOST, "/", now)).is_none() {
            return Err(SessionError::NotSignedIn);
        }
        let user_agent = user_agent.trim();
        if user_agent.is_empty()
            || user_agent.len() > MAX_USER_AGENT_BYTES
            || user_agent.chars().any(char::is_control)
        {
            return Err(SessionError::BadUserAgent);
        }
        Ok(Self {
            cookies,
            user_agent: user_agent.to_owned(),
            method,
            captured_at: now,
            expired: false,
        })
    }

    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            method: self.method,
            captured_at: self.captured_at,
            cookie_count: self.cookies.len(),
            has_login_info: has_login_info(self.cookies.iter()),
            expired: self.expired,
            user_agent: self.user_agent.clone(),
        }
    }
}

/// True once a login window holds a complete session. yt-dlp treats cookies without
/// `LOGIN_INFO` as signed out, so wait for it too.
pub fn is_signed_in(cookies: &[Cookie], now: u64) -> bool {
    let sent = matching(cookies, HOST, "/", now);
    sapisid(&sent).is_some() && has_login_info(sent.into_iter())
}

fn has_login_info<'a>(mut cookies: impl Iterator<Item = &'a Cookie>) -> bool {
    cookies.any(|c| c.name == "LOGIN_INFO" && !c.value.is_empty())
}

/// A secret key/value store: Credential Manager in the app, memory in tests.
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SessionError>;
    fn set(&self, key: &str, value: &[u8]) -> Result<(), SessionError>;
    /// Deleting a missing key succeeds.
    fn delete(&self, key: &str) -> Result<(), SessionError>;
}

/// Windows Credential Manager through `keyring`.
pub struct KeyringStore {
    service: &'static str,
}

impl KeyringStore {
    pub fn new(service: &'static str) -> Self {
        Self { service }
    }
}

impl KeyringStore {
    fn entry(&self, key: &str) -> Result<keyring::Entry, SessionError> {
        keyring::Entry::new(self.service, key).map_err(store_error)
    }
}

/// `keyring` error messages never contain the secret itself.
fn store_error(error: keyring::Error) -> SessionError {
    SessionError::Store(error.to_string())
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SessionError> {
        match self.entry(key)?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(store_error(error)),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<(), SessionError> {
        self.entry(key)?.set_secret(value).map_err(store_error)
    }

    fn delete(&self, key: &str) -> Result<(), SessionError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(store_error(error)),
        }
    }
}

/// Saves the session as numbered chunks plus a manifest, because one credential is too small.
pub struct SessionVault<S> {
    store: S,
}

impl<S: SecretStore> SessionVault<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn save(&self, session: &Session) -> Result<(), SessionError> {
        let bytes = serde_json::to_vec(session).map_err(|_| SessionError::Corrupt)?;
        let chunks = bytes.chunks(MAX_SECRET_BYTES);
        let count = chunks.len();
        if count > MAX_CHUNKS {
            return Err(SessionError::TooLarge);
        }
        // Old entries go first, so old and new chunks can never be read as one session.
        self.clear()?;
        let written = chunks
            .enumerate()
            .try_for_each(|(index, chunk)| self.store.set(&chunk_key(index), chunk))
            .and_then(|()| self.store.set(MANIFEST_KEY, count.to_string().as_bytes()));
        if written.is_err() {
            // Best effort: the write error is the one worth reporting.
            let _ = self.clear();
        }
        written
    }

    pub fn load(&self) -> Result<Option<Session>, SessionError> {
        let Some(manifest) = self.store.get(MANIFEST_KEY)? else {
            return Ok(None);
        };
        let count = std::str::from_utf8(&manifest)
            .ok()
            .and_then(|text| text.parse::<usize>().ok())
            .filter(|count| (1..=MAX_CHUNKS).contains(count))
            .ok_or(SessionError::Corrupt)?;
        let mut bytes = Vec::new();
        for index in 0..count {
            let chunk = self.store.get(&chunk_key(index))?;
            bytes.extend(chunk.ok_or(SessionError::Corrupt)?);
        }
        // The parser's message could quote stored content, so it is dropped.
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| SessionError::Corrupt)
    }

    /// Sweeps every possible chunk, so nothing survives an interrupted earlier save.
    pub fn clear(&self) -> Result<(), SessionError> {
        self.store.delete(MANIFEST_KEY)?;
        (0..MAX_CHUNKS).try_for_each(|index| self.store.delete(&chunk_key(index)))
    }
}

fn chunk_key(index: usize) -> String {
    format!("session-chunk-{index}")
}

#[cfg(test)]
pub(crate) mod fake {
    use std::{cell::RefCell, collections::BTreeMap};

    use super::*;

    /// In-memory store with the Windows size limit.
    #[derive(Default)]
    pub struct MemoryStore {
        pub entries: RefCell<BTreeMap<String, Vec<u8>>>,
        pub fail_set_of: RefCell<Option<String>>,
    }

    impl SecretStore for &MemoryStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SessionError> {
            Ok(self.entries.borrow().get(key).cloned())
        }

        fn set(&self, key: &str, value: &[u8]) -> Result<(), SessionError> {
            if self.fail_set_of.borrow().as_deref() == Some(key) {
                return Err(SessionError::Store("write failed".into()));
            }
            if value.len() > MAX_SECRET_BYTES {
                return Err(SessionError::Store("secret too long".into()));
            }
            self.entries.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), SessionError> {
            self.entries.borrow_mut().remove(key);
            Ok(())
        }
    }

    pub fn cookie(name: &str, value: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: ".youtube.com".into(),
            path: "/".into(),
            secure: true,
            http_only: false,
            expires: None,
        }
    }

    pub fn session() -> Session {
        Session {
            cookies: vec![
                cookie("SAPISID", "fake-sapisid"),
                cookie("LOGIN_INFO", "fake-login-info"),
            ],
            user_agent: "FakeAgent/1.0".into(),
            method: CaptureMethod::LoginWindow,
            captured_at: 1_000,
            expired: false,
        }
    }

    /// Several kilobytes, like a real signed-in cookie set.
    pub fn large_session() -> Session {
        let mut large = session();
        large
            .cookies
            .extend((0..12).map(|i| cookie(&format!("C{i}"), &"v".repeat(500))));
        large
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::*, *};

    fn keys(store: &MemoryStore) -> Vec<String> {
        store.entries.borrow().keys().cloned().collect()
    }

    #[test]
    fn a_new_session_keeps_only_youtube_cookies() {
        let google = Cookie {
            domain: ".google.com".into(),
            ..cookie("SID", "other-site")
        };

        let session = Session::new(
            vec![google, cookie("SAPISID", "a")],
            " Agent/1.0 ",
            CaptureMethod::Import,
            42,
        )
        .unwrap();

        assert_eq!(session.cookies, [cookie("SAPISID", "a")]);
        assert_eq!(session.user_agent, "Agent/1.0");
        assert_eq!(session.captured_at, 42);
        assert!(!session.expired);
    }

    #[test]
    fn a_new_session_needs_a_sapisid_cookie_that_has_not_expired() {
        let expired = Cookie {
            expires: Some(10),
            ..cookie("SAPISID", "a")
        };

        for cookies in [vec![cookie("PREF", "x")], vec![expired]] {
            assert_eq!(
                Session::new(cookies, "Agent/1.0", CaptureMethod::Import, 42),
                Err(SessionError::NotSignedIn)
            );
        }
    }

    #[test]
    fn a_new_session_rejects_an_unusable_user_agent() {
        for agent in ["", "  ", "Agent\r\nX-Evil: 1", &"a".repeat(513)] {
            assert_eq!(
                Session::new(
                    vec![cookie("SAPISID", "a")],
                    agent,
                    CaptureMethod::Import,
                    42
                ),
                Err(SessionError::BadUserAgent),
                "agent: {agent:?}"
            );
        }
    }

    #[test]
    fn status_describes_the_session_without_cookie_values() {
        let status = session().status();

        assert_eq!(
            status,
            SessionStatus {
                method: CaptureMethod::LoginWindow,
                captured_at: 1_000,
                cookie_count: 2,
                has_login_info: true,
                expired: false,
                user_agent: "FakeAgent/1.0".into(),
            }
        );
        assert!(!serde_json::to_string(&status).unwrap().contains("fake-"));
    }

    #[test]
    fn debug_output_never_shows_cookie_values() {
        assert!(!format!("{:?}", session()).contains("fake-"));
    }

    #[test]
    fn a_login_is_complete_with_both_login_info_and_a_sapisid() {
        let sapisid = cookie("__Secure-3PAPISID", "a");
        let login = cookie("LOGIN_INFO", "b");

        assert!(is_signed_in(&[sapisid.clone(), login.clone()], 0));
        assert!(!is_signed_in(&[sapisid], 0));
        assert!(!is_signed_in(&[login], 0));
        assert!(!is_signed_in(&[], 0));
    }

    #[test]
    fn loads_nothing_from_an_empty_store() {
        let store = MemoryStore::default();

        assert_eq!(SessionVault::new(&store).load(), Ok(None));
    }

    #[test]
    fn round_trips_a_session_larger_than_one_credential() {
        let store = MemoryStore::default();
        let vault = SessionVault::new(&store);

        vault.save(&large_session()).unwrap();

        assert_eq!(vault.load(), Ok(Some(large_session())));
        assert!(keys(&store).len() > 3, "keys: {:?}", keys(&store));
    }

    #[test]
    fn saving_a_smaller_session_leaves_no_old_chunks() {
        let store = MemoryStore::default();
        let vault = SessionVault::new(&store);
        vault.save(&large_session()).unwrap();

        vault.save(&session()).unwrap();

        assert_eq!(keys(&store), ["session-chunk-0", "session-manifest"]);
        assert_eq!(vault.load(), Ok(Some(session())));
    }

    #[test]
    fn clear_removes_every_entry() {
        let store = MemoryStore::default();
        let vault = SessionVault::new(&store);
        vault.save(&large_session()).unwrap();

        vault.clear().unwrap();
        vault.clear().unwrap();

        assert_eq!(keys(&store), Vec::<String>::new());
    }

    #[test]
    fn refuses_a_session_that_needs_too_many_entries() {
        let store = MemoryStore::default();
        let mut huge = session();
        huge.cookies
            .push(cookie("BIG", &"v".repeat(MAX_SECRET_BYTES * 16)));

        assert_eq!(
            SessionVault::new(&store).save(&huge),
            Err(SessionError::TooLarge)
        );
        assert_eq!(keys(&store), Vec::<String>::new());
    }

    #[test]
    fn a_failed_save_leaves_no_half_written_session() {
        let store = MemoryStore::default();
        let vault = SessionVault::new(&store);
        vault.save(&session()).unwrap();
        *store.fail_set_of.borrow_mut() = Some("session-chunk-1".into());

        assert!(vault.save(&large_session()).is_err());

        assert_eq!(vault.load(), Ok(None));
        assert_eq!(keys(&store), Vec::<String>::new());
    }

    #[test]
    fn reports_a_missing_chunk_or_unreadable_content_as_corrupt() {
        let store = MemoryStore::default();
        let vault = SessionVault::new(&store);
        vault.save(&large_session()).unwrap();
        store.entries.borrow_mut().remove("session-chunk-1");
        assert_eq!(vault.load(), Err(SessionError::Corrupt));

        vault.save(&session()).unwrap();
        store
            .entries
            .borrow_mut()
            .insert("session-chunk-0".into(), b"not json".to_vec());
        assert_eq!(vault.load(), Err(SessionError::Corrupt));

        store
            .entries
            .borrow_mut()
            .insert("session-manifest".into(), b"999".to_vec());
        assert_eq!(vault.load(), Err(SessionError::Corrupt));
    }

    /// Writes fake data to the real Credential Manager under a self-test name, then removes it.
    /// Run by hand: `cargo test -- --ignored`.
    #[test]
    #[ignore = "touches the real Windows Credential Manager"]
    fn real_credential_manager_round_trip() {
        let vault = SessionVault::new(KeyringStore::new("lumen-spike-ytm-selftest"));
        vault.clear().unwrap();

        vault.save(&large_session()).unwrap();
        let loaded = vault.load();
        vault.clear().unwrap();

        assert_eq!(loaded, Ok(Some(large_session())));
        assert_eq!(vault.load(), Ok(None));
    }
}
