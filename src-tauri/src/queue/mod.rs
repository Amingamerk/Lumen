//! The play order. Pure logic: no clock, no Tauri, no catalog.

use uuid::Uuid;

/// One place in the Queue. The same Track queued twice has two entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Unique per insertion and never reused, so the UI can name one entry.
    pub key: u32,
    pub track: Uuid,
    /// Whether the Recommender picked it, rather than the user.
    pub autoplay: bool,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    #[error("there is no track at position {index} in a list of {len}")]
    OutOfRange { index: usize, len: usize },
    #[error("no queue entry has the key {key}")]
    UnknownEntry { key: u32 },
}

/// Entries in play order with one of them current. History is the entries before
/// the current one.
#[derive(Debug, Default)]
pub struct Queue {
    entries: Vec<Entry>,
    /// Index of the current entry; 0 while the Queue is empty.
    current: usize,
    next_key: u32,
    revision: u64,
}

impl Queue {
    /// Replaces the whole Queue and makes `tracks[start]` current.
    pub fn replace(&mut self, tracks: Vec<Uuid>, start: usize) -> Result<(), QueueError> {
        if start >= tracks.len() {
            return Err(QueueError::OutOfRange {
                index: start,
                len: tracks.len(),
            });
        }
        self.entries.clear();
        for track in tracks {
            let entry = self.entry(track, false);
            self.entries.push(entry);
        }
        self.current = start;
        self.revision += 1;
        Ok(())
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.current)
    }

    /// The entries before the current one, oldest first.
    pub fn history(&self) -> &[Entry] {
        &self.entries[..self.current]
    }

    /// The entries after the current one.
    pub fn upcoming(&self) -> &[Entry] {
        self.entries.get(self.current + 1..).unwrap_or_default()
    }

    /// Puts a Recommender pick at the end.
    pub fn append_autoplay(&mut self, track: Uuid) {
        let entry = self.entry(track, true);
        self.entries.push(entry);
        self.revision += 1;
    }

    /// Empties the Queue and returns a key no entry has had, for what plays
    /// outside it.
    pub fn clear(&mut self) -> u32 {
        self.entries.clear();
        self.current = 0;
        self.revision += 1;
        let key = self.next_key;
        self.next_key += 1;
        key
    }

    /// Makes the next entry current. `false` when there is none.
    pub fn advance(&mut self) -> bool {
        if self.upcoming().is_empty() {
            return false;
        }
        self.current += 1;
        self.revision += 1;
        true
    }

    /// Makes the previous entry current. `false` when there is none.
    pub fn retreat(&mut self) -> bool {
        if self.current == 0 {
            return false;
        }
        self.current -= 1;
        self.revision += 1;
        true
    }

    /// Moves the upcoming entry `key` in front of `before`, or to the end when
    /// that is `None`. A played or current anchor means the front of the
    /// upcoming entries, and a played or current `key` stays where it is.
    /// Returns whether the order changed.
    pub fn move_before(&mut self, key: u32, before: Option<u32>) -> Result<bool, QueueError> {
        let from = self.index_of(key)?;
        let anchor = before.map(|key| self.index_of(key)).transpose()?;
        if from <= self.current {
            return Ok(false);
        }
        // The index the entry goes in front of, never ahead of the current one.
        let gap = anchor.map_or(self.entries.len(), |at| at.max(self.current + 1));
        if gap == from || gap == from + 1 {
            return Ok(false);
        }
        let entry = self.entries.remove(from);
        let to = if gap > from { gap - 1 } else { gap };
        self.entries.insert(to, entry);
        self.revision += 1;
        Ok(true)
    }

    fn entry(&mut self, track: Uuid, autoplay: bool) -> Entry {
        let key = self.next_key;
        self.next_key += 1;
        Entry {
            key,
            track,
            autoplay,
        }
    }

    fn index_of(&self, key: u32) -> Result<usize, QueueError> {
        self.entries
            .iter()
            .position(|entry| entry.key == key)
            .ok_or(QueueError::UnknownEntry { key })
    }

    /// Grows with every change.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> Uuid {
        Uuid::from_u128(u128::from(n))
    }

    fn queue(tracks: &[u32], start: usize) -> Queue {
        let mut queue = Queue::default();
        queue
            .replace(tracks.iter().copied().map(id).collect(), start)
            .unwrap();
        queue
    }

    /// Tracks 1..=5, starting at the given index.
    fn five(start: usize) -> Queue {
        queue(&[1, 2, 3, 4, 5], start)
    }

    fn numbers(entries: &[Entry]) -> Vec<u128> {
        entries.iter().map(|e| e.track.as_u128()).collect()
    }

    fn current(queue: &Queue) -> u128 {
        queue.current().unwrap().track.as_u128()
    }

    /// The key of the first entry holding track `n`.
    fn key(queue: &Queue, n: u32) -> u32 {
        queue.entries.iter().find(|e| e.track == id(n)).unwrap().key
    }

    #[test]
    fn a_new_queue_is_empty() {
        let mut queue = Queue::default();

        assert_eq!(queue.current(), None);
        assert!(queue.upcoming().is_empty());
        assert!(queue.history().is_empty());
        assert!(!queue.advance());
        assert!(!queue.retreat());
        assert_eq!(queue.revision(), 0);
    }

    #[test]
    fn replace_starts_at_the_chosen_track() {
        let mut queue = five(2);

        assert_eq!(current(&queue), 3);
        assert_eq!(numbers(queue.upcoming()), [4, 5]);
        assert_eq!(numbers(queue.history()), [1, 2]);
        assert!(queue.retreat());
        assert_eq!(current(&queue), 2);
    }

    #[test]
    fn replace_past_the_end_keeps_the_old_queue() {
        let mut queue = five(1);
        let revision = queue.revision();

        let result = queue.replace((1..=5).map(id).collect(), 5);

        assert_eq!(result, Err(QueueError::OutOfRange { index: 5, len: 5 }));
        assert_eq!(current(&queue), 2);
        assert_eq!(queue.revision(), revision);
    }

    #[test]
    fn replace_rejects_an_empty_list() {
        assert_eq!(
            Queue::default().replace(Vec::new(), 0),
            Err(QueueError::OutOfRange { index: 0, len: 0 })
        );
    }

    #[test]
    fn keys_are_never_reused_across_replacements() {
        let mut queue = five(0);
        let old: Vec<u32> = queue.entries.iter().map(|e| e.key).collect();

        queue.replace((1..=5).map(id).collect(), 0).unwrap();

        assert!(queue.entries.iter().all(|e| !old.contains(&e.key)));
    }

    #[test]
    fn replaced_entries_are_not_autoplay() {
        assert!(five(0).entries.iter().all(|e| !e.autoplay));
    }

    #[test]
    fn an_autoplay_pick_goes_to_the_end_marked_and_with_a_new_key() {
        let mut queue = five(4);
        let revision = queue.revision();

        queue.append_autoplay(id(1));

        let added = *queue.upcoming().last().unwrap();
        assert_eq!((added.track, added.autoplay), (id(1), true));
        assert!(queue.history().iter().all(|e| e.key != added.key));
        assert_ne!(added.key, queue.current().unwrap().key);
        assert_eq!(queue.revision(), revision + 1);
        assert_eq!(current(&queue), 5);
    }

    #[test]
    fn the_same_track_twice_gets_two_entries_that_move_separately() {
        let mut queue = queue(&[9, 1, 2, 1], 0);
        let first = queue.upcoming()[0].key;
        let second = queue.upcoming()[2].key;
        assert_ne!(first, second);

        assert_eq!(queue.move_before(second, Some(first)), Ok(true));

        let keys: Vec<u32> = queue.upcoming().iter().map(|e| e.key).collect();
        assert_eq!(keys[..2], [second, first]);
        assert_eq!(numbers(queue.upcoming()), [1, 1, 2]);
    }

    #[test]
    fn advance_stops_at_the_last_entry() {
        let mut queue = five(4);

        assert!(!queue.advance());

        assert_eq!(current(&queue), 5);
        assert!(queue.upcoming().is_empty());
    }

    #[test]
    fn retreat_stops_at_the_first_entry() {
        let mut queue = five(0);

        assert!(!queue.retreat());

        assert_eq!(current(&queue), 1);
    }

    #[test]
    fn moves_an_upcoming_entry_before_another() {
        let mut queue = five(0);

        let moved = queue.move_before(key(&queue, 5), Some(key(&queue, 3)));

        assert_eq!(moved, Ok(true));
        assert_eq!(current(&queue), 1);
        assert_eq!(numbers(queue.upcoming()), [2, 5, 3, 4]);
    }

    #[test]
    fn moves_an_entry_further_back() {
        let mut queue = five(0);

        queue
            .move_before(key(&queue, 2), Some(key(&queue, 5)))
            .unwrap();

        assert_eq!(numbers(queue.upcoming()), [3, 4, 2, 5]);
    }

    #[test]
    fn no_anchor_moves_to_the_end() {
        let mut queue = five(0);

        assert_eq!(queue.move_before(key(&queue, 2), None), Ok(true));

        assert_eq!(numbers(queue.upcoming()), [3, 4, 5, 2]);
    }

    #[test]
    fn dropping_in_place_changes_nothing() {
        let mut queue = five(0);
        let revision = queue.revision();
        let (three, four, five) = (key(&queue, 3), key(&queue, 4), key(&queue, 5));

        assert_eq!(queue.move_before(three, Some(three)), Ok(false));
        assert_eq!(queue.move_before(three, Some(four)), Ok(false));
        assert_eq!(queue.move_before(five, None), Ok(false));

        assert_eq!(numbers(queue.upcoming()), [2, 3, 4, 5]);
        assert_eq!(queue.revision(), revision);
    }

    #[test]
    fn a_played_or_current_entry_does_not_move() {
        let mut queue = five(2);

        assert_eq!(queue.move_before(key(&queue, 1), None), Ok(false));
        assert_eq!(queue.move_before(key(&queue, 3), None), Ok(false));

        assert_eq!(numbers(&queue.entries), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_played_anchor_means_the_front() {
        let mut queue = five(0);
        queue.advance();
        queue.advance();

        let moved = queue.move_before(key(&queue, 5), Some(key(&queue, 2)));

        assert_eq!(moved, Ok(true));
        assert_eq!(current(&queue), 3);
        assert_eq!(numbers(queue.upcoming()), [5, 4]);
        assert_eq!(numbers(queue.history()), [1, 2]);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let mut queue = five(0);
        let unknown = Err(QueueError::UnknownEntry { key: 999 });

        assert_eq!(queue.move_before(999, None), unknown);
        assert_eq!(queue.move_before(key(&queue, 3), Some(999)), unknown);

        assert_eq!(numbers(queue.upcoming()), [2, 3, 4, 5]);
    }

    #[test]
    fn revision_bumps_only_on_change() {
        let mut queue = five(0);
        assert_eq!(queue.revision(), 1);

        queue.advance();
        assert_eq!(queue.revision(), 2);
        queue.retreat();
        assert_eq!(queue.revision(), 3);
        queue.retreat();
        assert_eq!(queue.revision(), 3);
        queue
            .move_before(key(&queue, 5), Some(key(&queue, 2)))
            .unwrap();
        assert_eq!(queue.revision(), 4);
        queue
            .move_before(key(&queue, 5), Some(key(&queue, 2)))
            .unwrap();
        assert_eq!(queue.revision(), 4);
    }

    #[test]
    fn moves_across_a_5000_entry_queue() {
        let tracks: Vec<u32> = (1..=5_000).collect();
        let mut queue = queue(&tracks, 0);

        queue
            .move_before(key(&queue, 5_000), Some(key(&queue, 2)))
            .unwrap();

        let upcoming = numbers(queue.upcoming());
        assert_eq!(upcoming.len(), 4_999);
        assert_eq!(upcoming[..3], [5_000, 2, 3]);
        assert_eq!(upcoming.last(), Some(&4_999));
    }
}
