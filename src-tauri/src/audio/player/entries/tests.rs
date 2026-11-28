//! Queued library Tracks over a fake engine, a clock moved by hand and no
//! network: what the Queue asks to have resolved, and when.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::audio::{EngineError, FakeEngine, FakeTracks};
use crate::catalog::{Catalog, SourceKind, SourceRef};
use crate::library::EndReason;
use crate::sources::youtube::testing::FakeClock;

const NOW_S: u64 = 1_700_000_000;

/// The Recommender has nothing to offer.
struct Silent;

impl Recommender for Silent {
    fn recommend(&self, _: &[Uuid], _: Uuid, _: &[Uuid]) -> Option<Uuid> {
        None
    }
}

/// Offers one library Track, once.
struct Offers(Uuid);

impl Recommender for Offers {
    fn recommend(&self, _: &[Uuid], current: Uuid, upcoming: &[Uuid]) -> Option<Uuid> {
        (current != self.0 && !upcoming.contains(&self.0)).then_some(self.0)
    }
}

struct Rig {
    player: Player<FakeEngine>,
    engine: FakeEngine,
    tracks: FakeTracks,
}

fn rig_with(count: u128, recommender: Box<dyn Recommender>) -> Rig {
    let engine = FakeEngine::default();
    let tracks = FakeTracks::of(count);
    let mut player = Player::new(engine.clone(), Arc::new(Catalog::empty()), recommender)
        .with_clock(FakeClock::at(NOW_S * 1_000));
    player.set_tracks(Box::new(tracks.clone()));
    Rig {
        player,
        engine,
        tracks,
    }
}

fn rig(count: u128) -> Rig {
    rig_with(count, Box::new(Silent))
}

fn stream(video: &str) -> Media {
    Media::Stream {
        url: format!("https://a.googlevideo.com/{video}"),
        headers: Vec::new(),
        expires_at: Some(NOW_S + 21_600),
    }
}

fn no_stream(skip: bool) -> NoStream {
    NoStream {
        reason: "this song is unavailable".to_owned(),
        retry: false,
        skip,
    }
}

fn title(state: &PlayerState) -> &str {
    &state.current.as_ref().unwrap().track.title
}

impl Rig {
    /// The video the watcher is asked to resolve, if one is due.
    fn due(&mut self) -> Option<(u32, String)> {
        self.player.due_fetch().map(|f| (f.ticket, f.video))
    }

    /// The due fetch resolves, and the engine opens the stream.
    fn open(&mut self) -> PlayerState {
        let (ticket, video) = self.due().expect("a fetch is due");
        self.player
            .finish_fetch(ticket, Ok(stream(&video)))
            .unwrap();
        self.engine.fake().duration = Duration::from_secs(200);
        match self.player.on_engine_event(EngineEvent::Loaded) {
            Some(PlayerEvent::Changed(state)) => state,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    /// The song plays to its end.
    fn end(&mut self) -> PlayerState {
        self.engine.fake().position = Duration::from_secs(200);
        match self.player.on_engine_event(EngineEvent::Ended) {
            Some(PlayerEvent::Changed(state)) => state,
            other => panic!("expected a change, got {other:?}"),
        }
    }
}

#[test]
fn playing_row_n_of_a_long_list_makes_it_current_with_the_rest_upcoming() {
    let mut rig = rig(5_000);

    let state = rig.player.play_list(FakeTracks::ids(5_000), 1_234).unwrap();

    assert_eq!(title(&state), "Track 1235");
    assert_eq!(state.status, PlaybackStatus::Loading);
    assert_eq!((state.history, state.upcoming), (1_234, 5_000 - 1_235));
    let slice = rig.player.queue_slice(0, 3);
    let titles: Vec<_> = slice.items.iter().map(|i| i.track.title.as_str()).collect();
    assert_eq!(titles, ["Track 1236", "Track 1237", "Track 1238"]);
    assert_eq!(slice.total, 5_000 - 1_235);
    // Only the current entry is asked for, and nothing is loaded before it is.
    assert_eq!(rig.due().unwrap().1, "v1235");
    assert_eq!(rig.due(), None);
    assert!(rig.engine.fake().loads.is_empty());
    // Nothing per row travels with the state.
    assert!(serde_json::to_string(&state).unwrap().len() < 1_000);
}

#[test]
fn a_resolved_entry_loads_and_plays() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();

    let state = rig.open();

    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(rig.engine.fake().loads.len(), 1);
    assert!(rig.engine.fake().playing);
    assert!(state.can_next && state.can_previous);
}

#[test]
fn the_end_of_a_song_moves_to_the_next_entry_and_asks_for_that_one_only() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    rig.open();

    let state = rig.end();

    assert_eq!(title(&state), "Track 2");
    assert_eq!(state.status, PlaybackStatus::Loading);
    assert_eq!((state.history, state.upcoming), (1, 1));
    assert_eq!(rig.due().unwrap().1, "v2");
    assert_eq!(rig.due(), None);
}

#[test]
fn next_and_previous_move_through_the_list() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 1).unwrap();
    rig.open();

    let next = rig.player.next_track().unwrap();
    assert_eq!(title(&next), "Track 3");
    assert_eq!(rig.due().unwrap().1, "v3");
    rig.player
        .finish_fetch(rig.player.tickets, Ok(stream("v3")));
    rig.engine.fake().position = Duration::from_secs(1);

    let back = rig.player.previous_track().unwrap();

    assert_eq!(title(&back), "Track 2");
    assert_eq!(rig.due().unwrap().1, "v2");
}

#[test]
fn the_last_entry_ending_stays_put_ended_without_a_recommendation() {
    let mut rig = rig(2);
    rig.player.play_list(FakeTracks::ids(2), 1).unwrap();
    rig.open();

    let state = rig.end();

    assert_eq!(title(&state), "Track 2");
    assert_eq!(state.status, PlaybackStatus::Ended);
    assert!(!state.can_next);
}

#[test]
fn a_recommendation_goes_on_after_the_last_entry() {
    let mut rig = rig_with(3, Box::new(Offers(Uuid::from_u128(99))));
    rig.tracks.add(FakeTracks::track(99));
    rig.player.play_list(FakeTracks::ids(2), 1).unwrap();
    rig.open();

    let state = rig.end();

    assert_eq!(title(&state), "Track 99");
    assert!(state.current.as_ref().unwrap().autoplay);
    assert_eq!(rig.due().unwrap().1, "v99");
}

#[test]
fn a_song_that_cannot_be_resolved_is_skipped_with_a_notice_and_the_next_one_plays() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    let (ticket, _) = rig.due().unwrap();

    let state = rig
        .player
        .finish_fetch(ticket, Err(no_stream(true)))
        .unwrap();

    assert_eq!(title(&state), "Track 2");
    assert_eq!(state.status, PlaybackStatus::Loading);
    let skipped = state.skipped.unwrap();
    assert_eq!(
        skipped.message,
        "Lumen skipped “Track 1”: this song is unavailable."
    );
    assert_eq!(rig.due().unwrap().1, "v2");
}

#[test]
fn a_problem_with_the_account_stops_instead_of_skipping() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    let (ticket, _) = rig.due().unwrap();

    let state = rig
        .player
        .finish_fetch(ticket, Err(no_stream(false)))
        .unwrap();

    assert_eq!(title(&state), "Track 1");
    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(state.skipped, None);
    assert_eq!(rig.due(), None);
}

#[test]
fn the_last_entry_failing_has_nowhere_to_skip_to() {
    let mut rig = rig(2);
    rig.player.play_list(FakeTracks::ids(2), 1).unwrap();
    let (ticket, _) = rig.due().unwrap();

    let state = rig
        .player
        .finish_fetch(ticket, Err(no_stream(true)))
        .unwrap();

    assert_eq!(title(&state), "Track 2");
    assert_eq!(state.status, PlaybackStatus::Error);
    assert!(state.error.unwrap().contains("Track 2"));
}

#[test]
fn skipping_stops_after_five_in_a_row() {
    let mut rig = rig(20);
    rig.player.play_list(FakeTracks::ids(20), 0).unwrap();
    let mut last = None;

    for _ in 0..6 {
        let (ticket, _) = rig.due().unwrap();
        last = rig.player.finish_fetch(ticket, Err(no_stream(true)));
    }

    let state = last.unwrap();
    assert_eq!(title(&state), "Track 6");
    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(rig.due(), None);
}

#[test]
fn a_song_with_no_youtube_source_is_skipped_without_a_request() {
    let mut rig = rig(3);
    let mut local = FakeTracks::track(1);
    local.sources = vec![SourceRef {
        kind: SourceKind::Spotify,
        id: "s1".to_owned(),
    }];
    rig.tracks.add(local);

    let state = rig.player.play_list(FakeTracks::ids(3), 0).unwrap();

    assert_eq!(title(&state), "Track 2");
    assert!(state.skipped.unwrap().message.contains("Track 1"));
    assert_eq!(rig.due().unwrap().1, "v2");
}

#[test]
fn choosing_another_list_makes_the_old_request_stale() {
    let mut rig = rig(4);
    rig.player.play_list(FakeTracks::ids(4), 0).unwrap();
    let (old, _) = rig.due().unwrap();
    rig.player.play_list(FakeTracks::ids(4), 2).unwrap();

    assert!(!rig.player.awaits_fetch(old));
    assert_eq!(rig.player.finish_fetch(old, Ok(stream("v1"))), None);
    assert!(rig.engine.fake().loads.is_empty());
    assert_eq!(rig.due().unwrap().1, "v3");
}

#[test]
fn a_start_past_the_end_changes_nothing() {
    let mut rig = rig(3);

    let refused = rig.player.play_list(FakeTracks::ids(3), 3);

    assert!(matches!(refused, Err(PlayerError::Queue(_))));
    assert_eq!(rig.player.state().current, None);
}

#[test]
fn the_history_is_listed_latest_first_a_window_at_a_time() {
    let mut rig = rig(5_000);
    rig.player.play_list(FakeTracks::ids(5_000), 4_000).unwrap();

    let slice = rig.player.history_slice(0, 3);

    let titles: Vec<_> = slice.items.iter().map(|i| i.track.title.as_str()).collect();
    assert_eq!(titles, ["Track 4000", "Track 3999", "Track 3998"]);
    assert_eq!(slice.total, 4_000);
    assert_eq!(rig.player.history_slice(3_999, 10).items.len(), 1);
    assert!(rig.player.history_slice(4_000, 10).items.is_empty());
}

#[test]
fn a_refused_load_after_resolving_does_not_skip() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    rig.engine.fake().refuse = Some(EngineError::Unplayable);
    let (ticket, video) = rig.due().unwrap();

    let state = rig.player.finish_fetch(ticket, Ok(stream(&video))).unwrap();

    assert_eq!(title(&state), "Track 1");
    assert_ne!(state.status, PlaybackStatus::Playing);
}

/// Remembers why each play ended.
type Plays = Vec<(String, Option<EndReason>)>;

#[derive(Clone, Default)]
struct Reasons(Arc<std::sync::Mutex<Plays>>);

impl PlayLog for Reasons {
    fn started(&self, track: &Track, _: u64) -> crate::library::PlayId {
        self.0.lock().unwrap().push((track.title.clone(), None));
        self.0.lock().unwrap().len() as crate::library::PlayId
    }

    fn ended(&self, play: crate::library::PlayId, _: Duration, reason: EndReason) {
        self.0.lock().unwrap()[play as usize - 1].1 = Some(reason);
    }
}

#[test]
fn plays_through_the_queue_end_as_finished_skipped_and_replaced() {
    let mut rig = rig(4);
    let reasons = Reasons::default();
    rig.player.set_log(Box::new(reasons.clone()));
    rig.player.play_list(FakeTracks::ids(4), 0).unwrap();
    rig.open();
    rig.end();
    rig.open();
    rig.player.next_track().unwrap();
    rig.open();
    rig.player.play_list(FakeTracks::ids(4), 0).unwrap();
    rig.open();

    let seen = reasons.0.lock().unwrap().clone();

    assert_eq!(
        seen,
        [
            ("Track 1".to_owned(), Some(EndReason::Finished)),
            ("Track 2".to_owned(), Some(EndReason::Skipped)),
            ("Track 3".to_owned(), Some(EndReason::Replaced)),
            ("Track 1".to_owned(), None),
        ]
    );
}

#[test]
fn a_queued_song_that_gave_up_is_asked_for_again_by_play() {
    let mut rig = rig(2);
    rig.player.play_list(FakeTracks::ids(2), 1).unwrap();
    let (ticket, _) = rig.due().unwrap();
    rig.player
        .finish_fetch(ticket, Err(no_stream(true)))
        .unwrap();
    assert_eq!(rig.player.state().status, PlaybackStatus::Error);
    assert_eq!(rig.player.failed_stream(), None);

    let state = rig.player.toggle_play().unwrap();

    assert_eq!(state.status, PlaybackStatus::Loading);
    assert_eq!(title(&state), "Track 2");
    assert_eq!(state.upcoming, 0);
    assert_eq!(rig.due().unwrap().1, "v2");
}

fn upcoming_titles(rig: &Rig) -> Vec<String> {
    let items = rig.player.queue_slice(0, 50).items;
    items.into_iter().map(|i| i.track.title).collect()
}

#[test]
fn play_next_while_playing_neither_restarts_nor_reloads_the_song() {
    let mut rig = rig(5);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    rig.open();
    let loads = rig.engine.fake().loads.len();
    let before = rig.player.state();

    let state = rig
        .player
        .enqueue(Uuid::from_u128(5), Placement::Next)
        .unwrap()
        .unwrap();

    assert_eq!(upcoming_titles(&rig), ["Track 5", "Track 2", "Track 3"]);
    assert_eq!(title(&state), "Track 1");
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.position_ms, before.position_ms);
    assert!(state.queue_revision > before.queue_revision);
    assert_eq!(rig.engine.fake().loads.len(), loads);
    assert_eq!(rig.due(), None);
    assert!(rig.engine.fake().playing);
}

#[test]
fn add_to_queue_puts_the_song_last_and_the_same_song_twice_is_two_entries() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();

    rig.player
        .enqueue(Uuid::from_u128(2), Placement::Last)
        .unwrap();

    assert_eq!(upcoming_titles(&rig), ["Track 2", "Track 3", "Track 2"]);
    let keys: Vec<u32> = rig
        .player
        .queue_slice(0, 9)
        .items
        .iter()
        .map(|i| i.key)
        .collect();
    assert_ne!(keys[0], keys[2]);
}

#[test]
fn queueing_a_song_the_library_does_not_know_is_refused() {
    let mut rig = rig(3);
    rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    let before = rig.player.state();

    let refused = rig.player.enqueue(Uuid::from_u128(99), Placement::Next);

    assert_eq!(
        refused,
        Err(PlayerError::UnknownTrack {
            id: Uuid::from_u128(99)
        })
    );
    assert_eq!(rig.player.state().queue_revision, before.queue_revision);
}

#[test]
fn queueing_with_nothing_playing_starts_that_song() {
    let mut rig = rig(3);

    let state = rig
        .player
        .enqueue(Uuid::from_u128(2), Placement::Last)
        .unwrap()
        .unwrap();

    assert_eq!(title(&state), "Track 2");
    assert_eq!(state.status, PlaybackStatus::Loading);
    assert_eq!(rig.due().unwrap().1, "v2");
}

#[test]
fn queueing_while_a_standalone_song_plays_keeps_it_playing_and_queues_behind_it() {
    let mut rig = rig(3);
    let row = TrackRow::from(&FakeTracks::track(1));
    let (key, _) = rig.player.begin_stream("v1".to_owned(), row);
    rig.player.finish_stream(key, Ok(stream("v1")));
    rig.player.on_engine_event(EngineEvent::Loaded);
    let loads = rig.engine.fake().loads.len();

    let state = rig
        .player
        .enqueue(Uuid::from_u128(3), Placement::Next)
        .unwrap()
        .unwrap();

    assert_eq!(title(&state), "Track 1");
    assert_eq!(state.current.as_ref().unwrap().key, key);
    assert_eq!(upcoming_titles(&rig), ["Track 3"]);
    assert_eq!(rig.engine.fake().loads.len(), loads);
    // When it ends, the queued song follows.
    rig.engine.fake().position = Duration::from_secs(200);
    rig.player.on_engine_event(EngineEvent::Ended);
    assert_eq!(rig.due().unwrap().1, "v3");
}

#[test]
fn the_current_entry_cannot_be_removed_and_an_unknown_key_errors() {
    let mut rig = rig(3);
    let state = rig.player.play_list(FakeTracks::ids(3), 0).unwrap();
    let current = state.current.unwrap().key;

    assert_eq!(
        rig.player.remove_queue_entry(current),
        Err(PlayerError::Queue(QueueError::NotUpcoming { key: current }))
    );
    assert_eq!(
        rig.player.remove_queue_entry(999),
        Err(PlayerError::Queue(QueueError::UnknownEntry { key: 999 }))
    );
    assert_eq!(upcoming_titles(&rig), ["Track 2", "Track 3"]);
}

#[test]
fn removing_and_clearing_leave_the_playing_song_alone() {
    let mut rig = rig(4);
    rig.player.play_list(FakeTracks::ids(4), 1).unwrap();
    rig.open();
    let loads = rig.engine.fake().loads.len();
    let three = rig.player.queue_slice(0, 9).items[0].key;

    let removed = rig.player.remove_queue_entry(three).unwrap();
    assert_eq!(upcoming_titles(&rig), ["Track 4"]);
    assert_eq!(removed.status, PlaybackStatus::Playing);

    let cleared = rig.player.clear_queue().unwrap();
    assert_eq!((cleared.upcoming, cleared.history), (0, 1));
    assert_eq!(title(&cleared), "Track 2");
    assert_eq!(cleared.status, PlaybackStatus::Playing);
    assert_eq!(rig.player.clear_queue(), None);
    assert_eq!(rig.engine.fake().loads.len(), loads);
}
