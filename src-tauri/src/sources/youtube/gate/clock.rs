//! The time seam: tests move time by hand.

use std::future::Future;
use std::time::{Duration, Instant, UNIX_EPOCH};

pub trait GateClock: Send + Sync {
    /// Unix milliseconds. Cool-downs are saved in this time.
    fn wall_ms(&self) -> u64;
    /// Time since some fixed point. Spacing uses it, so a clock change cannot
    /// shorten it.
    fn monotonic(&self) -> Duration;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
    /// A varying number used to spread out cool-downs.
    fn jitter(&self) -> u64;
}

pub struct SystemClock {
    started: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl GateClock for SystemClock {
    fn wall_ms(&self) -> u64 {
        // A clock set before 1970 reads as zero, which only lengthens a wait.
        UNIX_EPOCH.elapsed().map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
    }

    fn monotonic(&self) -> Duration {
        self.started.elapsed()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    fn jitter(&self) -> u64 {
        UNIX_EPOCH
            .elapsed()
            .map_or(0, |since| u64::from(since.subsec_nanos()))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::{Arc, Mutex};

    use tokio::sync::oneshot;

    use super::*;

    /// Time that moves only when slept or advanced. Clones share it.
    #[derive(Clone, Default)]
    pub struct FakeClock {
        inner: Arc<Mutex<Time>>,
    }

    #[derive(Default)]
    struct Time {
        wall_ms: u64,
        monotonic: Duration,
        slept: Vec<Duration>,
        held: Option<oneshot::Receiver<()>>,
    }

    impl FakeClock {
        pub fn at(wall_ms: u64) -> Self {
            let clock = Self::default();
            clock.inner.lock().unwrap().wall_ms = wall_ms;
            clock
        }

        pub fn advance(&self, by: Duration) {
            let mut time = self.inner.lock().unwrap();
            time.wall_ms += u64::try_from(by.as_millis()).unwrap();
            time.monotonic += by;
        }

        pub fn slept(&self) -> Vec<Duration> {
            self.inner.lock().unwrap().slept.clone()
        }

        /// The next sleep lasts until this is sent to or dropped.
        pub fn hold_next_sleep(&self) -> oneshot::Sender<()> {
            let (release, held) = oneshot::channel();
            self.inner.lock().unwrap().held = Some(held);
            release
        }
    }

    impl GateClock for FakeClock {
        fn wall_ms(&self) -> u64 {
            self.inner.lock().unwrap().wall_ms
        }

        fn monotonic(&self) -> Duration {
            self.inner.lock().unwrap().monotonic
        }

        async fn sleep(&self, duration: Duration) {
            let held = {
                let mut time = self.inner.lock().unwrap();
                time.slept.push(duration);
                time.held.take()
            };
            if let Some(held) = held {
                let _ = held.await;
            }
            self.advance(duration);
        }

        fn jitter(&self) -> u64 {
            0
        }
    }
}
