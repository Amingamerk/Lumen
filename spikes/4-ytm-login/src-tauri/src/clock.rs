//! The time seam: tests move time by hand.

use std::{future::Future, time::Duration};

pub trait Clock: Send + Sync {
    /// Unix seconds.
    fn now(&self) -> u64;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
    /// A varying number used to spread out cool-downs.
    fn jitter(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        // A clock set before 1970 reads as zero; the server then rejects the hash.
        std::time::UNIX_EPOCH
            .elapsed()
            .map_or(0, |since| since.as_secs())
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    fn jitter(&self) -> u64 {
        std::time::UNIX_EPOCH
            .elapsed()
            .map_or(0, |since| u64::from(since.subsec_nanos()))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use super::*;

    pub struct FakeClock {
        pub now: Mutex<u64>,
        pub slept: Mutex<Vec<Duration>>,
    }

    impl FakeClock {
        pub fn at(now: u64) -> Self {
            Self {
                now: Mutex::new(now),
                slept: Mutex::default(),
            }
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> u64 {
            *self.now.lock().unwrap()
        }

        async fn sleep(&self, duration: Duration) {
            self.slept.lock().unwrap().push(duration);
            *self.now.lock().unwrap() += duration.as_secs();
        }

        fn jitter(&self) -> u64 {
            0
        }
    }
}
