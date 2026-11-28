//! Carries out what the player's stream recovery asks for: one watcher wakes
//! when something changes or a deadline passes, resolves through the gate and
//! hands the player what came of it. The same watcher runs the resolves ahead
//! of the Queue, one at a time, and the library reads they need.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tokio::sync::Notify;

use super::playback::{AppResolver, youtube_ref};
use super::player::AppPlayer;
use super::youtube::Service;
use super::{EventSink, lock};
use crate::audio::{Fetch, Lookup, NoStream, PlayerState};
use crate::resolver::{ResolveError, Resolver};
use crate::secrets::SecretStore;
use crate::sources::youtube::{GateClock, GateView, Transport, YouTube};

/// What the watcher does next.
#[derive(Debug)]
enum Next {
    /// Resolve this for the current song, then look again.
    Fetch(Fetch),
    /// Resolve this before its turn, then look again.
    Ahead(Fetch),
    /// Read the library for the entries to come, then look again.
    LookUp(Lookup),
    /// Nothing to do until then, or until something changes.
    Sleep(Option<Duration>),
}

/// Work the watcher's loop does itself, between its looks at the player.
type Work<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Looks after the player for as long as the app runs. `wake` is told of every
/// change to the player.
pub async fn watch<R: Runtime>(
    app: AppHandle<R>,
    player: Arc<Mutex<AppPlayer>>,
    youtube: Arc<Service>,
    resolver: Arc<AppResolver>,
    wake: Arc<Notify>,
) {
    let start = |request| {
        let (app, player) = (app.clone(), Arc::clone(&player));
        let resolver = Arc::clone(&resolver);
        // On its own, so it can be cancelled while it runs.
        tauri::async_runtime::spawn(async move {
            fetch(&player, resolver.as_ref(), &app, request).await;
        });
    };
    look_after(&player, &youtube, resolver.as_ref(), &app, &wake, start).await;
}

/// The watcher's loop. `start` runs a fetch for the current song.
async fn look_after<S: SecretStore, T: Transport, G: GateClock>(
    player: &Mutex<AppPlayer>,
    youtube: &YouTube<S, T, G>,
    resolver: &impl Resolver,
    sink: &impl EventSink,
    wake: &Notify,
    mut start: impl FnMut(Fetch),
) {
    // A cool-down or an expired session shows here.
    let mut statuses = youtube.watch();
    // The resolve ahead runs in here, one at a time, so that it can be dropped,
    // which ends its run.
    let mut ahead: Option<(u32, Work<Option<PlayerState>>)> = None;
    let mut looking: Option<Work<()>> = None;
    let mut account = youtube.account().generation();
    loop {
        let generation = youtube.account().generation();
        if std::mem::replace(&mut account, generation) != generation {
            // What was found ahead belonged to the other session.
            lock(player).stop_ahead();
        }
        let next = step(player, youtube, resolver, sink);
        ahead = ahead.filter(|(ticket, _)| lock(player).awaits_ahead(*ticket));
        match next {
            Next::Fetch(request) => start(request),
            Next::Ahead(request) => {
                let ticket = request.ticket;
                let run = resolve_ahead(player, resolver, request);
                ahead = Some((ticket, Box::pin(run)));
            }
            Next::LookUp(lookup) => looking = Some(Box::pin(look_up(player, lookup))),
            Next::Sleep(wait) => tokio::select! {
                () = wake.notified() => {}
                changed = statuses.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
                state = finished(ahead.as_mut().map(|(_, run)| run)), if ahead.is_some() => {
                    ahead = None;
                    announce(state, sink);
                }
                () = finished(looking.as_mut()), if looking.is_some() => looking = None,
            },
        }
    }
}

/// Returns once `work` is done; never without any.
async fn finished<T>(work: Option<&mut Work<'_, T>>) -> T {
    match work {
        Some(work) => work.await,
        None => std::future::pending().await,
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
    // Nothing is resolved ahead while the gate is closed.
    let open = youtube.status().gate == GateView::Ready;
    let mut player = lock(player);
    if open {
        player.review_ahead();
    } else {
        player.stop_ahead();
    }
    if let Some(request) = player.due_fetch() {
        return Next::Fetch(request);
    }
    if open {
        if let Some(lookup) = player.due_lookup() {
            return Next::LookUp(lookup);
        }
        if let Some(request) = player.due_ahead() {
            return Next::Ahead(request);
        }
    }
    Next::Sleep(player.recovery_wake())
}

/// Reads the library on a blocking thread, with the player free meanwhile.
async fn look_up(player: &Mutex<AppPlayer>, lookup: Lookup) {
    let lost = lookup.unanswered();
    let found = tokio::task::spawn_blocking(move || lookup.run()).await;
    lock(player).learn(found.unwrap_or(lost));
}

/// Resolves an entry before its turn. The resolver keeps the stream, which the
/// entry's own fetch then finds. Dropping this ends the run. The change to
/// announce, if the current song was waiting and got none.
async fn resolve_ahead(
    player: &Mutex<AppPlayer>,
    resolver: &impl Resolver,
    request: Fetch,
) -> Option<PlayerState> {
    let wanted = || lock(player).may_run_ahead(request.ticket);
    let outcome = resolver.resolve(&youtube_ref(&request.video), wanted).await;
    let outcome = outcome.map(|stream| stream.expires_at).map_err(no_stream);
    lock(player).finish_ahead(request.ticket, outcome)
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
        .map_err(no_stream);
    let finished = lock(player).finish_fetch(request.ticket, outcome);
    announce(finished, sink);
}

/// What the player makes of a resolve that gave no stream.
fn no_stream(error: ResolveError) -> NoStream {
    NoStream {
        reason: error.to_string(),
        retry: error.worth_retrying(),
        skip: error.is_about_the_song(),
    }
}

fn announce(state: Option<PlayerState>, sink: &impl EventSink) {
    if let Some(state) = state {
        // A failure means the window is gone: nobody is left to tell.
        let _ = sink.player_changed(&state);
    }
}

#[cfg(test)]
mod ahead_tests;
#[cfg(test)]
mod queue_tests;
#[cfg(test)]
mod tests;
