//! The stored SoundCloud session. Secrets live in Windows Credential Manager only.

use serde::{Deserialize, Serialize};

use crate::{error::Error, http::Identity, secret::Secret};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub oauth_token: Secret,
    pub client_id: Option<Secret>,
    pub identity: Identity,
    /// Set after a 401/403 that a fresh `client_id` did not fix.
    pub expired: bool,
}

/// What the webview may see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionStatus {
    pub logged_in: bool,
    pub expired: bool,
    pub oauth_token: String,
    pub client_id: String,
    pub user_agent: Option<String>,
}

pub trait SessionStore: Send + Sync {
    fn load(&self) -> Result<Option<Session>, Error>;
    fn save(&self, session: &Session) -> Result<(), Error>;
    fn clear(&self) -> Result<(), Error>;
}

/// Accepts a bare `oauth_token` value, or one copied from an `Authorization` header.
/// The character set is narrow so a pasted value cannot add headers or cookies.
pub fn parse_token(input: &str) -> Result<Secret, Error> {
    let value = input.trim();
    let value = value
        .strip_prefix("OAuth")
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        .map_or(value, str::trim_start);
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-_.".contains(c);
    if value.is_empty() || value.len() > 1024 || !value.chars().all(allowed) {
        return Err(Error::InvalidToken);
    }
    Ok(Secret::new(value))
}

pub fn status(session: Option<&Session>) -> SessionStatus {
    let show = |secret: Option<&Secret>| secret.map_or_else(|| "absent".into(), Secret::masked);
    SessionStatus {
        logged_in: session.is_some_and(|s| !s.expired),
        expired: session.is_some_and(|s| s.expired),
        oauth_token: show(session.map(|s| &s.oauth_token)),
        client_id: show(session.and_then(|s| s.client_id.as_ref())),
        user_agent: session.map(|s| s.identity.user_agent.clone()),
    }
}

/// Stores a new login. A cached `client_id` is public and stays valid, so it is kept.
pub fn save_login(
    store: &impl SessionStore,
    token: Secret,
    identity: Identity,
) -> Result<(), Error> {
    // An unreadable old entry must not block signing in again.
    let client_id = store.load().ok().flatten().and_then(|s| s.client_id);
    store.save(&Session {
        oauth_token: token,
        client_id,
        identity,
        expired: false,
    })
}

/// Session entry in Windows Credential Manager.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    // keyring errors can carry the stored bytes, so their text is never passed on.
    fn entry(&self) -> Result<keyring::Entry, Error> {
        keyring::Entry::new(&self.service, "session")
            .map_err(|_| Error::Store("Credential Manager is unavailable"))
    }
}

impl SessionStore for KeyringStore {
    fn load(&self) -> Result<Option<Session>, Error> {
        match self.entry()?.get_secret() {
            Ok(stored) => decode(&stored).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(Error::Store("could not read the session")),
        }
    }

    fn save(&self, session: &Session) -> Result<(), Error> {
        // Bytes, not a password: Windows stores a password as UTF-16, which halves the
        // 2560 byte room of a credential.
        self.entry()?
            .set_secret(encode(session)?.as_bytes())
            .map_err(|_| Error::Store("could not save the session"))
    }

    fn clear(&self) -> Result<(), Error> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(Error::Store("could not delete the session")),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    oauth_token: String,
    client_id: Option<String>,
    user_agent: String,
    language: Option<String>,
    expired: bool,
}

fn encode(session: &Session) -> Result<String, Error> {
    serde_json::to_string(&Stored {
        oauth_token: session.oauth_token.expose().to_string(),
        client_id: session.client_id.as_ref().map(|id| id.expose().to_string()),
        user_agent: session.identity.user_agent.clone(),
        language: session.identity.language.clone(),
        expired: session.expired,
    })
    .map_err(|_| Error::Store("could not encode the session"))
}

fn decode(stored: &[u8]) -> Result<Session, Error> {
    // serde errors can quote the input, so they are replaced.
    let stored: Stored = serde_json::from_slice(stored)
        .map_err(|_| Error::Store("the stored session is unreadable"))?;
    Ok(Session {
        oauth_token: Secret::new(stored.oauth_token),
        client_id: stored.client_id.map(Secret::new),
        identity: Identity {
            user_agent: stored.user_agent,
            language: stored.language,
        },
        expired: stored.expired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{CLIENT_ID, MemoryStore, TOKEN, identity, session};

    #[test]
    fn accepts_a_token_value_and_strips_the_header_scheme() {
        for input in [TOKEN.to_string(), format!("  OAuth {TOKEN}\r\n")] {
            assert_eq!(parse_token(&input), Ok(Secret::new(TOKEN)));
        }
    }

    #[test]
    fn rejects_values_that_could_not_be_a_token() {
        let too_long = "a".repeat(1025);
        for input in [
            "",
            "   ",
            "OAuth ",
            "two words",
            "abc\r\nX-Injected: 1",
            "abc;oauth=1",
            "\"quoted\"",
            "t\u{00f8}ken",
            &too_long,
        ] {
            assert_eq!(parse_token(input), Err(Error::InvalidToken), "{input:?}");
        }
    }

    #[test]
    fn status_masks_the_token_and_client_id() {
        let status = status(Some(&session(Some(CLIENT_ID))));
        assert!(status.logged_in);
        assert!(!status.expired);
        assert_eq!(status.oauth_token, format!("present (len={})", TOKEN.len()));
        assert_eq!(status.client_id, "present (len=32)");
        assert_eq!(status.user_agent.as_deref(), Some("TestAgent/1.0"));

        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains(TOKEN) && !json.contains(CLIENT_ID));
    }

    #[test]
    fn status_without_a_session_reports_absent() {
        let status = status(None);
        assert!(!status.logged_in);
        assert_eq!(status.oauth_token, "absent");
        assert_eq!(status.client_id, "absent");
        assert_eq!(status.user_agent, None);
    }

    #[test]
    fn debug_output_of_a_session_has_no_secrets() {
        let debug = format!("{:?}", session(Some(CLIENT_ID)));
        assert!(!debug.contains(TOKEN) && !debug.contains(CLIENT_ID));
    }

    #[test]
    fn a_new_login_keeps_the_cached_client_id_and_clears_expiry() {
        let mut old = session(Some(CLIENT_ID));
        old.expired = true;
        let store = MemoryStore::with(old);

        save_login(&store, Secret::new("2-new-token"), identity()).unwrap();

        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.oauth_token, Secret::new("2-new-token"));
        assert_eq!(saved.client_id, Some(Secret::new(CLIENT_ID)));
        assert!(!saved.expired);
    }

    #[test]
    fn the_stored_form_round_trips() {
        let mut original = session(Some(CLIENT_ID));
        original.expired = true;
        assert_eq!(decode(encode(&original).unwrap().as_bytes()), Ok(original));
    }

    #[test]
    fn unreadable_stored_data_is_not_echoed() {
        let stored = format!(r#"{{"oauth_token":"{TOKEN}","expired":"{TOKEN}"}}"#);
        let error = decode(stored.as_bytes()).unwrap_err();
        assert!(!format!("{error} {error:?}").contains(TOKEN));
    }

    // A web token is short today; a signed (JWT style) one would be much longer.
    #[cfg(windows)]
    #[test]
    fn credential_manager_holds_the_longest_accepted_token() {
        let service = format!("dev.lumen.spike.soundcloud.long-{}", std::process::id());
        let store = KeyringStore::new(&service);
        let mut original = session(Some(CLIENT_ID));
        original.oauth_token = parse_token(&"a".repeat(1024)).unwrap();
        original.identity.user_agent = "u".repeat(256);

        let saved = store.save(&original);
        let loaded = store.load();
        store.clear().unwrap();

        assert_eq!(saved, Ok(()));
        assert_eq!(loaded, Ok(Some(original)));
    }

    // Uses the real Credential Manager under a throwaway name with fake values. This
    // catches a keyring build that silently keeps secrets in memory.
    #[cfg(windows)]
    #[test]
    fn credential_manager_saves_loads_and_clears() {
        let service = format!("dev.lumen.spike.soundcloud.test-{}", std::process::id());
        let writer = KeyringStore::new(&service);
        let original = session(Some(CLIENT_ID));

        writer.save(&original).unwrap();
        let loaded = KeyringStore::new(&service).load();
        let cleared = writer.clear();

        assert_eq!(loaded, Ok(Some(original)));
        assert_eq!(cleared, Ok(()));
        assert_eq!(writer.load(), Ok(None));
        assert_eq!(writer.clear(), Ok(()), "clearing twice is fine");
    }
}
