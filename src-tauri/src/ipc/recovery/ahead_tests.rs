//! Resolving ahead of the Queue: the watcher's own loop over a fake resolver and
//! a fake engine, with a clock moved by hand. Nothing here contacts YouTube.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{Notify, oneshot};
use tokio::task::yield_now;
use uuid::Uuid;

use super::super::tests::RecordingSink;
use super::*;
use crate::audio::{EngineEvent, FakeEngine, FakeTracks, Placement, PlaybackStatus, Player};
use crate::catalog::{Catalog, TrackRow};
use crate::queue::Repeat;
use crate::recs::StaticRecommender;
use crate::resolver::testing::FakeResolver;
use crate::resolver::{ProcessOutput, ResolveError, StreamInfo, YtDlp, YtDlpResolver};
use crate::secrets::fake::MemoryStore;
use crate::sources::youtube::testing::{
    FakeClock, FakeTransport, TempDir, music_page, signed_in_cookies,
};
use crate::sources::youtube::{Account, Session};

const NOW_MS: u64 = 1_700_000_000_000;
const SECOND: Duration = Duration::from_secs(1);

type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;
type Answer = oneshot::Sender<Result<StreamInfo, ResolveError>>;

struct Rig<R = FakeResolver> {
    _gate: TempDir,
    clock: FakeClock,
    youtube: Arc<Service>,
    resolver: R,
    engine: FakeEngine,
    player: Mutex<AppPlayer>,
    sink: RecordingSink,
    wake: Arc<Notify>,
    /// Fetches for the current song that the watcher started.
    fetches: Mutex<VecDeque<Fetch>>,
}

/// A signed-in app with a library of Tracks 1 to 9.
fn rig() -> Rig {
    rig_with(FakeTracks::of(9), |_, _| FakeResolver::default())
}

fn sign_in(youtube: &Service) {
    let session = Session::capture(music_page(), signed_in_cookies()).unwrap();
    let attempt = youtube.account().begin_login();
    youtube.complete_login(attempt, &session, |_| {}).unwrap();
}

fn rig_with<R: Resolver>(
    tracks: FakeTracks,
    resolver: impl FnOnce(&FakeClock, &Arc<Service>) -> R,
) -> Rig<R> {
    let gate = TempDir::new();
    let clock = FakeClock::at(NOW_MS);
    let youtube = Arc::new(Service::new(
        Account::new(MemoryStore::default()),
        FakeTransport::default(),
        clock.clone(),
        gate.path().join("youtube-gate.json"),
        |_| {},
    ));
    sign_in(&youtube);
    let resolver = resolver(&clock, &youtube);
    let engine = FakeEngine::default();
    let catalog = Arc::new(Catalog::empty());
    let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
    let boxed: Box<dyn crate::audio::Engine> = Box::new(engine.clone());
    let mut player = Player::new(boxed, catalog, recommender)
        .with_clock(clock.clone())
        .with_seed(7);
    player.set_tracks(Box::new(tracks));
    let wake = Arc::new(Notify::new());
    let changed = Arc::clone(&wake);
    player.on_change(move || changed.notify_one());
    Rig {
        _gate: gate,
        clock,
        youtube,
        resolver,
        engine,
        player: Mutex::new(player),
        sink: RecordingSink::default(),
        wake,
        fetches: Mutex::default(),
    }
}

/// A stream with six hours left.
fn stream() -> StreamInfo {
    StreamInfo::fake("https://a.googlevideo.com/x", NOW_MS / 1_000 + 21_600)
}

async fn settle() {
    for _ in 0..20 {
        yield_now().await;
    }
}

impl<R: Resolver> Rig<R> {
    /// Runs `script` while the watcher looks after the player.
    async fn watching(&self, script: impl Future<Output = ()>) {
        let watcher = look_after(
            &self.player,
            &self.youtube,
            &self.resolver,
            &self.sink,
            &self.wake,
            |request| self.fetches.lock().unwrap().push_back(request),
        );
        tokio::select! {
            () = watcher => panic!("the watcher ended"),
            () = script => {}
        }
    }

    /// Does one thing to the player and lets the watcher react.
    async fn act<T>(&self, act: impl FnOnce(&mut AppPlayer) -> T) -> T {
        let done = act(&mut lock(&self.player));
        settle().await;
        done
    }

    /// Queues Tracks 1 to `count`, resolves the first and lets it play.
    async fn start(&self, count: u128) {
        let list = FakeTracks::ids(count);
        self.act(|p| p.play_list(list, 0).unwrap()).await;
        self.current_arrives().await;
    }

    /// Resolves the stream the current song waits for and opens it.
    async fn current_arrives(&self) {
        self.current_resolves().await;
        self.open().await;
    }

    async fn current_resolves(&self) {
        let request = self.fetches.lock().unwrap().pop_front();
        let request = request.expect("the current song asks for its stream");
        fetch(&self.player, &self.resolver, &self.sink, request).await;
        settle().await;
    }

    /// The engine opens what it was given.
    async fn open(&self) {
        self.engine.fake().duration = 200 * SECOND;
        self.act(|p| p.on_engine_event(EngineEvent::Loaded)).await;
    }

    async fn pass(&self, time: Duration) {
        self.clock.advance(time);
        self.wake.notify_one();
        settle().await;
    }

    /// The fetches the current song waits for.
    fn waiting(&self) -> usize {
        self.fetches.lock().unwrap().len()
    }
}

impl Rig {
    /// Answers every resolve that is not queued some other way.
    fn answer_all(&self) {
        for _ in 0..12 {
            self.resolver.answer(Ok(stream()));
        }
    }

    /// The videos asked for, in order.
    fn asked(&self) -> Vec<String> {
        self.resolver.resolved().into_iter().map(|r| r.id).collect()
    }

    /// Plays Track 1 of `count` while the resolve of Track 2 stays in progress.
    async fn resolving_the_second(&self, count: u128) -> Answer {
        self.resolver.answer(Ok(stream()));
        let pending = self.resolver.answer_later();
        self.start(count).await;
        assert_eq!(self.asked(), ["v1", "v2"]);
        pending
    }

    async fn answered(&self, answer: Answer) {
        answer.send(Ok(stream())).expect("the resolve still waits");
        settle().await;
    }
}

#[tokio::test]
async fn a_playing_song_resolves_the_next_entry_then_the_one_after() {
    let rig = rig();
    rig.resolver.answer(Ok(stream()));
    rig.resolver.answer(Ok(stream()));
    rig.resolver.answer(Ok(stream()));

    rig.watching(async {
        let list = FakeTracks::ids(4);
        rig.act(|p| p.play_list(list, 0).unwrap()).await;
        rig.current_resolves().await;
        // Not before the current song plays.
        assert_eq!(rig.asked(), ["v1"]);

        rig.open().await;

        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);
        // And no further, however often the watcher looks.
        rig.pass(SECOND).await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);
        // Nothing was handed to the engine but the current song.
        assert_eq!(rig.engine.fake().loads.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_second_pre_resolve_waits_for_the_first_to_finish() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        let first = rig.resolver.answer_later();
        let _second = rig.resolver.answer_later();
        let list = FakeTracks::ids(3);
        rig.act(|p| p.play_list(list, 0).unwrap()).await;
        rig.current_arrives().await;

        rig.pass(SECOND).await;
        assert_eq!(rig.asked(), ["v1", "v2"]);
        assert_eq!(rig.resolver.alive(), 1);

        rig.answered(first).await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);
        assert_eq!(rig.resolver.alive(), 1);
    })
    .await;
}

#[tokio::test]
async fn an_edit_that_changes_the_next_entry_cancels_the_pending_resolve() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        let pending = rig.resolver.answer_later();
        let _new = rig.resolver.answer_later();
        rig.start(3).await;
        assert_eq!(rig.asked(), ["v1", "v2"]);

        rig.act(|p| p.enqueue(Uuid::from_u128(9), Placement::Next).unwrap())
            .await;

        assert!(pending.is_closed(), "the old resolve is still running");
        assert_eq!(rig.asked(), ["v1", "v2", "v9"]);
        assert_eq!(rig.resolver.alive(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_refused_pre_resolved_entry_is_resolved_again_once() {
    let rig = rig();
    for _ in 0..8 {
        rig.resolver.answer(Ok(stream()));
    }

    rig.watching(async {
        rig.start(3).await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);
        rig.act(|p| p.next_track()).await;
        rig.current_arrives().await;
        let before = rig.asked().len();

        // What libmpv reports when the stream's host answers 403.
        rig.act(|p| p.on_engine_event(EngineEvent::Failed)).await;
        rig.pass(2 * SECOND).await;
        rig.current_arrives().await;
        rig.pass(SECOND).await;

        assert_eq!(rig.asked()[before..], ["v2"]);
        assert_eq!(rig.resolver.forgotten(), [youtube_ref("v2")]);
    })
    .await;
}

#[tokio::test]
async fn skipping_to_the_entry_being_resolved_waits_for_that_answer() {
    let rig = rig();

    rig.watching(async {
        let pending = rig.resolving_the_second(3).await;

        rig.act(|p| p.next_track()).await;

        assert!(!pending.is_closed(), "its resolve was given up");
        assert_eq!((rig.waiting(), rig.resolver.alive()), (0, 1));
        rig.resolver.answer(Ok(stream()));
        rig.answered(pending).await;
        // Now the song asks for it, which the resolver has kept.
        assert_eq!(rig.waiting(), 1);
    })
    .await;
}

#[tokio::test]
async fn skipping_to_a_resolved_entry_leaves_the_one_after_running() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        rig.resolver.answer(Ok(stream()));
        let third = rig.resolver.answer_later();
        rig.start(3).await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);

        rig.act(|p| p.next_track()).await;

        assert!(!third.is_closed());
        assert_eq!(rig.waiting(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_refused_pre_resolved_song_goes_before_the_pending_resolve() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        rig.resolver.answer(Ok(stream()));
        let third = rig.resolver.answer_later();
        rig.resolver.answer(Ok(stream()));
        rig.start(3).await;
        rig.act(|p| p.next_track()).await;
        rig.current_arrives().await;
        assert!(!third.is_closed());

        rig.act(|p| p.on_engine_event(EngineEvent::Failed)).await;

        assert!(third.is_closed());
    })
    .await;
}

#[tokio::test]
async fn a_song_the_user_starts_ends_the_pending_resolve() {
    let rig = rig();

    rig.watching(async {
        let pending = rig.resolving_the_second(3).await;

        let row = TrackRow::untagged("Paper".to_owned());
        rig.act(|p| p.begin_stream("other".to_owned(), row)).await;

        assert!(pending.is_closed());
        assert_eq!(rig.resolver.alive(), 0);
        assert_eq!(rig.asked(), ["v1", "v2"]);
    })
    .await;
}

#[tokio::test]
async fn jumping_to_another_song_ends_the_pending_resolve_first() {
    let rig = rig();

    rig.watching(async {
        let pending = rig.resolving_the_second(9).await;

        let list = FakeTracks::ids(9);
        rig.act(|p| p.play_list(list, 6).unwrap()).await;

        assert!(pending.is_closed());
        assert_eq!((rig.waiting(), rig.resolver.alive()), (1, 0));
    })
    .await;
}

#[tokio::test]
async fn a_failing_current_song_goes_before_the_pending_resolve() {
    let rig = rig();

    rig.watching(async {
        let pending = rig.resolving_the_second(3).await;

        rig.act(|p| p.on_engine_event(EngineEvent::Failed)).await;

        assert!(pending.is_closed());
        rig.answer_all();
        rig.pass(2 * SECOND).await;
        rig.current_arrives().await;
        assert_eq!(rig.asked()[2..], ["v1", "v2", "v3"]);
    })
    .await;
}

#[tokio::test]
async fn a_song_that_failed_ahead_goes_before_the_pending_resolve() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        rig.resolver.answer(Ok(stream()));
        rig.resolver.answer(Err(ResolveError::Failed));
        rig.resolver.answer(Ok(stream()));
        let fourth = rig.resolver.answer_later();
        rig.start(4).await;
        rig.act(|p| p.next_track()).await;
        rig.current_arrives().await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3", "v2", "v4"]);

        // Track 3 needs a run of its own now.
        rig.act(|p| p.next_track()).await;

        assert!(fourth.is_closed());
        assert_eq!(rig.waiting(), 1);
    })
    .await;
}

#[tokio::test]
async fn removing_or_clearing_the_entry_ends_its_resolve() {
    let rig = rig();

    rig.watching(async {
        let second = rig.resolving_the_second(3).await;
        let third = rig.resolver.answer_later();

        let key = rig.act(|p| p.queue_slice(0, 1).items[0].key).await;
        rig.act(|p| p.remove_queue_entry(key).unwrap()).await;
        assert!(second.is_closed());
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);

        rig.act(|p| p.clear_queue()).await;
        assert!(third.is_closed());
        assert_eq!(rig.resolver.alive(), 0);
    })
    .await;
}

#[tokio::test]
async fn a_shuffle_that_moves_the_next_entry_ends_its_resolve() {
    let rig = rig();

    rig.watching(async {
        let pending = rig.resolving_the_second(9).await;
        let _new = rig.resolver.answer_later();

        // With this seed another song follows.
        let next = rig
            .act(|p| {
                p.set_shuffle(true);
                p.queue_slice(0, 1).items[0].track.title.clone()
            })
            .await;

        assert_ne!(next, "Track 2");
        assert!(pending.is_closed());
        assert_eq!(rig.resolver.alive(), 1);
    })
    .await;
}

#[tokio::test]
async fn nothing_is_resolved_ahead_while_one_song_repeats() {
    let rig = rig();

    rig.watching(async {
        rig.answer_all();
        rig.act(|p| p.set_repeat(Repeat::One)).await;
        rig.start(3).await;
        assert_eq!(rig.asked(), ["v1"]);

        rig.act(|p| p.set_repeat(Repeat::All)).await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);
    })
    .await;
}

#[tokio::test]
async fn a_repeating_queue_resolves_its_first_entry_after_its_last() {
    let rig = rig();

    rig.watching(async {
        rig.answer_all();
        rig.act(|p| p.set_repeat(Repeat::All)).await;
        let list = FakeTracks::ids(4);
        rig.act(|p| p.play_list(list, 3).unwrap()).await;
        rig.current_arrives().await;

        assert_eq!(rig.asked(), ["v4", "v1", "v2"]);
    })
    .await;
}

#[tokio::test]
async fn a_paused_song_starts_no_resolve_ahead() {
    let rig = rig();

    rig.watching(async {
        rig.answer_all();
        rig.act(|p| p.play_list(FakeTracks::ids(3), 0).unwrap())
            .await;
        rig.current_resolves().await;
        rig.act(|p| p.toggle_play()).await;
        rig.open().await;
        assert_eq!(rig.asked(), ["v1"]);

        rig.act(|p| p.toggle_play()).await;
        assert_eq!(rig.asked(), ["v1", "v2", "v3"]);
    })
    .await;
}

#[tokio::test]
async fn a_failed_resolve_ahead_waits_for_the_next_song() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        rig.resolver.answer(Err(ResolveError::Failed));
        rig.start(4).await;
        rig.pass(5 * SECOND).await;
        assert_eq!(rig.asked(), ["v1", "v2"]);

        // The failed one is asked for as the current song, by the usual rules.
        rig.answer_all();
        rig.act(|p| p.next_track()).await;
        rig.current_arrives().await;

        assert_eq!(rig.asked(), ["v1", "v2", "v2", "v3", "v4"]);
    })
    .await;
}

#[tokio::test]
async fn a_failure_ahead_is_forgotten_once_the_song_has_had_its_turn() {
    let rig = rig();

    rig.watching(async {
        rig.resolver.answer(Ok(stream()));
        rig.resolver.answer(Err(ResolveError::Failed));
        rig.answer_all();
        rig.act(|p| p.set_repeat(Repeat::All)).await;
        rig.start(2).await;
        for _ in 0..2 {
            rig.act(|p| p.next_track()).await;
            rig.current_arrives().await;
        }

        // Round the Queue and back on Track 1, Track 2 is resolved ahead again.
        assert_eq!(rig.asked(), ["v1", "v2", "v2", "v1", "v1", "v2"]);
    })
    .await;
}

#[tokio::test]
async fn a_paused_gate_ends_and_holds_back_the_resolve_ahead() {
    let rig = rig();

    rig.watching(async {
        let pending = rig.resolving_the_second(3).await;

        for _ in 0..3 {
            rig.youtube.count_failure();
        }
        settle().await;
        rig.pass(5 * SECOND).await;

        assert!(pending.is_closed());
        assert_eq!(rig.asked(), ["v1", "v2"]);
        rig.answer_all();
        rig.youtube.resume();
        settle().await;
        assert_eq!(rig.asked(), ["v1", "v2", "v2", "v3"]);
    })
    .await;
}

#[tokio::test]
async fn another_login_resolves_the_next_entries_again() {
    let rig = rig();

    rig.watching(async {
        rig.answer_all();
        rig.start(3).await;

        sign_in(&rig.youtube);
        rig.pass(SECOND).await;

        assert_eq!(rig.asked(), ["v1", "v2", "v3", "v2", "v3"]);
    })
    .await;
}

#[tokio::test]
async fn a_stream_that_grew_too_old_is_resolved_ahead_again() {
    let rig = rig();

    rig.watching(async {
        rig.answer_all();
        rig.start(2).await;
        assert_eq!(rig.asked(), ["v1", "v2"]);

        rig.act(|p| p.toggle_play()).await;
        rig.clock.advance(Duration::from_secs(21_600 - 599));
        rig.act(|p| p.toggle_play()).await;

        assert_eq!(rig.asked().iter().filter(|v| *v == "v2").count(), 2);
    })
    .await;
}

/// yt-dlp as the real resolver runs it: counts runs and how many are alive.
#[derive(Default)]
struct Ytdlp {
    /// The video of each run, in order.
    runs: Mutex<Vec<String>>,
    /// Taken by the next run of that video, which then waits for it.
    hold: Mutex<Option<(String, oneshot::Receiver<()>)>>,
    /// What that video's runs say on standard error, failing them.
    refuse: Mutex<Option<(String, &'static str)>>,
    alive: AtomicUsize,
    most_alive: AtomicUsize,
}

struct Alive<'a>(&'a AtomicUsize);

impl Drop for Alive<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl YtDlp for Arc<Ytdlp> {
    fn deno(&self) -> Result<PathBuf, ResolveError> {
        Ok("deno.exe".into())
    }

    async fn run(&self, args: Vec<OsString>) -> Result<ProcessOutput, ResolveError> {
        let url = args.last().unwrap().to_string_lossy().into_owned();
        let video = url.rsplit('=').next().unwrap().to_owned();
        let alive = self.alive.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_alive.fetch_max(alive, Ordering::SeqCst);
        let _alive = Alive(&self.alive);
        self.runs.lock().unwrap().push(video.clone());
        let held = self
            .hold
            .lock()
            .unwrap()
            .take_if(|(held, _)| *held == video);
        if let Some((_, hold)) = held {
            let _ = hold.await;
        }
        let refused = self.refuse.lock().unwrap().clone();
        let stderr = refused
            .filter(|(refused, _)| *refused == video)
            .map(|(_, stderr)| stderr);
        let url = format!("https://a.googlevideo.com/{video}?expire=1700021600");
        Ok(ProcessOutput {
            success: stderr.is_none(),
            stdout: json!({ "url": url }).to_string().into_bytes(),
            stderr: stderr.unwrap_or_default().as_bytes().to_vec(),
        })
    }

    fn shut_down(&self) {}
}

type RealResolver = YtDlpResolver<Arc<Ytdlp>, FakeClock, MemoryStore, FakeTransport, FakeClock>;

/// The video of Track `n` in [`real`]: eleven characters, as YouTube's are.
fn video(n: u128) -> String {
    format!("video{n:06}")
}

/// The app over the real resolver and gate, with only yt-dlp faked.
fn real() -> (Rig<RealResolver>, Arc<Ytdlp>, tempfile::TempDir) {
    let tracks = FakeTracks::default();
    for n in 1..=9 {
        let mut track = FakeTracks::track(n);
        track.sources[0].id = video(n);
        tracks.add(track);
    }
    let ytdlp = Arc::new(Ytdlp::default());
    let jars = tempfile::tempdir().unwrap();
    let rig = rig_with(tracks, |clock, youtube| {
        YtDlpResolver::new(
            ytdlp.clone(),
            clock.clone(),
            youtube.clone(),
            jars.path().to_owned(),
            "cache".into(),
        )
    });
    (rig, ytdlp, jars)
}

fn videos(numbers: &[u128]) -> Vec<String> {
    numbers.iter().copied().map(video).collect()
}

#[tokio::test]
async fn the_next_song_after_a_resolve_ahead_starts_with_no_run() {
    let (rig, ytdlp, _jars) = real();

    rig.watching(async {
        rig.start(9).await;
        assert_eq!(*ytdlp.runs.lock().unwrap(), videos(&[1, 2, 3]));
        // The gate spaced them.
        assert_eq!(rig.clock.slept(), [5 * SECOND, 5 * SECOND]);
        let (held, hold) = oneshot::channel();
        *ytdlp.hold.lock().unwrap() = Some((video(4), hold));

        rig.act(|p| p.next_track()).await;
        // Even while the run for the entry after the next is going on.
        rig.current_resolves().await;

        assert_eq!(rig.engine.fake().loads.len(), 2);
        assert_eq!(*ytdlp.runs.lock().unwrap(), videos(&[1, 2, 3]));
        rig.open().await;
        assert_eq!(*ytdlp.runs.lock().unwrap(), videos(&[1, 2, 3, 4]));
        drop(held);
    })
    .await;
}

#[tokio::test]
async fn skipping_to_the_entry_yt_dlp_is_on_starts_no_second_run() {
    let (rig, ytdlp, _jars) = real();

    rig.watching(async {
        let (release, hold) = oneshot::channel();
        *ytdlp.hold.lock().unwrap() = Some((video(2), hold));
        rig.start(2).await;

        rig.act(|p| p.next_track()).await;
        release.send(()).unwrap();
        settle().await;
        rig.current_resolves().await;

        assert_eq!(*ytdlp.runs.lock().unwrap(), videos(&[1, 2]));
        assert_eq!(rig.engine.fake().loads.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_song_started_during_a_run_ahead_never_runs_beside_it() {
    let (rig, ytdlp, _jars) = real();

    rig.watching(async {
        let (_held, hold) = oneshot::channel();
        *ytdlp.hold.lock().unwrap() = Some((video(2), hold));
        rig.start(3).await;
        let list = FakeTracks::ids(9);
        rig.act(|p| p.play_list(list, 6).unwrap()).await;

        rig.current_resolves().await;

        assert_eq!(*ytdlp.runs.lock().unwrap(), videos(&[1, 2, 7]));
        assert_eq!(ytdlp.most_alive.load(Ordering::SeqCst), 1);
        assert_eq!(rig.engine.fake().loads.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_429_ahead_cools_down_and_asks_for_nothing_more() {
    let (rig, ytdlp, _jars) = real();
    *ytdlp.refuse.lock().unwrap() = Some((video(2), "HTTP Error 429: Too Many Requests"));

    rig.watching(async {
        rig.start(3).await;
        rig.pass(SECOND).await;

        assert!(rig.youtube.ready().is_err());
        assert_eq!(*ytdlp.runs.lock().unwrap(), videos(&[1, 2]));
        // The current song plays on.
        assert_eq!(rig.engine.fake().loads.len(), 1);
        assert_eq!(lock(&rig.player).state().status, PlaybackStatus::Playing);
    })
    .await;
}
