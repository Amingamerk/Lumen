//! YouTube Music as a [`Source`].

use super::gate::GateClock;
use super::search::Parsed;
use super::service::YouTube;
use super::transport::Transport;
use crate::catalog::{SourceRef, Track};
use crate::secrets::SecretStore;
use crate::sources::{Source, SourceError};

impl<S: SecretStore + Send + Sync, T: Transport, C: GateClock> Source for YouTube<S, T, C> {
    async fn search(
        &self,
        query: &str,
        limit: usize,
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> Result<Vec<Track>, SourceError> {
        let mut tracks = match self.search_songs(query, wanted).await? {
            Parsed::Songs(tracks) => tracks,
            Parsed::NoResults => Vec::new(),
        };
        tracks.truncate(limit);
        Ok(tracks)
    }

    /// YouTube Music's radio arrives with recommendations (Milestone 6). Until
    /// then nothing is asked of the account.
    async fn related(&self, _reference: &SourceRef) -> Result<Vec<Track>, SourceError> {
        Err(SourceError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::catalog::SourceKind;
    use crate::secrets::fake::MemoryStore;
    use crate::sources::youtube::session::fake::{music_page, signed_in_cookies};
    use crate::sources::youtube::testing::{FakeClock, TempDir};
    use crate::sources::youtube::transport::Response;
    use crate::sources::youtube::transport::fake::FakeTransport;
    use crate::sources::youtube::{Account, Session};

    const SONGS: &str = include_str!("fixtures/songs.json");

    type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;

    struct Fixture {
        _dir: TempDir,
        transport: FakeTransport,
        service: Arc<Service>,
    }

    fn signed_in() -> Fixture {
        let dir = TempDir::new();
        let transport = FakeTransport::default();
        let service = Arc::new(Service::new(
            Account::new(MemoryStore::default()),
            transport.clone(),
            FakeClock::at(1_700_000_000_000),
            dir.path().join("youtube-gate.json"),
            |_| {},
        ));
        let session = Session::capture(music_page(), signed_in_cookies()).unwrap();
        let attempt = service.account().begin_login();
        service.complete_login(attempt, &session, |_| {}).unwrap();
        Fixture {
            _dir: dir,
            transport,
            service,
        }
    }

    fn seed() -> SourceRef {
        SourceRef {
            kind: SourceKind::YouTube,
            id: "dQw4w9WgXcQ".to_owned(),
        }
    }

    #[tokio::test]
    async fn related_is_unsupported_and_sends_nothing() {
        let f = signed_in();

        let outcome = f.service.related(&seed()).await;

        assert_eq!(outcome, Err(SourceError::Unsupported));
        assert_eq!(f.transport.sent(), 0);
    }

    #[tokio::test]
    async fn a_search_gives_at_most_the_limit() {
        let f = signed_in();
        f.transport.answer(Ok(Response {
            status: 200,
            body: SONGS.to_owned(),
            ..Response::default()
        }));

        let tracks = f.service.search("paper", 2, || true).await.unwrap();

        assert_eq!(tracks.len(), 2);
        assert_eq!(f.transport.sent(), 1);
    }
}
