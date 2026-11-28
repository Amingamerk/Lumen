//! Shuffle and repeat over a fake engine: what Next and the end of a song do.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::audio::{FakeEngine, FakeTracks};
use crate::catalog::Catalog;
use crate::sources::youtube::testing::FakeClock;

const NOW_S: u64 = 1_700_000_000;
const SEED: u64 = 7;

/// The Recommender has nothing to offer.
struct Silent;

impl Recommender for Silent {
    fn recommend(&self, _: &[Uuid], _: Uuid, _: &[Uuid]) -> Option<Uuid> {
        None
    }
}

struct Rig {
    player: Player<FakeEngine>,
    engine: FakeEngine,
}

/// A library of `count` Tracks, with nothing playing yet.
fn rig_with(count: u128, recommender: Box<dyn Recommender>) -> Rig {
    let engine = FakeEngine::default();
    let mut player = Player::new(engine.clone(), Arc::new(Catalog::empty()), recommender)
        .with_clock(FakeClock::at(NOW_S * 1_000))
        .with_seed(SEED);
    player.set_tracks(Box::new(FakeTracks::of(count + 20)));
    Rig { player, engine }
}

fn rig(count: u128) -> Rig {
    rig_with(count, Box::new(Silent))
}

/// Playing the Tracks 1 to `count` from the given row.
fn playing(count: u128, start: usize) -> Rig {
    let mut rig = rig(count);
    rig.player.play_list(FakeTracks::ids(count), start).unwrap();
    rig.open();
    rig
}

fn title(state: &PlayerState) -> &str {
    &state.current.as_ref().unwrap().track.title
}

fn changed(event: Option<PlayerEvent>) -> PlayerState {
    match event {
        Some(PlayerEvent::Changed(state)) => state,
        other => panic!("expected a change, got {other:?}"),
    }
}

impl Rig {
    /// The due fetch, if there is one, resolves and the engine opens what is
    /// loading.
    fn open(&mut self) -> PlayerState {
        if let Some(fetch) = self.player.due_fetch() {
            let media = Media::Stream {
                url: format!("https://a.googlevideo.com/{}", fetch.video),
                headers: Vec::new(),
                expires_at: Some(NOW_S + 21_600),
            };
            self.player.finish_fetch(fetch.ticket, Ok(media)).unwrap();
        }
        self.engine.fake().duration = Duration::from_secs(200);
        changed(self.player.on_engine_event(EngineEvent::Loaded))
    }

    /// The song plays to its end.
    fn end(&mut self) -> PlayerState {
        self.engine.fake().position = Duration::from_secs(200);
        changed(self.player.on_engine_event(EngineEvent::Ended))
    }

    fn upcoming(&self) -> Vec<String> {
        self.player
            .queue_slice(0, MAX_SLICE)
            .items
            .into_iter()
            .map(|item| item.track.title)
            .collect()
    }
}

#[test]
fn repeat_all_wraps_to_the_first_song_after_the_last() {
    let mut rig = playing(3, 2);
    rig.player.set_repeat(Repeat::All);

    let state = rig.end();

    assert_eq!(title(&state), "Track 1");
    assert_eq!((state.history, state.upcoming), (0, 2));
    assert_eq!(rig.open().status, PlaybackStatus::Playing);
}

#[test]
fn repeat_one_replays_a_song_that_ended_by_itself() {
    let mut rig = playing(3, 0);
    rig.player.set_repeat(Repeat::One);
    let key = rig.player.state().current.unwrap().key;

    let ended = rig.end();
    let replay = rig.open();

    assert_eq!(title(&ended), "Track 1");
    assert_eq!(replay.status, PlaybackStatus::Playing);
    assert_eq!(replay.current.unwrap().key, key);
    assert_eq!((replay.history, replay.upcoming), (0, 2));
    let loads = rig.engine.fake().loads.clone();
    assert_eq!(loads.len(), 2);
    // From the start, with the stream it already had.
    assert_eq!(loads[1], loads[0]);
}

#[test]
fn next_still_advances_under_repeat_one() {
    let mut rig = playing(3, 0);
    rig.player.set_repeat(Repeat::One).expect("repeat changes");

    let state = rig.player.next_track().unwrap();

    assert_eq!(title(&state), "Track 2");
}

#[test]
fn play_next_while_shuffled_plays_that_song_next() {
    let mut rig = playing(10, 0);
    let shuffled = rig.player.set_shuffle(true).expect("shuffle turns on");
    assert!(shuffled.upcoming == 9 && rig.upcoming()[0] != "Track 2");

    rig.player
        .enqueue(Uuid::from_u128(15), Placement::Next)
        .unwrap();

    assert_eq!(rig.upcoming()[0], "Track 15");
    assert_eq!(title(&rig.end()), "Track 15");
}

/// Offers one library Track, whenever it is not waiting.
struct Offers(Uuid);

impl Recommender for Offers {
    fn recommend(&self, _: &[Uuid], current: Uuid, upcoming: &[Uuid]) -> Option<Uuid> {
        (current != self.0 && !upcoming.contains(&self.0)).then_some(self.0)
    }
}

/// Playing the last of two Tracks, with a pick on offer.
fn at_the_end_with_a_pick() -> Rig {
    let mut rig = rig_with(2, Box::new(Offers(Uuid::from_u128(15))));
    rig.player.play_list(FakeTracks::ids(2), 1).unwrap();
    rig.open();
    rig
}

#[test]
fn a_pick_follows_the_end_only_with_repeat_off() {
    let mut rig = at_the_end_with_a_pick();

    assert_eq!(title(&rig.end()), "Track 15");
}

#[test]
fn repeat_all_starts_over_instead_of_picking() {
    let mut rig = at_the_end_with_a_pick();
    rig.player.set_repeat(Repeat::All);

    let state = rig.player.next_track().unwrap();

    assert_eq!(title(&state), "Track 1");
    assert_eq!((state.history, state.upcoming), (0, 1));
}

#[test]
fn repeat_one_has_no_next_past_the_last_song_and_picks_nothing() {
    let mut rig = at_the_end_with_a_pick();
    let state = rig.player.set_repeat(Repeat::One).unwrap();

    assert!(!state.can_next);
    assert_eq!(rig.player.next_track(), None);
    assert_eq!(title(&rig.end()), "Track 2");
    assert_eq!(rig.player.state().upcoming, 0);
}

#[test]
fn next_can_always_go_on_under_repeat_all() {
    let mut rig = playing(2, 1);
    assert!(!rig.player.state().can_next);

    let state = rig.player.set_repeat(Repeat::All).unwrap();

    assert!(state.can_next);
}

#[test]
fn previous_goes_back_through_what_played_while_shuffled() {
    let mut rig = playing(10, 0);
    rig.player.set_shuffle(true);
    let second = title(&rig.end()).to_owned();
    rig.open();
    rig.end();
    rig.open();

    let back = rig.player.previous_track().unwrap();
    assert_eq!(title(&back), second);
    rig.open();
    assert_eq!(title(&rig.player.previous_track().unwrap()), "Track 1");
}

#[test]
fn previous_after_starting_over_returns_to_the_last_song() {
    let mut rig = playing(3, 2);
    rig.player.set_repeat(Repeat::All);
    rig.end();
    rig.open();

    let state = rig.player.previous_track().unwrap();

    assert_eq!(title(&state), "Track 3");
}

#[test]
fn a_song_that_stands_alone_repeats_in_either_mode() {
    for repeat in [Repeat::One, Repeat::All] {
        let mut rig = rig(0);
        let row = TrackRow::from(&FakeTracks::track(1));
        rig.player.play_media(Media::File("song.wav".into()), row);
        rig.player.on_engine_event(EngineEvent::Loaded);
        rig.player.set_repeat(repeat);

        let ended = rig.end();

        assert_eq!(ended.status, PlaybackStatus::Loading);
        assert_eq!(rig.engine.fake().loads.len(), 2);
    }
}

#[test]
fn repeat_one_resolves_again_when_the_stream_has_expired() {
    let clock = FakeClock::at(NOW_S * 1_000);
    let mut rig = playing(2, 0);
    rig.player = rig.player.with_clock(clock.clone());
    rig.player.set_repeat(Repeat::One);
    clock.advance(Duration::from_secs(30_000));

    rig.end();

    let fetch = rig.player.due_fetch().expect("a fresh stream is asked for");
    assert_eq!(fetch.video, "v1");
    assert_eq!(rig.engine.fake().loads.len(), 1);
}

#[test]
fn a_song_that_cannot_be_played_is_not_tried_again_and_again() {
    let mut rig = rig(1);
    rig.player.set_repeat(Repeat::All);
    rig.player.play_list(FakeTracks::ids(1), 0).unwrap();
    let fetch = rig.player.due_fetch().unwrap();

    let state = rig
        .player
        .finish_fetch(
            fetch.ticket,
            Err(NoStream {
                reason: "this song is unavailable".to_owned(),
                retry: false,
                skip: true,
            }),
        )
        .unwrap();

    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(rig.player.due_fetch(), None);
}

#[test]
fn the_state_says_which_modes_are_on() {
    let mut rig = playing(3, 0);
    assert_eq!(rig.player.modes(), (false, Repeat::Off));
    let before = rig.player.state();

    let shuffled = rig.player.toggle_shuffle().unwrap();
    let repeating = rig.player.cycle_repeat().unwrap();

    assert!(shuffled.shuffle && shuffled.queue_revision > before.queue_revision);
    assert_eq!(repeating.repeat, Repeat::All);
    assert_eq!(rig.player.cycle_repeat().unwrap().repeat, Repeat::One);
    assert_eq!(rig.player.cycle_repeat().unwrap().repeat, Repeat::Off);
    assert!(!rig.player.toggle_shuffle().unwrap().shuffle);
    assert_eq!(rig.player.set_shuffle(false), None);
}

#[test]
fn shuffling_5000_songs_keeps_the_state_small() {
    let mut rig = rig(5_000);
    rig.player.play_list(FakeTracks::ids(5_000), 0).unwrap();

    let state = rig.player.set_shuffle(true).unwrap();

    assert_eq!(state.upcoming, 4_999);
    assert!(serde_json::to_vec(&state).unwrap().len() < 1_024);
}
