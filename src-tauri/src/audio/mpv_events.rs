//! Turns what mpv reports into what the player is told.

use std::time::{Duration, Instant};

use libmpv2::events::{Event, PropertyData};
use libmpv2::mpv_end_file_reason;

use super::EngineEvent;

/// The least time between two reports of progress: at most four a second.
const PROGRESS_SPACING: Duration = Duration::from_millis(250);

/// Where the current media is, as last heard from mpv.
#[derive(Debug, Default)]
pub struct Timeline {
    /// A load was asked for and its file has not started: whatever mpv still
    /// says is about the one before.
    pub opening: bool,
    pub position: Duration,
    /// Zero while unknown.
    pub duration: Duration,
    last_progress: Option<Instant>,
}

impl Timeline {
    pub fn opening_at(start: Duration) -> Self {
        Self {
            opening: true,
            position: start,
            ..Self::default()
        }
    }
}

/// Records what `event` says and returns what the player should hear of it.
///
/// libmpv2 hands a failed load or playback over as an error result in place of
/// the end-of-file event. mpv's own words are dropped here: they can hold a
/// Stream URL.
pub fn translate(
    timeline: &mut Timeline,
    event: libmpv2::Result<Event<'_>>,
    now: Instant,
) -> Option<EngineEvent> {
    if timeline.opening {
        if matches!(event, Ok(Event::StartFile)) {
            timeline.opening = false;
        }
        return None;
    }
    match event {
        Err(_) => Some(EngineEvent::Failed),
        Ok(Event::EndFile(reason)) => match reason {
            mpv_end_file_reason::Eof => Some(EngineEvent::Ended),
            // A redirect means a playlist, which would lead somewhere else.
            mpv_end_file_reason::Error | mpv_end_file_reason::Redirect => Some(EngineEvent::Failed),
            _ => None,
        },
        Ok(Event::PlaybackRestart) => Some(EngineEvent::Loaded),
        Ok(Event::PropertyChange {
            name,
            change: PropertyData::Double(seconds),
            ..
        }) => match name {
            "time-pos" => {
                timeline.position = time(seconds)?;
                let due = timeline
                    .last_progress
                    .is_none_or(|last| now.duration_since(last) >= PROGRESS_SPACING);
                due.then(|| {
                    timeline.last_progress = Some(now);
                    EngineEvent::Progress
                })
            }
            "duration" => {
                timeline.duration = time(seconds)?;
                Some(EngineEvent::Duration)
            }
            _ => None,
        },
        Ok(_) => None,
    }
}

/// mpv's seconds as a time. Some files start a little before zero.
pub fn time(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() {
        return None;
    }
    Duration::try_from_secs_f64(seconds.max(0.0)).ok()
}
