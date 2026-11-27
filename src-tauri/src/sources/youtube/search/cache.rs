//! What recent searches found, so a repeat sends nothing. It holds Tracks only,
//! never a reply.

use std::time::Duration;

use super::parse::Parsed;
use crate::sources::youtube::account::Generation;

/// How many queries are kept.
const CAPACITY: usize = 50;
/// How long an entry counts, from when it was stored.
const LIFETIME: Duration = Duration::from_secs(3_600);

/// The form a query is remembered under.
pub fn key(query: &str) -> String {
    query.trim().to_lowercase()
}

#[derive(Debug)]
struct Entry {
    key: String,
    found: Parsed,
    stored: Duration,
}

/// The entries of one account generation.
#[derive(Debug, Default)]
pub struct Cache {
    generation: Generation,
    /// The most recently used is last.
    entries: Vec<Entry>,
}

impl Cache {
    /// Empties the cache if the account has changed.
    pub fn follow(&mut self, generation: Generation) {
        if generation != self.generation {
            self.generation = generation;
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// What `query` found less than an hour before `now`.
    pub fn get(&mut self, query: &str, now: Duration) -> Option<Parsed> {
        self.entries
            .retain(|entry| now.saturating_sub(entry.stored) < LIFETIME);
        let key = key(query);
        let at = self.entries.iter().position(|entry| entry.key == key)?;
        let entry = self.entries.remove(at);
        let found = entry.found.clone();
        self.entries.push(entry);
        Some(found)
    }

    /// Keeps what a session of `generation` found. A reply for a replaced
    /// session is dropped.
    pub fn put(&mut self, generation: Generation, query: &str, found: Parsed, now: Duration) {
        if generation != self.generation {
            return;
        }
        let key = key(query);
        self.entries.retain(|entry| entry.key != key);
        if self.entries.len() >= CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push(Entry {
            key,
            found,
            stored: now,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::fake::MemoryStore;
    use crate::sources::youtube::account::Account;

    const HOUR: Duration = Duration::from_secs(3_600);

    /// Two generations of one account.
    fn generations() -> (Generation, Generation) {
        let account = Account::new(MemoryStore::default());
        let first = account.generation();
        account.disconnect(|_| {}).unwrap();
        (first, account.generation())
    }

    fn cache() -> Cache {
        let mut cache = Cache::default();
        cache.follow(generations().0);
        cache
    }

    fn put(cache: &mut Cache, query: &str, now: Duration) {
        cache.put(generations().0, query, Parsed::NoResults, now);
    }

    #[test]
    fn a_query_is_found_again_whatever_its_case_and_outer_spaces() {
        let mut cache = cache();

        put(&mut cache, "  Paper Lanterns ", Duration::ZERO);

        assert_eq!(
            cache.get("paper LANTERNS", Duration::ZERO),
            Some(Parsed::NoResults)
        );
        assert_eq!(cache.get("paper  lanterns", Duration::ZERO), None);
    }

    #[test]
    fn an_entry_lasts_an_hour_from_when_it_was_stored() {
        let mut cache = cache();
        put(&mut cache, "salt", Duration::ZERO);

        assert!(cache.get("salt", HOUR - Duration::from_millis(1)).is_some());
        assert_eq!(cache.get("salt", HOUR), None);
    }

    #[test]
    fn the_fifty_first_query_drops_the_least_recently_used() {
        let mut cache = cache();
        for n in 0..50 {
            put(&mut cache, &n.to_string(), Duration::ZERO);
        }
        // Using the oldest saves it.
        assert!(cache.get("0", Duration::ZERO).is_some());

        put(&mut cache, "50", Duration::ZERO);

        assert_eq!(cache.get("1", Duration::ZERO), None);
        for kept in ["0", "2", "49", "50"] {
            assert!(cache.get(kept, Duration::ZERO).is_some(), "{kept}");
        }
    }

    #[test]
    fn storing_a_query_again_takes_no_second_place() {
        let mut cache = cache();
        for n in 0..50 {
            put(&mut cache, &n.to_string(), Duration::ZERO);
        }

        put(&mut cache, "49", Duration::ZERO);

        assert!(cache.get("0", Duration::ZERO).is_some());
    }

    #[test]
    fn another_account_generation_empties_it() {
        let (first, second) = generations();
        let mut cache = cache();
        put(&mut cache, "salt", Duration::ZERO);
        cache.follow(first);
        assert!(cache.get("salt", Duration::ZERO).is_some());

        cache.follow(second);

        assert_eq!(cache.get("salt", Duration::ZERO), None);
        // A late reply for the old session is not kept.
        cache.put(first, "salt", Parsed::NoResults, Duration::ZERO);
        assert_eq!(cache.get("salt", Duration::ZERO), None);
    }

    #[test]
    fn it_can_be_cleared() {
        let mut cache = cache();
        put(&mut cache, "salt", Duration::ZERO);

        cache.clear();

        assert_eq!(cache.get("salt", Duration::ZERO), None);
    }
}
