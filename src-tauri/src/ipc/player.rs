//! Player commands in, player events out. The rules live in `audio`.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;

use tauri::{AppHandle, Manager, Runtime, State};
use uuid::Uuid;

use super::{EventSink, IpcError, SharedState, lock};
use crate::audio::{
    Player, PlayerError, PlayerEvent, PlayerState, QueueSlice, SimulatedEngine, SystemClock, Wake,
    ticker,
};
use crate::catalog::Catalog;
use crate::recs::StaticRecommender;

/// Events sent to the UI. Mirrored in `src/lib/ipc.ts`.
pub const PLAYER_CHANGED: &str = "player-changed";
pub const PLAYER_POSITION: &str = "player-position";

type AppPlayer = Player<SimulatedEngine>;

/// The player behind its own lock, so playback never waits for a search.
pub struct PlayerHandle {
    player: Arc<Mutex<AppPlayer>>,
    wake: Wake,
}

impl PlayerHandle {
    /// A player over `catalog`, and what its clock thread waits on.
    fn new(catalog: Arc<Catalog>) -> (Self, Receiver<()>) {
        let engine = SimulatedEngine::new(SystemClock::default());
        let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
        let player = Arc::new(Mutex::new(Player::new(engine, catalog, recommender)));
        let (wake, woken) = Wake::channel();
        (Self { player, wake }, woken)
    }
}

/// Sets the player up and starts its clock.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> std::io::Result<()> {
    let catalog = lock(app.state::<SharedState>().inner()).catalog();
    let (handle, woken) = PlayerHandle::new(catalog);
    let player = Arc::clone(&handle.player);
    app.manage(handle);
    let app = app.clone();
    thread::Builder::new()
        .name("player-clock".to_owned())
        .spawn(move || {
            ticker::run(&player, woken, |event| {
                // A failure means the window is gone: nobody is left to tell.
                let _ = announce(&event, &app);
            });
        })?;
    Ok(())
}

/// Sends what the clock found out.
fn announce(event: &PlayerEvent, sink: &impl EventSink) -> Result<(), IpcError> {
    match event {
        PlayerEvent::Changed(state) => sink.player_changed(state),
        PlayerEvent::Position(position) => sink.player_position(position),
    }
}

/// Runs one change on the player. Only a real change wakes the clock and is
/// announced; the answer to a command is the event.
fn command(
    handle: &PlayerHandle,
    sink: &impl EventSink,
    act: impl FnOnce(&mut AppPlayer) -> Result<Option<PlayerState>, PlayerError>,
) -> Result<(), IpcError> {
    // The lock is released before the event goes out. The revision keeps order.
    let changed = act(&mut lock(&handle.player))?;
    if let Some(state) = changed {
        handle.wake.wake();
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
    use crate::audio::{NowPlaying, PlaybackStatus, PlayerPosition, QueueItem};
    use crate::catalog::{SourceKind, SourceTag, TrackRow};

    fn handle() -> (PlayerHandle, Receiver<()>) {
        PlayerHandle::new(Arc::default())
    }

    fn play(handle: &PlayerHandle, sink: &RecordingSink) {
        command(handle, sink, |p| {
            p.play_collection("night-shift", 0).map(Some)
        })
        .unwrap();
    }

    #[test]
    fn a_command_announces_the_new_state_once() {
        let (handle, woken) = handle();
        let sink = RecordingSink::default();

        play(&handle, &sink);

        assert_eq!(*sink.states.borrow(), [lock(&handle.player).state()]);
        assert_eq!(sink.states.borrow()[0].revision, 1);
        assert_eq!(woken.try_recv(), Ok(()));
    }

    #[test]
    fn a_command_does_not_wait_for_a_busy_clock() {
        let (handle, _woken) = handle();
        let sink = RecordingSink::default();

        // Nothing reads the wake channel here, so it is full after the first.
        play(&handle, &sink);
        play(&handle, &sink);

        assert_eq!(sink.states.borrow().len(), 2);
    }

    #[test]
    fn a_no_op_stays_quiet() {
        let (handle, woken) = handle();
        let sink = RecordingSink::default();

        command(&handle, &sink, |p| Ok(p.toggle_play())).unwrap();

        assert!(sink.states.borrow().is_empty());
        assert!(woken.try_recv().is_err());
    }

    #[test]
    fn a_failed_command_stays_quiet() {
        let (handle, woken) = handle();
        let sink = RecordingSink::default();

        let error = command(&handle, &sink, |p| p.play_collection("nope", 0).map(Some))
            .expect_err("there is no such collection");

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!("no playlist or album has the id “nope”")
        );
        assert!(sink.states.borrow().is_empty());
        assert!(woken.try_recv().is_err());
    }

    #[test]
    fn clock_events_go_out_as_changes_or_positions() {
        let (handle, _woken) = handle();
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
            upcoming: 5,
            queue_revision: 3,
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
                "upcoming": 5,
                "queueRevision": 3
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
            (PlaybackStatus::Paused, "paused"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(wire));
        }
    }
}
