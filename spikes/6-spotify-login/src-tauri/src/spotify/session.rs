//! The one saved login, kept only in Windows Credential Manager.

use keyring::Entry;
use serde::{Deserialize, Serialize};

use super::{error::SpotifyError, secret::Secret};

/// What the WebView2 build that logged in says about itself. Every request
/// sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct Browser {
    pub user_agent: String,
    /// The `Accept-Language` header value.
    pub accept_language: String,
}

impl Browser {
    /// From `navigator.userAgent` and `navigator.languages`.
    pub fn new(user_agent: &str, languages: &[String]) -> Result<Self, SpotifyError> {
        validate_user_agent(user_agent)?;
        Ok(Self {
            user_agent: user_agent.to_owned(),
            accept_language: accept_language(languages),
        })
    }
}

/// The header Chromium builds from its language list.
fn accept_language(languages: &[String]) -> String {
    let is_tag = |tag: &&String| {
        (1..=35).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let weighted: Vec<String> = languages
        .iter()
        .filter(is_tag)
        .take(10)
        .enumerate()
        .map(|(index, tag)| match index {
            0 => tag.clone(),
            _ => format!("{tag};q=0.{}", 10usize.saturating_sub(index).max(1)),
        })
        .collect();
    if weighted.is_empty() {
        "en".to_owned()
    } else {
        weighted.join(",")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub sp_dc: Secret,
    pub browser: Browser,
    /// Random id for the client token, stable for the life of the session.
    pub device_id: String,
    /// Set after a 401, 403 or a rejected cookie. Blocks all requests.
    pub expired: bool,
}

impl Session {
    /// Validates a captured or pasted cookie. Errors never repeat the input.
    pub fn new(sp_dc: &str, browser: Browser) -> Result<Self, SpotifyError> {
        let value = sp_dc.trim();
        let value = value.strip_prefix("sp_dc=").unwrap_or(value);
        let value = value.strip_suffix(';').unwrap_or(value);
        let one_cookie_value = (10..=1024).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_graphic() && !b";,\"\\".contains(&b));
        if !one_cookie_value {
            return Err(SpotifyError::InvalidInput(
                "that is not an sp_dc cookie value",
            ));
        }
        Ok(Self {
            sp_dc: Secret::new(value),
            browser,
            device_id: random_device_id()?,
            expired: false,
        })
    }
}

/// 32 random hex digits.
pub fn random_device_id() -> Result<String, SpotifyError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SpotifyError::InvalidInput("no system randomness"))?;
    Ok(format!("{:032x}", u128::from_be_bytes(bytes)))
}

fn validate_user_agent(user_agent: &str) -> Result<(), SpotifyError> {
    let usable = user_agent.starts_with("Mozilla/")
        && user_agent.len() <= 512
        && user_agent.bytes().all(|b| (b' '..=b'~').contains(&b));
    usable
        .then_some(())
        .ok_or(SpotifyError::InvalidInput("unusable user agent"))
}

/// The JSON kept in Credential Manager.
#[derive(Serialize, Deserialize)]
struct Stored {
    sp_dc: String,
    user_agent: String,
    accept_language: String,
    device_id: String,
    expired: bool,
}

fn encode(session: &Session) -> Result<Vec<u8>, SpotifyError> {
    serde_json::to_vec(&Stored {
        sp_dc: session.sp_dc.expose().to_owned(),
        user_agent: session.browser.user_agent.clone(),
        accept_language: session.browser.accept_language.clone(),
        device_id: session.device_id.clone(),
        expired: session.expired,
    })
    .map_err(|_| store_error("could not encode the session"))
}

fn decode(bytes: &[u8]) -> Result<Session, SpotifyError> {
    // The serde error is dropped on purpose: it can quote the secret.
    let stored: Stored = serde_json::from_slice(bytes)
        .map_err(|_| store_error("the saved session is unreadable, clear it and log in again"))?;
    Ok(Session {
        sp_dc: Secret::new(stored.sp_dc),
        browser: Browser {
            user_agent: stored.user_agent,
            accept_language: stored.accept_language,
        },
        device_id: stored.device_id,
        expired: stored.expired,
    })
}

/// Keyring errors are replaced by fixed text: some of them carry the stored bytes.
fn store_error(what: &str) -> SpotifyError {
    SpotifyError::Store(what.to_owned())
}

pub trait SessionStore: Send + Sync {
    fn load(&self) -> Result<Option<Session>, SpotifyError>;
    fn save(&self, session: &Session) -> Result<(), SpotifyError>;
    /// Succeeds when there was nothing to delete.
    fn clear(&self) -> Result<(), SpotifyError>;
}

/// One generic credential in Windows Credential Manager.
pub struct KeyringStore {
    service: &'static str,
    account: &'static str,
}

impl KeyringStore {
    pub const fn new(service: &'static str, account: &'static str) -> Self {
        Self { service, account }
    }

    fn entry(&self) -> Result<Entry, SpotifyError> {
        Entry::new(self.service, self.account)
            .map_err(|_| store_error("the credential store is unavailable"))
    }
}

impl SessionStore for KeyringStore {
    fn load(&self) -> Result<Option<Session>, SpotifyError> {
        match self.entry()?.get_secret() {
            Ok(bytes) => decode(&bytes).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(store_error("could not read the saved session")),
        }
    }

    fn save(&self, session: &Session) -> Result<(), SpotifyError> {
        self.entry()?
            .set_secret(&encode(session)?)
            .map_err(|_| store_error("could not save the session"))
    }

    fn clear(&self) -> Result<(), SpotifyError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(store_error("could not delete the saved session")),
        }
    }
}

/// Saves a fresh login, which replaces any earlier session.
pub fn save_login(
    store: &dyn SessionStore,
    sp_dc: &str,
    browser: Browser,
) -> Result<(), SpotifyError> {
    let mut session = Session::new(sp_dc, browser)?;
    // An unreadable old session must not block a new login.
    if let Ok(Some(old)) = store.load() {
        session.device_id = old.device_id;
    }
    store.save(&session)
}

/// In-memory stand-in for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<Option<Session>>);

#[cfg(test)]
impl SessionStore for MemoryStore {
    fn load(&self) -> Result<Option<Session>, SpotifyError> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, session: &Session) -> Result<(), SpotifyError> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }

    fn clear(&self) -> Result<(), SpotifyError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/158.0.0.0";

    fn browser() -> Browser {
        Browser::new(UA, &["da-DK".to_owned(), "da".to_owned()]).unwrap()
    }

    #[test]
    fn builds_accept_language_like_chromium() {
        let header = |tags: &[&str]| {
            let tags: Vec<String> = tags.iter().map(|t| (*t).to_owned()).collect();
            accept_language(&tags)
        };
        assert_eq!(
            header(&["da-DK", "da", "en-US", "en"]),
            "da-DK,da;q=0.9,en-US;q=0.8,en;q=0.7"
        );
        assert_eq!(header(&[]), "en");
        // Anything that is not a language tag never reaches a header.
        assert_eq!(header(&["", "en\r\nCookie: x", "da", "a;q=1"]), "da");
        let many = header(&["en"; 40]);
        assert_eq!(many.split(',').count(), 10);
        assert!(many.ends_with("en;q=0.1"), "{many}");
    }

    #[test]
    fn accepts_a_bare_or_prefixed_cookie() {
        for input in ["FAKE-cookie_value.123", "  sp_dc=FAKE-cookie_value.123;\n"] {
            let session = Session::new(input, browser()).unwrap();
            assert_eq!(session.sp_dc.expose(), "FAKE-cookie_value.123");
            assert_eq!(session.browser, browser());
            assert!(!session.expired);
        }
    }

    #[test]
    fn rejects_input_that_is_not_one_cookie_value() {
        let long = "a".repeat(1025);
        for input in [
            "",
            "short",
            "FAKE-cookie; sp_key=other",
            "FAKE cookie value",
            "FAKE-cookie\r\nHost: evil",
            "FAKE-cookie-\u{e6}\u{f8}",
            &long,
        ] {
            let error = Session::new(input, browser()).unwrap_err();
            assert_eq!(
                error,
                SpotifyError::InvalidInput("that is not an sp_dc cookie value"),
                "{input:?}"
            );
        }
    }

    #[test]
    fn rejects_a_user_agent_that_cannot_be_a_header() {
        for agent in ["", "curl/8", "Mozilla/5.0\r\nCookie: x"] {
            assert!(Browser::new(agent, &[]).is_err(), "{agent:?}");
        }
    }

    #[test]
    fn device_ids_are_random_hex() {
        let first = random_device_id().unwrap();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, random_device_id().unwrap());
    }

    #[test]
    fn a_new_login_replaces_the_session_but_keeps_the_device_id() {
        let store = MemoryStore::default();
        save_login(&store, "FAKE-first-cookie", browser()).unwrap();
        let mut first = store.load().unwrap().unwrap();
        first.expired = true;
        store.save(&first).unwrap();

        save_login(&store, "FAKE-second-cookie", browser()).unwrap();
        let second = store.load().unwrap().unwrap();
        assert_eq!(second.sp_dc.expose(), "FAKE-second-cookie");
        assert_eq!(second.device_id, first.device_id);
        assert!(!second.expired);
    }

    #[test]
    fn a_bad_login_leaves_the_saved_session_alone() {
        let store = MemoryStore::default();
        save_login(&store, "FAKE-first-cookie", browser()).unwrap();
        let before = store.load().unwrap();
        assert!(save_login(&store, "bad cookie", browser()).is_err());
        assert_eq!(store.load().unwrap(), before);
    }

    #[test]
    fn debug_output_hides_the_cookie() {
        let session = Session::new("FAKE-cookie_value", browser()).unwrap();
        assert!(!format!("{session:?}").contains("FAKE"));
    }

    #[test]
    fn survives_encoding_for_the_credential_store() {
        let mut session = Session::new("FAKE-cookie_value", browser()).unwrap();
        session.expired = true;
        assert_eq!(decode(&encode(&session).unwrap()), Ok(session));
        let error = decode(b"FAKE-cookie_value").unwrap_err();
        assert!(!error.to_string().contains("FAKE"));
    }

    /// Touches the real Credential Manager under a test-only entry.
    /// Run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "writes to Windows Credential Manager"]
    fn round_trips_through_credential_manager() {
        let store = KeyringStore::new("dev.lumen.spike.spotify-login.test", "round-trip");
        store.clear().unwrap();
        assert_eq!(store.load(), Ok(None));
        // Long enough to cover a real cookie plus user agent.
        let session = Session::new(&"F".repeat(400), browser()).unwrap();
        store.save(&session).unwrap();
        let loaded = store.load();
        store.clear().unwrap();
        assert_eq!(loaded, Ok(Some(session)));
        assert_eq!(store.load(), Ok(None));
        store.clear().unwrap();
    }
}
