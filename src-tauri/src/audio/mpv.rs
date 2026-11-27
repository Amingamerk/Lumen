//! libmpv behind the [`Engine`] seam. Commands go in through [`MpvEngine`];
//! what happens comes out of [`MpvEvents`], on a thread of its own.
//!
//! mpv's log is never requested, and nothing mpv says leaves this module: its
//! messages hold Stream URLs.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use libmpv2::events::Event;
use libmpv2::{Format, Mpv, mpv_end_file_reason};

use super::local::local_path;
use super::mpv_events::{Timeline, time, translate};
use super::{Engine, EngineError, EngineEvent, Media};

/// Audio only with small buffers (docs/PROJECT.md section 2), and nothing read
/// from the user's mpv setup.
const OPTIONS: &[(&str, &str)] = &[
    ("vid", "no"),
    ("audio-display", "no"),
    ("demuxer-max-bytes", "8MiB"),
    ("demuxer-max-back-bytes", "1MiB"),
    ("cache-secs", "20"),
    ("config", "no"),
    ("load-scripts", "no"),
    // Lumen resolves streams itself.
    ("ytdl", "no"),
    ("osc", "no"),
    ("load-stats-overlay", "no"),
    ("load-osd-console", "no"),
    ("load-auto-profiles", "no"),
    ("sub-auto", "no"),
    ("audio-file-auto", "no"),
    ("cover-art-auto", "no"),
    ("terminal", "no"),
    ("input-default-bindings", "no"),
    // The media keys are Lumen's own (Milestone 4).
    ("input-media-keys", "no"),
    ("tls-verify", "yes"),
    ("audio-client-name", "Lumen"),
];

/// libmpv's codes only: no paths or URLs.
#[derive(Debug, thiserror::Error)]
#[error("libmpv could not start: {0}")]
pub struct MpvStartError(#[from] libmpv2::Error);

pub struct MpvEngine {
    mpv: Arc<Mpv>,
    timeline: Arc<Mutex<Timeline>>,
    /// Restored for media that brings no `User-Agent` of its own.
    user_agent: String,
}

impl MpvEngine {
    pub fn new() -> Result<(Self, MpvEvents), MpvStartError> {
        Self::start(&[])
    }

    /// Plays to nowhere, in real time: no audio device is opened.
    #[cfg(test)]
    pub fn silent() -> Result<(Self, MpvEvents), MpvStartError> {
        Self::start(&[("ao", "null")])
    }

    fn start(extra: &[(&str, &str)]) -> Result<(Self, MpvEvents), MpvStartError> {
        let mpv = Mpv::with_initializer(|init| {
            for (name, value) in OPTIONS.iter().chain(extra) {
                init.set_option(name, *value)?;
            }
            Ok(())
        })?;
        mpv.observe_property("time-pos", Format::Double, 1)?;
        mpv.observe_property("duration", Format::Double, 2)?;
        let user_agent = mpv.get_property::<String>("user-agent")?;
        let mpv = Arc::new(mpv);
        let timeline = Arc::default();
        let events = MpvEvents {
            mpv: Arc::clone(&mpv),
            timeline: Arc::clone(&timeline),
        };
        let engine = Self {
            mpv,
            timeline,
            user_agent,
        };
        Ok((engine, events))
    }

    fn open(&mut self, media: &Media, start: Duration) -> Result<(), EngineError> {
        let (target, headers) = match media {
            Media::Sample { .. } => return Err(EngineError::Unplayable),
            Media::File(path) => {
                let path = local_path(path)
                    .filter(|path| path.is_file())
                    .and_then(|path| path.into_os_string().into_string().ok())
                    .ok_or(EngineError::NotFound)?;
                (path, [].as_slice())
            }
            Media::Stream { url, headers } => (url.clone(), headers.as_slice()),
        };
        if !headers.iter().all(|(name, value)| is_header(name, value)) {
            return Err(EngineError::Unplayable);
        }
        self.apply(&target, headers, start)
            .map_err(|_| EngineError::Unplayable)
    }

    fn apply(
        &mut self,
        target: &str,
        headers: &[(String, String)],
        start: Duration,
    ) -> libmpv2::Result<()> {
        let is_agent = |name: &str| name.eq_ignore_ascii_case("user-agent");
        let agent = headers
            .iter()
            .find(|(name, _)| is_agent(name))
            .map_or(self.user_agent.as_str(), |(_, value)| value);
        self.mpv.set_property("user-agent", agent)?;
        self.mpv
            .command("change-list", &["http-header-fields", "clr", ""])?;
        // One at a time: values may contain commas.
        for (name, value) in headers.iter().filter(|(name, _)| !is_agent(name)) {
            let field = format!("{name}: {value}");
            self.mpv
                .command("change-list", &["http-header-fields", "append", &field])?;
        }
        self.mpv.set_property("pause", true)?;
        let start_at = if start.is_zero() {
            "none".to_owned()
        } else {
            seconds(start)
        };
        self.mpv.set_property("start", start_at.as_str())?;
        *self.timeline() = Timeline::opening_at(start);
        self.mpv.command("loadfile", &[target])
    }

    fn timeline(&self) -> MutexGuard<'_, Timeline> {
        lock(&self.timeline)
    }

    /// Ends [`MpvEvents`] without dropping the engine. Whoever reads the events
    /// can then let go of the player that owns it.
    pub fn shutdown_handle(&self) -> MpvShutdown {
        MpvShutdown(Arc::clone(&self.mpv))
    }
}

/// Tells mpv to quit, so its event thread can end.
pub struct MpvShutdown(Arc<Mpv>);

impl MpvShutdown {
    pub fn shut_down(&self) {
        // mpv refuses only when it is already shutting down.
        let _ = self.0.command("quit", &[]);
    }
}

impl Engine for MpvEngine {
    fn plays_samples(&self) -> bool {
        false
    }

    fn load(&mut self, media: &Media, start: Duration) -> Result<(), EngineError> {
        self.open(media, start).inspect_err(|_| {
            // Nothing may keep playing under a track that failed.
            let _ = self.mpv.command("stop", &[]);
            *self.timeline() = Timeline::default();
        })
    }

    fn stop(&mut self) {
        // mpv refuses only when it is shutting down.
        let _ = self.mpv.command("stop", &[]);
        *self.timeline() = Timeline::default();
    }

    // mpv refuses these only when nothing is loaded, and then there is nothing
    // to change: a failure of the media itself arrives as an event.
    fn set_playing(&mut self, playing: bool) {
        let _ = self.mpv.set_property("pause", !playing);
    }

    fn seek(&mut self, position: Duration) {
        self.timeline().position = position;
        let _ = self
            .mpv
            .command("seek", &[&seconds(position), "absolute+exact"]);
    }

    fn set_volume(&mut self, volume: u8) {
        let _ = self.mpv.set_property("volume", i64::from(volume));
    }

    fn position(&self) -> Duration {
        self.timeline().position
    }

    fn duration(&self) -> Duration {
        self.timeline().duration
    }
}

impl Drop for MpvEngine {
    /// Ends [`MpvEvents`], which holds another reference to mpv. In the app the
    /// event thread keeps the engine alive, so it ends through [`MpvShutdown`].
    fn drop(&mut self) {
        let _ = self.mpv.command("quit", &[]);
    }
}

#[cfg(test)]
impl MpvEngine {
    pub fn option_text_of(&self, property: &str) -> String {
        self.mpv.get_property(property).unwrap()
    }

    pub fn option_text(&self, option: &str) -> String {
        self.option_text_of(&format!("options/{option}"))
    }

    pub fn option_number(&self, option: &str) -> f64 {
        self.mpv.get_property(&format!("options/{option}")).unwrap()
    }
}

/// What mpv reports. One thread reads it.
pub struct MpvEvents {
    mpv: Arc<Mpv>,
    timeline: Arc<Mutex<Timeline>>,
}

/// The outcome of one look at mpv's event queue.
pub enum Step {
    Event(EngineEvent),
    /// Nothing the player needs to hear.
    Quiet,
    Shutdown,
}

impl MpvEvents {
    /// Sleeps until there is something for the player. `None` once mpv is gone.
    pub fn wait(&mut self) -> Option<EngineEvent> {
        loop {
            // A negative timeout waits without one, so a paused player never
            // wakes.
            match self.look(-1.0) {
                Step::Event(event) => return Some(event),
                Step::Quiet => {}
                Step::Shutdown => return None,
            }
        }
    }

    #[cfg(test)]
    pub fn step(&mut self, timeout: Duration) -> Step {
        self.look(timeout.as_secs_f64())
    }

    fn look(&mut self, timeout: f64) -> Step {
        let Some(event) = self.mpv.wait_event(timeout) else {
            return Step::Quiet;
        };
        if matches!(event, Ok(Event::Shutdown)) {
            return Step::Shutdown;
        }
        let redirect = matches!(event, Ok(Event::EndFile(mpv_end_file_reason::Redirect)));
        if matches!(event, Ok(Event::PlaybackRestart)) {
            self.read_timeline();
        }
        let report = translate(&mut lock(&self.timeline), event, Instant::now());
        match report {
            Some(report) => {
                if redirect {
                    let _ = self.mpv.command("stop", &[]);
                }
                Step::Event(report)
            }
            None => Step::Quiet,
        }
    }

    /// Reads where the media is as it opens. The observed values can have
    /// passed while it was still opening, and mpv repeats none of them.
    fn read_timeline(&self) {
        let read = |name| self.mpv.get_property::<f64>(name).ok().and_then(time);
        let (position, duration) = (read("time-pos"), read("duration"));
        let mut timeline = lock(&self.timeline);
        if timeline.opening {
            return;
        }
        if let Some(position) = position {
            timeline.position = position;
        }
        if let Some(duration) = duration {
            timeline.duration = duration;
        }
    }
}

fn lock(timeline: &Mutex<Timeline>) -> MutexGuard<'_, Timeline> {
    timeline.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A time as mpv's commands take it.
fn seconds(time: Duration) -> String {
    format!("{:.3}", time.as_secs_f64())
}

/// Whether the pair stays one header line when sent.
fn is_header(name: &str, value: &str) -> bool {
    let breaks = |text: &str| text.contains(['\r', '\n', '\0']);
    !name.is_empty() && !name.contains(':') && !breaks(name) && !breaks(value)
}
