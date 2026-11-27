//! The one stored session: what state it is in, and the only ways to change it.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;

use super::session::Session;
use crate::secrets::{ChunkedSecret, SecretError, SecretStore};

/// All the UI may know about the account.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountStatus {
    Connected,
    Expired,
    Disconnected,
}

/// One opened login window. Only the latest one may store a session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoginAttempt(u64);

/// The one YouTube Music account. Its lock keeps writes to the store whole.
pub struct Account<S> {
    inner: Mutex<Inner<S>>,
}

struct Inner<S> {
    vault: ChunkedSecret<S>,
    attempts: u64,
    /// The attempt that may still store a session.
    pending: Option<LoginAttempt>,
}

impl<S: SecretStore> Account<S> {
    pub fn new(store: S) -> Self {
        Self {
            inner: Mutex::new(Inner {
                vault: ChunkedSecret::new(store),
                attempts: 0,
                pending: None,
            }),
        }
    }

    /// A store that holds nothing readable counts as disconnected.
    pub fn status(&self) -> Result<AccountStatus, SecretError> {
        self.lock().status()
    }

    pub fn begin_login(&self) -> LoginAttempt {
        let mut inner = self.lock();
        inner.attempts += 1;
        inner.pending = Some(LoginAttempt(inner.attempts));
        LoginAttempt(inner.attempts)
    }

    /// Stores the session a login attempt captured and says whether it did. A
    /// cancelled, replaced or finished attempt stores nothing.
    ///
    /// `announce` runs under the lock, so announcements arrive in the order the
    /// changes happened.
    pub fn complete_login(
        &self,
        attempt: LoginAttempt,
        session: &Session,
        announce: impl FnOnce(AccountStatus),
    ) -> Result<bool, SecretError> {
        let mut inner = self.lock();
        if inner.pending != Some(attempt) {
            return Ok(false);
        }
        inner.pending = None;
        // The error is dropped: it could quote the content.
        let bytes = serde_json::to_vec(session).map_err(|_| SecretError::Store)?;
        let saved = inner.vault.save(&bytes);
        inner.announce(announce);
        saved.map(|()| true)
    }

    /// Removes the session and cancels a login in progress.
    pub fn disconnect(&self, announce: impl FnOnce(AccountStatus)) -> Result<(), SecretError> {
        let mut inner = self.lock();
        inner.pending = None;
        let cleared = inner.vault.clear();
        inner.announce(announce);
        cleared
    }

    /// A panic elsewhere must not lock the account for good.
    fn lock(&self) -> MutexGuard<'_, Inner<S>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<S: SecretStore> Inner<S> {
    fn status(&self) -> Result<AccountStatus, SecretError> {
        let session = self
            .vault
            .load()?
            .and_then(|bytes| serde_json::from_slice::<Session>(&bytes).ok());
        Ok(match session {
            Some(session) if session.expired => AccountStatus::Expired,
            Some(_) => AccountStatus::Connected,
            None => AccountStatus::Disconnected,
        })
    }

    /// Says what the store holds now. If it cannot be read, the failure the
    /// caller already has is the one to report.
    fn announce(&self, announce: impl FnOnce(AccountStatus)) {
        if let Ok(status) = self.status() {
            announce(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::fake::MemoryStore;
    use crate::sources::youtube::session::fake::{cookie, music_page, signed_in_cookies};

    fn session() -> Session {
        Session::capture(music_page(), signed_in_cookies()).unwrap()
    }

    /// Several kilobytes, like a real signed-in cookie set.
    fn large_session(cookies: usize) -> Session {
        let mut large = session();
        large.cookies.extend((0..cookies).map(|i| {
            let mut big = cookie(&format!("C{i}"), ".youtube.com");
            big.value = "v".repeat(500);
            big
        }));
        large
    }

    /// Signs in and returns what was announced.
    fn sign_in(account: &Account<MemoryStore>, session: &Session) -> Vec<AccountStatus> {
        let mut announced = Vec::new();
        let attempt = account.begin_login();
        account
            .complete_login(attempt, session, |status| announced.push(status))
            .unwrap();
        announced
    }

    #[test]
    fn starts_disconnected() {
        let account = Account::new(MemoryStore::default());

        assert_eq!(account.status(), Ok(AccountStatus::Disconnected));
    }

    #[test]
    fn a_completed_login_is_connected_and_stays_so_after_a_restart() {
        let store = MemoryStore::default();

        let announced = sign_in(&Account::new(store.clone()), &large_session(12));

        assert_eq!(announced, [AccountStatus::Connected]);
        assert!(store.keys().len() > 3, "{:?}", store.keys());
        assert_eq!(Account::new(store).status(), Ok(AccountStatus::Connected));
    }

    #[test]
    fn a_session_marked_expired_reads_as_expired() {
        let account = Account::new(MemoryStore::default());
        let expired = Session {
            expired: true,
            ..session()
        };

        sign_in(&account, &expired);

        assert_eq!(account.status(), Ok(AccountStatus::Expired));
    }

    #[test]
    fn an_interrupted_save_reads_as_disconnected() {
        let store = MemoryStore::default();
        let account = Account::new(store.clone());
        sign_in(&account, &session());
        // The old entries are cleared (17 changes), two chunks land, then nothing.
        *store.changes_left.lock().unwrap() = Some(19);
        let attempt = account.begin_login();
        let mut announced = Vec::new();

        let saved = account.complete_login(attempt, &large_session(12), |s| announced.push(s));

        assert_eq!(saved, Err(SecretError::Store));
        assert!(store.keys().contains(&"chunk-1".to_owned()));
        assert_eq!(announced, [AccountStatus::Disconnected]);
        assert_eq!(
            Account::new(store).status(),
            Ok(AccountStatus::Disconnected)
        );
    }

    #[test]
    fn an_oversized_session_is_refused_and_the_old_one_kept() {
        let account = Account::new(MemoryStore::default());
        sign_in(&account, &session());
        let attempt = account.begin_login();
        let mut announced = Vec::new();

        let saved = account.complete_login(attempt, &large_session(100), |s| announced.push(s));

        assert_eq!(saved, Err(SecretError::TooLarge));
        assert_eq!(announced, [AccountStatus::Connected]);
        assert_eq!(account.status(), Ok(AccountStatus::Connected));
    }

    #[test]
    fn unreadable_content_reads_as_disconnected() {
        let store = MemoryStore::default();
        let account = Account::new(store.clone());
        sign_in(&account, &session());
        store
            .entries
            .lock()
            .unwrap()
            .insert("chunk-0".to_owned(), b"not a session".to_vec());

        assert_eq!(account.status(), Ok(AccountStatus::Disconnected));
    }

    #[test]
    fn disconnect_removes_every_entry() {
        let store = MemoryStore::default();
        let account = Account::new(store.clone());
        sign_in(&account, &large_session(12));
        let mut announced = Vec::new();

        account.disconnect(|status| announced.push(status)).unwrap();

        assert_eq!(announced, [AccountStatus::Disconnected]);
        assert_eq!(store.keys(), Vec::<String>::new());
    }

    #[test]
    fn disconnect_cancels_a_login_in_progress() {
        let store = MemoryStore::default();
        let account = Account::new(store.clone());
        let attempt = account.begin_login();
        account.disconnect(|_| {}).unwrap();

        let saved = account.complete_login(attempt, &session(), |_| panic!("announced"));

        assert_eq!(saved, Ok(false));
        assert_eq!(store.keys(), Vec::<String>::new());
    }

    #[test]
    fn an_attempt_stores_one_session_only() {
        let account = Account::new(MemoryStore::default());
        let attempt = account.begin_login();
        assert_eq!(
            account.complete_login(attempt, &session(), |_| {}),
            Ok(true)
        );

        let again = account.complete_login(attempt, &session(), |_| panic!("announced"));

        assert_eq!(again, Ok(false));
    }

    #[test]
    fn a_replaced_attempt_stores_nothing() {
        let store = MemoryStore::default();
        let account = Account::new(store.clone());
        let old = account.begin_login();
        let _new = account.begin_login();

        let saved = account.complete_login(old, &session(), |_| panic!("announced"));

        assert_eq!(saved, Ok(false));
        assert_eq!(store.keys(), Vec::<String>::new());
    }

    #[test]
    fn a_login_that_never_completes_keeps_the_old_session() {
        let store = MemoryStore::default();
        let account = Account::new(store.clone());
        sign_in(&account, &session());
        let before = store.entries.lock().unwrap().clone();

        let _closed_before_sign_in = account.begin_login();

        assert!(*store.entries.lock().unwrap() == before);
        assert_eq!(account.status(), Ok(AccountStatus::Connected));
    }

    #[test]
    fn the_ui_receives_one_word() {
        let words = [
            AccountStatus::Connected,
            AccountStatus::Expired,
            AccountStatus::Disconnected,
        ]
        .map(|status| serde_json::to_string(&status).unwrap());

        assert_eq!(
            words,
            [r#""connected""#, r#""expired""#, r#""disconnected""#]
        );
    }
}
