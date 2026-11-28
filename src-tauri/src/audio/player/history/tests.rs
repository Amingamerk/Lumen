//! Plays are logged once audio starts, and only then.

use std::sync::{Arc, Mutex};

use super::*;
use crate::audio::{EngineEvent, FakeEngine};
use crate::recs::StaticRecommender;
use crate::sources::youtube::testing::FakeClock;

const NOW_MS: u64 = 1_700_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Logged {
    Started {
        title: String,
        at_ms: u64,
    },
    Ended {
        play: PlayId,
        listened: Duration,
        reason: EndReason,
    },
}

#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<Logged>>>);

impl PlayLog for Journal {
    fn started(&self, track: &Track, started_at_ms: u64) -> PlayId {
        let mut all = self.0.lock().unwrap();
        all.push(Logged::Started {
            title: track.title.clone(),
            at_ms: started_at_ms,
        });
        all.iter()
            .filter(|l| matches!(l, Logged::Started { .. }))
            .count() as PlayId
    }

    fn ended(&self, play: PlayId, listened: Duration, reason: EndReason) {
        self.0.lock().unwrap().push(Logged::Ended {
            play,
            listened,
            reason,
        });
    }
}

impl Journal {
    fn all(&self) -> Vec<Logged> {
        self.0.lock().unwrap().clone()
    }

    fn starts(&self) -> usize {
        self.all()
            .iter()
            .filter(|l| matches!(l, Logged::Started { .. }))
            .count()
    }
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn stream() -> Media {
    Media::Stream {
        url: "https://a.googlevideo.com/x".to_owned(),
        headers: Vec::new(),
        expires_at: None,
    }
}

struct Rig {
    player: Player<FakeEngine>,
    engine: FakeEngine,
    clock: FakeClock,
    journal: Journal,
}

fn song(title: &str, n: u128) -> TrackRow {
    TrackRow {
        id: Uuid::from_u128(n),
        title: title.to_owned(),
        ..TrackRow::untagged(String::new())
    }
}

fn rig() -> Rig {
    let engine = FakeEngine::default();
    let clock = FakeClock::at(NOW_MS);
    let journal = Journal::default();
    let catalog = Arc::<Catalog>::default();
    let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
    let player = Player::new(engine.clone(), catalog, recommender)
        .with_clock(clock.clone())
        .with_log(Box::new(journal.clone()));
    Rig {
        player,
        engine,
        clock,
        journal,
    }
}

impl Rig {
    /// Chooses a song and resolves it, but its audio has not started.
    fn choose(&mut self, title: &str, n: u128) -> u32 {
        let (key, _) = self
            .player
            .begin_stream(format!("video{n:06}"), song(title, n));
        self.player.finish_stream(key, Ok(stream())).unwrap();
        key
    }

    /// The engine opens what it was given: the audio starts.
    fn open(&mut self) {
        self.engine.fake().duration = secs(200);
        self.player.on_engine_event(EngineEvent::Loaded);
    }

    fn play(&mut self, title: &str, n: u128) {
        self.choose(title, n);
        self.open();
    }

    fn listen(&self, time: Duration) {
        self.clock.advance(time);
    }
}

#[test]
fn nothing_is_logged_until_the_audio_starts() {
    let mut rig = rig();

    rig.choose("Paper", 1);
    rig.listen(secs(3));

    assert!(rig.journal.all().is_empty());

    rig.open();

    assert_eq!(
        rig.journal.all(),
        [Logged::Started {
            title: "Paper".to_owned(),
            at_ms: NOW_MS + 3_000
        }]
    );
}

#[test]
fn pausing_and_seeking_add_no_rows_and_no_listened_time() {
    let mut rig = rig();
    rig.play("Paper", 1);
    rig.listen(secs(10));
    rig.player.toggle_play().unwrap();
    rig.listen(secs(30));
    rig.player.toggle_play().unwrap();
    rig.listen(secs(5));
    rig.player.seek(150_000).unwrap();
    rig.player.seek(10_000).unwrap();

    rig.player.shutdown();

    assert_eq!(rig.journal.starts(), 1);
    assert_eq!(
        rig.journal.all()[1],
        Logged::Ended {
            play: 1,
            listened: secs(15),
            reason: EndReason::Exit
        }
    );
}

#[test]
fn recovering_a_stream_adds_no_second_row() {
    let mut rig = rig();
    rig.play("Paper", 1);
    rig.listen(secs(20));
    rig.engine.fake().position = secs(20);
    rig.player.on_engine_event(EngineEvent::Failed).unwrap();
    // Reconnecting: the fetch is due after the first wait.
    rig.listen(secs(2));
    let fetch = rig.player.due_fetch().expect("a fetch is due");
    rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    rig.open();
    rig.listen(secs(7));

    rig.player.shutdown();

    assert_eq!(rig.journal.starts(), 1, "{:?}", rig.journal.all());
    assert_eq!(
        rig.journal.all()[1],
        Logged::Ended {
            play: 1,
            listened: secs(27),
            reason: EndReason::Exit
        }
    );
}

#[test]
fn replacing_an_expired_stream_adds_no_second_row() {
    let mut rig = rig();
    let soon = NOW_MS / 1_000 + 60;
    let (key, _) = rig
        .player
        .begin_stream("video000001".to_owned(), song("Paper", 1));
    let expiring = Media::Stream {
        url: format!("https://a.googlevideo.com/x?expire={soon}"),
        headers: Vec::new(),
        expires_at: Some(soon),
    };
    rig.player.finish_stream(key, Ok(expiring)).unwrap();
    rig.open();
    rig.listen(secs(10));
    rig.player.toggle_play().unwrap();
    // The URL runs out while paused; resuming asks for a new one.
    rig.listen(secs(100));
    rig.player.toggle_play().unwrap();
    let fetch = rig.player.due_fetch().expect("the expired URL is replaced");
    rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    rig.open();
    rig.listen(secs(5));

    rig.player.shutdown();

    assert_eq!(rig.journal.starts(), 1, "{:?}", rig.journal.all());
    assert_eq!(
        rig.journal.all()[1],
        Logged::Ended {
            play: 1,
            listened: secs(15),
            reason: EndReason::Exit
        }
    );
}

#[test]
fn choosing_another_song_ends_the_play_and_starts_the_next() {
    let mut rig = rig();
    rig.play("Paper", 1);
    rig.listen(secs(12));

    rig.play("Stone", 2);

    let all = rig.journal.all();
    assert_eq!(rig.journal.starts(), 2);
    assert_eq!(
        all[1],
        Logged::Ended {
            play: 1,
            listened: secs(12),
            reason: EndReason::Replaced
        }
    );
}

#[test]
fn a_song_that_plays_to_its_end_is_finished_and_playing_it_again_is_a_new_play() {
    let mut rig = rig();
    rig.play("Paper", 1);
    rig.listen(secs(200));
    rig.engine.fake().position = secs(200);

    rig.player.on_engine_event(EngineEvent::Ended).unwrap();

    assert_eq!(
        rig.journal.all()[1],
        Logged::Ended {
            play: 1,
            listened: secs(200),
            reason: EndReason::Finished
        }
    );
    rig.player.toggle_play().unwrap();
    rig.open();
    assert_eq!(rig.journal.starts(), 2);
}

#[test]
fn giving_up_on_a_song_ends_its_play_as_failed() {
    let mut rig = rig();
    rig.play("Paper", 1);
    rig.listen(secs(6));
    // A file or live stream the engine cannot go on with, no recovery left.
    rig.player.direct.as_mut().unwrap().video = None;

    rig.player.on_engine_event(EngineEvent::Failed).unwrap();

    assert_eq!(
        rig.journal.all()[1],
        Logged::Ended {
            play: 1,
            listened: secs(6),
            reason: EndReason::Failed
        }
    );
}

#[test]
fn a_stream_that_never_resolved_logs_nothing() {
    let mut rig = rig();
    let (key, _) = rig
        .player
        .begin_stream("video000001".to_owned(), song("Paper", 1));

    rig.player
        .finish_stream(key, Err("unavailable".to_owned()))
        .unwrap();

    assert!(rig.journal.all().is_empty());
}

#[test]
fn a_local_file_is_not_a_song_of_the_library() {
    let mut rig = rig();

    rig.player.play_media(
        Media::File("C:/music/tone.wav".into()),
        TrackRow::untagged("tone".to_owned()),
    );
    rig.open();

    assert!(rig.journal.all().is_empty());
}

#[test]
fn nothing_is_logged_after_shutdown() {
    let mut rig = rig();
    rig.play("Paper", 1);
    rig.player.shutdown();

    rig.play("Stone", 2);

    assert_eq!(rig.journal.starts(), 1);
}

mod queued {
    use super::*;
    use crate::audio::SimulatedEngine;
    use crate::audio::simulated::ManualClock;

    /// Two sample songs in the Queue, the first playing.
    fn queued() -> (
        Player<SimulatedEngine<ManualClock>>,
        FakeClock,
        Journal,
        ManualClock,
    ) {
        let tracks = (1..=2)
            .map(|n| Track {
                id: Uuid::from_u128(n),
                title: format!("Track {n}"),
                artists: vec!["Mira Sol".to_owned()],
                album: None,
                duration_ms: 10_000,
                artwork_url: None,
                isrc: None,
                sources: vec![SourceRef {
                    kind: SourceKind::YouTube,
                    id: format!("ref-{n}"),
                }],
            })
            .collect();
        let catalog = Arc::new(Catalog::of(tracks));
        let engine_clock = ManualClock::default();
        let clock = FakeClock::at(NOW_MS);
        let journal = Journal::default();
        let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
        let mut player = Player::new(
            SimulatedEngine::new(engine_clock.clone()),
            catalog,
            recommender,
        )
        .with_clock(clock.clone())
        .with_log(Box::new(journal.clone()));
        player
            .play_tracks(vec![Uuid::from_u128(1), Uuid::from_u128(2)], 0)
            .unwrap();
        (player, clock, journal, engine_clock)
    }

    #[test]
    fn next_ends_the_play_as_skipped_and_starts_the_following_one() {
        let (mut player, clock, journal, _) = queued();
        clock.advance(secs(4));

        player.next_track().unwrap();

        let all = journal.all();
        assert_eq!(all.len(), 3, "{all:?}");
        assert_eq!(
            all[1],
            Logged::Ended {
                play: 1,
                listened: secs(4),
                reason: EndReason::Skipped
            }
        );
        assert!(matches!(&all[2], Logged::Started { title, .. } if title == "Track 2"));
    }

    #[test]
    fn restarting_a_song_with_previous_adds_no_row() {
        let (mut player, clock, journal, _) = queued();
        clock.advance(secs(4));

        player.previous_track().unwrap();
        player.shutdown();

        assert_eq!(journal.starts(), 1, "{:?}", journal.all());
    }
}
