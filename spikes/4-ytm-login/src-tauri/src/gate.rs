//! One limiter for every call to YouTube (PROJECT.md 3c): spacing, cool-down after 429, and a
//! hard pause on bot checks or repeated failures. The caller passes the time in.

/// Minimum seconds between two runs.
pub const MIN_INTERVAL_S: u64 = 10;
const FIRST_COOLDOWN_S: u64 = 60;
const MAX_COOLDOWN_S: u64 = 3_600;
const MAX_FAILURES: u32 = 3;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum GateError {
    #[error("too soon after the last run: wait {0} s")]
    TooSoon(u64),
    #[error("YouTube asked us to slow down (429): cooling down for {0} s")]
    CoolingDown(u64),
    #[error("paused: {0}. Restart the app only after checking the account in a browser")]
    Paused(&'static str),
}

#[derive(Debug, Default)]
pub struct Gate {
    ready_at: u64,
    cooldown_until: u64,
    rate_limits: u32,
    failures: u32,
    paused: Option<&'static str>,
}

impl Gate {
    /// Asks to start a run now.
    pub fn enter(&mut self, now: u64) -> Result<(), GateError> {
        if let Some(reason) = self.paused {
            return Err(GateError::Paused(reason));
        }
        if now < self.cooldown_until {
            return Err(GateError::CoolingDown(self.cooldown_until - now));
        }
        if now < self.ready_at {
            return Err(GateError::TooSoon(self.ready_at - now));
        }
        self.ready_at = now.saturating_add(MIN_INTERVAL_S);
        Ok(())
    }

    /// Starts a cool-down and returns its length. `Retry-After` is a minimum; without it the
    /// wait doubles each time. `jitter` only ever adds to it.
    pub fn rate_limited(&mut self, now: u64, retry_after: Option<u64>, jitter: u64) -> u64 {
        // 60 s shifted by at most 6 already passes the one hour cap.
        let backoff = (FIRST_COOLDOWN_S << self.rate_limits.min(6)).min(MAX_COOLDOWN_S);
        let base = retry_after.unwrap_or(backoff);
        self.rate_limits += 1;
        let wait = base.saturating_add(jitter % (base / 4 + 1));
        self.cooldown_until = self.cooldown_until.max(now.saturating_add(wait));
        wait
    }

    pub fn success(&mut self) {
        self.rate_limits = 0;
        self.failures = 0;
    }

    /// Counts an unexplained failure and pauses after too many in a row.
    pub fn failure(&mut self) {
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            self.pause("three failures in a row");
        }
    }

    pub fn pause(&mut self, reason: &'static str) {
        self.paused.get_or_insert(reason);
    }
}

/// `Retry-After` is either seconds or an HTTP date.
pub fn parse_retry_after(value: &str, now: u64) -> Option<u64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse() {
        return Some(seconds);
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let at = date.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(at.saturating_sub(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_runs_apart() {
        let mut gate = Gate::default();

        assert_eq!(gate.enter(1_000), Ok(()));
        assert_eq!(gate.enter(1_003), Err(GateError::TooSoon(7)));
        assert_eq!(gate.enter(1_010), Ok(()));
    }

    #[test]
    fn honors_retry_after_as_a_minimum_and_adds_jitter() {
        let mut gate = Gate::default();

        let waited = gate.rate_limited(1_000, Some(120), 7);

        assert_eq!(waited, 127);
        assert_eq!(gate.enter(1_126), Err(GateError::CoolingDown(1)));
        assert_eq!(gate.enter(1_127), Ok(()));
    }

    #[test]
    fn jitter_stays_within_a_quarter_of_the_wait() {
        let mut gate = Gate::default();

        for jitter in [0, 1, 25, 26, 1_000, u64::MAX] {
            let waited = gate.rate_limited(0, Some(100), jitter);
            assert!((100..=125).contains(&waited), "{jitter} -> {waited}");
        }
    }

    #[test]
    fn backs_off_exponentially_without_retry_after_up_to_an_hour() {
        let mut gate = Gate::default();

        let waits: Vec<u64> = (0..8).map(|_| gate.rate_limited(0, None, 0)).collect();

        assert_eq!(waits, [60, 120, 240, 480, 960, 1_920, 3_600, 3_600]);
    }

    #[test]
    fn a_success_resets_the_backoff() {
        let mut gate = Gate::default();
        gate.rate_limited(0, None, 0);

        gate.success();

        assert_eq!(gate.rate_limited(0, None, 0), 60);
    }

    #[test]
    fn a_later_short_wait_does_not_shorten_a_running_cooldown() {
        let mut gate = Gate::default();
        gate.rate_limited(0, Some(600), 0);

        gate.rate_limited(0, Some(1), 0);

        assert_eq!(gate.enter(100), Err(GateError::CoolingDown(500)));
    }

    #[test]
    fn pauses_after_three_failures_in_a_row() {
        let mut gate = Gate::default();
        gate.failure();
        gate.failure();
        gate.success();
        gate.failure();
        gate.failure();
        assert_eq!(gate.enter(100), Ok(()));

        gate.failure();

        assert!(matches!(gate.enter(10_000), Err(GateError::Paused(_))));
    }

    #[test]
    fn a_pause_does_not_expire() {
        let mut gate = Gate::default();

        gate.pause("bot check");
        gate.success();

        assert_eq!(
            gate.enter(u64::MAX / 2),
            Err(GateError::Paused("bot check"))
        );
    }

    #[test]
    fn reads_retry_after_as_seconds_or_as_an_http_date() {
        // 784111777 is the example date from RFC 9110.
        let date = "Sun, 06 Nov 1994 08:49:37 GMT";

        assert_eq!(parse_retry_after(" 30 ", 0), Some(30));
        assert_eq!(parse_retry_after(date, 784_111_700), Some(77));
        assert_eq!(parse_retry_after(date, 784_111_999), Some(0));
        assert_eq!(parse_retry_after("soon", 0), None);
    }
}
