//! Playing a live YouTube Music song: the player shows it at once, the resolver
//! finds its stream, and the player loads it if it is still wanted.

use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};

use super::account::AccountHandle;
use super::player::PlayerHandle;
use super::{EventSink, IpcError};
use crate::audio::PlaybackStatus;
use crate::catalog::TrackRow;
use crate::resolver::{Resolver, Sidecars, jar_dir};
use crate::sources::youtube::SystemClock;

/// yt-dlp's cache, in the app's local data folder.
const CACHE_FOLDER: &str = "yt-dlp-cache";

/// The resolver as the app runs it.
pub type AppResolver = Resolver<Sidecars, SystemClock>;

/// Sets the resolver up, which sweeps leftover cookie files. Runs nothing.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let cache = app.path().app_local_data_dir()?.join(CACHE_FOLDER);
    app.manage(Arc::new(AppResolver::new(
        Sidecars::bundled(),
        SystemClock::default(),
        jar_dir(),
        cache,
    )));
    Ok(())
}

/// Ends a resolve in progress, so no yt-dlp or Deno outlives the app.
pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    if let Some(resolver) = app.try_state::<Arc<AppResolver>>() {
        resolver.shut_down();
    }
}

/// Drops the stream of `video`, or every stream.
pub fn forget<R: Runtime>(app: &AppHandle<R>, video: Option<&str>) {
    if let Some(resolver) = app.try_state::<Arc<AppResolver>>() {
        match video {
            Some(video) => resolver.forget(video),
            None => resolver.forget_all(),
        }
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
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let resolver = Arc::clone(app.state::<Arc<AppResolver>>().inner());
        let youtube = Arc::clone(app.state::<AccountHandle>().youtube());
        let outcome = resolver
            .resolve(&youtube, &video, || pending.wanted())
            .await
            .map_err(|error| error.to_string());
        if let Some(state) = pending.finish(outcome) {
            // A stream the engine refused is not used again.
            if state.status == PlaybackStatus::Error {
                resolver.forget(&video);
            }
            // A failure means the window is gone: nobody is left to tell.
            let _ = app.player_changed(&state);
        }
    });
    Ok(())
}
