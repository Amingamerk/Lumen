//! Player commands in, player events out. The rules live in `audio`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime, State};
use tokio::sync::Notify;
use uuid::Uuid;

use super::account::AccountHandle;
use super::library::{LibraryHandle, blocking, find_track};
use super::playback::{AppResolver, play_stream, resolve_in_background};
use super::{EventSink, IpcError, SharedState, lock, recovery};
use crate::audio::{
    Engine, EngineEvent, Media, MpvEngine, MpvEvents, MpvShutdown, Placement, Player, PlayerError,
    PlayerEvent, PlayerState, QueueSlice, TrackLookup, file_media,
};
use crate::catalog::{Catalog, Track, TrackRow};
use crate::library::{Library, PlayLog};
use crate::queue::Repeat;
use crate::recs::StaticRecommender;
use crate::state::SearchPick;

/// Events sent to the UI. Mirrored in `src/lib/ipc.ts`.
pub const PLAYER_CHANGED: &str = "player-changed";
pub const PLAYER_POSITION: &str = "player-position";

/// libmpv in the app, stand-ins in tests.
pub(super) type AppPlayer = Player<Box<dyn Engine>>;

/// The settings shuffle and repeat are kept under.
const SHUFFLE_SETTING: &str = "shuffle";
const REPEAT_SETTING: &str = "repeat";

/// The player behind its own lock, so playback never waits for a search. Clones
/// share it, so one can move to a blocking thread.
#[derive(Clone)]
pub struct PlayerHandle {
    player: Arc<Mutex<AppPlayer>>,
    /// Held while shuffle and repeat are written.
    saving: Arc<Mutex<()>>,
}

impl PlayerHandle {
    fn new(catalog: Arc<Catalog>, engine: Box<dyn Engine>) -> Self {
        let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
        let player = Arc::new(Mutex::new(Player::new(engine, catalog, recommender)));
        Self {
            player,
            saving: Arc::default(),
        }
    }

    /// Where the plays of this run are written.
    fn log_to(&self, log: Box<dyn PlayLog>) {
        lock(&self.player).set_log(log);
    }

    /// The song that is current, as the library would keep it.
    pub fn current_track(&self) -> Option<Track> {
        lock(&self.player).current_track()
    }

    /// Plays a file before the UI is there to ask: it reads the state later.
    fn play_file(&self, path: PathBuf) {
        let (media, row) = file_media(path);
        lock(&self.player).play_media(media, row);
    }
}

/// The library's Tracks, found by id for the Queue.
struct LibraryTracks(Arc<Library>);

impl TrackLookup for LibraryTracks {
    fn track(&self, id: Uuid) -> Option<Track> {
        self.0.track(id).ok().flatten()
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
    /// Queues the Tracks by id, plays the one at `start` and announces it.
    pub fn play_list(
        &self,
        sink: &impl EventSink,
        tracks: Vec<Uuid>,
        start: usize,
    ) -> Result<(), IpcError> {
        command(self, sink, |p| p.play_list(tracks, start).map(Some))
    }

    /// Adds a song the library knows to the Queue and announces it.
    pub fn enqueue(
        &self,
        sink: &impl EventSink,
        track: Uuid,
        placement: Placement,
    ) -> Result<(), IpcError> {
        command(self, sink, |p| p.enqueue(track, placement))
    }

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

    /// Asks again for the stream that failed, if one did, and announces it.
    pub fn retry_stream(
        &self,
        sink: &impl EventSink,
    ) -> Result<Option<(String, Pending)>, IpcError> {
        // The lock is released before the event goes out. The revision keeps order.
        let retried = lock(&self.player).retry_stream();
        let Some((video, key, state)) = retried else {
            return Ok(None);
        };
        sink.player_changed(&state)?;
        let player = Arc::clone(&self.player);
        Ok(Some((video, Pending { player, key })))
    }
}

impl PlayerHandle {
    /// Takes up shuffle and repeat as the last run left them. A setting that
    /// is missing or cannot be read means off.
    fn restore_modes(&self, library: &Library) {
        let kept = |key| library.setting(key).ok().flatten();
        let shuffle = kept(SHUFFLE_SETTING).as_deref() == Some("on");
        let repeat = match kept(REPEAT_SETTING).as_deref() {
            Some("all") => Repeat::All,
            Some("one") => Repeat::One,
            _ => Repeat::Off,
        };
        let mut player = lock(&self.player);
        player.set_shuffle(shuffle);
        player.set_repeat(repeat);
    }

    /// Changes shuffle or repeat, announces it and keeps both for the next
    /// launch. Writes to the database, so it belongs on a blocking thread.
    fn change_modes(
        &self,
        sink: &impl EventSink,
        library: Option<&Library>,
        act: impl FnOnce(&mut AppPlayer) -> Option<PlayerState>,
    ) -> Result<(), IpcError> {
        let announced = command(self, sink, |p| Ok(act(p)));
        if let Some(library) = library {
            self.save_modes(library);
        }
        announced
    }

    fn save_modes(&self, library: &Library) {
        // The player is read once it is this write's turn, so the last write
        // holds the latest modes.
        let _turn = lock(&self.saving);
        let (shuffle, repeat) = lock(&self.player).modes();
        let shuffle = if shuffle { "on" } else { "off" };
        let repeat = match repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        };
        let saved = library
            .set_setting(SHUFFLE_SETTING, shuffle)
            .and_then(|()| library.set_setting(REPEAT_SETTING, repeat));
        // The modes hold for this run either way.
        if let Err(error) = saved {
            eprintln!("lumen: could not keep shuffle and repeat: {error}");
        }
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
    if let Some(writer) = super::library::play_log(app) {
        handle.log_to(Box::new(writer));
    }
    if let Some(library) = super::library::open_shared(app) {
        handle.restore_modes(&library);
        lock(&handle.player).set_tracks(Box::new(LibraryTracks(library)));
    }
    if let Some(path) = file {
        handle.play_file(path);
    }
    let player = Arc::clone(&handle.player);
    let wake = Arc::new(Notify::new());
    let changed = Arc::clone(&wake);
    lock(&player).on_change(move || changed.notify_one());
    app.manage(handle);
    tauri::async_runtime::spawn(recovery::watch(
        app.clone(),
        Arc::clone(&player),
        Arc::clone(app.state::<AccountHandle>().youtube()),
        Arc::clone(app.state::<Arc<AppResolver>>().inner()),
        wake,
    ));
    let sink = app.clone();
    let thread = listen(events, move |event| {
        // A failure means the window is gone: nobody is left to tell.
        let _ = relay(&player, event, &sink);
    })?;
    app.manage(AudioThread::new(shutdown, thread));
    Ok(())
}

/// Ends playback before the app exits, so mpv shuts down cleanly.
pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    if let Some(audio) = app.try_state::<AudioThread>() {
        audio.stop();
    }
    // The song that plays now ends with the app, and the history says so.
    if let Some(handle) = app.try_state::<PlayerHandle>() {
        lock(&handle.player).shutdown();
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
    match player.retry_stream(&app)? {
        Some((video, pending)) => {
            resolve_in_background(&app, pending, video);
            Ok(())
        }
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

#[tauri::command]
pub async fn toggle_shuffle<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    library: State<'_, LibraryHandle>,
) -> Result<(), IpcError> {
    let (player, library) = (player.inner().clone(), library.inner().clone());
    blocking(move || {
        let library = library.library().ok().map(Arc::as_ref);
        player.change_modes(&app, library, AppPlayer::toggle_shuffle)
    })
    .await
}

/// Moves the repeat mode on: off, all, one.
#[tauri::command]
pub async fn cycle_repeat<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    library: State<'_, LibraryHandle>,
) -> Result<(), IpcError> {
    let (player, library) = (player.inner().clone(), library.inner().clone());
    blocking(move || {
        let library = library.library().ok().map(Arc::as_ref);
        player.change_modes(&app, library, AppPlayer::cycle_repeat)
    })
    .await
}

/// The visible window of the upcoming entries. Their rows come from the library,
/// so this waits on a blocking thread.
#[tauri::command]
pub async fn get_queue(
    player: State<'_, PlayerHandle>,
    offset: usize,
    limit: usize,
) -> Result<QueueSlice, IpcError> {
    let player = Arc::clone(&player.player);
    blocking(move || Ok(lock(&player).queue_slice(offset, limit))).await
}

/// The visible window of the entries played before the current one, the latest
/// first.
#[tauri::command]
pub async fn get_queue_history(
    player: State<'_, PlayerHandle>,
    offset: usize,
    limit: usize,
) -> Result<QueueSlice, IpcError> {
    let player = Arc::clone(&player.player);
    blocking(move || Ok(lock(&player).history_slice(offset, limit))).await
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

/// Adds a song to the Queue: one of the results on show, a catalog song, what
/// plays now or one the library holds. The song is kept in the library first,
/// so the Queue finds it by id, and nothing is asked of any service.
#[tauri::command]
pub async fn queue_track<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    player: State<'_, PlayerHandle>,
    library: State<'_, LibraryHandle>,
    track_id: Uuid,
    placement: Placement,
) -> Result<(), IpcError> {
    // The app's lock is released before the player's is taken.
    let (known, is_sample) = {
        let state = lock(&state);
        (
            state.known_track(track_id),
            state.catalog().track(track_id).is_some(),
        )
    };
    let current = player.current_track();
    let known = known.or_else(|| current.clone().filter(|track| track.id == track_id));
    let handle = library.inner().clone();
    let id = blocking(move || {
        if is_sample {
            return Ok(track_id);
        }
        let track = find_track(&handle, known, track_id)?;
        let library = handle.library()?;
        // A stand-alone song that plays now becomes the first entry.
        if let Some(current) = current {
            library.keep(&current)?;
        }
        Ok(library.keep(&track)?)
    })
    .await?;
    player.enqueue(&app, id, placement)
}

#[tauri::command]
pub fn remove_queue_entry<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
    key: u32,
) -> Result<(), IpcError> {
    command(&player, &app, |p| p.remove_queue_entry(key).map(Some))
}

/// Drops the upcoming entries. The current song keeps playing.
#[tauri::command]
pub fn clear_queue<R: Runtime>(
    app: AppHandle<R>,
    player: State<'_, PlayerHandle>,
) -> Result<(), IpcError> {
    command(&player, &app, |p| Ok(p.clear_queue()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::RecordingSink;
    use super::*;
    use crate::audio::{
        FakeEngine, FakeTracks, NowPlaying, PlaybackStatus, PlayerPosition, QueueItem,
        SimulatedEngine, Skipped, SystemClock,
    };
    use crate::catalog::{SourceKind, SourceTag};
    use crate::library::testing::{library_at, memory, reopen};

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
    fn playing_a_list_announces_the_chosen_song_once_and_a_bad_start_stays_quiet() {
        let engine = FakeEngine::default();
        let handle = PlayerHandle::new(Arc::new(Catalog::empty()), Box::new(engine.clone()));
        lock(&handle.player).set_tracks(Box::new(FakeTracks::of(3)));
        let sink = RecordingSink::default();

        handle.play_list(&sink, FakeTracks::ids(3), 1).unwrap();
        let refused = handle.play_list(&sink, FakeTracks::ids(3), 3);

        assert!(refused.is_err());
        let states = sink.states.borrow();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].current.as_ref().unwrap().track.title, "Track 2");
        assert_eq!((states[0].history, states[0].upcoming), (1, 1));
        assert_eq!(states[0].status, PlaybackStatus::Loading);
        assert!(engine.fake().loads.is_empty());
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

    /// Changes the modes as the two commands do.
    fn press(handle: &PlayerHandle, library: Option<&Library>, sink: &RecordingSink) {
        handle
            .change_modes(sink, library, AppPlayer::toggle_shuffle)
            .unwrap();
        for _ in 0..2 {
            handle
                .change_modes(sink, library, AppPlayer::cycle_repeat)
                .unwrap();
        }
    }

    #[test]
    fn shuffle_and_repeat_are_announced_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let (library, _) = library_at(&path);
        let before = handle();
        let sink = RecordingSink::default();

        press(&before, Some(&library), &sink);

        let announced: Vec<_> = sink
            .states
            .borrow()
            .iter()
            .map(|s| (s.shuffle, s.repeat))
            .collect();
        assert_eq!(
            announced,
            [
                (true, Repeat::Off),
                (true, Repeat::All),
                (true, Repeat::One)
            ]
        );
        drop(library);

        let after = handle();
        after.restore_modes(&reopen(&path));

        let state = lock(&after.player).state();
        assert_eq!((state.shuffle, state.repeat), (true, Repeat::One));
    }

    #[test]
    fn modes_go_back_off_for_the_next_launch_too() {
        let (library, _) = memory();
        let handle_before = handle();
        let sink = RecordingSink::default();
        press(&handle_before, Some(&library), &sink);
        handle_before
            .change_modes(&sink, Some(&library), AppPlayer::toggle_shuffle)
            .unwrap();
        handle_before
            .change_modes(&sink, Some(&library), AppPlayer::cycle_repeat)
            .unwrap();

        let after = handle();
        after.restore_modes(&library);

        assert_eq!(lock(&after.player).modes(), (false, Repeat::Off));
        assert_eq!(library.setting("shuffle").unwrap().as_deref(), Some("off"));
        assert_eq!(library.setting("repeat").unwrap().as_deref(), Some("off"));
    }

    #[test]
    fn a_new_or_unreadable_setting_leaves_the_mode_off() {
        let (library, _) = memory();
        let fresh = handle();
        fresh.restore_modes(&library);
        assert_eq!(lock(&fresh.player).modes(), (false, Repeat::Off));

        library.set_setting("shuffle", "maybe").unwrap();
        library.set_setting("repeat", "twice").unwrap();
        fresh.restore_modes(&library);

        assert_eq!(lock(&fresh.player).modes(), (false, Repeat::Off));
        assert_eq!(lock(&fresh.player).state().revision, 0);
    }

    #[test]
    fn without_a_library_the_modes_still_change() {
        let handle = handle();
        let sink = RecordingSink::default();

        press(&handle, None, &sink);

        assert_eq!(lock(&handle.player).modes(), (true, Repeat::One));
        assert_eq!(sink.states.borrow().len(), 3);
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
            has_art: true,
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
            history: 2,
            queue_revision: 3,
            shuffle: true,
            repeat: Repeat::One,
            error: None,
            skipped: Some(Box::new(Skipped {
                key: 1,
                message: "Lumen skipped “Harbor Lights”.".to_owned(),
            })),
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
                "history": 2,
                "queueRevision": 3,
                "shuffle": true,
                "repeat": "one",
                "error": null,
                "skipped": { "key": 1, "message": "Lumen skipped “Harbor Lights”." }
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
            (PlaybackStatus::Reconnecting, "reconnecting"),
            (PlaybackStatus::Ended, "ended"),
            (PlaybackStatus::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(wire));
        }
    }
}
