//! Recovery of a live stream. A stream that will not load, fails, stands still
//! or ends early is let go of, and a fresh URL is asked for twice at most. An
//! expired URL is replaced before it is used. The player only decides: whoever
//! watches it resolves, and hands the outcome back.

use std::time::Duration;

use super::*;
use crate::sources::youtube::GateClock;

/// The least wait before each automatic re-resolve of one started playback.
const WAITS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];
/// Playing this long without moving on is a stall.
const STALL_AFTER: Duration = Duration::from_secs(10);
/// An end further than this before the known length came too early.
const EARLY_END: Duration = Duration::from_secs(5);

/// The time the recovery rules go by.
pub trait PlayerClock: Send {
    /// Time since some fixed point.
    fn monotonic(&self) -> Duration;
    /// Unix milliseconds, which Stream URLs expire in.
    fn wall_ms(&self) -> u64;
}

impl<C: GateClock> PlayerClock for C {
    fn monotonic(&self) -> Duration {
        GateClock::monotonic(self)
    }

    fn wall_ms(&self) -> u64 {
        GateClock::wall_ms(self)
    }
}

/// A fresh stream the player asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    /// Names this request: an answer to another one is stale.
    pub ticket: u32,
    /// The YouTube video to resolve.
    pub video: String,
}

/// Why a fetch gave no stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoStream {
    /// In words for the user.
    pub reason: String,
    /// Whether asking again could help.
    pub retry: bool,
}

/// A fetch that is planned or in progress.
#[derive(Debug)]
struct Planned {
    ticket: u32,
    /// The earliest it may start.
    due: Duration,
    started: bool,
    /// Whether it is one of the two automatic attempts. Replacing an expired
    /// URL is not.
    attempt: bool,
}

/// What one direct playback remembers about keeping its stream alive.
#[derive(Debug, Default)]
pub(super) struct Recovery {
    /// Automatic re-resolves started.
    attempts: usize,
    /// Where a stream that is gone goes on from.
    pub(super) resume_at: Duration,
    /// The length last known, for while there is no stream.
    pub(super) duration: Duration,
    /// When the stream was lost to a failure.
    failed_at: Duration,
    /// The URL was let go of because it had expired, not because it failed.
    expired: bool,
    planned: Option<Planned>,
    /// From a failure until the new stream opens.
    pub(super) reconnecting: bool,
    /// The furthest sign of life, and when it was seen.
    position: Duration,
    moved_at: Duration,
}

impl<E: Engine> Player<E> {
    #[cfg(test)]
    pub fn with_clock(mut self, clock: impl PlayerClock + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Calls `wake` after every change, which may move the next deadline.
    pub fn on_change(&mut self, wake: impl Fn() + Send + 'static) {
        self.wake = Some(Box::new(wake));
    }

    /// Drops a stream that has stood still too long.
    pub fn check_stall(&mut self) -> Option<PlayerState> {
        let deadline = self.stall_deadline()?;
        if self.clock.monotonic() < deadline {
            return None;
        }
        let at = self.reached();
        self.drop_failed(at);
        Some(self.commit())
    }

    /// Takes the planned fetch once its time has come.
    pub fn due_fetch(&mut self) -> Option<Fetch> {
        let now = self.clock.monotonic();
        let direct = self.direct.as_mut()?;
        let video = direct.video.clone()?;
        let planned = direct.recovery.planned.as_mut()?;
        if planned.started || now < planned.due {
            return None;
        }
        planned.started = true;
        let ticket = planned.ticket;
        if planned.attempt {
            direct.recovery.attempts += 1;
        }
        Some(Fetch { ticket, video })
    }

    /// Whether the answer to the fetch `ticket` is still wanted.
    pub fn awaits_fetch(&self, ticket: u32) -> bool {
        self.planned()
            .is_some_and(|planned| planned.started && planned.ticket == ticket)
    }

    /// Whether a fetch is planned or in progress.
    pub fn fetch_pending(&self) -> bool {
        self.planned().is_some()
    }

    /// Takes the answer to the fetch `ticket`. A stale one changes nothing.
    pub fn finish_fetch(
        &mut self,
        ticket: u32,
        outcome: Result<Media, NoStream>,
    ) -> Option<PlayerState> {
        if !self.awaits_fetch(ticket) {
            return None;
        }
        let now = self.clock.monotonic();
        let direct = self.direct.as_mut()?;
        direct.recovery.planned = None;
        match outcome {
            Ok(media) => {
                direct.media = Some(media);
                let (start, playing) = (direct.recovery.resume_at, self.playing);
                self.load(start, playing);
            }
            Err(no) if no.retry => {
                direct.recovery.expired = false;
                direct.recovery.failed_at = now;
                // Paused, it waits to be resumed.
                if self.playing {
                    self.plan_attempt(Some(&no.reason));
                }
            }
            Err(no) => self.stop_recovering(Some(&no.reason)),
        }
        Some(self.commit())
    }

    /// Gives up the planned or running fetch, saying why.
    pub fn abandon_fetch(&mut self, reason: &str) -> Option<PlayerState> {
        if !self.fetch_pending() {
            return None;
        }
        self.stop_recovering(Some(reason));
        Some(self.commit())
    }

    /// How long until [`Self::check_stall`] or [`Self::due_fetch`] has
    /// something to do, if time alone will bring that about.
    pub fn recovery_wake(&self) -> Option<Duration> {
        let fetch = self
            .planned()
            .filter(|planned| !planned.started)
            .map(|planned| planned.due);
        let next = fetch.into_iter().chain(self.stall_deadline()).min()?;
        Some(next.saturating_sub(self.clock.monotonic()))
    }

    /// Whether both attempts have just been used up. Said once.
    pub fn take_gave_up(&mut self) -> bool {
        std::mem::take(&mut self.gave_up)
    }

    /// The video whose stream was just let go of, so that it is not used again.
    /// Said once.
    pub fn take_dropped(&mut self) -> Option<String> {
        self.dropped.take()
    }

    /// Asks again for the stream that failed, as a new playback that goes on
    /// where the old one stopped. Answers as [`Self::begin_stream`] does.
    pub fn retry_stream(&mut self) -> Option<(String, u32, PlayerState)> {
        let (video, row) = self.failed_stream()?;
        let recovery = &self.direct.as_ref()?.recovery;
        let (resume_at, duration) = (recovery.resume_at, recovery.duration);
        let (key, _) = self.begin_stream(video.clone(), row);
        let recovery = &mut self.direct.as_mut()?.recovery;
        recovery.resume_at = resume_at;
        recovery.duration = duration;
        let state = self.commit();
        Some((video, key, state))
    }

    /// Whether what is loaded, or was, is a live stream that can be resolved
    /// again.
    pub(super) fn recoverable(&self) -> bool {
        self.direct.as_ref().is_some_and(|direct| {
            direct.video.is_some() && matches!(direct.media, Some(Media::Stream { .. }))
        })
    }

    /// Whether the loaded URL no longer works.
    pub(super) fn expired(&self) -> bool {
        let media = self.direct.as_ref().and_then(|d| d.media.as_ref());
        let Some(Media::Stream {
            expires_at: Some(at),
            ..
        }) = media
        else {
            return false;
        };
        self.recoverable() && self.clock.wall_ms() >= at.saturating_mul(1_000)
    }

    /// Where the loaded stream has come to.
    pub(super) fn reached(&self) -> Duration {
        match &self.direct {
            // Before it opens, the engine has nothing to say.
            Some(direct) if self.phase != Phase::Open => direct.recovery.resume_at,
            _ => self.engine.position(),
        }
    }

    /// Whether an end the engine reports came well before the known length.
    pub(super) fn ended_early(&self) -> bool {
        let Some(direct) = &self.direct else {
            return false;
        };
        let known = [self.engine.duration(), direct.recovery.duration]
            .into_iter()
            .find(|length| !length.is_zero())
            .unwrap_or(Duration::from_millis(direct.row.duration_ms.into()));
        known > self.reached() + EARLY_END
    }

    /// Notes a sign that the stream is alive, or that it was just asked for one.
    pub(super) fn watch_from_now(&mut self) {
        let (now, position) = (self.clock.monotonic(), self.engine.position());
        if let Some(direct) = &mut self.direct {
            direct.recovery.moved_at = now;
            direct.recovery.position = position;
        }
    }

    /// Notes progress, if the position has moved.
    pub(super) fn watch_progress(&mut self) {
        let position = self.engine.position();
        if self
            .direct
            .as_ref()
            .is_some_and(|d| d.recovery.position != position)
        {
            self.watch_from_now();
        }
    }

    /// Lets go of a stream that failed at `at` and, unless paused, plans the
    /// next attempt.
    pub(super) fn drop_failed(&mut self, at: Duration) {
        let now = self.clock.monotonic();
        self.release(at);
        if let Some(direct) = &mut self.direct {
            direct.recovery.expired = false;
            direct.recovery.failed_at = now;
        }
        if self.playing {
            self.plan_attempt(None);
        }
    }

    /// Lets go of an expired URL and asks for a new one at once, to go on from
    /// `at`.
    pub(super) fn replace_expired(&mut self, at: Duration) {
        self.release(at);
        if let Some(direct) = &mut self.direct {
            direct.recovery.expired = true;
        }
        self.plan_replacement();
    }

    /// Play or pause while there is no stream.
    pub(super) fn toggle_without_stream(&mut self) {
        let playing = !self.playing;
        self.set_playing(playing);
        let Some(direct) = &mut self.direct else {
            return;
        };
        if !playing {
            // A pause cancels what was planned; its answer is stale.
            direct.recovery.planned = None;
            direct.recovery.reconnecting = false;
        } else if direct.recovery.expired {
            self.plan_replacement();
        } else {
            self.plan_attempt(None);
        }
    }

    /// Moves where a stream that is gone goes on from.
    pub(super) fn seek_without_stream(&mut self, target: Duration) -> bool {
        let Some(direct) = &mut self.direct else {
            return false;
        };
        let recovery = &mut direct.recovery;
        let target = if recovery.duration.is_zero() {
            target
        } else {
            target.min(recovery.duration)
        };
        if target == recovery.resume_at {
            return false;
        }
        recovery.resume_at = target;
        if recovery.expired && recovery.planned.is_none() {
            self.plan_replacement();
        }
        true
    }

    fn planned(&self) -> Option<&Planned> {
        self.direct.as_ref()?.recovery.planned.as_ref()
    }

    /// When the playing stream counts as stalled.
    fn stall_deadline(&self) -> Option<Duration> {
        let watched = self.playing && matches!(self.phase, Phase::Loading | Phase::Open);
        let direct = self.direct.as_ref().filter(|_| watched)?;
        self.recoverable()
            .then(|| direct.recovery.moved_at + STALL_AFTER)
    }

    /// Stops the engine and forgets the stream, keeping the place.
    fn release(&mut self, at: Duration) {
        let duration = self.engine.duration();
        self.engine.stop();
        self.phase = Phase::Broken;
        if let Some(direct) = &mut self.direct {
            direct.media = None;
            direct.recovery.resume_at = at;
            if !duration.is_zero() {
                direct.recovery.duration = duration;
            }
            direct.recovery.planned = None;
            direct.recovery.reconnecting = false;
            self.dropped = direct.video.clone();
        }
    }

    fn plan_replacement(&mut self) {
        let now = self.clock.monotonic();
        self.plan(now, false);
    }

    /// Plans the next automatic attempt, or stops if both are used up.
    fn plan_attempt(&mut self, reason: Option<&str>) {
        let Some(direct) = &mut self.direct else {
            return;
        };
        match WAITS.get(direct.recovery.attempts) {
            Some(&wait) => {
                let due = direct.recovery.failed_at + wait;
                direct.recovery.reconnecting = true;
                self.plan(due, true);
            }
            None => {
                self.gave_up = true;
                self.stop_recovering(reason);
            }
        }
    }

    fn plan(&mut self, due: Duration, attempt: bool) {
        self.tickets = self.tickets.wrapping_add(1);
        let ticket = self.tickets;
        if let Some(direct) = &mut self.direct {
            direct.recovery.planned = Some(Planned {
                ticket,
                due,
                started: false,
                attempt,
            });
        }
    }

    /// Ends in the error state, from which only a retry goes on.
    fn stop_recovering(&mut self, reason: Option<&str>) {
        self.set_playing(false);
        let failed = match (reason, self.direct.as_ref()) {
            (Some(reason), Some(direct)) => Phase::Failed(format!(
                "Lumen could not play “{}”: {reason}.",
                direct.row.title
            )),
            _ => self.failure(EngineError::Unplayable),
        };
        self.phase = failed;
        if let Some(direct) = &mut self.direct {
            direct.recovery.planned = None;
            direct.recovery.reconnecting = false;
        }
    }
}

#[cfg(test)]
mod tests;
