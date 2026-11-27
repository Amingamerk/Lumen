//! Player commands in, player events out. The rules live in `audio`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime, State};
use uuid::Uuid;

use super::playback::{self, play_stream};
use super::{EventSink, IpcError, SharedState, lock};
use crate::audio::{
    Engine, EngineEvent, Media, MpvEngine, MpvEvents, MpvShutdown, Player, PlayerError,
    PlayerEvent, PlayerState, QueueSlice, file_media,
};
use crate::catalog::{Catalog, TrackRow};
use crate::recs::StaticRecommender;
use crate::state::SearchPick;

/// Events sent to the UI. Mirrored in `src/lib/ipc.ts`.
pub const PLAYER_CHANGED: &str = "player-changed";
pub const PLAYER_POSITION: &str = "player-position";

/// libmpv in the app, stand-ins in tests.
type AppPlayer = Player<Box<dyn Engine>>;

/// The player behind its own lock, so playback never waits for a search.
pub struct PlayerHandle {
    player: Arc<Mutex<AppPlayer>>,
}

impl PlayerHandle {
    fn new(catalog: Arc<Catalog>, engine: Box<dyn Engine>) -> Self {
        let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
        let player = Arc::new(Mutex::new(Player::new(engine, catalog, recommender)));
        Self { player }
    }

    /// Plays a file before the UI is there to ask: it reads the state later.
    fn play_file(&self, path: PathBuf) {
        let (media, row) = file_media(path);
        lock(&self.player).play_media(media, row);
    }
}

/// A song waiting for its stream. It outlives the command that chose it.
pub struct Pending {
    player: Arc<Mutex<AppPlayer>>,
    key: u32,
}

impl Pending {
    /// Whether it is still the song to play.
    pub fn wanted(&self) -> bool {
        lock(&self.player).awaits(self.key)
    }

    /// The change the outcome made, if it was still wanted.
    pub fn finish(&self, outcome: Result<Media, String>) -> Option<PlayerState> {
        lock(&self.player).finish_stream(self.key, outcome)
    }
}

impl PlayerHandle {
    /// Shows `row` as loading and announces it.
    pub fn begin_stream(
        &self,
        sink: &impl EventSink,
        video: String,
        row: TrackRow,
    ) -> Result<Pending, IpcError> {
        // The lock is released before the event goes out. The revision keeps order.
        let (key, state) = lock(&self.player).begin_stream(video, row);
        sink.player_changed(&state)?;
        Ok(Pending {
            player: Arc::clone(&self.player),
            key,
        })
    }
}

/// How long [`AudioThread::stop`] waits for mpv to shut down.
const STOP_PATIENCE: Duration = Duration::from_secs(2);

/// The thread that reads mpv's events. It holds the player, and with it the
/// engine, so dropping the engine cannot end it: [`AudioThread::stop`] does.
pub struct AudioThread {
    shutdown: MpvShutdown,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl AudioThread {
    fn new(shutdown: MpvShutdown, thread: JoinHandle<()>) -> Self {
        Self {
            shutdown,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// Tells mpv to quit and waits a little for the thread to end. Whether it
    /// ended; if not, the process exit ends it.
    pub fn stop(&self) -> bool {
        self.shutdown.shut_down();
        let Some(thread) = lock(&self.thread).take() else {
            return true;
        };
        let started = Instant::now();
        while !thread.is_finished() {
            if started.elapsed() >= STOP_PATIENCE {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        thread.join().is_ok()
    }
}

/// Sets the player up over libmpv, plays `file` if the launch asked for one,
/// and starts listening to mpv.
pub fn start<R: Runtime>(app: &AppHandle<R>, file: Option<PathBuf>) -> anyhow::Result<()> {
    let catalog = lock(app.state::<SharedState>().inner()).catalog();
    let (engine, events) = MpvEngine::new()?;
    let shutdown = engine.shutdown_handle();
    let handle = PlayerHandle::new(catalog, Box::new(engine));
    if let Some(path) = file {
        handle.play_file(path);
    }
    let player = Arc::clone(&handle.player);
    app.manage(handle);
    let sink = app.clone();
    let thread = listen(events, move |event| {
        // A failure means the window is gone: nobody is left to tell.
        let _ = relay(&player, event, &sink);
        if event == EngineEvent::Failed {
            // A URL that would not play is not used again.
            let failed = lock(&player).failed_stream();
            if let Some((video, _)) = failed {
                playback::forget(&sink, Some(&video));
            }
        }
    })?;
    app.manage(AudioThread::new(shutdown, thread));
    Ok(())
}

/// Ends playback before the app exits, so mpv shuts down cleanly.
pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    if let Some(audio) = app.try_state::<AudioThread>() {
        audio.stop();
    }
}

/// Hands each event mpv reports to `heard`, on a thread of its own, until mpv
/// shuts down.
fn listen(
    mut events: MpvEvents,
    mut heard: impl FnMut(EngineEvent) + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("mpv-events".to_owned())
        .spawn(move || {
            while let Some(event) = events.wait() {
                heard(event);
            }
        })
}

/// Gives the player what the engine heard, and tells the UI what came of it.
fn relay(
    player: &Mutex<AppPlayer>,
    event: EngineEvent,
    sink: &impl EventSink,
) -> Result<(), IpcError> {
    // The lock is released before the event goes out. The revision keeps order.
    let heard = lock(player).on_engine_event(event);
    match heard {
        Some(heard) => announce(&heard, sink),
        None => Ok(()),
    }
}

/// Sends what the engine found out.
fn announce(event: &PlayerEvent, sink: &impl EventSink) -> Result<(), IpcError> {
    match event {
        PlayerEvent::Changed(state) => sink.player_changed(state),
        PlayerEvent::Position(position) => sink.player_position(position),
    }
}

/// Runs one change on the player. Only a real change is announced; the answer to
/// a command is the event.
fn command(
    handle: &PlayerHandle,
    sink: &impl EventSink,
    act: impl FnOnce(&mut AppPlayer) -> Result<Option<PlayerState>, PlayerError>,
) -> Result<(), IpcError> {
    // The lock is released before the event goes out. The revision keeps order.
    let changed = act(&mut lock(&handle.player))?;
    if let Some(state) = changed {
        sink.player_changed(&state)?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_player_state(player: State<'_, PlayerHandle>) -> PlayerState {
    lock(&player.player).state()
}

#[tauri::command]
pub fn play_collection<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    id: String,
    index: usize,
) -> Result<(), IpcError> {
    command(&player, &app, |p| p.play_collection(&id, index).map(Some))
}

#[tauri::command]
pub fn play_search_result<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    player: State<'_, PlayerHandle>,
    track_id: Uuid,
) -> Result<(), IpcError> {
    // The app's lock is released before the player's is taken.
    let pick = lock(&state).search_pick(track_id)?;
    match pick {
        SearchPick::Samples { tracks, start } => {
            command(&player, &app, |p| p.play_tracks(tracks, start).map(Some))
        }
        SearchPick::Live { video, row } => play_stream(&app, &player, video, row),
    }
}

#[tauri::command]
pub fn toggle_play<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
) -> Result<(), IpcError> {
    let failed = lock(&player.player).failed_stream();
    match failed {
        Some((video, row)) => play_stream(&app, &player, video, row),
        None => command(&player, &app, |p| Ok(p.toggle_play())),
    }
}

#[tauri::command]
pub fn next_track<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.next_track()))
}

#[tauri::command]
pub fn previous_track<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.previous_track()))
}

#[tauri::command]
pub fn seek<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    position_ms: i64,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.seek(position_ms)))
}

#[tauri::command]
pub fn seek_by<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    delta_ms: i64,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.seek_by(delta_ms)))
}

#[tauri::command]
pub fn set_volume<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    volume: i64,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.set_volume(volume)))
}

#[tauri::command]
pub fn change_volume<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    delta: i64,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.change_volume(delta)))
}

/// The visible window of the upcoming entries.
#[tauri::command]
pub fn get_queue(player: State<'_, PlayerHandle>, offset: usize, limit: usize) -> QueueSlice {
    lock(&player.player).queue_slice(offset, limit)
}

#[tauri::command]
pub fn move_queue_entry<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    key: u32,
    before: Option<u32>,
) -> Result<(), IpcError> {
    command(&player, &app, |p| p.move_queue_entry(key, before))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::RecordingSink;
    use super::*;
    use crate::audio::{
        FakeEngine, NowPlaying, PlaybackStatus, PlayerPosition, QueueItem, SimulatedEngine,
        SystemClock,
    };
    use crate::catalog::{SourceKind, SourceTag};

    /// A player that can play the catalog's samples.
    fn handle() -> PlayerHandle {
        let engine = SimulatedEngine::new(SystemClock::default());
        PlayerHandle::new(Arc::default(), Box::new(engine))
    }

    /// A player over an engine like libmpv: real audio only.
    fn real() -> (PlayerHandle, FakeEngine) {
        let engine = FakeEngine::default();
        let handle = PlayerHandle::new(Arc::default(), Box::new(engine.clone()));
        (handle, engine)
    }

    #[test]
    fn stopping_ends_the_event_thread_while_it_holds_the_player() {
        let (engine, events) = MpvEngine::silent().expect("libmpv-2.dll starts");
        let shutdown = engine.shutdown_handle();
        let handle = PlayerHandle::new(Arc::default(), Box::new(engine));
        let player = Arc::clone(&handle.player);
        let thread = listen(events, move |_| {
            let _ = lock(&player).state();
        })
        .unwrap();
        let audio = AudioThread::new(shutdown, thread);

        assert!(audio.stop(), "the event thread is still running");
        drop(handle);
    }

    fn play(handle: &PlayerHandle, sink: &RecordingSink) -> Result<(), IpcError> {
        command(handle, sink, |p| {
            p.play_collection("night-shift", 0).map(Some)
        })
    }

    #[test]
    fn a_command_announces_the_new_state_once() {
        let handle = handle();
        let sink = RecordingSink::default();

        play(&handle, &sink).unwrap();

        assert_eq!(*sink.states.borrow(), [lock(&handle.player).state()]);
        assert_eq!(sink.states.borrow()[0].revision, 1);
    }

    #[test]
    fn a_no_op_stays_quiet() {
        let handle = handle();
        let sink = RecordingSink::default();

        command(&handle, &sink, |p| Ok(p.toggle_play())).unwrap();

        assert!(sink.states.borrow().is_empty());
    }

    #[test]
    fn a_failed_command_stays_quiet() {
        let handle = handle();
        let sink = RecordingSink::default();

        let error = command(&handle, &sink, |p| p.play_collection("nope", 0).map(Some))
            .expect_err("there is no such collection");

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!("no playlist or album has the id “nope”")
        );
        assert!(sink.states.borrow().is_empty());
    }

    #[test]
    fn a_sample_track_is_refused_in_words_and_changes_nothing() {
        let (handle, _) = real();
        let sink = RecordingSink::default();

        let error = play(&handle, &sink).expect_err("samples have no audio");

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!("sample track, cannot be played")
        );
        assert!(sink.states.borrow().is_empty());
    }

    #[test]
    fn a_launch_file_is_loaded_and_the_engine_is_relayed() {
        let (handle, engine) = real();
        let sink = RecordingSink::default();

        handle.play_file(PathBuf::from(r"C:\music\tone.wav"));
        let state = lock(&handle.player).state();
        assert_eq!(state.status, PlaybackStatus::Loading);
        assert_eq!(state.current.unwrap().track.title, "tone");

        engine.fake().duration = std::time::Duration::from_secs(9);
        relay(&handle.player, EngineEvent::Loaded, &sink).unwrap();
        engine.fake().position = std::time::Duration::from_secs(2);
        relay(&handle.player, EngineEvent::Progress, &sink).unwrap();
        relay(&handle.player, EngineEvent::Progress, &sink).unwrap();
        relay(&handle.player, EngineEvent::Failed, &sink).unwrap();

        let statuses: Vec<PlaybackStatus> = sink.states.borrow().iter().map(|s| s.status).collect();
        assert_eq!(statuses, [PlaybackStatus::Playing, PlaybackStatus::Error]);
        assert_eq!(sink.positions.borrow().len(), 1);
    }

    #[test]
    fn engine_events_go_out_as_changes_or_positions() {
        let handle = handle();
        let sink = RecordingSink::default();
        let state = lock(&handle.player).state();
        let position = PlayerPosition {
            revision: 2,
            position_ms: 3_000,
        };

        announce(&PlayerEvent::Changed(state.clone()), &sink).unwrap();
        announce(&PlayerEvent::Position(position), &sink).unwrap();

        assert_eq!(*sink.states.borrow(), [state]);
        assert_eq!(*sink.positions.borrow(), [position]);
    }

    #[test]
    fn player_payloads_serialize_for_the_ui() {
        let track = TrackRow {
            id: Uuid::from_u128(7),
            title: "Harbor Lights".to_owned(),
            artists: vec!["Mira Sol".to_owned()],
            album: None,
            duration_ms: 201_000,
            sources: vec![SourceTag {
                source: SourceKind::YouTube,
                badge: "YT",
            }],
        };
        let wire_track = serde_json::to_value(&track).unwrap();
        let state = PlayerState {
            revision: 4,
            status: PlaybackStatus::Playing,
            current: Some(NowPlaying {
                key: 2,
                track: track.clone(),
                autoplay: false,
            }),
            position_ms: 1_000,
            duration_ms: 201_000,
            volume: 80,
            can_next: true,
            can_previous: true,
            upcoming: 5,
            queue_revision: 3,
            error: None,
        };

        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            json!({
                "revision": 4,
                "status": "playing",
                "current": { "key": 2, "track": wire_track, "autoplay": false },
                "positionMs": 1_000,
                "durationMs": 201_000,
                "volume": 80,
                "canNext": true,
                "canPrevious": true,
                "upcoming": 5,
                "queueRevision": 3,
                "error": null
            })
        );
        assert_eq!(
            serde_json::to_value(PlayerPosition {
                revision: 4,
                position_ms: 2_000
            })
            .unwrap(),
            json!({ "revision": 4, "positionMs": 2_000 })
        );
        let slice = QueueSlice {
            queue_revision: 3,
            offset: 0,
            total: 5,
            items: vec![QueueItem {
                key: 8,
                track,
                autoplay: true,
            }],
        };
        assert_eq!(
            serde_json::to_value(&slice).unwrap(),
            json!({
                "queueRevision": 3,
                "offset": 0,
                "total": 5,
                "items": [{ "key": 8, "track": wire_track, "autoplay": true }]
            })
        );
        for (status, wire) in [
            (PlaybackStatus::Idle, "idle"),
            (PlaybackStatus::Loading, "loading"),
            (PlaybackStatus::Paused, "paused"),
            (PlaybackStatus::Ended, "ended"),
            (PlaybackStatus::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(wire));
        }
    }
}
