//! Secrets in Windows Credential Manager. A secret larger than one credential
//! is split into numbered chunks plus a manifest.

use std::collections::HashMap;

use keyring_core::api::CredentialStoreApi;
use keyring_core::{Entry, Error};
use windows_native_keyring_store::Store;

/// Windows rejects a generic credential blob above this size
/// (`CRED_MAX_CREDENTIAL_BLOB_SIZE`).
const MAX_ENTRY_BYTES: usize = 2560;
/// Upper bound on the chunks one secret may use: 40 KiB.
const MAX_CHUNKS: usize = 16;
const MANIFEST_KEY: &str = "manifest";

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SecretError {
    #[error("Windows Credential Manager failed")]
    Store,
    #[error("the secret is too large to store")]
    TooLarge,
}

/// A secret key/value store: Credential Manager in the app, memory in tests.
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError>;
    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError>;
    /// Deleting a missing key succeeds.
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// Windows Credential Manager, through the `keyring` project's Windows store.
pub struct WindowsStore {
    service: &'static str,
}

impl WindowsStore {
    pub fn new(service: &'static str) -> Self {
        Self { service }
    }

    /// Entries persist on this machine only. The default, `Enterprise`, can
    /// roam with a domain profile.
    fn entry(&self, key: &str) -> Result<Entry, SecretError> {
        let local = HashMap::from([("persistence", "Local")]);
        Store::new()
            .and_then(|store| store.build(self.service, key, Some(&local)))
            .map_err(|_| SecretError::Store)
    }
}

/// The store's own errors can carry stored bytes, so they are dropped.
impl SecretStore for WindowsStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError> {
        match self.entry(key)?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(Error::NoEntry) => Ok(None),
            Err(_) => Err(SecretError::Store),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
        self.entry(key)?
            .set_secret(value)
            .map_err(|_| SecretError::Store)
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => Err(SecretError::Store),
        }
    }
}

/// One secret of any reasonable size in a [`SecretStore`].
pub struct ChunkedSecret<S> {
    store: S,
}

impl<S: SecretStore> ChunkedSecret<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn save(&self, secret: &[u8]) -> Result<(), SecretError> {
        let chunks = secret.chunks(MAX_ENTRY_BYTES);
        let count = chunks.len();
        if count > MAX_CHUNKS {
            return Err(SecretError::TooLarge);
        }
        // Old entries go first, the manifest comes back last: a half written
        // secret is never read, and never mixed with the old one.
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

    /// Sweeps every possible chunk, so nothing survives an earlier longer or
    /// interrupted save.
    pub fn clear(&self) -> Result<(), SecretError> {
        self.store.delete(MANIFEST_KEY)?;
        (0..MAX_CHUNKS).try_for_each(|index| self.store.delete(&chunk_key(index)))
    }

    pub fn load(&self) -> Result<Option<Vec<u8>>, SecretError> {
        let Some(manifest) = self.store.get(MANIFEST_KEY)? else {
            return Ok(None);
        };
        let Some(count) = std::str::from_utf8(&manifest)
            .ok()
            .and_then(|text| text.parse::<usize>().ok())
        else {
            return Ok(None);
        };
        let mut secret = Vec::new();
        for index in 0..count {
            match self.store.get(&chunk_key(index))? {
                Some(chunk) => secret.extend(chunk),
                None => return Ok(None),
            }
        }
        Ok(Some(secret))
    }
}

fn chunk_key(index: usize) -> String {
    format!("chunk-{index}")
}

#[cfg(test)]
pub mod fake {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// An in-memory store with the Windows size limit. Clones share entries.
    #[derive(Debug, Clone, Default)]
    pub struct MemoryStore {
        pub entries: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        /// Writing this key fails, while everything else works.
        pub fail_set_of: Arc<Mutex<Option<String>>>,
        /// How many more writes and deletes succeed before the store fails, as
        /// if the process had been killed. `None` means no limit.
        pub changes_left: Arc<Mutex<Option<usize>>>,
    }

    impl MemoryStore {
        pub fn keys(&self) -> Vec<String> {
            self.entries.lock().unwrap().keys().cloned().collect()
        }

        fn change(&self) -> Result<(), SecretError> {
            match self.changes_left.lock().unwrap().as_mut() {
                Some(0) => Err(SecretError::Store),
                Some(left) => {
                    *left -= 1;
                    Ok(())
                }
                None => Ok(()),
            }
        }
    }

    impl SecretStore for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>, SecretError> {
            Ok(self.entries.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
            self.change()?;
            if self.fail_set_of.lock().unwrap().as_deref() == Some(key) {
                return Err(SecretError::Store);
            }
            if value.len() > MAX_ENTRY_BYTES {
                return Err(SecretError::Store);
            }
            self.entries
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), SecretError> {
            self.change()?;
            self.entries.lock().unwrap().remove(key);
            Ok(())
        }
    }

    /// Bytes that differ from chunk to chunk, so a mix-up shows.
    pub fn secret(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|at| seed.wrapping_add((at % 251) as u8))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::*, *};

    #[test]
    fn round_trips_a_secret_larger_than_one_credential() {
        let store = MemoryStore::default();
        let vault = ChunkedSecret::new(store.clone());
        let large = secret(3 * MAX_ENTRY_BYTES + 7, 1);

        vault.save(&large).unwrap();

        assert!(vault.load() == Ok(Some(large)), "the secret changed");
        assert_eq!(
            store.keys(),
            ["chunk-0", "chunk-1", "chunk-2", "chunk-3", "manifest"]
        );
    }

    /// A save killed after any number of writes must read as the old secret,
    /// the new one, or nothing. Never as a mix.
    #[test]
    fn an_interrupted_save_reads_as_no_secret_never_as_a_mix() {
        let old = secret(3 * MAX_ENTRY_BYTES, 1);
        let new = secret(2 * MAX_ENTRY_BYTES, 101);
        for changes in 0.. {
            let store = MemoryStore::default();
            let vault = ChunkedSecret::new(store.clone());
            vault.save(&old).unwrap();
            *store.changes_left.lock().unwrap() = Some(changes);

            let saved = vault.save(&new);

            let loaded = vault.load().unwrap();
            if saved.is_ok() {
                assert!(loaded == Some(new), "changes: {changes}");
                break;
            }
            // Only a save that never changed anything may leave the old one.
            let untouched = changes == 0 && loaded.as_ref() == Some(&old);
            assert!(loaded.is_none() || untouched, "changes: {changes}");
        }
    }

    #[test]
    fn refuses_an_oversized_secret_and_keeps_the_old_one() {
        let store = MemoryStore::default();
        let vault = ChunkedSecret::new(store.clone());
        let old = secret(40, 1);
        vault.save(&old).unwrap();

        let saved = vault.save(&secret(MAX_CHUNKS * MAX_ENTRY_BYTES + 1, 101));

        assert_eq!(saved, Err(SecretError::TooLarge));
        assert!(vault.load() == Ok(Some(old)));
        assert_eq!(store.keys(), ["chunk-0", "manifest"]);
    }

    #[test]
    fn accepts_the_largest_secret() {
        let vault = ChunkedSecret::new(MemoryStore::default());
        let largest = secret(MAX_CHUNKS * MAX_ENTRY_BYTES, 1);

        vault.save(&largest).unwrap();

        assert!(vault.load() == Ok(Some(largest)));
    }

    #[test]
    fn clear_removes_every_entry() {
        let store = MemoryStore::default();
        let vault = ChunkedSecret::new(store.clone());
        // A smaller secret after a larger one must not hide the larger one's chunks.
        vault.save(&secret(5 * MAX_ENTRY_BYTES, 1)).unwrap();
        vault.save(&secret(40, 101)).unwrap();

        vault.clear().unwrap();
        vault.clear().unwrap();

        assert_eq!(store.keys(), Vec::<String>::new());
        assert_eq!(vault.load(), Ok(None));
    }

    #[test]
    fn clear_removes_what_an_interrupted_save_left() {
        let store = MemoryStore::default();
        let vault = ChunkedSecret::new(store.clone());
        // The manifest delete and two chunks, then nothing more.
        *store.changes_left.lock().unwrap() = Some(3);
        assert!(vault.save(&secret(5 * MAX_ENTRY_BYTES, 1)).is_err());
        *store.changes_left.lock().unwrap() = None;

        vault.clear().unwrap();

        assert_eq!(store.keys(), Vec::<String>::new());
    }

    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        let store = MemoryStore::default();
        let vault = ChunkedSecret::new(store.clone());
        *store.fail_set_of.lock().unwrap() = Some("chunk-1".to_owned());

        let saved = vault.save(&secret(3 * MAX_ENTRY_BYTES, 1));

        assert_eq!(saved, Err(SecretError::Store));
        assert_eq!(store.keys(), Vec::<String>::new());
    }

    /// Writes fake bytes to the real Credential Manager under a self-test name,
    /// then removes them. Run by hand: `cargo test -- --ignored`.
    #[test]
    #[ignore = "touches the real Windows Credential Manager"]
    fn real_credential_manager_round_trip() {
        const SERVICE: &str = "lumen-selftest";
        let vault = ChunkedSecret::new(WindowsStore::new(SERVICE));
        let large = secret(3 * MAX_ENTRY_BYTES + 7, 1);
        vault.clear().unwrap();

        vault.save(&large).unwrap();
        let loaded = vault.load();
        let persistence = ["chunk-0", "manifest"].map(|key| {
            let entry = WindowsStore::new(SERVICE).entry(key).unwrap();
            entry.get_attributes().unwrap()["persistence"].clone()
        });
        vault.clear().unwrap();

        assert!(loaded == Ok(Some(large)));
        assert_eq!(persistence, ["Local", "Local"]);
        assert_eq!(vault.load(), Ok(None));
    }
}
