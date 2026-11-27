//! What mpv reports, turned into what the player is told. No mpv runs here.

use std::time::{Duration, Instant};

use libmpv2::events::{Event, PropertyData};
use libmpv2::{Error, mpv_end_file_reason, mpv_error};

use super::EngineEvent;
use super::mpv_events::{Timeline, translate};

fn property(name: &'static str, value: f64) -> Event<'static> {
    Event::PropertyChange {
        name,
        change: PropertyData::Double(value),
        reply_userdata: 0,
    }
}

fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Feeds one event to an open timeline.
fn report(event: Event<'static>) -> Option<EngineEvent> {
    translate(&mut Timeline::default(), Ok(event), Instant::now())
}

#[test]
fn the_end_of_a_file_depends_on_the_reason() {
    for (reason, expected) in [
        (mpv_end_file_reason::Eof, Some(EngineEvent::Ended)),
        (mpv_end_file_reason::Error, Some(EngineEvent::Failed)),
        // A playlist file would send mpv on to somewhere else.
        (mpv_end_file_reason::Redirect, Some(EngineEvent::Failed)),
        (mpv_end_file_reason::Stop, None),
        (mpv_end_file_reason::Quit, None),
    ] {
        assert_eq!(report(Event::EndFile(reason)), expected, "{reason}");
    }
}

#[test]
fn an_error_result_in_place_of_an_end_of_file_event_is_a_failure() {
    let rejected = Err(Error::Raw(mpv_error::LoadingFailed));

    let reported = translate(&mut Timeline::default(), rejected, Instant::now());

    assert_eq!(reported, Some(EngineEvent::Failed));
}

#[test]
fn a_restart_means_the_media_is_open() {
    assert_eq!(report(Event::PlaybackRestart), Some(EngineEvent::Loaded));
}

#[test]
fn positions_are_kept_always_and_reported_at_most_four_times_a_second() {
    let mut timeline = Timeline::default();
    let start = Instant::now();
    let mut reported = 0;

    // Every 20 ms for two seconds.
    for step in 0..100 {
        let time = step * 20;
        let event = property("time-pos", time as f64 / 1_000.0);
        let now = start + millis(time);
        reported += u32::from(translate(&mut timeline, Ok(event), now).is_some());
        assert_eq!(timeline.position, millis(time));
    }

    assert_eq!(reported, 8);
}

#[test]
fn the_length_is_kept_and_reported() {
    let mut timeline = Timeline::default();

    let reported = translate(
        &mut timeline,
        Ok(property("duration", 12.5)),
        Instant::now(),
    );

    assert_eq!(reported, Some(EngineEvent::Duration));
    assert_eq!(timeline.duration, millis(12_500));
}

#[test]
fn nonsense_and_unavailable_values_are_ignored() {
    let mut timeline = Timeline::default();
    timeline.position = millis(500);
    let gone = Event::PropertyChange {
        name: "time-pos",
        change: PropertyData::None,
        reply_userdata: 0,
    };

    for event in [
        gone,
        property("time-pos", f64::NAN),
        property("time-pos", f64::INFINITY),
        property("volume", 3.0),
    ] {
        assert_eq!(translate(&mut timeline, Ok(event), Instant::now()), None);
    }
    assert_eq!(timeline.position, millis(500));
    // A little before zero is how some files begin.
    translate(
        &mut timeline,
        Ok(property("time-pos", -0.2)),
        Instant::now(),
    );
    assert_eq!(timeline.position, Duration::ZERO);
}

#[test]
fn nothing_counts_between_a_load_and_the_start_of_its_file() {
    let mut timeline = Timeline::opening_at(millis(2_000));
    let now = Instant::now();
    let old = [
        Ok(Event::EndFile(mpv_end_file_reason::Eof)),
        Err(Error::Raw(mpv_error::LoadingFailed)),
        Ok(Event::PlaybackRestart),
        Ok(property("time-pos", 9.0)),
        Ok(property("duration", 10.0)),
    ];

    for event in old {
        assert_eq!(translate(&mut timeline, event, now), None);
    }
    assert_eq!(
        (timeline.position, timeline.duration),
        (millis(2_000), Duration::ZERO)
    );

    assert_eq!(translate(&mut timeline, Ok(Event::StartFile), now), None);
    assert_eq!(
        translate(&mut timeline, Ok(Event::PlaybackRestart), now),
        Some(EngineEvent::Loaded)
    );
}
