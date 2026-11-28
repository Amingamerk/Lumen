//! Shuffle and repeat as the player carries them out.

use super::*;

impl<E: Engine> Player<E> {
    /// Fixes the orders shuffle picks, so a test can expect them.
    #[cfg(test)]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.queue = Queue::seeded(seed);
        self
    }

    /// Whether shuffle is on, and the repeat mode.
    pub fn modes(&self) -> (bool, Repeat) {
        (self.queue.shuffled(), self.queue.repeat())
    }

    pub fn set_shuffle(&mut self, on: bool) -> Option<PlayerState> {
        self.queue.set_shuffle(on).then(|| self.commit())
    }

    pub fn toggle_shuffle(&mut self) -> Option<PlayerState> {
        self.set_shuffle(!self.queue.shuffled())
    }

    pub fn set_repeat(&mut self, repeat: Repeat) -> Option<PlayerState> {
        self.queue.set_repeat(repeat).then(|| self.commit())
    }

    /// Off, all, one, and off again.
    pub fn cycle_repeat(&mut self) -> Option<PlayerState> {
        self.set_repeat(self.queue.repeat().following())
    }

    /// Whether a song that ended by itself plays again. One that stands outside
    /// the Queue is all there is to repeat.
    pub(super) fn repeats_song(&self) -> bool {
        match self.queue.repeat() {
            Repeat::Off => false,
            Repeat::All => self.direct.is_some() && !self.is_queued(),
            Repeat::One => true,
        }
    }

    /// Plays the current song from the start, as a new play.
    pub(super) fn replay(&mut self) {
        self.end_listen(EndReason::Finished);
        if let Some(direct) = &mut self.direct {
            direct.recovery = Recovery::default();
        }
        self.set_playing(true);
        if self.expired() {
            self.replace_expired(Duration::ZERO);
        } else {
            self.load(Duration::ZERO, true);
        }
    }
}

#[cfg(test)]
mod tests;
