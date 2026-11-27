//! The player with real audio (a file or a stream), over an engine a test drives.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::fake::FakeEngine;
use super::{
    EngineError, EngineEvent, Media, PlaybackStatus, Player, PlayerError, PlayerEvent,
    PlayerPosition, PlayerState,
};
use crate::catalog::{Catalog, MAX_SLICE, TrackRow};
use crate::recs::StaticRecommender;

const MARKER: &str = "SECRET-MARKER";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn idle() -> (Player<FakeEngine>, FakeEngine) {
    let catalog = Arc::new(Catalog::default());
    let engine = FakeEngine::default();
    let recommender = Box::new(StaticRecommender::new(Arc::clone(&catalog)));
    (Player::new(engine.clone(), catalog, recommender), engine)
}

fn file() -> Media {
    Media::File(PathBuf::from(r"C:\music\tone.wav"))
}

fn stream() -> Media {
    Media::Stream {
        url: format!("https://media.invalid/{MARKER}/audio?sig={MARKER}"),
        headers: vec![("Cookie".to_owned(), MARKER.to_owned())],
    }
}

fn row() -> TrackRow {
    TrackRow::untagged("tone".to_owned())
}

/// A file the engine has opened: ten seconds long and playing.
fn playing() -> (Player<FakeEngine>, FakeEngine) {
    let (mut player, engine) = idle();
    player.play_media(file(), row());
    engine.fake().duration = secs(10);
    player.on_engine_event(EngineEvent::Loaded).unwrap();
    (player, engine)
}

fn changed(event: Option<PlayerEvent>) -> PlayerState {
    match event {
        Some(PlayerEvent::Changed(state)) => state,
        other => panic!("expected a change, got {other:?}"),
    }
}

#[test]
fn a_sample_track_cannot_be_played_when_idle() {
    let (mut player, engine) = idle();
    let before = player.state();

    let refused = player.play_collection("night-shift", 0);

    assert_eq!(refused, Err(PlayerError::SampleTrack));
    assert_eq!(
        PlayerError::SampleTrack.to_string(),
        "sample track, cannot be played"
    );
    assert_eq!(player.state(), before);
    assert!(engine.fake().loads.is_empty());
}

#[test]
fn a_sample_track_leaves_real_playback_unchanged() {
    let (mut player, engine) = playing();
    let before = player.state();

    for refused in [
        player.play_collection("night-shift", 1),
        player.play_tracks(vec![Uuid::from_u128(1)], 0),
    ] {
        assert_eq!(refused, Err(PlayerError::SampleTrack));
    }

    assert_eq!(player.state(), before);
    assert_eq!(engine.fake().loads.len(), 1);
    assert!(engine.fake().playing);
}

#[test]
fn media_is_loading_until_the_engine_has_opened_it() {
    let (mut player, engine) = idle();

    let loading = player.play_media(file(), row());

    assert_eq!(loading.status, PlaybackStatus::Loading);
    assert_eq!(loading.current.as_ref().unwrap().track.title, "tone");
    assert_eq!((loading.position_ms, loading.duration_ms), (0, 0));
    assert_eq!(engine.fake().loads, [(file(), Duration::ZERO)]);
    assert!(engine.fake().playing);

    engine.fake().duration = secs(10);
    let open = changed(player.on_engine_event(EngineEvent::Loaded));

    assert_eq!(open.status, PlaybackStatus::Playing);
    assert_eq!(open.duration_ms, 10_000);
    assert_eq!(open.current.unwrap().track.duration_ms, 10_000);
    assert!(open.revision > loading.revision);
    assert_eq!(player.on_engine_event(EngineEvent::Loaded), None);
}

#[test]
fn pausing_while_loading_stays_paused_once_open() {
    let (mut player, engine) = idle();
    player.play_media(file(), row());

    let paused = player.toggle_play().unwrap();
    let open = changed(player.on_engine_event(EngineEvent::Loaded));

    assert_eq!(paused.status, PlaybackStatus::Paused);
    assert_eq!(open.status, PlaybackStatus::Paused);
    assert!(!engine.fake().playing);
}

#[test]
fn real_audio_stands_alone_with_nothing_before_or_after_it() {
    let (mut player, engine) = idle();
    engine.fake().plays_samples = true;
    player.play_collection("night-shift", 2).unwrap();
    let queued = player.state();
    assert!(queued.can_next && queued.can_previous);

    let state = player.play_media(file(), row());

    assert!(!state.can_next && !state.can_previous);
    assert_eq!(state.upcoming, 0);
    assert!(state.queue_revision > queued.queue_revision);
    assert!(!state.current.as_ref().unwrap().autoplay);
    assert_ne!(
        state.current.as_ref().unwrap().key,
        queued.current.unwrap().key
    );
    assert_eq!(player.queue_slice(0, MAX_SLICE).total, 0);
    assert_eq!(player.next_track(), None);
    assert_eq!(player.previous_track(), None);
}

#[test]
fn a_queued_sample_replaces_real_audio_where_the_engine_plays_samples() {
    let (mut player, engine) = playing();
    engine.fake().plays_samples = true;

    let state = player.play_collection("night-shift", 0).unwrap();

    assert_eq!(state.current.unwrap().track.id.as_u128(), 24);
    assert_eq!(state.upcoming, 4);
    assert!(state.can_next);
}

#[test]
fn positions_are_sent_once_a_second_and_only_while_playing() {
    let (mut player, engine) = playing();
    let revision = player.state().revision;
    let mut sent = Vec::new();

    // Far more often than the UI wants to hear.
    for step in 1..=30 {
        engine.fake().position = millis(step * 100);
        sent.extend(player.on_engine_event(EngineEvent::Progress));
    }

    let positions: Vec<PlayerEvent> = [1_000, 2_000, 3_000]
        .map(|position_ms| {
            PlayerEvent::Position(PlayerPosition {
                revision,
                position_ms,
            })
        })
        .into();
    assert_eq!(sent, positions);

    player.toggle_play();
    engine.fake().position = secs(7);
    assert_eq!(player.on_engine_event(EngineEvent::Progress), None);
}

#[test]
fn no_position_is_sent_while_loading() {
    let (mut player, engine) = idle();
    player.play_media(file(), row());

    engine.fake().position = secs(2);

    assert_eq!(player.on_engine_event(EngineEvent::Progress), None);
}

#[test]
fn a_new_length_is_announced_once() {
    let (mut player, engine) = playing();
    assert_eq!(player.on_engine_event(EngineEvent::Duration), None);

    engine.fake().duration = secs(12);

    let state = changed(player.on_engine_event(EngineEvent::Duration));
    assert_eq!(state.duration_ms, 12_000);
    assert_eq!(player.on_engine_event(EngineEvent::Duration), None);
}

#[test]
fn seeking_clamps_to_the_media() {
    let (mut player, engine) = playing();

    assert_eq!(player.seek(4_000).unwrap().position_ms, 4_000);
    assert_eq!(player.seek(-1).unwrap().position_ms, 0);
    assert_eq!(player.seek(i64::MAX).unwrap().position_ms, 10_000);
    assert_eq!(player.seek_by(-2_000).unwrap().position_ms, 8_000);
    assert_eq!(engine.fake().position, secs(8));
}

#[test]
fn seeking_waits_for_the_media_to_open() {
    let (mut player, _) = idle();
    player.play_media(file(), row());

    assert_eq!(player.seek(1_000), None);
}

#[test]
fn volume_clamps_and_reaches_the_engine() {
    let (mut player, engine) = idle();
    assert_eq!(engine.fake().volume, 80);

    assert_eq!(player.set_volume(250).unwrap().volume, 100);
    assert_eq!(engine.fake().volume, 100);
    assert_eq!(player.change_volume(-130).unwrap().volume, 0);
    assert_eq!(engine.fake().volume, 0);
}

#[test]
fn the_end_stops_playback_and_nothing_advances() {
    let (mut player, engine) = playing();
    engine.fake().position = millis(9_950);
    let key = player.state().current.unwrap().key;

    let ended = changed(player.on_engine_event(EngineEvent::Ended));

    assert_eq!(ended.status, PlaybackStatus::Ended);
    assert_eq!((ended.position_ms, ended.duration_ms), (10_000, 10_000));
    assert_eq!(ended.current.unwrap().key, key);
    assert_eq!(engine.fake().loads.len(), 1);
    for event in [EngineEvent::Ended, EngineEvent::Progress] {
        assert_eq!(player.on_engine_event(event), None);
    }
}

#[test]
fn play_after_the_end_starts_over() {
    let (mut player, engine) = playing();
    player.on_engine_event(EngineEvent::Ended);

    let again = player.toggle_play().unwrap();

    assert_eq!(again.status, PlaybackStatus::Loading);
    assert_eq!(engine.fake().loads[1], (file(), Duration::ZERO));
    assert!(engine.fake().playing);
}

#[test]
fn seeking_after_the_end_opens_it_there_paused() {
    let (mut player, engine) = playing();
    player.on_engine_event(EngineEvent::Ended);

    let state = player.seek(4_000).unwrap();

    assert_eq!(state.status, PlaybackStatus::Paused);
    assert_eq!(state.position_ms, 4_000);
    assert_eq!(engine.fake().loads[1], (file(), secs(4)));
    assert!(!engine.fake().playing);
}

#[test]
fn a_playback_failure_puts_the_player_in_the_error_state() {
    let (mut player, engine) = playing();
    engine.fake().position = secs(3);

    let failed = changed(player.on_engine_event(EngineEvent::Failed));

    assert_eq!(failed.status, PlaybackStatus::Error);
    assert_eq!(failed.error.as_deref(), Some("Lumen could not play “tone”."));
    assert!(!engine.fake().playing);
    for event in [
        EngineEvent::Failed,
        EngineEvent::Progress,
        EngineEvent::Ended,
        EngineEvent::Loaded,
        EngineEvent::Duration,
    ] {
        assert_eq!(player.on_engine_event(event), None, "{event:?}");
    }
    assert_eq!(player.seek(1_000), None);
}

#[test]
fn a_load_failure_puts_the_player_in_the_error_state() {
    let (mut player, _) = idle();
    player.play_media(file(), row());

    let failed = changed(player.on_engine_event(EngineEvent::Failed));

    assert_eq!(failed.status, PlaybackStatus::Error);
}

#[test]
fn a_refused_load_says_the_file_is_missing() {
    let (mut player, engine) = idle();
    engine.fake().refuse = Some(EngineError::NotFound);

    let state = player.play_media(file(), row());

    assert_eq!(state.status, PlaybackStatus::Error);
    assert_eq!(state.error.as_deref(), Some("Lumen could not find “tone”."));
    assert!(!engine.fake().playing);
}

#[test]
fn play_after_a_failure_tries_again() {
    let (mut player, engine) = playing();
    player.on_engine_event(EngineEvent::Failed);

    let again = player.toggle_play().unwrap();

    assert_eq!(again.status, PlaybackStatus::Loading);
    assert_eq!(again.error, None);
    assert_eq!(engine.fake().loads.len(), 2);
}

#[test]
fn a_stream_url_and_its_headers_never_reach_the_ui_or_a_log() {
    let (mut player, engine) = idle();
    let mut seen = vec![format!("{:?}", stream())];
    let mut note = |text: String| seen.push(text);

    let state = player.play_media(stream(), row());
    note(serde_json::to_string(&state).unwrap());
    note(format!("{state:?}"));
    engine.fake().duration = secs(10);
    note(format!("{:?}", player.on_engine_event(EngineEvent::Loaded)));
    let failed = changed(player.on_engine_event(EngineEvent::Failed));
    note(serde_json::to_string(&failed).unwrap());
    engine.fake().refuse = Some(EngineError::Unplayable);
    let refused = player.play_media(stream(), row());
    note(serde_json::to_string(&refused).unwrap());
    note(format!("{:?} {}", EngineError::Unplayable, EngineError::Unplayable));

    for text in seen {
        assert!(!text.contains(MARKER), "{text}");
        assert!(!text.contains("media.invalid"), "{text}");
    }
    assert_eq!(engine.fake().loads[0].0, stream());
}
