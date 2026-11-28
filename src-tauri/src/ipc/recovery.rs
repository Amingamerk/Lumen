//! Carries out what the player's stream recovery asks for: one watcher wakes
//! when something changes or a deadline passes, resolves through the gate and
//! hands the player what came of it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tokio::sync::Notify;

use super::playback::{AppResolver, youtube_ref};
use super::player::AppPlayer;
use super::youtube::Service;
use super::{EventSink, lock};
use crate::audio::{Fetch, NoStream, PlayerState};
use crate::resolver::Resolver;
use crate::secrets::SecretStore;
use crate::sources::youtube::{GateClock, Transport, YouTube};

/// What the watcher does next.
#[derive(Debug)]
enum Next {
    /// Resolve this, then look again.
    Fetch(Fetch),
    /// Nothing to do until then, or until something changes.
    Sleep(Option<Duration>),
}

/// Looks after the player for as long as the app runs. `wake` is told of every
/// change to the player.
pub async fn watch<R: Runtime>(
    app: AppHandle<R>,
    player: Arc<Mutex<AppPlayer>>,
    youtube: Arc<Service>,
    resolver: Arc<AppResolver>,
    wake: Arc<Notify>,
) {
    // A cool-down or an expired session shows here.
    let mut statuses = youtube.watch();
    loop {
        match step(&player, &youtube, resolver.as_ref(), &app) {
            Next::Fetch(request) => {
                let (app, player) = (app.clone(), Arc::clone(&player));
                let resolver = Arc::clone(&resolver);
                // On its own, so it can be cancelled while it runs.
                tauri::async_runtime::spawn(async move {
                    fetch(&player, resolver.as_ref(), &app, request).await;
                });
            }
            Next::Sleep(wait) => tokio::select! {
                () = wake.notified() => {}
                changed = statuses.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
            },
        }
    }
}

/// One look at the player: finds a stall, tells the gate of a recovery that gave
/// up, cancels one that may no longer send, and says what to do next.
fn step<S: SecretStore, T: Transport, G: GateClock>(
    player: &Mutex<AppPlayer>,
    youtube: &YouTube<S, T, G>,
    resolver: &impl Resolver,
    sink: &impl EventSink,
) -> Next {
    // The player's lock is never held while YouTube's state is asked or changed.
    let stalled = lock(player).check_stall();
    announce(stalled, sink);
    let gave_up = lock(player).take_gave_up();
    if gave_up {
        youtube.count_failure();
    }
    let pending = lock(player).fetch_pending();
    if pending && let Err(blocked) = youtube.ready() {
        let abandoned = lock(player).abandon_fetch(&blocked.to_string());
        announce(abandoned, sink);
    }
    let dropped = lock(player).take_dropped();
    if let Some(video) = dropped {
        resolver.forget(&youtube_ref(&video));
    }
    let mut player = lock(player);
    match player.due_fetch() {
        Some(request) => Next::Fetch(request),
        None => Next::Sleep(player.recovery_wake()),
    }
}

/// Resolves the stream asked for and hands the player the outcome, unless
/// nobody waits for it any more.
async fn fetch(
    player: &Mutex<AppPlayer>,
    resolver: &impl Resolver,
    sink: &impl EventSink,
    request: Fetch,
) {
    let wanted = || lock(player).awaits_fetch(request.ticket);
    let outcome = resolver
        .resolve(&youtube_ref(&request.video), wanted)
        .await
        .map(|stream| stream.media())
        .map_err(|error| NoStream {
            reason: error.to_string(),
            retry: error.worth_retrying(),
            skip: error.is_about_the_song(),
        });
    let finished = lock(player).finish_fetch(request.ticket, outcome);
    announce(finished, sink);
}

fn announce(state: Option<PlayerState>, sink: &impl EventSink) {
    if let Some(state) = state {
        // A failure means the window is gone: nobody is left to tell.
        let _ = sink.player_changed(&state);
    }
}

#[cfg(test)]
mod queue_tests;
#[cfg(test)]
mod tests;
