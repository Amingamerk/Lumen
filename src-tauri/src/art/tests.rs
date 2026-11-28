//! Artwork through its public face: addresses in, thumbnails out, with a fake
//! image host and a clock moved by hand.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Url;

use super::*;

const COVER: &str = "https://i.ytimg.com/vi/cover/hq720.jpg";

/// A picture of noise, so that it does not compress to nothing.
fn picture(seed: u8, width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let n = x
            .wrapping_mul(2_654_435_761)
            .wrapping_add(y.wrapping_mul(40_503))
            .wrapping_add(u32::from(seed).wrapping_mul(97));
        image::Rgb([(n >> 3) as u8, (n >> 11) as u8, (n >> 19) as u8])
    });
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn dimensions(jpeg: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap();
    (image.width(), image.height())
}

type Answer = Result<Vec<u8>, FetchError>;

/// An image host that answers from a table and counts what it was asked.
#[derive(Clone, Default)]
struct FakeHost {
    answers: Arc<Mutex<HashMap<String, Answer>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl FakeHost {
    fn serving(self, address: &str, answer: Result<Vec<u8>, FetchError>) -> Self {
        self.answers
            .lock()
            .unwrap()
            .insert(address.to_owned(), answer);
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Fetch for FakeHost {
    async fn get(&self, url: Url) -> Result<Vec<u8>, FetchError> {
        self.asked.lock().unwrap().push(url.to_string());
        // A real fetch takes a while; let the other callers arrive.
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        self.answers
            .lock()
            .unwrap()
            .get(url.as_str())
            .cloned()
            .unwrap_or(Err(FetchError::Status(404)))
    }
}

#[derive(Clone, Default)]
struct HandClock(Arc<AtomicU64>);

impl HandClock {
    fn advance(&self, by: Duration) {
        self.0
            .fetch_add(u64::try_from(by.as_millis()).unwrap(), Ordering::SeqCst);
    }
}

impl Clock for HandClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }

    fn jitter_ms(&self) -> u64 {
        0
    }
}

fn cache_in(
    dir: &std::path::Path,
    host: &FakeHost,
    clock: &HandClock,
    cap: u64,
) -> ArtCache<FakeHost, HandClock> {
    ArtCache::open(host.clone(), clock.clone(), dir.to_path_buf(), cap).unwrap()
}

fn open(host: &FakeHost) -> (tempfile::TempDir, ArtCache<FakeHost, HandClock>) {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(dir.path(), host, &HandClock::default(), CACHE_CAP_BYTES);
    (dir, cache)
}

fn bytes_on_disk(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum()
}

#[tokio::test]
async fn an_address_off_the_allowlist_is_refused_and_never_fetched() {
    let host = FakeHost::default();
    let (_dir, cache) = open(&host);

    for address in [
        "https://evil.example/a.jpg",
        "http://i.ytimg.com/a.jpg",
        "https://i.ytimg.com.evil.example/a.jpg",
        "file:///C:/Windows/win.ini",
        "http://127.0.0.1:8080/a.jpg",
    ] {
        let result = cache.get(address, Size::Small).await;
        assert_eq!(result, Err(ArtError::NotAllowed), "{address}");
    }

    assert_eq!(host.asked(), Vec::<String>::new());
}

#[tokio::test]
async fn concurrent_requests_for_one_picture_fetch_it_once() {
    let host = FakeHost::default().serving(COVER, Ok(picture(1, 544, 544)));
    let (_dir, cache) = open(&host);

    let (a, b, c, d) = tokio::join!(
        cache.get(COVER, Size::Small),
        cache.get(COVER, Size::Small),
        cache.get(COVER, Size::Large),
        cache.get(COVER, Size::Small),
    );

    assert!(a.is_ok() && b.is_ok() && c.is_ok() && d.is_ok());
    assert_eq!(host.asked().len(), 1);
    // Both sizes were made from that one fetch.
    cache.get(COVER, Size::Large).await.unwrap();
    assert_eq!(host.asked().len(), 1);
}

#[tokio::test]
async fn a_picture_is_cut_square_and_resized_to_the_size_asked() {
    let host = FakeHost::default().serving(COVER, Ok(picture(2, 800, 450)));
    let (_dir, cache) = open(&host);

    let small = cache.get(COVER, Size::Small).await.unwrap();
    let large = cache.get(COVER, Size::Large).await.unwrap();

    assert_eq!(dimensions(&small), (64, 64));
    assert_eq!(dimensions(&large), (300, 300));
}

#[tokio::test]
async fn a_picture_that_is_no_picture_is_refused_and_not_kept() {
    let host = FakeHost::default().serving(COVER, Ok(b"<html>not an image</html>".to_vec()));
    let (dir, cache) = open(&host);

    let result = cache.get(COVER, Size::Small).await;

    assert!(matches!(result, Err(ArtError::Decode(_))), "{result:?}");
    assert_eq!(bytes_on_disk(dir.path()), 0);
}

#[tokio::test]
async fn a_picture_with_too_many_pixels_is_refused() {
    // Small to send, large to decode.
    let huge = picture(3, 3_000, 8);
    let host = FakeHost::default().serving(COVER, Ok(huge));
    let (_dir, cache) = open(&host);

    let result = cache.get(COVER, Size::Small).await;

    assert!(matches!(result, Err(ArtError::Decode(_))), "{result:?}");
}

#[tokio::test]
async fn kept_pictures_show_without_the_network() {
    let host = FakeHost::default().serving(COVER, Ok(picture(4, 544, 544)));
    let dir = tempfile::tempdir().unwrap();
    let clock = HandClock::default();
    let first = cache_in(dir.path(), &host, &clock, CACHE_CAP_BYTES);
    let before = first.get(COVER, Size::Small).await.unwrap();
    drop(first);

    // The next run, offline: the host answers nothing.
    let offline = FakeHost::default();
    let second = cache_in(dir.path(), &offline, &clock, CACHE_CAP_BYTES);
    let after = second.get(COVER, Size::Small).await.unwrap();

    assert_eq!(before, after);
    assert_eq!(offline.asked(), Vec::<String>::new());
}

#[tokio::test]
async fn the_disk_cache_stays_under_its_cap_and_drops_the_oldest_first() {
    let covers: Vec<String> = (0..4)
        .map(|n| format!("https://i.ytimg.com/vi/cover{n}/hq720.jpg"))
        .collect();
    let mut host = FakeHost::default();
    for (n, cover) in covers.iter().enumerate() {
        host = host.serving(cover, Ok(picture(u8::try_from(n).unwrap(), 544, 544)));
    }
    // One picture's two sizes weigh this much; the cap holds two pictures.
    let measure = tempfile::tempdir().unwrap();
    let probe = cache_in(
        measure.path(),
        &host,
        &HandClock::default(),
        CACHE_CAP_BYTES,
    );
    probe.get(&covers[0], Size::Large).await.unwrap();
    let one = bytes_on_disk(measure.path());
    let cap = one * 5 / 2;
    host.asked.lock().unwrap().clear();

    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(dir.path(), &host, &HandClock::default(), cap);
    for cover in &covers[..3] {
        cache.get(cover, Size::Large).await.unwrap();
    }

    assert!(bytes_on_disk(dir.path()) <= cap);
    // The newest is still kept, the oldest was dropped.
    host.asked.lock().unwrap().clear();
    cache.get(&covers[2], Size::Large).await.unwrap();
    assert_eq!(host.asked(), Vec::<String>::new());
    cache.get(&covers[0], Size::Large).await.unwrap();
    assert_eq!(host.asked(), vec![covers[0].clone()]);
}

#[tokio::test]
async fn after_a_429_nothing_is_fetched_until_the_wait_has_passed() {
    let slow = FetchError::RateLimited {
        retry_after: Some(Duration::from_secs(120)),
    };
    let host = FakeHost::default().serving(COVER, Err(slow)).serving(
        "https://i.ytimg.com/vi/other/hq720.jpg",
        Ok(picture(5, 544, 544)),
    );
    let dir = tempfile::tempdir().unwrap();
    let clock = HandClock::default();
    let cache = cache_in(dir.path(), &host, &clock, CACHE_CAP_BYTES);

    let first = cache.get(COVER, Size::Small).await;
    let other = "https://i.ytimg.com/vi/other/hq720.jpg";
    let during = cache.get(other, Size::Small).await;

    assert!(matches!(
        first,
        Err(ArtError::Fetch(FetchError::RateLimited { .. }))
    ));
    assert_eq!(during, Err(ArtError::BackedOff));
    assert_eq!(host.asked().len(), 1);

    clock.advance(Duration::from_secs(119));
    assert_eq!(
        cache.get(other, Size::Small).await,
        Err(ArtError::BackedOff)
    );
    clock.advance(Duration::from_secs(2));
    assert!(cache.get(other, Size::Small).await.is_ok());
}

#[tokio::test]
async fn repeated_429s_without_a_retry_after_wait_longer_each_time() {
    let refusal = FetchError::RateLimited { retry_after: None };
    let host = FakeHost::default().serving(COVER, Err(refusal));
    let (dir, _) = open(&host);
    let clock = HandClock::default();
    let cache = cache_in(dir.path(), &host, &clock, CACHE_CAP_BYTES);
    let other = "https://i.ytimg.com/vi/other/hq720.jpg";

    let _ = cache.get(COVER, Size::Small).await;
    clock.advance(FIRST_BACKOFF + Duration::from_secs(2));
    // The wait is over, so this is asked again, and refused again.
    let _ = cache.get(COVER, Size::Large).await;
    clock.advance(FIRST_BACKOFF + Duration::from_secs(2));

    assert_eq!(
        cache.get(other, Size::Small).await,
        Err(ArtError::BackedOff)
    );
    assert_eq!(host.asked().len(), 2);
}

#[tokio::test]
async fn a_picture_that_failed_is_left_alone_for_a_while() {
    let host = FakeHost::default();
    let dir = tempfile::tempdir().unwrap();
    let clock = HandClock::default();
    let cache = cache_in(dir.path(), &host, &clock, CACHE_CAP_BYTES);

    let first = cache.get(COVER, Size::Small).await;
    let second = cache.get(COVER, Size::Small).await;

    assert_eq!(first, Err(ArtError::Fetch(FetchError::Status(404))));
    assert_eq!(second, Err(ArtError::RecentlyFailed));
    assert_eq!(host.asked().len(), 1);

    clock.advance(RETRY_FAILED_AFTER + Duration::from_secs(1));
    let third = cache.get(COVER, Size::Small).await;
    assert_eq!(third, Err(ArtError::Fetch(FetchError::Status(404))));
    assert_eq!(host.asked().len(), 2);
}
