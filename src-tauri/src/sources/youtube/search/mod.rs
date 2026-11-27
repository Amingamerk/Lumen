//! Live song search: the cache first, then one request through the gate.

mod cache;
mod parse;

pub use cache::Cache;
pub use parse::Parsed;

use super::account::Generation;
use super::client::{self, Fault};
use super::gate::{GateClock, GateError, GateView, Kind, Verdict};
use super::service::{ServiceError, YouTube};
use super::session::Session;
use super::transport::Transport;
use crate::secrets::SecretStore;

impl<S: SecretStore, T: Transport, C: GateClock> YouTube<S, T, C> {
    /// What a recent search for `query` found. While nothing may be sent,
    /// nothing is shown either: the error says why.
    pub fn cached_songs(&self, query: &str) -> Result<Option<Parsed>, ServiceError> {
        let generation = match self.account().session_for_request() {
            Ok((generation, _)) => generation,
            Err(unusable) => {
                self.searches().clear();
                return Err(unusable.into());
            }
        };
        match self.gate().view() {
            GateView::Ready => {}
            GateView::CoolingDown { .. } => return Err(GateError::CoolingDown.into()),
            GateView::Paused { .. } => return Err(GateError::Paused.into()),
        }
        Ok(self.recall(generation, query))
    }

    /// The songs matching `query`. Sends one request at most, and none if the
    /// query is blank, the answer is cached or `wanted` says no once it is this
    /// search's turn.
    pub async fn search_songs(
        &self,
        query: &str,
        wanted: impl Fn() -> bool,
    ) -> Result<Parsed, ServiceError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Parsed::NoResults);
        }
        if let Some(found) = self.cached_songs(query)? {
            return Ok(found);
        }
        let outcome = self
            .through_gate(Kind::Api, wanted, |lease| async move {
                // The same query may have been answered while this one waited.
                if let Some(found) = self.recall(lease.generation, query) {
                    return (Verdict::Neutral, Ok(found));
                }
                let build = |session: &Session, now_ms| client::search(session, query, now_ms);
                let (verdict, found) = self
                    .send("search", lease.session, build, parse::songs)
                    .await;
                // An answer that was paid for is kept, wanted or not.
                if let Ok(found) = &found {
                    let now = self.gate().monotonic();
                    self.searches()
                        .put(lease.generation, query, found.clone(), now);
                }
                (verdict, found)
            })
            .await?;
        if outcome == Err(Fault::Rejected) {
            self.searches().clear();
        }
        Ok(outcome?)
    }

    fn recall(&self, generation: Generation, query: &str) -> Option<Parsed> {
        let now = self.gate().monotonic();
        let mut searches = self.searches();
        searches.follow(generation);
        searches.get(query, now)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use tokio::task::{JoinHandle, yield_now};

    use super::*;
    use crate::secrets::fake::MemoryStore;
    use crate::sources::youtube::account::Account;
    use crate::sources::youtube::gate::{FakeClock, TempDir};
    use crate::sources::youtube::session::fake::{music_page, signed_in_cookies};
    use crate::sources::youtube::transport::fake::FakeTransport;
    use crate::sources::youtube::transport::{Response, TransportError};

    const SEARCH: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
    const SONGS: &str = include_str!("../fixtures/songs.json");
    const NOTHING: &str = include_str!("../fixtures/no_results.json");
    const STRANGE: &str = include_str!("../fixtures/unknown_layout.json");
    const SIGNED_OUT: &str = include_str!("../fixtures/signed_out.json");

    type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;

    struct Fixture {
        _dir: TempDir,
        transport: FakeTransport,
        service: Arc<Service>,
    }

    fn signed_out() -> Fixture {
        let dir = TempDir::new();
        let transport = FakeTransport::default();
        let service = Arc::new(Service::new(
            Account::new(MemoryStore::default()),
            transport.clone(),
            FakeClock::at(1_700_000_000_000),
            dir.path().join("youtube-gate.json"),
            |_| {},
        ));
        Fixture {
            _dir: dir,
            transport,
            service,
        }
    }

    /// A signed-in service.
    fn fixture() -> Fixture {
        let fixture = signed_out();
        sign_in(&fixture.service);
        fixture
    }

    fn sign_in(service: &Service) {
        let session = Session::capture(music_page(), signed_in_cookies()).unwrap();
        let attempt = service.account().begin_login();
        service.complete_login(attempt, &session, |_| {}).unwrap();
    }

    fn ok(body: &str) -> Result<Response, TransportError> {
        Ok(Response {
            status: 200,
            body: body.to_owned(),
            ..Response::default()
        })
    }

    fn status(status: u16) -> Result<Response, TransportError> {
        Ok(Response {
            status,
            ..Response::default()
        })
    }

    async fn search(f: &Fixture, query: &str) -> Result<Parsed, ServiceError> {
        f.service.search_songs(query, || true).await
    }

    fn song_count(found: Result<Parsed, ServiceError>) -> usize {
        match found {
            Ok(Parsed::Songs(tracks)) => tracks.len(),
            other => panic!("expected songs, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_search_is_one_request_even_when_there_is_more_to_fetch() {
        let f = fixture();
        f.transport.answer(ok(SONGS));
        assert!(SONGS.contains("continuations"));

        assert_eq!(song_count(search(&f, " Paper Lanterns ").await), 3);

        assert_eq!(f.transport.urls(), [SEARCH]);
    }

    #[tokio::test]
    async fn a_repeated_search_sends_nothing() {
        let f = fixture();
        f.transport.answer(ok(SONGS));
        f.transport.answer(ok(NOTHING));
        search(&f, "paper").await.unwrap();
        assert_eq!(search(&f, "zzz").await, Ok(Parsed::NoResults));

        assert_eq!(song_count(search(&f, "  PAPER ").await), 3);
        assert_eq!(search(&f, "zzz").await, Ok(Parsed::NoResults));
        assert_eq!(
            song_count(f.service.cached_songs("Paper").map(Option::unwrap)),
            3
        );
        assert_eq!(f.service.cached_songs("unseen"), Ok(None));

        assert_eq!(f.transport.sent(), 2);
    }

    #[tokio::test]
    async fn a_blank_search_sends_nothing() {
        let f = fixture();

        assert_eq!(search(&f, "   ").await, Ok(Parsed::NoResults));

        assert_eq!(f.transport.sent(), 0);
    }

    /// Lets spawned tasks run as far as they can.
    async fn settle() {
        for _ in 0..10 {
            yield_now().await;
        }
    }

    /// A search that is wanted while `newest` still names it.
    fn queued(
        f: &Fixture,
        newest: &Arc<AtomicU32>,
        number: u32,
        query: &'static str,
    ) -> JoinHandle<Result<Parsed, ServiceError>> {
        let (service, newest) = (f.service.clone(), newest.clone());
        tokio::spawn(async move {
            let wanted = || newest.load(Ordering::SeqCst) == number;
            service.search_songs(query, wanted).await
        })
    }

    #[tokio::test]
    async fn of_the_searches_that_wait_only_the_newest_is_sent() {
        let f = fixture();
        let newest = Arc::new(AtomicU32::new(1));
        let answer = f.transport.hold();
        f.transport.answer(ok(NOTHING));
        let first = queued(&f, &newest, 1, "first");
        settle().await;
        newest.store(2, Ordering::SeqCst);
        let second = queued(&f, &newest, 2, "second");
        settle().await;
        newest.store(3, Ordering::SeqCst);
        let third = queued(&f, &newest, 3, "third");
        settle().await;

        answer.send(ok(SONGS)).unwrap();

        // It was sent, so its answer counts and is kept.
        assert_eq!(song_count(first.await.unwrap()), 3);
        assert_eq!(
            second.await.unwrap(),
            Err(ServiceError::Gate(GateError::Superseded))
        );
        assert_eq!(third.await.unwrap(), Ok(Parsed::NoResults));
        assert_eq!(f.transport.sent(), 2);
        assert_eq!(song_count(search(&f, "first").await), 3);
        assert_eq!(f.transport.sent(), 2);
    }

    #[tokio::test]
    async fn a_query_asked_again_while_in_flight_is_sent_once() {
        let f = fixture();
        let newest = Arc::new(AtomicU32::new(1));
        let answer = f.transport.hold();
        let first = queued(&f, &newest, 1, "a");
        settle().await;
        newest.store(2, Ordering::SeqCst);
        let other = queued(&f, &newest, 2, "b");
        settle().await;
        newest.store(3, Ordering::SeqCst);
        let again = queued(&f, &newest, 3, "a");
        settle().await;

        answer.send(ok(SONGS)).unwrap();

        assert_eq!(song_count(first.await.unwrap()), 3);
        assert!(other.await.unwrap().is_err());
        assert_eq!(song_count(again.await.unwrap()), 3);
        assert_eq!(f.transport.sent(), 1);
    }

    #[tokio::test]
    async fn the_third_unreadable_answer_pauses_and_none_is_kept() {
        let f = fixture();
        for _ in 0..3 {
            f.transport.answer(ok(STRANGE));

            assert_eq!(search(&f, "same").await, Err(ServiceError::Unexpected));
        }

        assert_eq!(
            search(&f, "same").await,
            Err(ServiceError::Gate(GateError::Paused))
        );
        assert_eq!(f.transport.sent(), 3);
    }

    /// Searches a cached query and a new one, and expects both refused.
    async fn refuses(f: &Fixture, error: ServiceError) {
        let sent = f.transport.sent();
        for query in ["kept", "new"] {
            assert_eq!(search(f, query).await, Err(error), "{query}");
            assert_eq!(f.service.cached_songs(query), Err(error), "{query}");
        }
        assert_eq!(f.transport.sent(), sent);
    }

    /// A signed-in service that has `kept` cached and then got `answer`.
    async fn after(answer: Result<Response, TransportError>) -> (Fixture, ServiceError) {
        let f = fixture();
        f.transport.answer(ok(SONGS));
        f.transport.answer(answer);
        search(&f, "kept").await.unwrap();
        let error = search(&f, "other").await.unwrap_err();
        (f, error)
    }

    #[tokio::test]
    async fn while_blocked_nothing_is_sent_or_shown() {
        refuses(&signed_out(), ServiceError::Disconnected).await;

        let (expired, error) = after(status(401)).await;
        assert_eq!(error, ServiceError::Rejected);
        refuses(&expired, ServiceError::Expired).await;

        let (cooling, error) = after(status(429)).await;
        assert_eq!(error, ServiceError::RateLimited);
        refuses(&cooling, ServiceError::Gate(GateError::CoolingDown)).await;

        let captcha = Ok(Response {
            status: 429,
            body: "captcha".to_owned(),
            ..Response::default()
        });
        let (paused, error) = after(captcha).await;
        assert_eq!(error, ServiceError::Blocked);
        refuses(&paused, ServiceError::Gate(GateError::Paused)).await;

        let (disconnected, _) = after(status(500)).await;
        disconnected.service.disconnect(|_| {}).unwrap();
        refuses(&disconnected, ServiceError::Disconnected).await;
    }

    #[tokio::test]
    async fn each_failure_has_its_own_error() {
        for (answer, error) in [
            (Err(TransportError::Connect), ServiceError::Offline),
            (Err(TransportError::Timeout), ServiceError::Offline),
            (status(500), ServiceError::Unexpected),
            (ok(SIGNED_OUT), ServiceError::Rejected),
            (status(429), ServiceError::RateLimited),
        ] {
            let f = fixture();
            f.transport.answer(answer);

            assert_eq!(search(&f, "salt").await, Err(error));
        }
    }

    #[tokio::test]
    async fn a_new_login_empties_the_cache() {
        let f = fixture();
        f.transport.answer(ok(SONGS));
        f.transport.answer(ok(NOTHING));
        search(&f, "salt").await.unwrap();

        sign_in(&f.service);

        assert_eq!(f.service.cached_songs("salt"), Ok(None));
        assert_eq!(search(&f, "salt").await, Ok(Parsed::NoResults));
        assert_eq!(f.transport.sent(), 2);
    }
}
