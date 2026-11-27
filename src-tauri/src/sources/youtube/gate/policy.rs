//! The gate's rules as pure state: what a reply does to it and what it blocks.
//! Times are passed in as Unix milliseconds.

use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

const FIRST_BACKOFF_MS: u64 = 60_000;
const MAX_BACKOFF_MS: u64 = 3_600_000;
const MAX_FAILURES: u32 = 3;

/// What one run's outcome means to the gate.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Success,
    /// A 429. `retry_after` is the header as received.
    RateLimited {
        retry_after: Option<String>,
    },
    /// A captcha or "unusual traffic" answer.
    BotCheck,
    Failure,
    /// The session was refused. Counts nothing: the account handles it.
    Rejected,
    /// Nothing counted: the request never reached the service.
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    BotCheck,
    RepeatedFailures,
    StateUnreadable,
    StateUnwritable,
    ExpiryUnsaved,
}

/// Whether the gate lets requests through. The wire shape is mirrored in
/// `src/lib/ipc.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GateView {
    Ready,
    CoolingDown { until_ms: u64 },
    Paused { reason: PauseReason },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GateState {
    /// Zero when there never was a cool-down.
    pub cooldown_until_ms: u64,
    /// The backoff step: cool-downs since the last success.
    pub rate_limits: u32,
    /// Failures in a row.
    pub failures: u32,
    pub paused: Option<PauseReason>,
}

impl GateState {
    /// `jitter` is any number; it only ever lengthens a wait.
    pub fn apply(&mut self, verdict: &Verdict, now_ms: u64, jitter: u64) {
        match verdict {
            Verdict::Success => {
                self.failures = 0;
                self.rate_limits = 0;
            }
            Verdict::RateLimited { retry_after } => {
                let minimum = retry_after
                    .as_deref()
                    .and_then(|value| parse_retry_after(value, now_ms));
                self.cool_down(minimum.unwrap_or(0), now_ms, jitter);
            }
            // A cool-down too, so Resume cannot send the next request at once.
            Verdict::BotCheck => {
                self.cool_down(0, now_ms, jitter);
                self.pause(PauseReason::BotCheck);
            }
            Verdict::Failure => {
                self.failures = self.failures.saturating_add(1);
                if self.failures >= MAX_FAILURES {
                    self.pause(PauseReason::RepeatedFailures);
                }
            }
            Verdict::Rejected | Verdict::Neutral => {}
        }
    }

    /// Starts the next backoff step, waiting at least `minimum_ms`. A running
    /// cool-down is never shortened.
    fn cool_down(&mut self, minimum_ms: u64, now_ms: u64, jitter: u64) {
        // Six doublings already pass the cap.
        let backoff = (FIRST_BACKOFF_MS << self.rate_limits.min(6)).min(MAX_BACKOFF_MS);
        let base = backoff.max(minimum_ms);
        let wait = base.saturating_add(jitter % (base / 4 + 1));
        self.rate_limits = self.rate_limits.saturating_add(1);
        self.cooldown_until_ms = self.cooldown_until_ms.max(now_ms.saturating_add(wait));
    }

    /// The first reason stays until Resume.
    pub fn pause(&mut self, reason: PauseReason) {
        self.paused.get_or_insert(reason);
    }

    pub fn view(&self, now_ms: u64) -> GateView {
        match self.paused {
            Some(reason) => GateView::Paused { reason },
            None if now_ms < self.cooldown_until_ms => GateView::CoolingDown {
                until_ms: self.cooldown_until_ms,
            },
            None => GateView::Ready,
        }
    }

    /// Ends a pause. A running cool-down and the backoff step stay.
    pub fn resume(&mut self) {
        self.paused = None;
        self.failures = 0;
    }
}

/// The wait a `Retry-After` header asks for, from seconds or an HTTP date.
fn parse_retry_after(value: &str, now_ms: u64) -> Option<u64> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        // Only too many digits can fail here: that is still a very long wait.
        let seconds: u64 = value.parse().unwrap_or(u64::MAX);
        return Some(seconds.saturating_mul(1_000));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let at = date.duration_since(UNIX_EPOCH).ok()?.as_millis();
    Some(u64::try_from(at).unwrap_or(u64::MAX).saturating_sub(now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    fn limited(retry_after: Option<&str>) -> Verdict {
        Verdict::RateLimited {
            retry_after: retry_after.map(str::to_owned),
        }
    }

    /// Applies one verdict at `NOW` and returns how long the cool-down then is.
    fn wait(state: &mut GateState, verdict: Verdict, jitter: u64) -> u64 {
        state.apply(&verdict, NOW, jitter);
        match state.view(NOW) {
            GateView::CoolingDown { until_ms } => until_ms - NOW,
            _ => 0,
        }
    }

    #[test]
    fn a_429_cools_down_for_at_least_retry_after() {
        let mut state = GateState::default();

        assert_eq!(wait(&mut state, limited(Some("120")), 0), 120_000);

        assert_eq!(
            state.view(NOW + 119_999),
            GateView::CoolingDown {
                until_ms: NOW + 120_000
            }
        );
        assert_eq!(state.view(NOW + 120_000), GateView::Ready);
    }

    #[test]
    fn without_retry_after_the_wait_doubles_up_to_an_hour() {
        let mut state = GateState::default();

        let waits: Vec<u64> = (0..8)
            .map(|step| {
                // Each 429 arrives after the previous wait is over.
                let now = NOW + step * 10_000_000;
                state.apply(&limited(None), now, 0);
                (state.cooldown_until_ms - now) / 1_000
            })
            .collect();

        assert_eq!(waits, [60, 120, 240, 480, 960, 1_920, 3_600, 3_600]);
    }

    #[test]
    fn jitter_only_adds_up_to_a_quarter() {
        for jitter in [0, 1, 25_000, 25_001, 1_000_000, u64::MAX] {
            let waited = wait(&mut GateState::default(), limited(Some("100")), jitter);

            assert!((100_000..=125_000).contains(&waited), "{jitter}: {waited}");
        }
        let jittered = wait(&mut GateState::default(), limited(Some("100")), 25_000);
        assert_eq!(jittered, 125_000);
    }

    #[test]
    fn a_short_retry_after_still_waits_the_backoff_step() {
        for header in ["0", "5"] {
            assert_eq!(
                wait(&mut GateState::default(), limited(Some(header)), 0),
                60_000,
                "{header}"
            );
        }
    }

    #[test]
    fn a_short_wait_never_shortens_a_running_one() {
        let mut state = GateState::default();
        wait(&mut state, limited(Some("600")), 0);

        assert_eq!(wait(&mut state, limited(Some("1")), 0), 600_000);
    }

    #[test]
    fn retry_after_is_seconds_or_an_http_date() {
        // 90 s and 600 s after `NOW`, which is Tue, 14 Nov 2023 22:13:20 GMT.
        for (header, waited) in [
            (" 90 ", 90_000),
            ("Tue, 14 Nov 2023 22:23:20 GMT", 600_000),
            // Past dates, junk and signs fall back to the backoff step.
            ("Tue, 14 Nov 2023 22:00:00 GMT", 60_000),
            ("soon", 60_000),
            ("-5", 60_000),
            ("", 60_000),
        ] {
            assert_eq!(
                wait(&mut GateState::default(), limited(Some(header)), 0),
                waited,
                "{header:?}"
            );
        }
    }

    #[test]
    fn a_huge_retry_after_is_honored_without_overflow() {
        for header in ["18446744073709551615", "99999999999999999999999"] {
            let mut state = GateState::default();

            state.apply(&limited(Some(header)), NOW, u64::MAX);

            assert_eq!(state.cooldown_until_ms, u64::MAX, "{header}");
        }
    }

    #[test]
    fn a_bot_check_pauses_at_once_and_starts_a_growing_cooldown() {
        let mut state = GateState::default();

        state.apply(&Verdict::BotCheck, NOW, 0);

        let paused = GateView::Paused {
            reason: PauseReason::BotCheck,
        };
        assert_eq!(state.view(NOW), paused);
        state.resume();
        assert_eq!(
            state.view(NOW),
            GateView::CoolingDown {
                until_ms: NOW + 60_000
            }
        );
        state.apply(&Verdict::BotCheck, NOW + 60_000, 0);
        assert_eq!(state.cooldown_until_ms, NOW + 180_000);
    }

    #[test]
    fn the_third_failure_in_a_row_pauses() {
        let mut state = GateState::default();
        for verdict in [
            Verdict::Failure,
            Verdict::Failure,
            Verdict::Success,
            Verdict::Failure,
            Verdict::Failure,
        ] {
            state.apply(&verdict, NOW, 0);
        }
        assert_eq!(state.view(NOW), GateView::Ready);

        state.apply(&Verdict::Failure, NOW, 0);

        assert_eq!(
            state.view(NOW),
            GateView::Paused {
                reason: PauseReason::RepeatedFailures
            }
        );
    }

    #[test]
    fn rejected_and_neutral_neither_count_nor_reset() {
        let mut state = GateState::default();
        state.apply(&limited(None), 0, 0);
        state.apply(&Verdict::Failure, NOW, 0);
        let before = state.clone();

        state.apply(&Verdict::Rejected, NOW, 0);
        state.apply(&Verdict::Neutral, NOW, 0);

        assert_eq!(state, before);
    }

    #[test]
    fn a_success_resets_failures_and_backoff_only() {
        let mut state = GateState::default();
        state.apply(&limited(Some("600")), NOW, 0);
        state.apply(&Verdict::Failure, NOW, 0);

        state.apply(&Verdict::Success, NOW, 0);

        assert_eq!((state.failures, state.rate_limits), (0, 0));
        assert_eq!(state.cooldown_until_ms, NOW + 600_000);
    }

    #[test]
    fn only_resume_ends_a_pause_and_it_keeps_the_cooldown() {
        let mut state = GateState::default();
        state.apply(&limited(Some("600")), NOW, 0);
        for _ in 0..3 {
            state.apply(&Verdict::Failure, NOW, 0);
        }
        state.apply(&Verdict::Success, NOW, 0);
        assert!(matches!(state.view(u64::MAX), GateView::Paused { .. }));
        state.rate_limits = 2;

        state.resume();

        assert_eq!(
            state,
            GateState {
                cooldown_until_ms: NOW + 600_000,
                rate_limits: 2,
                failures: 0,
                paused: None,
            }
        );
    }
}
