//! `lumen://art/<size>/<track id>`: the webview asks for a picture by track id
//! and gets a JPEG. Where the picture lives never leaves Rust.

use std::path::PathBuf;

use tauri::http::{Response, header};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::OnceCell;
use uuid::Uuid;

use super::library::{LibraryHandle, blocking};
use super::{MAIN_WINDOW, SharedState, lock};
use crate::art::{
    ArtCache, ArtError, CACHE_CAP_BYTES, Clock, Fetch, HttpFetch, SystemClock, parse_request,
};

/// The protocol's name, registered in `lib.rs`.
pub const SCHEME: &str = "lumen";

/// The folder inside the app's cache folder.
const FOLDER: &str = "art";

/// What a request is answered with.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    fn status(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
        }
    }

    pub fn into_response(self) -> Response<Vec<u8>> {
        let ok = self.status == 200;
        let builder = Response::builder()
            .status(self.status)
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
        let builder = if ok {
            builder
                .header(header::CONTENT_TYPE, "image/jpeg")
                .header(header::CACHE_CONTROL, "private, max-age=86400")
        } else {
            builder.header(header::CACHE_CONTROL, "no-store")
        };
        builder
            .body(self.body)
            .unwrap_or_else(|_| Response::new(Vec::new()))
    }
}

/// Answers one request. `address_of` finds where a track's picture lives; it is
/// asked only for a path that names a size and a track.
pub async fn respond<F: Fetch, C: Clock>(
    cache: &ArtCache<F, C>,
    path: &str,
    address_of: impl AsyncFnOnce(Uuid) -> Option<String>,
) -> Reply {
    let Some((size, track)) = parse_request(path) else {
        return Reply::status(400);
    };
    let Some(address) = address_of(track).await else {
        return Reply::status(404);
    };
    match cache.get(&address, size).await {
        Ok(body) => Reply { status: 200, body },
        // No detail goes back: the webview shows its placeholder either way.
        Err(ArtError::BackedOff) => Reply::status(503),
        Err(_) => Reply::status(404),
    }
}

/// The art cache as the app holds it. It opens on the first picture asked for,
/// so starting the app does not count what is on disk.
pub struct ArtHandle {
    dir: PathBuf,
    cache: OnceCell<ArtCache<HttpFetch>>,
}

impl ArtHandle {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            cache: OnceCell::new(),
        }
    }

    /// A failed open is not kept, so the next picture tries again.
    async fn cache(&self) -> Result<&ArtCache<HttpFetch>, ArtError> {
        self.cache
            .get_or_try_init(|| async {
                let dir = self.dir.clone();
                tokio::task::spawn_blocking(move || {
                    ArtCache::open(
                        HttpFetch::default(),
                        SystemClock::default(),
                        dir,
                        CACHE_CAP_BYTES,
                    )
                })
                .await
                .unwrap_or_else(|_| Err(ArtError::Disk("opening was cut short".to_owned())))
            })
            .await
    }
}

/// Remembers where the cache lives.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let dir = app.path().app_cache_dir()?.join(FOLDER);
    app.manage(ArtHandle::new(dir));
    Ok(())
}

/// Where a picture comes from: the results on show (a song, an album, an
/// artist or a playlist) or the catalog, then the library.
async fn address_of<R: Runtime>(app: &AppHandle<R>, id: Uuid) -> Option<String> {
    let state = app.state::<SharedState>();
    let known = lock(&state).result_artwork(id);
    if known.is_some() {
        return known;
    }
    // The catalog's tracks.
    if let Some(track) = lock(&state).known_track(id) {
        return track.artwork_url;
    }
    let library = app.try_state::<LibraryHandle>()?.inner().clone();
    blocking(move || Ok(library.library()?.artwork_url(id)?))
        .await
        .ok()
        .flatten()
}

/// The protocol handler: only the app's own window may ask.
pub async fn serve<R: Runtime>(app: &AppHandle<R>, webview: &str, path: &str) -> Response<Vec<u8>> {
    if webview != MAIN_WINDOW {
        return Reply::status(403).into_response();
    }
    let Some(handle) = app.try_state::<ArtHandle>() else {
        return Reply::status(404).into_response();
    };
    let Ok(cache) = handle.cache().await else {
        return Reply::status(404).into_response();
    };
    respond(cache, path, async |id| address_of(app, id).await)
        .await
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use reqwest::Url;

    use super::*;
    use crate::art::FetchError;

    const ID: &str = "0b9d3f0e-5c5e-5a53-8f6a-3c1f6a7d2b11";

    #[derive(Clone, Default)]
    struct Host(Arc<AtomicUsize>);

    impl Fetch for Host {
        async fn get(&self, _url: Url) -> Result<Vec<u8>, FetchError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let image = image::RgbImage::from_pixel(120, 80, image::Rgb([200, 30, 90]));
            let mut bytes = Vec::new();
            image
                .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();
            Ok(bytes)
        }
    }

    struct Still;

    impl Clock for Still {
        fn now(&self) -> Duration {
            Duration::ZERO
        }

        fn jitter_ms(&self) -> u64 {
            0
        }
    }

    fn cache(host: &Host) -> (tempfile::TempDir, ArtCache<Host, Still>) {
        let dir = tempfile::tempdir().unwrap();
        let cache = ArtCache::open(host.clone(), Still, dir.path().to_path_buf(), 1 << 20).unwrap();
        (dir, cache)
    }

    #[tokio::test]
    async fn a_known_track_gets_a_jpeg() {
        let host = Host::default();
        let (_dir, cache) = cache(&host);

        let reply = respond(&cache, &format!("/art/64/{ID}"), async |_| {
            Some("https://i.ytimg.com/vi/a/hq720.jpg".to_owned())
        })
        .await;

        assert_eq!(reply.status, 200);
        assert_eq!(&reply.body[..3], [0xFF, 0xD8, 0xFF], "a JPEG starts so");
        let response = reply.into_response();
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }

    #[tokio::test]
    async fn a_path_that_names_no_picture_is_refused_before_anything_is_looked_up() {
        let host = Host::default();
        let (_dir, cache) = cache(&host);

        for path in ["/art/64/../../x", "/art/65/", "/other"] {
            let reply = respond(&cache, path, async |_| -> Option<String> {
                panic!("nothing should be looked up for {path}")
            })
            .await;
            assert_eq!(reply, Reply::status(400), "{path}");
        }
        assert_eq!(host.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_track_without_a_picture_is_not_found() {
        let host = Host::default();
        let (_dir, cache) = cache(&host);

        let reply = respond(&cache, &format!("/art/300/{ID}"), async |_| None).await;

        assert_eq!(reply, Reply::status(404));
    }

    #[tokio::test]
    async fn an_address_off_the_allowlist_is_not_found_and_not_fetched() {
        let host = Host::default();
        let (_dir, cache) = cache(&host);

        let reply = respond(&cache, &format!("/art/64/{ID}"), async |_| {
            Some("https://evil.example/a.jpg".to_owned())
        })
        .await;

        assert_eq!(reply, Reply::status(404));
        assert_eq!(host.0.load(Ordering::SeqCst), 0);
    }
}
