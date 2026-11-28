//! Shuffle and repeat. Shuffle changes only the order entries play in: the
//! list order is kept beside it, and turning shuffle off goes back to it.

use std::collections::HashMap;

use uuid::Uuid;

use super::{Entry, Queue};

/// What happens when the Queue, or a song, runs out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    /// The Queue starts over after its last entry.
    All,
    /// A song that ends by itself plays again.
    One,
}

impl Repeat {
    /// The mode the repeat control goes to from this one.
    pub fn following(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// What a shuffled Queue remembers.
#[derive(Debug, Default)]
pub(super) struct Shuffle {
    /// Every entry in list order, the one it has without shuffle.
    pub(super) list: Vec<Entry>,
    /// How many entries at the front of the play order were there before
    /// shuffle was turned on. They are shuffled when the Queue starts over.
    ordered: usize,
}

/// A small seeded source of random places (SplitMix64). Unseeded, it starts
/// from the system's random numbers.
#[derive(Debug)]
pub(super) struct Dice(u64);

impl Default for Dice {
    fn default() -> Self {
        Self(Uuid::new_v4().as_u64_pair().0)
    }
}

impl Dice {
    fn roll(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut n = self.0;
        n = (n ^ (n >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        n = (n ^ (n >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        n ^ (n >> 31)
    }

    /// A number below `bound`, which must not be 0.
    fn below(&mut self, bound: usize) -> usize {
        // The high half of the product is below `bound`, so it fits.
        ((u128::from(self.roll()) * bound as u128) >> 64) as usize
    }

    /// Shuffles the songs the user queued among their own places. The
    /// Recommender's picks stay where they are, after them.
    pub(super) fn shuffle(&mut self, entries: &mut [Entry]) {
        let places: Vec<usize> = (0..entries.len())
            .filter(|&at| !entries[at].autoplay)
            .collect();
        // Fisher-Yates over those places.
        for last in (1..places.len()).rev() {
            entries.swap(places[last], places[self.below(last + 1)]);
        }
    }
}

impl Queue {
    /// A Queue whose shuffles follow from `seed`.
    #[cfg(test)]
    pub fn seeded(seed: u64) -> Self {
        Self {
            dice: Dice(seed),
            ..Self::default()
        }
    }

    pub fn shuffled(&self) -> bool {
        self.shuffle.is_some()
    }

    /// Turns shuffle on or off. The current entry and the played ones stay as
    /// they are; only what waits changes order. Returns whether it changed.
    pub fn set_shuffle(&mut self, on: bool) -> bool {
        if on == self.shuffled() {
            return false;
        }
        let from = (self.current + 1).min(self.entries.len());
        if on {
            self.shuffle = Some(Shuffle {
                list: self.entries.clone(),
                ordered: from,
            });
            self.dice.shuffle(&mut self.entries[from..]);
        } else if let Some(shuffle) = self.shuffle.take() {
            self.restore(&shuffle.list, from);
        }
        self.lap_end = None;
        self.revision += 1;
        true
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    /// Returns whether it changed.
    pub fn set_repeat(&mut self, repeat: Repeat) -> bool {
        let changed = repeat != self.repeat;
        self.repeat = repeat;
        changed
    }

    /// Puts a Queue that was just replaced into a shuffled order: the chosen
    /// entry leads, and every other one waits behind it.
    pub(super) fn shuffle_replaced(&mut self) {
        let Some(shuffle) = &mut self.shuffle else {
            return;
        };
        shuffle.list.clone_from(&self.entries);
        shuffle.ordered = 0;
        let chosen = self.entries.remove(self.current);
        self.entries.insert(0, chosen);
        self.current = 0;
        self.dice.shuffle(&mut self.entries[1..]);
    }

    /// Starts the Queue over from its first entry.
    pub(super) fn wrap(&mut self) {
        self.lap_end = self.current().map(|entry| entry.key);
        if let Some(shuffle) = &mut self.shuffle {
            // The last entry stays put, so going back finds it.
            let ordered = shuffle.ordered.min(self.entries.len().saturating_sub(1));
            self.dice.shuffle(&mut self.entries[..ordered]);
            shuffle.ordered = 0;
        }
        self.current = 0;
    }

    /// Puts the entries from `from` on back into list order: those after the
    /// current one in the list, then those before it that still wait. The
    /// picks at the end stay at the end.
    fn restore(&mut self, list: &[Entry], from: usize) {
        let place: HashMap<u32, usize> = list
            .iter()
            .enumerate()
            .map(|(at, entry)| (entry.key, at))
            .collect();
        let at = |entry: &Entry| place.get(&entry.key).copied();
        let here = self.current().and_then(at);
        let mut waiting = self.entries.split_off(from);
        waiting.sort_by_key(at);
        let split = waiting.partition_point(|entry| at(entry) < here);
        let earlier: Vec<Entry> = waiting.drain(..split).collect();
        let picks = waiting.iter().rev().take_while(|e| e.autoplay).count();
        let gap = waiting.len() - picks;
        waiting.splice(gap..gap, earlier);
        self.entries.append(&mut waiting);
    }
}

#[cfg(test)]
mod tests;
