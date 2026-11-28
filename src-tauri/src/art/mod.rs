//! Artwork: thumbnails fetched once, resized, kept on disk and served to the
//! webview through `lumen://art/...`. The webview only ever sees those
//! addresses; the remote ones stay here.

mod disk;
mod fetch;
mod host;
mod request;
mod resize;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::{OnceCell, Semaphore};

pub use fetch::{Fetch, FetchError, HttpFetch};
pub use request::parse as parse_request;

/// How much artwork stays on disk. The oldest goes first.
pub const CACHE_CAP_BYTES: u64 = 160 * 1024 * 1024;

/// The pictures Lumen shows, as pixels on a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    /// A row or the player bar.
    Small,
    /// A card or the head of a page.
    Large,
}

impl Size {
    pub const ALL: [Size; 2] = [Size::Small, Size::Large];

    pub fn px(self) -> u32 {
        match self {
            Self::Small => 64,
            Self::Large => 300,
        }
    }

    /// Exactly the text a protocol path holds.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Small => "64",
            Self::Large => "300",
        }
    }
}

/// Why a picture could not be had.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtError {
    #[error("artwork is not fetched from that address")]
    NotAllowed,
    #[error("the image host asked Lumen to wait")]
    BackedOff,
    #[error("this picture failed a moment ago")]
    RecentlyFailed,
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("the picture could not be read: {0}")]
    Decode(String),
    #[error("the picture could not be kept: {0}")]
    Disk(String),
}

/// Time as artwork needs it. A seam so tests move it by hand.
pub trait Clock: Send + Sync + 'static {
    /// Time since some fixed point.
    fn now(&self) -> Duration;
    /// A varying number of milliseconds, to spread out waits.
    fn jitter_ms(&self) -> u64;
}

#[derive(Debug)]
pub struct SystemClock(std::time::Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn jitter_ms(&self) -> u64 {
        u64::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.subsec_nanos()),
        ) % 1_000
    }
}

/// How many pictures are fetched and resized at once.
const CONCURRENCY: usize = 3;
/// The first wait after the image host refuses, doubled on each refusal.
const FIRST_BACKOFF: Duration = Duration::from_secs(30);
const LONGEST_BACKOFF: Duration = Duration::from_secs(15 * 60);
/// How long a picture that failed is left alone.
const RETRY_FAILED_AFTER: Duration = Duration::from_secs(10 * 60);
/// How many failures are remembered; a full list is forgotten at once.
const MAX_FAILURES: usize = 2_048;

/// Fetches, resizes and keeps artwork. It has its own limits: the account gate
/// is for the account, and image requests carry no session.
pub struct ArtCache<F, C = SystemClock> {
    fetcher: F,
    clock: C,
    disk: Arc<disk::Disk>,
    /// Bounds the pictures fetched and resized at once.
    work: Semaphore,
    /// Callers waiting for the same picture share one fetch.
    inflight: Mutex<HashMap<String, Arc<Made>>>,
    /// When the image host refused, and how often in a row.
    backoff: Mutex<Backoff>,
    /// Pictures that failed, and when they may be tried again.
    failed: Mutex<HashMap<String, Duration>>,
}

/// What one fetch came to; every caller waiting for it reads the same.
type Made = OnceCell<Result<(), ArtError>>;

#[derive(Debug, Default)]
struct Backoff {
    until: Duration,
    refusals: u32,
}

impl<F: Fetch, C: Clock> ArtCache<F, C> {
    /// Opens the cache in `dir`, keeping at most `cap` bytes there.
    pub fn open(fetcher: F, clock: C, dir: PathBuf, cap: u64) -> Result<Self, ArtError> {
        let disk = disk::Disk::open(dir, cap).map_err(|e| ArtError::Disk(e.to_string()))?;
        Ok(Self {
            fetcher,
            clock,
            disk: Arc::new(disk),
            work: Semaphore::new(CONCURRENCY),
            inflight: Mutex::default(),
            backoff: Mutex::default(),
            failed: Mutex::default(),
        })
    }

    /// The JPEG of the picture at `address`, `size` pixels on a side.
    pub async fn get(&self, address: &str, size: Size) -> Result<Vec<u8>, ArtError> {
        // First of all: nothing off the allowlist goes any further.
        let url = host::allowed(address).ok_or(ArtError::NotAllowed)?;
        let key = key(&url);
        if let Some(kept) = self.read(&key, size).await {
            return Ok(kept);
        }
        self.recently_failed(&key)?;

        let cell = Arc::clone(lock(&self.inflight).entry(key.clone()).or_default());
        let outcome = cell.get_or_init(|| self.produce(url, &key)).await.clone();
        {
            let mut inflight = lock(&self.inflight);
            if inflight.get(&key).is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                inflight.remove(&key);
            }
        }
        outcome?;
        self.read(&key, size)
            .await
            .ok_or_else(|| ArtError::Disk("the picture was dropped as it was made".to_owned()))
    }

    async fn read(&self, key: &str, size: Size) -> Option<Vec<u8>> {
        let (disk, key) = (Arc::clone(&self.disk), key.to_owned());
        tokio::task::spawn_blocking(move || disk.read(&key, size))
            .await
            .ok()?
    }

    /// Fetches the picture and keeps both its sizes. One run serves everyone
    /// waiting for it.
    async fn produce(&self, url: reqwest::Url, key: &str) -> Result<(), ArtError> {
        let Ok(_turn) = self.work.acquire().await else {
            return Err(ArtError::Disk("artwork is shutting down".to_owned()));
        };
        // Checked after the wait for a turn, so a refusal seen meanwhile counts.
        self.check_backoff()?;
        let result = self.fetch_and_keep(url, key).await;
        match &result {
            Ok(()) => lock(&self.backoff).refusals = 0,
            Err(ArtError::Fetch(FetchError::RateLimited { retry_after })) => {
                self.back_off(*retry_after);
            }
            // No fault of the picture: try again whenever it is asked for.
            Err(ArtError::Fetch(FetchError::Network) | ArtError::BackedOff) => {}
            Err(_) => self.remember_failure(key),
        }
        result
    }

    async fn fetch_and_keep(&self, url: reqwest::Url, key: &str) -> Result<(), ArtError> {
        let source = self.fetcher.get(url).await?;
        let (disk, key) = (Arc::clone(&self.disk), key.to_owned());
        // Decoding, resizing and writing are heavy: not on the async threads.
        tokio::task::spawn_blocking(move || {
            for (size, jpeg) in resize::thumbnails(&source)? {
                disk.write(&key, size, &jpeg)
                    .map_err(|error| ArtError::Disk(error.to_string()))?;
            }
            Ok(())
        })
        .await
        .map_err(|_| ArtError::Decode("the work was cut short".to_owned()))?
    }

    fn check_backoff(&self) -> Result<(), ArtError> {
        if self.clock.now() < lock(&self.backoff).until {
            return Err(ArtError::BackedOff);
        }
        Ok(())
    }

    /// The image host said 429: wait as long as it asks, or longer each time it
    /// refuses in a row, and a little more at random.
    fn back_off(&self, retry_after: Option<Duration>) {
        let mut backoff = lock(&self.backoff);
        backoff.refusals = backoff.refusals.saturating_add(1);
        let doubled = FIRST_BACKOFF.saturating_mul(1 << (backoff.refusals - 1).min(10));
        let wait = retry_after
            .unwrap_or(doubled)
            .clamp(Duration::from_secs(1), LONGEST_BACKOFF);
        backoff.until = self.clock.now() + wait + Duration::from_millis(self.clock.jitter_ms());
    }

    fn remember_failure(&self, key: &str) {
        let mut failed = lock(&self.failed);
        if failed.len() >= MAX_FAILURES {
            failed.clear();
        }
        failed.insert(key.to_owned(), self.clock.now() + RETRY_FAILED_AFTER);
    }

    fn recently_failed(&self, key: &str) -> Result<(), ArtError> {
        let mut failed = lock(&self.failed);
        match failed.get(key) {
            Some(retry_at) if self.clock.now() < *retry_at => Err(ArtError::RecentlyFailed),
            Some(_) => {
                failed.remove(key);
                Ok(())
            }
            None => Ok(()),
        }
    }
}

/// Names a picture on disk: the digest of its address.
fn key(url: &reqwest::Url) -> String {
    use sha1::{Digest, Sha1};
    Sha1::digest(url.as_str())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
