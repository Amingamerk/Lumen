//! The watcher resolving a queued list: only the current entry, one at a time,
//! with a fake resolver and a fake engine. Nothing here contacts YouTube.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::tests::RecordingSink;
use super::*;
use crate::audio::{EngineEvent, FakeEngine, FakeTracks, PlaybackStatus, Player};
use crate::catalog::Catalog;
use crate::recs::StaticRecommender;
use crate::resolver::testing::FakeResolver;
use crate::resolver::{ResolveError, StreamInfo};
use crate::secrets::fake::MemoryStore;
use crate::sources::youtube::testing::{
    FakeClock, FakeTransport, TempDir, music_page, signed_in_cookies,
};
use crate::sources::youtube::{Account, Session};

const NOW_MS: u64 = 1_700_000_000_000;

type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;

struct Rig {
    _gate: TempDir,
    youtube: Arc<Service>,
    resolver: FakeResolver,
    engine: FakeEngine,
    player: Mutex<AppPlayer>,
    sink: RecordingSink,
}

fn rig(tracks: u128) -> Rig {
    let gate = TempDir::new();
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
    let engine = FakeEngine::default();
    let catalog = Arc::new(Catalog::empty());
    let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
    let boxed: Box<dyn crate::audio::Engine> = Box::new(engine.clone());
    let mut player = Player::new(boxed, catalog, recommender).with_clock(clock);
    player.set_tracks(Box::new(FakeTracks::of(tracks)));
    Rig {
        _gate: gate,
        youtube,
        resolver: FakeResolver::default(),
        engine,
        player: Mutex::new(player),
        sink: RecordingSink::default(),
    }
}

fn stream() -> StreamInfo {
    StreamInfo::fake("https://a.googlevideo.com/x", 1_700_021_600)
}

impl Rig {
    fn play(&self, start: usize) {
        let ids = FakeTracks::ids(3);
        lock(&self.player).play_list(ids, start).unwrap();
    }

    /// One look of the watcher, and the resolve it asks for, if it asks.
    async fn watch(&self) {
        if let Next::Fetch(request) = step(&self.player, &self.youtube, &self.resolver, &self.sink)
        {
            fetch(&self.player, &self.resolver, &self.sink, request).await;
        }
    }

    /// The engine opens the stream and plays it to its end.
    fn play_to_the_end(&self) {
        self.engine.fake().duration = Duration::from_secs(200);
        let mut player = lock(&self.player);
        player.on_engine_event(EngineEvent::Loaded).unwrap();
        self.engine.fake().position = Duration::from_secs(200);
        player.on_engine_event(EngineEvent::Ended).unwrap();
    }

    fn asked(&self) -> Vec<String> {
        self.resolver.resolved().into_iter().map(|r| r.id).collect()
    }

    fn state(&self) -> crate::audio::PlayerState {
        lock(&self.player).state()
    }
}

#[tokio::test]
async fn the_end_of_a_song_resolves_the_next_entry_and_no_other() {
    let rig = rig(3);
    rig.resolver.answer(Ok(stream()));
    rig.resolver.answer(Ok(stream()));
    rig.play(0);

    rig.watch().await;
    rig.play_to_the_end();
    rig.watch().await;

    assert_eq!(rig.asked(), ["v1", "v2"]);
    assert_eq!(rig.state().current.unwrap().track.title, "Track 2");
    assert_eq!(rig.engine.fake().loads.len(), 2);
    // Nothing more is asked for until something changes.
    rig.watch().await;
    assert_eq!(rig.asked(), ["v1", "v2"]);
}

#[tokio::test]
async fn a_failed_resolve_skips_with_a_notice_and_the_next_song_is_resolved() {
    let rig = rig(3);
    rig.resolver.answer(Err(ResolveError::Unavailable));
    rig.resolver.answer(Ok(stream()));
    rig.play(0);

    rig.watch().await;

    let skipped = rig.sink.states.borrow().last().cloned().unwrap();
    assert_eq!(skipped.current.as_ref().unwrap().track.title, "Track 2");
    assert_eq!(
        skipped.skipped.unwrap().message,
        "Lumen skipped “Track 1”: this song is unavailable."
    );
    rig.watch().await;
    assert_eq!(rig.asked(), ["v1", "v2"]);
    assert_eq!(rig.engine.fake().loads.len(), 1);
}

#[tokio::test]
async fn a_refusal_by_the_account_stops_instead_of_skipping() {
    let rig = rig(3);
    rig.resolver.answer(Err(ResolveError::RateLimited));
    rig.play(0);

    rig.watch().await;
    rig.watch().await;

    assert_eq!(rig.asked(), ["v1"]);
    let state = rig.state();
    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(state.current.unwrap().track.title, "Track 1");
    assert_eq!(state.skipped, None);
}
