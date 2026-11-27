//! The thread that moves the simulated clock along. Milestone 2 deletes it:
//! libmpv reports from a thread of its own.

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Mutex, PoisonError};

use super::{Clock, Player, PlayerEvent, SimulatedEngine};

/// Tells the clock thread that the player changed, so it works out anew when to
/// look next. The thread ends once every `Wake` is gone.
#[derive(Debug, Clone)]
pub struct Wake(SyncSender<()>);

impl Wake {
    pub fn channel() -> (Self, Receiver<()>) {
        // One waiting message is enough: more would say nothing new.
        let (sender, receiver) = sync_channel(1);
        (Self(sender), receiver)
    }

    pub fn wake(&self) {
        // Full means a wake is already waiting; closed means no clock is left.
        let _ = self.0.try_send(());
    }
}

/// Runs the clock until `wake` closes. While nothing plays it sleeps without a
/// timeout, so a paused or idle player costs no wake-ups.
pub fn run<C: Clock>(
    player: &Mutex<Player<SimulatedEngine<C>>>,
    wake: Receiver<()>,
    mut emit: impl FnMut(PlayerEvent),
) {
    let lock = || player.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        let next = lock().next_wake();
        let waited = match next {
            Some(timeout) => wake.recv_timeout(timeout),
            None => wake.recv().map_err(RecvTimeoutError::from),
        };
        if waited == Err(RecvTimeoutError::Disconnected) {
            return;
        }
        // Looked at after a wake too, so a stream of commands cannot starve the
        // clock. The player reports nothing it already has.
        let event = lock().tick();
        if let Some(event) = event {
            emit(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::audio::SystemClock;
    use crate::catalog::Catalog;
    use crate::recs::StaticRecommender;

    #[test]
    fn the_clock_reports_while_playing_and_sleeps_when_paused() {
        let catalog = Arc::new(Catalog::default());
        let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
        let engine = SimulatedEngine::new(SystemClock::default());
        let player = Arc::new(Mutex::new(Player::new(engine, catalog, recommender)));
        let (wake, woken) = Wake::channel();
        let (sender, events) = channel();
        let clocked = Arc::clone(&player);
        let thread = thread::spawn(move || {
            run(&clocked, woken, |event| sender.send(event).unwrap());
        });

        player
            .lock()
            .unwrap()
            .play_collection("night-shift", 0)
            .unwrap();
        wake.wake();
        let first = events.recv_timeout(Duration::from_millis(1_500));
        assert!(
            matches!(first, Ok(PlayerEvent::Position(p)) if p.position_ms >= 1_000),
            "{first:?}"
        );

        player.lock().unwrap().toggle_play();
        wake.wake();
        // Let through anything sent just before the pause.
        while events.recv_timeout(Duration::from_millis(100)).is_ok() {}
        assert!(events.recv_timeout(Duration::from_millis(1_200)).is_err());

        drop(wake);
        thread.join().unwrap();
    }
}
