//! Playing a live YouTube Music song: the player shows it at once, the resolver
//! finds its stream, and the player loads it if it is still wanted.

use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};

use super::account::AccountHandle;
#[cfg(debug_assertions)]
use super::break_url::BreakUrl;
use super::player::{Pending, PlayerHandle};
use super::{EventSink, IpcError};
use crate::catalog::{SourceKind, SourceRef, TrackRow};
use crate::resolver::{Resolver, Sidecars, YtDlpResolver, jar_dir};
use crate::secrets::WindowsStore;
use crate::sources::youtube::{HttpTransport, SystemClock};

/// yt-dlp's cache, in the app's local data folder.
const CACHE_FOLDER: &str = "yt-dlp-cache";

/// The resolver as the app runs it.
pub type AppResolver =
    YtDlpResolver<Sidecars, SystemClock, WindowsStore, HttpTransport, SystemClock>;

/// Sets the resolver up, which sweeps leftover cookie files. Runs nothing.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let cache = app.path().app_local_data_dir()?.join(CACHE_FOLDER);
    let youtube = Arc::clone(app.state::<AccountHandle>().youtube());
    let mut resolver = AppResolver::new(
        Sidecars::bundled(),
        SystemClock::default(),
        youtube,
        jar_dir(),
        cache,
    );
    if let Some(library) = super::library::open_shared(app) {
        resolver = resolver.keeping_in(library);
    }
    app.manage(Arc::new(resolver));
    #[cfg(debug_assertions)]
    app.manage(BreakUrl::from_args(std::env::args_os()));
    Ok(())
}

/// Ends a resolve in progress, so no yt-dlp or Deno outlives the app.
pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    if let Some(resolver) = app.try_state::<Arc<AppResolver>>() {
        resolver.shut_down();
    }
}

/// Drops every stream, as the account has changed. The library is written on a
/// blocking thread, never under the caller's lock.
pub fn forget_all<R: Runtime>(app: &AppHandle<R>) {
    if let Some(resolver) = app.try_state::<Arc<AppResolver>>() {
        let resolver = Arc::clone(resolver.inner());
        tauri::async_runtime::spawn_blocking(move || resolver.forget_all());
    }
}

/// The song of a YouTube video id.
pub fn youtube_ref(video: &str) -> SourceRef {
    SourceRef {
        kind: SourceKind::YouTube,
        id: video.to_owned(),
    }
}

/// Shows the song as loading, then resolves and plays it in the background.
pub fn play_stream<R: Runtime>(
    app: &AppHandle<R>,
    player: &PlayerHandle,
    video: String,
    row: TrackRow,
) -> Result<(), IpcError> {
    let pending = player.begin_stream(app, video.clone(), row)?;
    resolve_in_background(app, pending, video);
    Ok(())
}

/// Resolves `video` for the song that waits for it, and plays it.
pub fn resolve_in_background<R: Runtime>(app: &AppHandle<R>, pending: Pending, video: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let resolver = Arc::clone(app.state::<Arc<AppResolver>>().inner());
        let outcome = resolver
            .resolve(&youtube_ref(&video), || pending.wanted())
            .await
            .map(|stream| stream.media())
            .map_err(|error| error.to_string());
        #[cfg(debug_assertions)]
        let outcome = outcome.map(|media| match pending.wanted() {
            true => app.state::<BreakUrl>().apply(media),
            false => media,
        });
        if let Some(state) = pending.finish(outcome) {
            // A failure means the window is gone: nobody is left to tell.
            let _ = app.player_changed(&state);
        }
    });
}
