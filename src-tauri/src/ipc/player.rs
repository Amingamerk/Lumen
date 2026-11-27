//! Player commands in, player events out. The rules live in `audio`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use tauri::{AppHandle, Manager, Runtime, State};
use uuid::Uuid;

use super::{EventSink, IpcError, SharedState, lock};
use crate::audio::{
    Engine, EngineEvent, MpvEngine, Player, PlayerError, PlayerEvent, PlayerState, QueueSlice,
    file_media,
};
use crate::catalog::Catalog;
use crate::recs::StaticRecommender;

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

/// Sets the player up over libmpv, plays `file` if the launch asked for one,
/// and starts listening to mpv.
pub fn start<R: Runtime>(app: &AppHandle<R>, file: Option<PathBuf>) -> anyhow::Result<()> {
    let catalog = lock(app.state::<SharedState>().inner()).catalog();
    let (engine, mut events) = MpvEngine::new()?;
    let handle = PlayerHandle::new(catalog, Box::new(engine));
    if let Some(path) = file {
        handle.play_file(path);
    }
    let player = Arc::clone(&handle.player);
    app.manage(handle);
    let app = app.clone();
    thread::Builder::new()
        .name("mpv-events".to_owned())
        .spawn(move || {
            while let Some(event) = events.wait() {
                // A failure means the window is gone: nobody is left to tell.
                let _ = relay(&player, event, &app);
            }
        })?;
    Ok(())
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
    let (tracks, start) = lock(&state).search_queue(track_id)?;
    command(&player, &app, |p| p.play_tracks(tracks, start).map(Some))
}

#[tauri::command]
pub fn toggle_play<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.toggle_play()))
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
    use crate::catalog::{SourceKind, SourceTag, TrackRow};

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
