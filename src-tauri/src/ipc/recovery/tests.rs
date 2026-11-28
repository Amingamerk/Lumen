//! The watcher over a fake engine, a fake yt-dlp and a clock moved by hand.
//! Nothing here contacts YouTube.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::oneshot;
use tokio::task::yield_now;

use super::super::tests::RecordingSink;
use super::*;
use crate::audio::{EngineEvent, FakeEngine, PlaybackStatus, Player};
use crate::catalog::TrackRow;
use crate::recs::StaticRecommender;
use crate::resolver::{ProcessOutput, ResolveError, YtDlp, YtDlpResolver};
use crate::secrets::fake::MemoryStore;
use crate::sources::youtube::testing::{
    FakeClock, FakeTransport, TempDir, music_page, signed_in_cookies,
};
use crate::sources::youtube::{Account, AccountStatus, Kind, Session, Verdict};

const NOW_MS: u64 = 1_700_000_000_000;
const VIDEO: &str = "dQw4w9WgXcQ";
const SECOND: Duration = Duration::from_secs(1);

type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;

#[derive(Default)]
struct Ytdlp {
    runs: AtomicUsize,
    /// What the next runs fail with.
    stderr: Mutex<Option<&'static str>>,
    /// Taken by the next run, which then waits for it.
    hold: Mutex<Option<oneshot::Receiver<()>>>,
}

impl YtDlp for Arc<Ytdlp> {
    fn deno(&self) -> Result<PathBuf, ResolveError> {
        Ok("deno.exe".into())
    }

    async fn run(&self, _: Vec<OsString>) -> Result<ProcessOutput, ResolveError> {
        let run = self.runs.fetch_add(1, Ordering::SeqCst);
        let hold = self.hold.lock().unwrap().take();
        if let Some(hold) = hold {
            hold.await.unwrap();
        }
        let stderr = *self.stderr.lock().unwrap();
        let url = format!("https://a.googlevideo.com/x?expire=1700021600&run={run}");
        Ok(ProcessOutput {
            success: stderr.is_none(),
            stdout: json!({ "url": url }).to_string().into_bytes(),
            stderr: stderr.unwrap_or_default().as_bytes().to_vec(),
        })
    }

    fn shut_down(&self) {}
}

struct Rig {
    _gate: TempDir,
    _jars: tempfile::TempDir,
    clock: FakeClock,
    ytdlp: Arc<Ytdlp>,
    youtube: Arc<Service>,
    resolver: YtDlpResolver<Arc<Ytdlp>, FakeClock, MemoryStore, FakeTransport, FakeClock>,
    engine: FakeEngine,
    player: Mutex<AppPlayer>,
    sink: RecordingSink,
}

/// A signed-in app playing a song of 200 seconds that yt-dlp resolved.
async fn playing() -> Rig {
    let gate = TempDir::new();
    let jars = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(NOW_MS);
    let youtube = Arc::new(Service::new(
        Account::new(MemoryStore::default()),
        FakeTransport::default(),
        clock.clone(),
        gate.path().join("youtube-gate.json"),
        |_| {},
    ));
    let session = Session::capture(music_page(), signed_in_cookies()).unwrap();
    let attempt = youtube.account().begin_login();
    youtube.complete_login(attempt, &session, |_| {}).unwrap();
    let ytdlp = Arc::new(Ytdlp::default());
    let resolver = YtDlpResolver::new(
        ytdlp.clone(),
        clock.clone(),
        youtube.clone(),
        jars.path().to_owned(),
        "cache".into(),
    );
    let engine = FakeEngine::default();
    let catalog = Arc::<crate::catalog::Catalog>::default();
    let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
    let boxed: Box<dyn crate::audio::Engine> = Box::new(engine.clone());
    let player = Player::new(boxed, catalog, recommender).with_clock(clock.clone());
    let rig = Rig {
        _gate: gate,
        _jars: jars,
        clock,
        ytdlp,
        youtube,
        resolver,
        engine,
        player: Mutex::new(player),
        sink: RecordingSink::default(),
    };
    let row = TrackRow::untagged("Paper".to_owned());
    let (key, _) = lock(&rig.player).begin_stream(VIDEO.to_owned(), row);
    let media = rig
        .resolver
        .resolve(&youtube_ref(VIDEO), || true)
        .await
        .map(|stream| stream.media());
    lock(&rig.player)
        .finish_stream(key, Ok(media.unwrap()))
        .unwrap();
    rig.open();
    rig
}

impl Rig {
    fn open(&self) {
        self.engine.fake().duration = 200 * SECOND;
        lock(&self.player)
            .on_engine_event(EngineEvent::Loaded)
            .unwrap();
    }

    /// What libmpv reports when the stream's host answers 403.
    fn fail(&self) {
        lock(&self.player)
            .on_engine_event(EngineEvent::Failed)
            .unwrap();
    }

    fn step(&self) -> Next {
        step(&self.player, &self.youtube, &self.resolver, &self.sink)
    }

    async fn fetch(&self, request: Fetch) {
        fetch(&self.player, &self.resolver, &self.sink, request).await;
    }

    /// Waits as the watcher is told to and takes the fetch that is then due.
    fn due(&self) -> Fetch {
        let Next::Sleep(Some(wait)) = self.step() else {
            panic!("expected a wait");
        };
        self.clock.advance(wait);
        match self.step() {
            Next::Fetch(request) => request,
            other => panic!("expected a fetch, got {other:?}"),
        }
    }

    fn runs(&self) -> usize {
        self.ytdlp.runs.load(Ordering::SeqCst)
    }

    fn loads(&self) -> usize {
        self.engine.fake().loads.len()
    }

    fn status(&self) -> PlaybackStatus {
        lock(&self.player).state().status
    }

    fn error(&self) -> String {
        lock(&self.player).state().error.unwrap_or_default()
    }

    fn account(&self) -> AccountStatus {
        self.youtube.account().status().unwrap()
    }

    /// Some other request ends this way.
    async fn elsewhere(&self, verdict: Verdict) {
        self.youtube
            .through_gate(Kind::Api, || true, |_| async { (verdict, ()) })
            .await
            .unwrap();
    }
}

async fn settle() {
    for _ in 0..20 {
        yield_now().await;
    }
}

#[tokio::test]
async fn a_refused_stream_is_resolved_afresh_through_the_gate() {
    let rig = playing().await;
    rig.fail();

    let request = rig.due();
    assert_eq!(rig.runs(), 1);
    rig.fetch(request).await;

    // The cached stream was dropped, and the gate spaced the second run.
    assert_eq!(rig.runs(), 2);
    assert_eq!(rig.clock.slept(), [3 * SECOND]);
    assert_eq!(rig.loads(), 2);
    assert_eq!(rig.account(), AccountStatus::Connected);
    assert_eq!(
        rig.sink.states.borrow().last().unwrap().status,
        PlaybackStatus::Reconnecting
    );
    rig.open();
    assert_eq!(rig.status(), PlaybackStatus::Playing);
}

#[tokio::test]
async fn a_stall_is_found_and_announced() {
    let rig = playing().await;
    assert!(matches!(rig.step(), Next::Sleep(Some(wait)) if wait == 10 * SECOND));
    rig.clock.advance(10 * SECOND);

    let next = rig.step();

    assert!(matches!(next, Next::Sleep(Some(wait)) if wait == 2 * SECOND));
    let states = rig.sink.states.borrow();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].status, PlaybackStatus::Reconnecting);
}

#[tokio::test]
async fn a_refusal_by_youtube_expires_the_session_and_ends_the_recovery() {
    let rig = playing().await;
    rig.fail();
    let request = rig.due();
    *rig.ytdlp.stderr.lock().unwrap() =
        Some("ERROR: [youtube] x: Unable to download API page: HTTP Error 403: Forbidden");

    rig.fetch(request).await;

    assert_eq!(rig.account(), AccountStatus::Expired);
    assert_eq!(rig.status(), PlaybackStatus::Error);
    assert!(rig.error().contains("Log in again"), "{}", rig.error());
    assert!(matches!(rig.step(), Next::Sleep(None)));
    assert_eq!(rig.runs(), 2);
}

#[tokio::test]
async fn a_cool_down_or_an_expired_session_cancels_the_planned_fetch() {
    for (verdict, reason) in [
        (Verdict::RateLimited { retry_after: None }, "cooling down"),
        (Verdict::Rejected, "expired"),
    ] {
        let rig = playing().await;
        rig.fail();
        rig.elsewhere(verdict).await;

        let next = rig.step();

        assert!(matches!(next, Next::Sleep(None)), "{next:?}");
        assert_eq!(rig.status(), PlaybackStatus::Error);
        assert!(rig.error().contains(reason), "{}", rig.error());
        assert_eq!(rig.sink.states.borrow().len(), 1);
        rig.clock.advance(3_600 * SECOND);
        assert!(matches!(rig.step(), Next::Sleep(None)));
        assert_eq!(rig.runs(), 1);
    }
}

#[tokio::test]
async fn an_answer_nobody_waits_for_any_more_is_not_loaded() {
    for cool_down in [false, true] {
        let rig = playing().await;
        rig.fail();
        let request = rig.due();
        let (release, hold) = oneshot::channel();
        *rig.ytdlp.hold.lock().unwrap() = Some(hold);
        let cancel = async {
            settle().await;
            assert_eq!(rig.runs(), 2);
            if cool_down {
                // Not a request of its own: the gate is busy with the fetch.
                rig.youtube.count_failure();
                rig.youtube.count_failure();
                rig.youtube.count_failure();
                rig.step();
            } else {
                lock(&rig.player).toggle_play();
            }
            release.send(()).unwrap();
        };

        tokio::join!(rig.fetch(request), cancel);

        assert_eq!(rig.loads(), 1);
        assert_ne!(rig.status(), PlaybackStatus::Reconnecting);
    }
}

#[tokio::test]
async fn a_cancelled_fetch_that_is_still_waiting_runs_nothing() {
    let rig = playing().await;
    rig.fail();
    let request = rig.due();
    lock(&rig.player).toggle_play();

    rig.fetch(request).await;

    assert_eq!(rig.runs(), 1);
    assert!(rig.sink.states.borrow().is_empty());
}

#[tokio::test]
async fn giving_up_counts_one_failure_at_the_gate() {
    let rig = playing().await;
    for _ in 0..2 {
        rig.fail();
        let request = rig.due();
        rig.fetch(request).await;
        rig.open();
    }
    rig.fail();

    assert!(matches!(rig.step(), Next::Sleep(None)));
    assert!(matches!(rig.step(), Next::Sleep(None)));

    assert_eq!(rig.status(), PlaybackStatus::Error);
    assert_eq!(rig.runs(), 3);
    // One is counted: two more make the three that pause it.
    rig.youtube.count_failure();
    assert!(rig.youtube.ready().is_ok());
    rig.youtube.count_failure();
    assert!(rig.youtube.ready().is_err());
}
