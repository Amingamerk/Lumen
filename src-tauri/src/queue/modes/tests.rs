//! Shuffle and repeat with a fixed seed, so every order is exact.

use uuid::Uuid;

use super::*;
use crate::queue::Entry;

const SEED: u64 = 7;

fn id(n: u32) -> Uuid {
    Uuid::from_u128(u128::from(n))
}

/// Tracks 1 to `count`, starting at the given index.
fn queue_seeded(seed: u64, count: u32, start: usize) -> Queue {
    let mut queue = Queue::seeded(seed);
    queue.replace((1..=count).map(id).collect(), start).unwrap();
    queue
}

fn queue(count: u32, start: usize) -> Queue {
    queue_seeded(SEED, count, start)
}

fn numbers(entries: &[Entry]) -> Vec<u128> {
    entries.iter().map(|e| e.track.as_u128()).collect()
}

fn current(queue: &Queue) -> u128 {
    queue.current().unwrap().track.as_u128()
}

/// The current track and every one after it, as `advance` reaches them.
fn walk(queue: &mut Queue) -> Vec<u128> {
    let mut heard = vec![current(queue)];
    while queue.advance() {
        heard.push(current(queue));
    }
    heard
}

#[test]
fn a_seeded_shuffle_visits_every_entry_once() {
    let mut shuffled = queue(100, 0);
    shuffled.set_shuffle(true);

    let heard = walk(&mut shuffled);

    let plain: Vec<u128> = (1..=100).collect();
    assert_ne!(heard, plain);
    let mut sorted = heard.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, plain);

    let mut again = queue(100, 0);
    again.set_shuffle(true);
    assert_eq!(walk(&mut again), heard);

    let mut other = queue_seeded(SEED + 1, 100, 0);
    other.set_shuffle(true);
    assert_ne!(walk(&mut other), heard);
}

#[test]
fn turning_shuffle_off_restores_the_original_order_from_the_current_entry() {
    let mut queue = queue(10, 0);
    queue.set_shuffle(true);
    for _ in 0..3 {
        assert!(queue.advance());
    }
    let here = *queue.current().unwrap();
    let played = numbers(queue.history());
    assert_ne!(played, [1, 2, 3]);

    queue.set_shuffle(false);

    assert_eq!(queue.current(), Some(&here));
    assert_eq!(numbers(queue.history()), played);
    // The songs after this one in list order, then those before it that wait.
    let now = here.track.as_u128();
    let waiting = |n: &u128| *n != now && !played.contains(n);
    let expected: Vec<u128> = (now + 1..=10).chain(1..now).filter(waiting).collect();
    assert_eq!(numbers(queue.upcoming()), expected);
}

#[test]
fn shuffle_on_and_off_again_leaves_every_entry_as_it_was() {
    let mut queue = queue(20, 4);
    let before: Vec<Entry> = queue.upcoming().to_vec();
    let history: Vec<Entry> = queue.history().to_vec();

    queue.set_shuffle(true);
    let mut shuffled = queue.upcoming().to_vec();
    assert_ne!(shuffled, before);
    assert_eq!(queue.history(), history);
    shuffled.sort_by_key(|e| e.key);
    assert_eq!(shuffled, before);
    queue.set_shuffle(false);

    assert_eq!(queue.upcoming(), before);
    assert_eq!(queue.history(), history);
}

#[test]
fn repeat_all_wraps_to_the_first_entry() {
    let mut queue = queue(3, 2);
    assert!(!queue.advance());
    queue.set_repeat(Repeat::All);

    assert!(queue.advance());

    assert_eq!(current(&queue), 1);
    assert!(queue.history().is_empty());
    assert_eq!(numbers(queue.upcoming()), [2, 3]);
}

#[test]
fn play_next_while_shuffled_plays_that_entry_next() {
    let mut queue = queue(10, 0);
    queue.set_shuffle(true);
    queue.advance();
    assert!(queue.shuffled());

    let key = queue.insert_next(id(99));

    assert_eq!(queue.upcoming()[0].key, key);
    assert!(queue.advance());
    assert_eq!(current(&queue), 99);
}

#[test]
fn going_back_walks_through_what_played() {
    let mut queue = queue(10, 0);
    queue.set_shuffle(true);
    let mut played = vec![current(&queue)];
    for _ in 0..4 {
        queue.advance();
        played.push(current(&queue));
    }

    let mut back = vec![current(&queue)];
    while queue.retreat() {
        back.push(current(&queue));
    }

    played.reverse();
    assert_eq!(back, played);
}

#[test]
fn going_back_over_a_wrap_returns_to_the_entry_it_ended_on() {
    for shuffled in [false, true] {
        let mut queue = queue(5, 0);
        queue.set_shuffle(shuffled);
        queue.set_repeat(Repeat::All);
        let mut played = walk_one_lap(&mut queue);
        queue.advance();

        assert!(queue.retreat());
        assert_eq!(current(&queue), played.pop().unwrap());
        assert!(queue.retreat());
        assert_eq!(current(&queue), played.pop().unwrap());
        assert!(queue.advance() && queue.advance());
        assert!(queue.history().is_empty(), "it starts over again");
    }
}

/// Everything up to the last entry.
fn walk_one_lap(queue: &mut Queue) -> Vec<u128> {
    let mut heard = vec![current(queue)];
    while !queue.upcoming().is_empty() {
        queue.advance();
        heard.push(current(queue));
    }
    heard
}

#[test]
fn a_queue_that_never_started_over_has_nothing_before_its_first_entry() {
    let mut queue = queue(5, 0);
    queue.set_repeat(Repeat::All);

    assert!(!queue.retreat());
    assert_eq!(current(&queue), 1);
}

#[test]
fn songs_that_were_there_before_shuffle_are_shuffled_when_the_queue_starts_over() {
    let mut queue = queue(40, 30);
    queue.set_repeat(Repeat::All);
    queue.set_shuffle(true);
    walk_one_lap(&mut queue);

    queue.advance();

    let lap = walk_one_lap(&mut queue);
    assert_ne!(lap[..31], (1..=31).collect::<Vec<u128>>()[..]);
    let mut sorted = lap;
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=40).collect::<Vec<u128>>());
}

#[test]
fn a_list_played_while_shuffled_leads_with_the_chosen_song_and_keeps_every_other() {
    let mut queue = queue(3, 0);
    queue.set_shuffle(true);

    queue.replace((1..=10).map(id).collect(), 6).unwrap();

    assert_eq!(current(&queue), 7);
    assert!(queue.history().is_empty());
    assert_eq!(queue.upcoming().len(), 9);
    assert_ne!(numbers(queue.upcoming()), [1, 2, 3, 4, 5, 6, 8, 9, 10]);
    queue.set_shuffle(false);
    assert_eq!(numbers(queue.upcoming()), [8, 9, 10, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn picks_stay_behind_the_users_songs_through_shuffle() {
    let mut queue = queue(6, 0);
    queue.append_autoplay(id(70));
    queue.append_autoplay(id(71));

    queue.set_shuffle(true);
    assert_eq!(numbers(&queue.upcoming()[5..]), [70, 71]);
    queue.insert_last(id(80));
    assert_eq!(numbers(&queue.upcoming()[5..]), [80, 70, 71]);
    // Some songs play, so that others are left from before the current one.
    for _ in 0..3 {
        queue.advance();
    }
    queue.set_shuffle(false);

    let upcoming = numbers(queue.upcoming());
    assert_eq!(upcoming[upcoming.len() - 2..], [70, 71]);
}

#[test]
fn edits_made_while_shuffled_hold_after_shuffle_is_off() {
    let mut queue = queue(6, 0);
    queue.set_shuffle(true);
    let removed = queue.upcoming()[0];
    queue.remove(removed.key).unwrap();
    queue.insert_last(id(80));
    queue.insert_next(id(90));

    queue.set_shuffle(false);

    let expected: Vec<u128> = std::iter::once(90)
        .chain((2..=6).filter(|n| *n != removed.track.as_u128()))
        .chain([80])
        .collect();
    assert_eq!(numbers(queue.upcoming()), expected);
}

#[test]
fn clearing_while_shuffled_leaves_nothing_to_come_back() {
    let mut queue = queue(6, 0);
    queue.set_shuffle(true);
    queue.advance();
    let played = numbers(queue.history());

    queue.clear_upcoming();
    queue.set_shuffle(false);

    assert_eq!(numbers(queue.history()), played);
    assert!(queue.upcoming().is_empty());
}

#[test]
fn turning_a_mode_on_twice_changes_nothing() {
    let mut queue = queue(6, 0);
    assert!(queue.set_shuffle(true));
    assert!(queue.set_repeat(Repeat::One));
    let revision = queue.revision();
    let order = numbers(queue.upcoming());

    assert!(!queue.set_shuffle(true));
    assert!(!queue.set_repeat(Repeat::One));

    assert_eq!(
        (queue.revision(), numbers(queue.upcoming())),
        (revision, order)
    );
}

#[test]
fn an_empty_queue_takes_both_modes() {
    let mut queue = Queue::seeded(SEED);
    queue.set_repeat(Repeat::All);
    queue.set_shuffle(true);

    assert!(!queue.advance());
    assert!(!queue.retreat());
    queue.set_shuffle(false);
    assert_eq!(queue.current(), None);
}

#[test]
fn shuffles_5000_entries_and_back_at_once() {
    let mut queue = queue(5_000, 0);
    let before = queue.upcoming().to_vec();
    let started = std::time::Instant::now();

    queue.set_shuffle(true);
    queue.set_shuffle(false);

    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(queue.upcoming(), before);
}

#[test]
fn the_repeat_control_goes_off_all_one() {
    assert_eq!(Repeat::Off.following(), Repeat::All);
    assert_eq!(Repeat::All.following(), Repeat::One);
    assert_eq!(Repeat::One.following(), Repeat::Off);
}

/// Starts the Queue over and gives the whole pass it then plays.
fn next_pass(queue: &mut Queue) -> Vec<u128> {
    queue.set_repeat(Repeat::All);
    walk_one_lap(queue);
    assert!(queue.advance());
    walk_one_lap(queue)
}

/// Tracks 1 to 10, three songs on in a shuffle that is then turned off.
fn unshuffled_after_three_songs() -> Queue {
    let mut queue = queue(10, 0);
    queue.set_shuffle(true);
    for _ in 0..3 {
        queue.advance();
    }
    queue.set_shuffle(false);
    assert_ne!(numbers(queue.history()), [1, 2, 3]);
    queue
}

#[test]
fn without_shuffle_the_queue_starts_over_in_list_order() {
    let mut queue = unshuffled_after_three_songs();

    assert_eq!(next_pass(&mut queue), (1..=10).collect::<Vec<u128>>());
}

#[test]
fn shuffling_again_keeps_the_original_list_order() {
    let mut queue = unshuffled_after_three_songs();
    let upcoming = numbers(queue.upcoming());

    queue.set_shuffle(true);
    queue.set_shuffle(false);

    assert_eq!(numbers(queue.upcoming()), upcoming);
    assert_eq!(next_pass(&mut queue), (1..=10).collect::<Vec<u128>>());
}

#[test]
fn going_back_over_a_wrap_walks_what_played_and_stops_where_it_began() {
    let mut queue = unshuffled_after_three_songs();
    queue.set_repeat(Repeat::All);
    let mut played = numbers(queue.history());
    played.extend(walk_one_lap(&mut queue));
    queue.advance();

    for want in played.iter().rev() {
        assert!(queue.retreat());
        assert_eq!(current(&queue), *want);
    }

    assert!(
        !queue.retreat(),
        "nothing is known to have played before that"
    );
    assert_eq!(current(&queue), played[0]);
}

#[test]
fn going_back_over_a_wrap_still_works_after_shuffle_is_toggled() {
    let mut queue = queue(5, 0);
    queue.set_repeat(Repeat::All);
    walk_one_lap(&mut queue);
    queue.advance();

    queue.set_shuffle(true);

    assert!(queue.retreat());
    assert_eq!(current(&queue), 5);
    assert_eq!(numbers(queue.history()), [1, 2, 3, 4]);
}

#[test]
fn going_on_again_returns_to_the_pass_that_going_back_left() {
    let mut queue = queue(40, 30);
    queue.set_repeat(Repeat::All);
    queue.set_shuffle(true);
    walk_one_lap(&mut queue);
    queue.advance();
    let here = *queue.current().unwrap();
    let upcoming = queue.upcoming().to_vec();

    queue.retreat();
    queue.advance();

    assert_eq!(queue.current(), Some(&here));
    assert_eq!(queue.upcoming(), upcoming);
}

#[test]
fn going_back_over_a_wrap_keeps_new_songs_and_leaves_removed_ones_out() {
    let mut queue = queue(5, 0);
    queue.set_repeat(Repeat::All);
    walk_one_lap(&mut queue);
    queue.advance();
    queue.remove(key_of(&queue, 3)).unwrap();
    queue.insert_last(id(9));

    queue.retreat();

    assert_eq!(current(&queue), 5);
    assert_eq!(numbers(queue.history()), [1, 2, 4]);
    assert_eq!(numbers(queue.upcoming()), [9]);
}

#[test]
fn a_drag_without_shuffle_changes_the_list_order() {
    let mut queue = unshuffled_after_three_songs();
    let here = current(&queue);
    let (first, last) = (queue.upcoming()[0], *queue.upcoming().last().unwrap());

    queue.move_before(last.key, Some(first.key)).unwrap();

    // It was dropped right behind the current song, and stays there.
    let pass = next_pass(&mut queue);
    let at = pass.iter().position(|n| *n == here).unwrap();
    assert_eq!(pass[at + 1], last.track.as_u128());
}

#[test]
fn a_drag_made_while_shuffled_leaves_the_list_order() {
    let mut queue = queue(10, 0);
    queue.set_shuffle(true);
    let (first, last) = (queue.upcoming()[0], *queue.upcoming().last().unwrap());
    queue.move_before(last.key, Some(first.key)).unwrap();

    queue.set_shuffle(false);

    assert_eq!(numbers(queue.upcoming()), (2..=10).collect::<Vec<u128>>());
}

/// The key of the first entry holding track `n`.
fn key_of(queue: &Queue, n: u32) -> u32 {
    let mut entries = queue
        .history()
        .iter()
        .chain(queue.current())
        .chain(queue.upcoming());
    entries.find(|e| e.track == id(n)).unwrap().key
}
