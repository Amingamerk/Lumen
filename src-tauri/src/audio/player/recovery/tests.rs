//! Recovery over a fake engine and a clock moved by hand.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::audio::{EngineError, FakeEngine};
use crate::recs::StaticRecommender;
use crate::sources::youtube::testing::FakeClock;

const VIDEO: &str = "dQw4w9WgXcQ";
const NOW_S: u64 = 1_700_000_000;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn stream() -> Media {
    expiring(NOW_S + 21_600)
}

fn expiring(at: u64) -> Media {
    Media::Stream {
        url: format!("https://a.googlevideo.com/x?expire={at}"),
        headers: Vec::new(),
        expires_at: Some(at),
    }
}

struct Rig {
    player: Player<FakeEngine>,
    engine: FakeEngine,
    clock: FakeClock,
}

/// A song whose stream is being resolved, with its key.
fn resolving() -> (Rig, u32) {
    let engine = FakeEngine::default();
    let clock = FakeClock::at(NOW_S * 1_000);
    let catalog = Arc::<crate::catalog::Catalog>::default();
    let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
    let mut player = Player::new(engine.clone(), catalog, recommender).with_clock(clock.clone());
    let (key, _) = player.begin_stream(VIDEO.to_owned(), TrackRow::untagged("Paper".to_owned()));
    let rig = Rig {
        player,
        engine,
        clock,
    };
    (rig, key)
}

/// A stream of 200 seconds that has opened and plays.
fn playing_of(media: Media) -> Rig {
    let (mut rig, key) = resolving();
    rig.player.finish_stream(key, Ok(media)).unwrap();
    rig.open();
    rig
}

fn playing() -> Rig {
    playing_of(stream())
}

impl Rig {
    /// The engine opens what it was last given.
    fn open(&mut self) {
        self.engine.fake().duration = secs(200);
        self.player.on_engine_event(EngineEvent::Loaded).unwrap();
    }

    /// The engine reports having come to `position`.
    fn at(&mut self, position: Duration) {
        self.engine.fake().position = position;
        self.player.on_engine_event(EngineEvent::Progress);
    }

    fn fail(&mut self) -> PlayerState {
        match self.player.on_engine_event(EngineEvent::Failed) {
            Some(PlayerEvent::Changed(state)) => state,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    fn end(&mut self) -> PlayerState {
        match self.player.on_engine_event(EngineEvent::Ended) {
            Some(PlayerEvent::Changed(state)) => state,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    /// Waits for the planned fetch and takes it.
    fn due_after(&mut self, wait: Duration) -> Fetch {
        self.clock.advance(wait - ms(1));
        assert_eq!(self.player.due_fetch(), None, "asked too early");
        self.clock.advance(ms(1));
        self.player.due_fetch().expect("a fetch is due")
    }

    /// One automatic attempt that gets a stream which then opens.
    fn recover(&mut self, wait: Duration) {
        let fetch = self.due_after(wait);
        self.player
            .finish_fetch(fetch.ticket, Ok(stream()))
            .unwrap();
        self.open();
    }

    fn loads(&self) -> Vec<Duration> {
        self.engine.fake().loads.iter().map(|(_, at)| *at).collect()
    }

    fn status(&self) -> PlaybackStatus {
        self.player.state().status
    }
}

fn no_stream(retry: bool) -> NoStream {
    NoStream {
        reason: "no answer".to_owned(),
        retry,
        skip: false,
    }
}

// Triggers.

#[test]
fn a_mid_track_failure_reconnects_and_resumes_where_it_was() {
    let mut rig = playing();
    rig.player.set_volume(35);
    rig.at(secs(42));

    let state = rig.fail();

    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    assert_eq!((state.position_ms, state.duration_ms), (42_000, 200_000));
    assert_eq!(state.error, None);
    assert_eq!(rig.player.take_dropped(), Some(VIDEO.to_owned()));
    let fetch = rig.due_after(secs(2));
    assert_eq!(fetch.video, VIDEO);
    let state = rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    assert_eq!(rig.loads(), [Duration::ZERO, secs(42)]);
    rig.open();
    assert_eq!(rig.status(), PlaybackStatus::Playing);
    assert!(rig.engine.fake().playing);
    assert_eq!(rig.engine.fake().volume, 35);
}

#[test]
fn a_load_the_engine_refuses_is_recovered() {
    let (mut rig, key) = resolving();
    rig.engine.fake().refuse = Some(EngineError::Unplayable);

    let state = rig.player.finish_stream(key, Ok(stream())).unwrap();

    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    rig.engine.fake().refuse = None;
    rig.recover(secs(2));
    assert_eq!(rig.status(), PlaybackStatus::Playing);
}

#[test]
fn ten_seconds_without_progress_is_a_stall() {
    let mut rig = playing();
    rig.clock.advance(secs(8));
    rig.at(secs(8));
    rig.clock.advance(secs(10) - ms(1));
    // Hearing the same position again is not progress.
    rig.at(secs(8));
    assert_eq!(rig.player.check_stall(), None);
    rig.clock.advance(ms(1));

    let state = rig.player.check_stall().expect("stalled");

    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    assert_eq!(state.position_ms, 8_000);
    assert_eq!(rig.player.check_stall(), None);
}

#[test]
fn a_stream_that_never_opens_stalls() {
    let (mut rig, key) = resolving();
    rig.clock.advance(secs(60));
    // Resolving is not a stall.
    assert_eq!(rig.player.check_stall(), None);
    rig.player.finish_stream(key, Ok(stream())).unwrap();
    rig.clock.advance(secs(10));

    assert_eq!(
        rig.player.check_stall().map(|s| s.status),
        Some(PlaybackStatus::Reconnecting)
    );
}

#[test]
fn a_paused_stream_never_stalls_and_gets_ten_fresh_seconds_on_resume() {
    let mut rig = playing();
    rig.player.toggle_play();
    rig.clock.advance(secs(3_600));
    assert_eq!(rig.player.check_stall(), None);
    assert_eq!(rig.player.recovery_wake(), None);

    rig.player.toggle_play();

    assert_eq!(rig.player.check_stall(), None);
    assert_eq!(rig.player.recovery_wake(), Some(secs(10)));
}

#[test]
fn a_seek_gets_ten_fresh_seconds() {
    let mut rig = playing();
    rig.clock.advance(secs(9));
    rig.player.seek(100_000);
    rig.clock.advance(secs(9));

    assert_eq!(rig.player.check_stall(), None);
}

#[test]
fn an_end_more_than_five_seconds_early_is_recovered() {
    let mut rig = playing();
    rig.at(secs(195) - ms(1));

    let state = rig.end();

    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    assert_eq!(state.position_ms, 194_999);
}

#[test]
fn an_end_within_five_seconds_is_the_end() {
    let mut rig = playing();
    rig.at(secs(195));

    let state = rig.end();

    assert_eq!(state.status, PlaybackStatus::Ended);
    assert_eq!(rig.player.recovery_wake(), None);
}

#[test]
fn a_failure_while_opening_again_resumes_at_the_new_place() {
    let mut rig = playing();
    rig.at(secs(40));
    rig.fail();
    rig.recover(secs(2));
    rig.at(secs(200));
    rig.end();
    rig.player.seek(30_000).unwrap();

    let state = rig.fail();

    assert_eq!(state.position_ms, 30_000);
}

#[test]
fn nothing_after_the_end_starts_a_recovery() {
    let mut rig = playing();
    rig.at(secs(200));
    rig.end();

    assert_eq!(rig.player.on_engine_event(EngineEvent::Failed), None);
    assert_eq!(rig.status(), PlaybackStatus::Ended);
    assert!(!rig.player.fetch_pending());
}

// The bound.

#[test]
fn two_attempts_then_an_error_that_counts_once() {
    let mut rig = playing();
    rig.at(secs(20));
    rig.fail();
    rig.recover(secs(2));
    rig.at(secs(30));
    rig.fail();
    rig.recover(secs(10));
    rig.at(secs(40));
    assert!(!rig.player.take_gave_up());

    let state = rig.fail();

    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(
        state.error.as_deref(),
        Some("Lumen could not play “Paper”.")
    );
    assert_eq!(state.position_ms, 40_000);
    assert!(rig.player.take_gave_up());
    assert!(!rig.player.take_gave_up());
    rig.clock.advance(secs(3_600));
    assert_eq!(rig.player.due_fetch(), None);
    assert_eq!(rig.player.recovery_wake(), None);
    assert_eq!(rig.loads(), [Duration::ZERO, secs(20), secs(30)]);
}

#[test]
fn resolves_that_fail_use_up_the_attempts() {
    let mut rig = playing();
    rig.fail();
    let first = rig.due_after(secs(2));
    let state = rig
        .player
        .finish_fetch(first.ticket, Err(no_stream(true)))
        .unwrap();
    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    let second = rig.due_after(secs(10));

    let state = rig
        .player
        .finish_fetch(second.ticket, Err(no_stream(true)))
        .unwrap();

    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(
        state.error.as_deref(),
        Some("Lumen could not play “Paper”: no answer.")
    );
    assert!(rig.player.take_gave_up());
}

#[test]
fn an_answer_not_worth_asking_again_stops_at_once() {
    let mut rig = playing();
    rig.fail();
    let fetch = rig.due_after(secs(2));

    let state = rig
        .player
        .finish_fetch(fetch.ticket, Err(no_stream(false)))
        .unwrap();

    assert_eq!(state.status, PlaybackStatus::Error);
    // The resolve itself has told the gate what it had to.
    assert!(!rig.player.take_gave_up());
    assert_eq!(rig.player.recovery_wake(), None);
}

#[test]
fn it_wakes_for_the_stall_watch_or_the_planned_fetch() {
    let mut rig = playing();
    assert_eq!(rig.player.recovery_wake(), Some(secs(10)));
    rig.clock.advance(secs(4));
    assert_eq!(rig.player.recovery_wake(), Some(secs(6)));

    rig.fail();
    assert_eq!(rig.player.recovery_wake(), Some(secs(2)));

    rig.due_after(secs(2));
    // A resolve in progress is not watched for a stall.
    assert_eq!(rig.player.recovery_wake(), None);
}

// Cancelling.

#[test]
fn a_pause_cancels_the_planned_fetch() {
    let mut rig = playing();
    rig.at(secs(42));
    rig.fail();

    let paused = rig.player.toggle_play().unwrap();

    assert_eq!(paused.status, PlaybackStatus::Paused);
    assert_eq!(paused.position_ms, 42_000);
    assert!(!rig.player.fetch_pending());
    rig.clock.advance(secs(3_600));
    assert_eq!(rig.player.due_fetch(), None);
}

#[test]
fn a_result_that_arrives_after_a_pause_is_not_loaded() {
    let mut rig = playing();
    rig.fail();
    let fetch = rig.due_after(secs(2));
    rig.player.toggle_play();

    assert!(!rig.player.awaits_fetch(fetch.ticket));
    assert_eq!(rig.player.finish_fetch(fetch.ticket, Ok(stream())), None);
    assert_eq!(rig.loads().len(), 1);

    // Resuming asks anew, and that was the second and last attempt's turn.
    let resumed = rig.player.toggle_play().unwrap();
    assert_eq!(resumed.status, PlaybackStatus::Reconnecting);
    let again = rig.due_after(secs(10) - secs(2));
    assert!(again.ticket != fetch.ticket);
    assert_eq!(rig.player.finish_fetch(fetch.ticket, Ok(stream())), None);
    assert!(
        rig.player
            .finish_fetch(again.ticket, Ok(stream()))
            .is_some()
    );
}

#[test]
fn a_failure_while_paused_waits_for_the_resume() {
    let mut rig = playing();
    rig.at(secs(42));
    rig.player.toggle_play();

    let state = rig.fail();

    assert_eq!(state.status, PlaybackStatus::Paused);
    assert!(!rig.player.fetch_pending());
    rig.clock.advance(secs(60));
    rig.player.toggle_play();
    let fetch = rig.player.due_fetch().expect("long past its delay");
    rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    assert_eq!(rig.loads(), [Duration::ZERO, secs(42)]);
    assert!(rig.engine.fake().playing);
}

#[test]
fn a_seek_while_a_stream_is_gone_moves_where_it_resumes() {
    let mut rig = playing();
    rig.player.toggle_play();
    rig.fail();

    let state = rig.player.seek(150_000).unwrap();

    assert_eq!(state.position_ms, 150_000);
    assert!(!rig.player.fetch_pending());
    assert_eq!(rig.player.seek_by(-10_000).unwrap().position_ms, 140_000);
    assert_eq!(rig.player.seek(999_999).unwrap().position_ms, 200_000);
}

#[test]
fn choosing_another_track_cancels_the_recovery() {
    let mut rig = playing();
    rig.fail();
    let fetch = rig.due_after(secs(2));

    let (key, _) = rig.player.begin_stream(
        "aaaaaaaaaaa".to_owned(),
        TrackRow::untagged("New".to_owned()),
    );

    assert!(!rig.player.awaits_fetch(fetch.ticket));
    assert_eq!(rig.player.finish_fetch(fetch.ticket, Ok(stream())), None);
    assert!(rig.player.awaits(key));
    assert_eq!(rig.loads().len(), 1);
    // The new song has both attempts.
    rig.player.finish_stream(key, Ok(stream())).unwrap();
    rig.open();
    rig.fail();
    rig.recover(secs(2));
}

#[test]
fn an_abandoned_recovery_says_why() {
    let mut rig = playing();
    rig.fail();
    let fetch = rig.due_after(secs(2));

    let state = rig.player.abandon_fetch("YouTube is paused").unwrap();

    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(
        state.error.as_deref(),
        Some("Lumen could not play “Paper”: YouTube is paused.")
    );
    assert_eq!(rig.player.finish_fetch(fetch.ticket, Ok(stream())), None);
    assert!(!rig.player.take_gave_up());
    assert_eq!(rig.player.abandon_fetch("again"), None);
}

// Expiry.

/// Paused at 42 seconds, with the URL expired since.
fn expired() -> Rig {
    let mut rig = playing_of(expiring(NOW_S + 100));
    rig.at(secs(42));
    rig.player.toggle_play();
    rig.clock.advance(secs(100));
    rig
}

#[test]
fn resuming_an_expired_stream_resolves_a_new_one_first() {
    let mut rig = expired();

    let state = rig.player.toggle_play().unwrap();

    assert_eq!(state.status, PlaybackStatus::Loading);
    assert_eq!(state.position_ms, 42_000);
    assert!(!rig.engine.fake().playing);
    assert_eq!(rig.loads().len(), 1);
    assert_eq!(rig.player.take_dropped(), Some(VIDEO.to_owned()));
    let fetch = rig.player.due_fetch().expect("at once");
    rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    assert_eq!(rig.loads(), [Duration::ZERO, secs(42)]);
    rig.open();
    assert_eq!(rig.status(), PlaybackStatus::Playing);
}

#[test]
fn resuming_before_the_expiry_resolves_nothing() {
    let mut rig = playing_of(expiring(NOW_S + 100));
    rig.player.toggle_play();
    rig.clock.advance(secs(99));

    let state = rig.player.toggle_play().unwrap();

    assert_eq!(state.status, PlaybackStatus::Playing);
    assert!(!rig.player.fetch_pending());
}

#[test]
fn seeking_an_expired_stream_resolves_first_and_stays_paused() {
    let mut rig = expired();

    let state = rig.player.seek(90_000).unwrap();

    assert_eq!(
        (state.status, state.position_ms),
        (PlaybackStatus::Paused, 90_000)
    );
    let fetch = rig.player.due_fetch().expect("at once");
    let state = rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    assert_eq!(state.status, PlaybackStatus::Paused);
    assert_eq!(rig.loads(), [Duration::ZERO, secs(90)]);
    assert!(!rig.engine.fake().playing);
}

#[test]
fn seeking_an_expired_stream_while_playing_resolves_first() {
    let mut rig = playing_of(expiring(NOW_S + 100));
    rig.clock.advance(secs(100));

    let state = rig.player.seek(90_000).unwrap();

    assert_eq!(state.status, PlaybackStatus::Loading);
    assert!(rig.player.due_fetch().is_some());
    assert_eq!(rig.loads().len(), 1);
}

#[test]
fn replaying_an_expired_stream_resolves_first() {
    let mut rig = playing_of(expiring(NOW_S + 300));
    rig.at(secs(200));
    rig.end();
    rig.clock.advance(secs(300));

    let state = rig.player.toggle_play().unwrap();

    assert_eq!(
        (state.status, state.position_ms),
        (PlaybackStatus::Loading, 0)
    );
    assert!(rig.player.due_fetch().is_some());
}

#[test]
fn replacing_an_expired_url_is_not_one_of_the_two_attempts() {
    let mut rig = expired();
    rig.player.toggle_play();
    let fetch = rig.player.due_fetch().unwrap();
    rig.player.finish_fetch(fetch.ticket, Ok(stream())).unwrap();
    rig.open();

    rig.fail();
    rig.recover(secs(2));
    rig.fail();
    rig.recover(secs(10));
    assert_eq!(rig.status(), PlaybackStatus::Playing);
}

#[test]
fn a_failed_replacement_of_an_expired_url_is_recovered() {
    let mut rig = expired();
    rig.player.toggle_play();
    let fetch = rig.player.due_fetch().unwrap();

    let state = rig
        .player
        .finish_fetch(fetch.ticket, Err(no_stream(true)))
        .unwrap();

    assert_eq!(state.status, PlaybackStatus::Reconnecting);
    rig.recover(secs(2));
}

#[test]
fn a_pause_cancels_replacing_an_expired_url() {
    let mut rig = expired();
    rig.player.toggle_play();
    let fetch = rig.player.due_fetch().unwrap();

    let state = rig.player.toggle_play().unwrap();

    assert_eq!(state.status, PlaybackStatus::Paused);
    assert_eq!(rig.player.finish_fetch(fetch.ticket, Ok(stream())), None);
    // Seeking now still needs a fresh URL.
    rig.player.seek(10_000).unwrap();
    assert!(rig.player.due_fetch().is_some());
}

// The manual retry.

#[test]
fn a_retry_starts_a_new_playback_where_the_old_one_stopped() {
    let mut rig = playing();
    rig.at(secs(20));
    rig.fail();
    rig.recover(secs(2));
    rig.fail();
    rig.recover(secs(10));
    rig.fail();
    assert_eq!(rig.player.toggle_play(), None);

    let (video, key, state) = rig.player.retry_stream().unwrap();

    assert_eq!(video, VIDEO);
    assert_eq!(
        (state.status, state.position_ms),
        (PlaybackStatus::Loading, 20_000)
    );
    rig.player.finish_stream(key, Ok(stream())).unwrap();
    assert_eq!(rig.loads().last(), Some(&secs(20)));
    rig.open();
    rig.fail();
    rig.recover(secs(2));
}

#[test]
fn only_a_failed_stream_can_be_retried() {
    let mut rig = playing();

    assert_eq!(rig.player.retry_stream(), None);
    assert_eq!(rig.status(), PlaybackStatus::Playing);
}

#[test]
fn every_change_wakes_whoever_watches() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut rig = playing();
    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    rig.player.on_change(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });

    rig.at(secs(3));
    assert_eq!(woken.load(Ordering::SeqCst), 0);
    rig.fail();

    assert_eq!(woken.load(Ordering::SeqCst), 1);
}
