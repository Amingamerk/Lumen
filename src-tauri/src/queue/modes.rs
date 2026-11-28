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
        self.shuffled
    }

    /// Turns shuffle on or off. The current entry and the played ones stay as
    /// they are; only what waits changes order. Returns whether it changed.
    pub fn set_shuffle(&mut self, on: bool) -> bool {
        if on == self.shuffled {
            return false;
        }
        let from = (self.current + 1).min(self.entries.len());
        if on {
            self.ordered = from;
            self.dice.shuffle(&mut self.entries[from..]);
        } else {
            self.restore(from);
        }
        self.shuffled = on;
        // The pass to go on to belonged to the other mode.
        self.pass_after = None;
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

    /// Puts a Queue that was just replaced into a shuffled order, if shuffle is
    /// on: the chosen entry leads, and every other one waits behind it.
    pub(super) fn shuffle_replaced(&mut self) {
        if !self.shuffled {
            return;
        }
        self.ordered = 0;
        let chosen = self.entries.remove(self.current);
        self.entries.insert(0, chosen);
        self.current = 0;
        self.dice.shuffle(&mut self.entries[1..]);
    }

    /// Starts the Queue over from its first entry: in list order, or shuffled.
    pub(super) fn wrap(&mut self) {
        let left = self.keys();
        if !self.shuffled {
            self.entries.clone_from(&self.list);
        } else if let Some(after) = self.pass_after.take() {
            self.arrange(&after);
        } else {
            // The last entry is not moved, so it does not come right back.
            let ordered = self.ordered.min(self.entries.len().saturating_sub(1));
            self.dice.shuffle(&mut self.entries[..ordered]);
            self.ordered = 0;
        }
        self.pass_after = None;
        self.pass_before = Some(left);
        self.current = 0;
    }

    /// Goes back to the end of the pass before this one, if it is still known.
    /// Entries added since then wait after it.
    pub(super) fn return_to_pass_before(&mut self) -> bool {
        let Some(before) = self.pass_before.take() else {
            return false;
        };
        let left = self.keys();
        let Some(last) = self.arrange(&before).checked_sub(1) else {
            return false;
        };
        self.current = last;
        self.pass_after = Some(left);
        true
    }

    pub(super) fn forget_passes(&mut self) {
        self.pass_before = None;
        self.pass_after = None;
    }

    fn keys(&self) -> Vec<u32> {
        self.entries.iter().map(|entry| entry.key).collect()
    }

    /// Puts the play order into that of `keys`, with the entries it does not
    /// name behind. Returns how many it names.
    fn arrange(&mut self, keys: &[u32]) -> usize {
        let place = places(keys.iter().copied());
        let at = |entry: &Entry| place.get(&entry.key).copied();
        self.entries
            .sort_by_key(|entry| at(entry).unwrap_or(usize::MAX));
        self.entries.iter().filter(|e| at(e).is_some()).count()
    }

    /// Puts the entries from `from` on back into list order: those after the
    /// current one in the list, then those before it that still wait. The
    /// picks at the end stay at the end.
    fn restore(&mut self, from: usize) {
        let place = places(self.list.iter().map(|entry| entry.key));
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

/// Where each key comes in an order.
fn places(keys: impl Iterator<Item = u32>) -> HashMap<u32, usize> {
    keys.enumerate().map(|(at, key)| (key, at)).collect()
}

#[cfg(test)]
mod tests;
