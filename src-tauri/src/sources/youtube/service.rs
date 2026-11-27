//! YouTube Music as the rest of the app uses it: the account, the gate and the
//! one way out to the network.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use serde_json::Value;
use tokio::sync::watch;

use super::account::{Account, AccountStatus, Generation, Unusable};
use super::client::{self, Fault};
use super::gate::{Gate, GateClock, GateError, GateView, Kind, PauseReason, Verdict};
use super::session::Session;
use super::transport::{Request, Transport};
use crate::secrets::SecretStore;

/// Why a call gave nothing. Fixed texts only.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum ServiceError {
    #[error("not signed in to YouTube Music")]
    Disconnected,
    #[error("the YouTube Music session has expired")]
    Expired,
    #[error(transparent)]
    Gate(#[from] GateError),
    #[error("could not reach YouTube Music")]
    Offline,
    #[error("YouTube Music refused the session")]
    Rejected,
    #[error("YouTube Music asked Lumen to slow down")]
    RateLimited,
    #[error("YouTube Music answered with a bot check")]
    Blocked,
    #[error("YouTube Music gave an unexpected answer")]
    Unexpected,
}

impl From<Fault> for ServiceError {
    fn from(fault: Fault) -> Self {
        match fault {
            Fault::Offline => Self::Offline,
            Fault::Rejected => Self::Rejected,
            Fault::RateLimited => Self::RateLimited,
            Fault::Blocked => Self::Blocked,
            Fault::Unexpected => Self::Unexpected,
        }
    }
}

impl From<Unusable> for ServiceError {
    fn from(unusable: Unusable) -> Self {
        match unusable {
            Unusable::Disconnected => Self::Disconnected,
            Unusable::Expired => Self::Expired,
        }
    }
}

/// Whether the stored session has been seen to work. Kept in memory only.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Check {
    #[default]
    Unchecked,
    Checking,
    Verified,
    Offline,
    Unexpected,
}

/// What the UI is told, besides the account status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServiceStatus {
    /// Rises with every change, so an older answer can be dropped.
    pub revision: u64,
    pub gate: GateView,
    pub check: Check,
}

/// The session one run may use.
pub struct Lease {
    pub generation: Generation,
    pub session: Session,
}

type Announce = Box<dyn Fn(AccountStatus) + Send + Sync>;

pub struct YouTube<S, T, C> {
    account: Account<S>,
    /// Private, so every request goes through [`Self::through_gate`].
    transport: T,
    gate: Gate<C>,
    /// The check, and the session it is about.
    check: Mutex<(Generation, Check)>,
    status: watch::Sender<ServiceStatus>,
    announce: Announce,
}

impl<S: SecretStore, T: Transport, C: GateClock> YouTube<S, T, C> {
    /// Takes up the gate state saved at `state_file` and sends nothing.
    /// `announce` is told when YouTube makes the session expire.
    pub fn new(
        account: Account<S>,
        transport: T,
        clock: C,
        state_file: PathBuf,
        announce: impl Fn(AccountStatus) + Send + Sync + 'static,
    ) -> Self {
        let gate = Gate::new(clock, state_file);
        let status = watch::Sender::new(ServiceStatus {
            revision: 0,
            gate: gate.view(),
            check: Check::Unchecked,
        });
        Self {
            account,
            transport,
            gate,
            check: Mutex::default(),
            status,
            announce: Box::new(announce),
        }
    }

    pub fn account(&self) -> &Account<S> {
        &self.account
    }

    /// The status as it is now.
    pub fn status(&self) -> ServiceStatus {
        self.refresh();
        *self.status.borrow()
    }

    /// The status as last published.
    pub fn watch(&self) -> watch::Receiver<ServiceStatus> {
        self.status.subscribe()
    }

    /// Publishes the status again, after time or the account has moved.
    pub fn refresh(&self) {
        // Read inside the update, so revisions follow the order of the changes.
        self.status.send_if_modified(|status| {
            let gate = self.gate.view();
            let (generation, check) = *self.check.lock().unwrap_or_else(PoisonError::into_inner);
            let check = if generation == self.account.generation() {
                check
            } else {
                Check::Unchecked
            };
            if (gate, check) == (status.gate, status.check) {
                return false;
            }
            *status = ServiceStatus {
                revision: status.revision + 1,
                gate,
                check,
            };
            true
        });
    }

    /// Ends a pause, if that can be saved. Sends nothing.
    pub fn resume(&self) {
        self.gate.resume();
        self.refresh();
    }

    /// Confirms the session with one authenticated browse.
    pub async fn verify(&self) -> Result<(), ServiceError> {
        let mut checked = None;
        let outcome = self
            .through_gate(
                Kind::Api,
                || true,
                |lease| {
                    checked = Some(lease.generation);
                    self.set_check(lease.generation, Check::Checking);
                    self.send(lease.session, client::browse, client::page)
                },
            )
            .await
            .and_then(|reply| reply.map_err(ServiceError::from));
        let check = match outcome {
            Ok(()) => Check::Verified,
            Err(ServiceError::Offline) => Check::Offline,
            Err(ServiceError::Unexpected) => Check::Unexpected,
            Err(_) => Check::Unchecked,
        };
        // A run the gate refused was about no session in particular.
        let generation = checked.unwrap_or_else(|| self.account.generation());
        self.set_check(generation, check);
        outcome
    }

    /// The one way out: waits for the gate, loads the session inside its turn
    /// and runs `work` with it. A session that is gone or rejected runs nothing,
    /// and one that `work` finds rejected expires before the next run.
    pub async fn through_gate<R, Fut>(
        &self,
        kind: Kind,
        wanted: impl Fn() -> bool,
        work: impl FnOnce(Lease) -> Fut,
    ) -> Result<R, ServiceError>
    where
        Fut: Future<Output = (Verdict, R)>,
    {
        let outcome = self
            .gate
            .run(kind, wanted, || async {
                let (generation, session) = match self.account.session_for_request() {
                    Ok(usable) => usable,
                    Err(unusable) => return (Verdict::Neutral, Err(unusable)),
                };
                let lease = Lease {
                    generation,
                    session,
                };
                let (verdict, output) = work(lease).await;
                match verdict {
                    Verdict::Success => self.set_check(generation, Check::Verified),
                    Verdict::Rejected => self.expire(generation),
                    _ => {}
                }
                (verdict, Ok(output))
            })
            .await;
        // The gate has applied the verdict by now.
        self.refresh();
        Ok(outcome??)
    }

    /// Builds one request, sends it once and judges the reply.
    async fn send<R>(
        &self,
        session: Session,
        build: impl FnOnce(&Session, u64) -> Option<Request>,
        parse: impl FnOnce(&Value) -> Option<R>,
    ) -> (Verdict, Result<R, Fault>) {
        let Some(request) = build(&session, self.gate.now_ms()) else {
            // No cookie left to sign with: only a new login helps.
            return (Verdict::Rejected, Err(Fault::Rejected));
        };
        client::classify(self.transport.post(request).await, parse)
    }

    fn set_check(&self, generation: Generation, check: Check) {
        // A late reply must not hide what is known about a newer session.
        if generation == self.account.generation() {
            *self.check.lock().unwrap_or_else(PoisonError::into_inner) = (generation, check);
        }
        self.refresh();
    }

    fn expire(&self, generation: Generation) {
        let saved = self.account.expire(generation, &self.announce);
        if saved.is_err() {
            self.gate.pause(PauseReason::ExpiryUnsaved);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tokio::task::yield_now;

    use super::*;
    use crate::secrets::fake::MemoryStore;
    use crate::sources::youtube::gate::{FakeClock, TempDir};
    use crate::sources::youtube::session::fake::{music_page, signed_in_cookies};
    use crate::sources::youtube::transport::fake::FakeTransport;
    use crate::sources::youtube::transport::{Response, TransportError};

    const NOW: u64 = 1_700_000_000_000;
    const BROWSE: &str = "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false";

    type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;

    struct Fixture {
        dir: TempDir,
        store: MemoryStore,
        transport: FakeTransport,
        clock: FakeClock,
        announced: Arc<Mutex<Vec<AccountStatus>>>,
        service: Arc<Service>,
    }

    /// A signed-in service.
    fn fixture() -> Fixture {
        let fixture = signed_out();
        sign_in(&fixture.service);
        fixture
    }

    fn signed_out() -> Fixture {
        let dir = TempDir::new();
        let store = MemoryStore::default();
        let transport = FakeTransport::default();
        let clock = FakeClock::at(NOW);
        let announced = Arc::default();
        let service = Arc::new(start(&dir, &store, &transport, &clock, &announced));
        Fixture {
            dir,
            store,
            transport,
            clock,
            announced,
            service,
        }
    }

    fn start(
        dir: &TempDir,
        store: &MemoryStore,
        transport: &FakeTransport,
        clock: &FakeClock,
        announced: &Arc<Mutex<Vec<AccountStatus>>>,
    ) -> Service {
        let announced = announced.clone();
        YouTube::new(
            Account::new(store.clone()),
            transport.clone(),
            clock.clone(),
            dir.path().join("youtube-gate.json"),
            move |status| announced.lock().unwrap().push(status),
        )
    }

    fn sign_in(service: &Service) {
        let session = Session::capture(music_page(), signed_in_cookies()).unwrap();
        let attempt = service.account().begin_login();
        service
            .account()
            .complete_login(attempt, &session, |_| {})
            .unwrap();
    }

    fn reply(status: u16, body: impl ToString) -> Result<Response, TransportError> {
        Ok(Response {
            status,
            body: body.to_string(),
            ..Response::default()
        })
    }

    fn page(logged_in: &str) -> Result<Response, TransportError> {
        let params = json!([{ "key": "logged_in", "value": logged_in }]);
        let body = json!({
            "responseContext": { "serviceTrackingParams": [{ "params": params }] },
            "contents": { "page": {} },
        });
        reply(200, body)
    }

    /// Lets spawned tasks run as far as they can.
    async fn settle() {
        for _ in 0..10 {
            yield_now().await;
        }
    }

    fn spawn_verify(f: &Fixture) -> tokio::task::JoinHandle<Result<(), ServiceError>> {
        let service = f.service.clone();
        tokio::spawn(async move { service.verify().await })
    }

    fn shown(service: &Service) -> (GateView, Check) {
        let status = service.status();
        (status.gate, status.check)
    }

    #[tokio::test]
    async fn starting_and_reading_the_status_send_nothing() {
        let f = fixture();

        assert_eq!(shown(&f.service), (GateView::Ready, Check::Unchecked));
        assert_eq!(f.service.account().status(), Ok(AccountStatus::Connected));
        f.service.refresh();
        f.service.resume();

        assert_eq!(f.transport.sent(), 0);
    }

    #[tokio::test]
    async fn verifying_sends_one_browse_and_ends_verified() {
        let f = fixture();
        let answer = f.transport.hold();
        let mut statuses = f.service.watch();

        let verify = spawn_verify(&f);
        settle().await;
        assert_eq!(statuses.borrow_and_update().check, Check::Checking);
        answer.send(page("1")).unwrap();

        assert_eq!(verify.await.unwrap(), Ok(()));
        assert_eq!(f.transport.urls(), [BROWSE]);
        assert_eq!(statuses.borrow_and_update().check, Check::Verified);
        assert_eq!(f.announced.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn the_verified_state_belongs_to_one_session() {
        let f = fixture();
        f.transport.answer(page("1"));
        f.service.verify().await.unwrap();
        let verified = f.service.status();

        sign_in(&f.service);
        f.service.refresh();

        assert_eq!(f.service.status().check, Check::Unchecked);
        assert!(f.service.status().revision > verified.revision);
    }

    #[tokio::test]
    async fn a_rejected_session_expires_and_is_never_sent_again() {
        for answer in [reply(401, ""), reply(403, ""), page("0")] {
            let f = fixture();
            f.transport.answer(answer);

            assert_eq!(f.service.verify().await, Err(ServiceError::Rejected));
            assert_eq!(f.service.verify().await, Err(ServiceError::Expired));

            assert_eq!(f.transport.sent(), 1);
            assert_eq!(*f.announced.lock().unwrap(), [AccountStatus::Expired]);
            assert_eq!(f.service.account().status(), Ok(AccountStatus::Expired));
            assert_eq!(shown(&f.service), (GateView::Ready, Check::Unchecked));
        }
    }

    #[tokio::test]
    async fn a_session_without_a_usable_cookie_expires_without_a_request() {
        let f = fixture();
        // Past every cookie's expiry.
        let mut session = Session::capture(music_page(), signed_in_cookies()).unwrap();
        for cookie in &mut session.cookies {
            cookie.expires = Some(1);
        }
        let attempt = f.service.account().begin_login();
        f.service
            .account()
            .complete_login(attempt, &session, |_| {})
            .unwrap();

        assert_eq!(f.service.verify().await, Err(ServiceError::Rejected));

        assert_eq!(f.transport.sent(), 0);
        assert_eq!(*f.announced.lock().unwrap(), [AccountStatus::Expired]);
    }

    #[tokio::test]
    async fn nothing_is_sent_when_signed_out() {
        let f = signed_out();

        assert_eq!(f.service.verify().await, Err(ServiceError::Disconnected));
        assert_eq!(f.transport.sent(), 0);
    }

    #[tokio::test]
    async fn a_caller_queued_behind_a_rejection_sends_nothing() {
        let f = fixture();
        let answer = f.transport.hold();
        let first = spawn_verify(&f);
        settle().await;
        let queued = spawn_verify(&f);
        settle().await;

        answer.send(reply(401, "")).unwrap();

        assert_eq!(first.await.unwrap(), Err(ServiceError::Rejected));
        assert_eq!(queued.await.unwrap(), Err(ServiceError::Expired));
        assert_eq!(f.transport.sent(), 1);
    }

    #[tokio::test]
    async fn a_late_rejection_leaves_a_newer_session_alone() {
        let f = fixture();
        let answer = f.transport.hold();
        let verify = spawn_verify(&f);
        settle().await;
        sign_in(&f.service);

        answer.send(reply(401, "")).unwrap();

        assert_eq!(verify.await.unwrap(), Err(ServiceError::Rejected));
        assert_eq!(f.service.account().status(), Ok(AccountStatus::Connected));
        assert!(f.announced.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_late_success_does_not_verify_a_newer_session() {
        let f = fixture();
        let answer = f.transport.hold();
        let verify = spawn_verify(&f);
        settle().await;
        sign_in(&f.service);

        answer.send(page("1")).unwrap();

        assert_eq!(verify.await.unwrap(), Ok(()));
        assert_eq!(f.service.status().check, Check::Unchecked);
    }

    #[tokio::test]
    async fn an_expiry_that_cannot_be_saved_pauses_and_still_stops_the_session() {
        let f = fixture();
        f.transport.answer(reply(401, ""));
        *f.store.changes_left.lock().unwrap() = Some(0);

        assert_eq!(f.service.verify().await, Err(ServiceError::Rejected));
        let paused = GateView::Paused {
            reason: PauseReason::ExpiryUnsaved,
        };
        assert_eq!(f.service.status().gate, paused);
        f.service.resume();
        assert_eq!(f.service.verify().await, Err(ServiceError::Expired));

        assert_eq!(f.transport.sent(), 1);
    }

    #[tokio::test]
    async fn each_kind_of_trouble_shows_differently() {
        let bot_check = reply(429, "unusual traffic");
        let cooling = GateView::CoolingDown {
            until_ms: NOW + 60_000,
        };
        let paused = GateView::Paused {
            reason: PauseReason::BotCheck,
        };
        for (answer, error, status) in [
            (
                Err(TransportError::Connect),
                ServiceError::Offline,
                (GateView::Ready, Check::Offline),
            ),
            (
                reply(500, ""),
                ServiceError::Unexpected,
                (GateView::Ready, Check::Unexpected),
            ),
            (
                reply(429, ""),
                ServiceError::RateLimited,
                (cooling, Check::Unchecked),
            ),
            (bot_check, ServiceError::Blocked, (paused, Check::Unchecked)),
        ] {
            let f = fixture();
            f.transport.answer(answer);

            assert_eq!(f.service.verify().await, Err(error));

            assert_eq!(shown(&f.service), status, "{error:?}");
            let published = *f.service.watch().borrow();
            assert_eq!(published, f.service.status());
        }
    }

    #[tokio::test]
    async fn a_third_failure_in_a_row_pauses() {
        let f = fixture();
        for _ in 0..3 {
            f.transport.answer(Err(TransportError::Timeout));
            assert_eq!(f.service.verify().await, Err(ServiceError::Offline));
        }

        assert_eq!(
            f.service.verify().await,
            Err(ServiceError::Gate(GateError::Paused))
        );
        assert_eq!(f.transport.sent(), 3);
    }

    #[tokio::test]
    async fn a_login_during_a_block_sends_nothing_and_clears_nothing() {
        for (answer, refusal) in [
            (reply(429, ""), GateError::CoolingDown),
            (reply(403, "captcha"), GateError::Paused),
        ] {
            let f = fixture();
            f.transport.answer(answer);
            assert!(f.service.verify().await.is_err());
            let blocked = f.service.status().gate;

            // A restart, then a new login.
            let restarted = start(&f.dir, &f.store, &f.transport, &f.clock, &f.announced);
            assert_eq!(restarted.status().gate, blocked);
            sign_in(&restarted);

            assert_eq!(restarted.verify().await, Err(ServiceError::Gate(refusal)));
            assert_eq!(shown(&restarted), (blocked, Check::Unchecked));
            assert_eq!(f.transport.sent(), 1);
        }
    }

    #[tokio::test]
    async fn the_status_follows_a_cooldown_and_a_resume() {
        let f = fixture();
        f.transport.answer(reply(403, "captcha"));
        assert!(f.service.verify().await.is_err());
        let mut statuses = f.service.watch();
        statuses.mark_unchanged();

        f.service.resume();
        assert!(matches!(
            statuses.borrow_and_update().gate,
            GateView::CoolingDown { .. }
        ));
        f.clock.advance(std::time::Duration::from_secs(60));
        f.service.refresh();

        assert_eq!(statuses.borrow_and_update().gate, GateView::Ready);
    }
}
