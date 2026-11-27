//! The one limiter every YouTube request passes (PROJECT.md 3c): one run at a
//! time, spaced apart, and none while cooling down or paused.

mod clock;
mod policy;
mod state_file;

use std::future::Future;
use std::path::PathBuf;
use std::sync::{MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::watch;

pub use clock::GateClock;
#[allow(unused_imports)] // Not wired up yet.
pub use clock::SystemClock;
#[cfg(test)]
pub(crate) use clock::fake::FakeClock;
use policy::GateState;
pub use policy::{GateView, PauseReason, Verdict};
use state_file::StateFile;
#[cfg(test)]
pub(crate) use state_file::fake::TempDir;

/// What a run does, which decides its spacing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Api,
    YtDlp,
}

/// Why a run did not happen.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum GateError {
    #[error("YouTube asked us to slow down: cooling down")]
    CoolingDown,
    #[error("YouTube is paused")]
    Paused,
    #[error("the request is no longer wanted")]
    Superseded,
}

const SPACING: Duration = Duration::from_secs(1);
const YT_DLP_SPACING: Duration = Duration::from_secs(5);

pub struct Gate<C> {
    clock: C,
    file: StateFile,
    /// Held for a whole run. Tokio's lock serves callers in order.
    turn: tokio::sync::Mutex<Ends>,
    /// Held briefly, so status and Resume never wait behind a request.
    state: std::sync::Mutex<GateState>,
    view: watch::Sender<GateView>,
}

/// When earlier runs ended, in monotonic time.
#[derive(Default)]
struct Ends {
    any: Option<Duration>,
    yt_dlp: Option<Duration>,
}

impl Ends {
    /// The earliest time a run of `kind` may start.
    fn earliest(&self, kind: Kind) -> Duration {
        let after_any = self.any.map(|end| end + SPACING);
        let after_yt_dlp = self
            .yt_dlp
            .filter(|_| kind == Kind::YtDlp)
            .map(|end| end + YT_DLP_SPACING);
        after_any.max(after_yt_dlp).unwrap_or_default()
    }
}

/// Records the end of a run, also when its future is dropped halfway: the
/// request may already be out.
struct Running<'a, C: GateClock> {
    ends: &'a mut Ends,
    clock: &'a C,
    kind: Kind,
}

impl<C: GateClock> Drop for Running<'_, C> {
    fn drop(&mut self) {
        let now = self.clock.monotonic();
        self.ends.any = Some(now);
        if self.kind == Kind::YtDlp {
            self.ends.yt_dlp = Some(now);
        }
    }
}

impl<C: GateClock> Gate<C> {
    /// Takes up the state saved at `path`.
    pub fn new(clock: C, path: PathBuf) -> Self {
        let file = StateFile::new(path);
        let state = file.load();
        let view = watch::Sender::new(state.view(clock.wall_ms()));
        Self {
            clock,
            file,
            turn: Default::default(),
            state: std::sync::Mutex::new(state),
            view,
        }
    }

    /// Waits for its turn, then runs `work` and applies its verdict. A blocked
    /// gate refuses at once; spacing only waits. `wanted` is asked again after
    /// waiting.
    pub async fn run<T, Fut>(
        &self,
        kind: Kind,
        wanted: impl Fn() -> bool,
        work: impl FnOnce() -> Fut,
    ) -> Result<T, GateError>
    where
        Fut: Future<Output = (Verdict, T)>,
    {
        self.check()?;
        let mut ends = self.turn.lock().await;
        let still_wanted = || {
            self.check()?;
            wanted().then_some(()).ok_or(GateError::Superseded)
        };
        still_wanted()?;
        let wait = ends.earliest(kind).saturating_sub(self.clock.monotonic());
        if !wait.is_zero() {
            self.clock.sleep(wait).await;
            still_wanted()?;
        }
        let running = Running {
            ends: &mut ends,
            clock: &self.clock,
            kind,
        };
        let (verdict, output) = work().await;
        drop(running);
        // Applied before the turn is given up, so the next caller sees it.
        self.apply(&verdict);
        Ok(output)
    }

    /// The clock the gate runs on, as Unix milliseconds.
    pub fn now_ms(&self) -> u64 {
        self.clock.wall_ms()
    }

    pub fn view(&self) -> GateView {
        self.lock().view(self.clock.wall_ms())
    }

    /// The view as last published.
    pub fn watch(&self) -> watch::Receiver<GateView> {
        self.view.subscribe()
    }

    /// Publishes the view again, once a cool-down has run out.
    pub fn refresh(&self) {
        self.publish(&self.lock());
    }

    /// Ends a pause, if that can be saved. Sends nothing.
    pub fn resume(&self) {
        let mut state = self.lock();
        let mut resumed = state.clone();
        resumed.resume();
        match self.file.save(&resumed) {
            Ok(()) => *state = resumed,
            Err(_) => state.paused = Some(PauseReason::StateUnwritable),
        }
        self.publish(&state);
    }

    pub fn pause(&self, reason: PauseReason) {
        self.change(|state| state.pause(reason));
    }

    fn apply(&self, verdict: &Verdict) {
        let (now, jitter) = (self.clock.wall_ms(), self.clock.jitter());
        self.change(|state| state.apply(verdict, now, jitter));
    }

    /// Saves every change. A change that cannot be saved would be lost in a
    /// restart, so it pauses.
    fn change(&self, change: impl FnOnce(&mut GateState)) {
        let mut state = self.lock();
        let before = state.clone();
        change(&mut state);
        if *state != before && self.file.save(&state).is_err() {
            state.paused = Some(PauseReason::StateUnwritable);
        }
        self.publish(&state);
    }

    /// Called with the state lock held, so views arrive in order.
    fn publish(&self, state: &GateState) {
        let view = state.view(self.clock.wall_ms());
        self.view.send_if_modified(|published| {
            let changed = *published != view;
            *published = view;
            changed
        });
    }

    fn check(&self) -> Result<(), GateError> {
        match self.view() {
            GateView::Ready => Ok(()),
            GateView::CoolingDown { .. } => Err(GateError::CoolingDown),
            GateView::Paused { .. } => Err(GateError::Paused),
        }
    }

    /// A panic elsewhere must not lock the gate for good.
    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio::sync::oneshot;
    use tokio::task::{JoinHandle, yield_now};

    use super::*;

    const NOW: u64 = 1_700_000_000_000;
    const SECOND: Duration = Duration::from_secs(1);

    struct Fixture {
        dir: TempDir,
        clock: FakeClock,
        gate: Arc<Gate<FakeClock>>,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new();
        let clock = FakeClock::at(NOW);
        let gate = Arc::new(Gate::new(clock.clone(), path(&dir)));
        Fixture { dir, clock, gate }
    }

    fn path(dir: &TempDir) -> PathBuf {
        dir.path().join("youtube-gate.json")
    }

    async fn run(gate: &Gate<FakeClock>, kind: Kind, verdict: Verdict) -> Result<(), GateError> {
        gate.run(kind, || true, || async { (verdict, ()) }).await
    }

    /// Lets spawned tasks run as far as they can.
    async fn settle() {
        for _ in 0..10 {
            yield_now().await;
        }
    }

    /// A run that records its name and, if given, waits to be let go.
    fn spawn(
        gate: &Arc<Gate<FakeClock>>,
        log: &Arc<Mutex<Vec<&'static str>>>,
        name: &'static str,
        hold: Option<oneshot::Receiver<Verdict>>,
    ) -> JoinHandle<Result<(), GateError>> {
        let (gate, log) = (gate.clone(), log.clone());
        tokio::spawn(async move {
            gate.run(
                Kind::Api,
                || true,
                || async move {
                    log.lock().unwrap().push(name);
                    let verdict = match hold {
                        Some(hold) => hold.await.unwrap(),
                        None => Verdict::Success,
                    };
                    (verdict, ())
                },
            )
            .await
        })
    }

    #[tokio::test]
    async fn one_run_at_a_time_in_the_order_they_came() {
        let f = fixture();
        let log = Arc::default();
        let (release, hold) = oneshot::channel();
        let first = spawn(&f.gate, &log, "first", Some(hold));
        settle().await;
        let others = ["second", "third", "fourth"].map(|name| spawn(&f.gate, &log, name, None));
        settle().await;
        assert_eq!(*log.lock().unwrap(), ["first"]);

        release.send(Verdict::Success).unwrap();

        assert_eq!(first.await.unwrap(), Ok(()));
        for other in others {
            assert_eq!(other.await.unwrap(), Ok(()));
        }
        assert_eq!(*log.lock().unwrap(), ["first", "second", "third", "fourth"]);
    }

    #[tokio::test]
    async fn api_runs_are_a_second_apart_counted_from_the_previous_end() {
        let f = fixture();
        let clock = f.clock.clone();
        f.gate
            .run(
                Kind::Api,
                || true,
                || async move {
                    clock.advance(10 * SECOND);
                    (Verdict::Success, ())
                },
            )
            .await
            .unwrap();

        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();
        f.clock.advance(Duration::from_millis(400));
        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();
        f.clock.advance(2 * SECOND);
        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();

        assert_eq!(f.clock.slept(), [SECOND, Duration::from_millis(600)]);
    }

    #[tokio::test]
    async fn yt_dlp_runs_are_five_seconds_apart() {
        let f = fixture();

        for kind in [Kind::YtDlp, Kind::Api, Kind::YtDlp, Kind::YtDlp] {
            run(&f.gate, kind, Verdict::Success).await.unwrap();
        }

        assert_eq!(f.clock.slept(), [SECOND, 4 * SECOND, 5 * SECOND]);
    }

    #[tokio::test]
    async fn a_dropped_run_still_spaces_the_next_one() {
        let f = fixture();
        let log = Arc::default();
        let (_release, hold) = oneshot::channel();
        let dropped = spawn(&f.gate, &log, "dropped", Some(hold));
        settle().await;
        dropped.abort();
        assert!(dropped.await.is_err());

        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();

        assert_eq!(f.clock.slept(), [SECOND]);
    }

    #[tokio::test]
    async fn a_blocked_gate_runs_nothing_for_new_or_queued_callers() {
        for (verdict, refusal) in [
            (Verdict::BotCheck, GateError::Paused),
            (
                Verdict::RateLimited { retry_after: None },
                GateError::CoolingDown,
            ),
        ] {
            let f = fixture();
            let log = Arc::default();
            let (release, hold) = oneshot::channel();
            let first = spawn(&f.gate, &log, "first", Some(hold));
            settle().await;
            let queued = spawn(&f.gate, &log, "queued", None);
            settle().await;

            release.send(verdict).unwrap();

            assert_eq!(first.await.unwrap(), Ok(()));
            assert_eq!(queued.await.unwrap(), Err(refusal));
            assert_eq!(
                spawn(&f.gate, &log, "new", None).await.unwrap(),
                Err(refusal)
            );
            assert_eq!(*log.lock().unwrap(), ["first"]);
            assert!(f.clock.slept().is_empty());
        }
    }

    #[tokio::test]
    async fn an_unwanted_caller_sends_nothing_and_delays_nobody() {
        let f = fixture();
        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();
        let (asked, sent) = (AtomicUsize::new(0), AtomicUsize::new(0));

        // Wanted when its turn comes, not after the spacing wait.
        let skipped = f
            .gate
            .run(
                Kind::Api,
                || asked.fetch_add(1, Ordering::SeqCst) == 0,
                || async { (Verdict::Failure, sent.fetch_add(1, Ordering::SeqCst)) },
            )
            .await;
        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();

        assert_eq!(skipped, Err(GateError::Superseded));
        assert_eq!(sent.load(Ordering::SeqCst), 0);
        assert_eq!(f.clock.slept(), [SECOND]);
    }

    #[tokio::test]
    async fn blocks_survive_a_restart_and_resume_is_saved() {
        let f = fixture();
        run(&f.gate, Kind::Api, Verdict::BotCheck).await.unwrap();
        let restart = || Gate::new(f.clock.clone(), path(&f.dir));
        let paused = GateView::Paused {
            reason: PauseReason::BotCheck,
        };
        assert_eq!(restart().view(), paused);

        restart().resume();

        let cooling = GateView::CoolingDown {
            until_ms: NOW + 60_000,
        };
        assert_eq!(restart().view(), cooling);
        assert_eq!(*restart().watch().borrow(), cooling);
        assert_eq!(
            run(&restart(), Kind::Api, Verdict::Success).await,
            Err(GateError::CoolingDown)
        );
    }

    #[tokio::test]
    async fn a_failed_save_pauses_until_a_resume_can_be_saved() {
        let f = fixture();
        // A folder where the file belongs makes every save fail.
        std::fs::create_dir_all(path(&f.dir).join("in-the-way")).unwrap();

        run(&f.gate, Kind::Api, Verdict::Failure).await.unwrap();

        let unwritable = GateView::Paused {
            reason: PauseReason::StateUnwritable,
        };
        assert_eq!(f.gate.view(), unwritable);
        f.gate.resume();
        assert_eq!(f.gate.view(), unwritable);
        std::fs::remove_dir_all(path(&f.dir)).unwrap();
        f.gate.resume();
        assert_eq!(f.gate.view(), GateView::Ready);
    }

    #[tokio::test]
    async fn every_change_of_the_view_is_published() {
        let f = fixture();
        let mut views = f.gate.watch();
        assert_eq!(*views.borrow_and_update(), GateView::Ready);
        run(&f.gate, Kind::Api, Verdict::Success).await.unwrap();
        assert!(!views.has_changed().unwrap());

        run(
            &f.gate,
            Kind::Api,
            Verdict::RateLimited { retry_after: None },
        )
        .await
        .unwrap();
        assert!(matches!(
            *views.borrow_and_update(),
            GateView::CoolingDown { .. }
        ));
        f.gate.refresh();
        assert!(!views.has_changed().unwrap());
        f.clock.advance(60 * SECOND);
        f.gate.refresh();
        assert_eq!(*views.borrow_and_update(), GateView::Ready);

        f.gate.pause(PauseReason::ExpiryUnsaved);
        let paused = GateView::Paused {
            reason: PauseReason::ExpiryUnsaved,
        };
        assert_eq!(*views.borrow_and_update(), paused);
        f.gate.resume();
        assert_eq!(*views.borrow_and_update(), GateView::Ready);
    }
}
