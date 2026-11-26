//! One limiter for every Spotify request: paced, and closed after a 429.

use std::time::{Duration, SystemTime};

use tokio::time::{Instant, sleep_until};

use super::error::SpotifyError;

pub const MIN_INTERVAL: Duration = Duration::from_millis(1000);
pub const MAX_JITTER: Duration = Duration::from_millis(500);
/// Cool-down for a 429 without `Retry-After`. Doubles with each one in a row.
pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
pub const MAX_COOLDOWN: Duration = Duration::from_secs(3600);
/// Longest `Retry-After` taken literally. Anything longer is cut to a day.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(86_400);

#[derive(Default)]
pub struct Gate {
    next_allowed: Option<Instant>,
    blocked_until: Option<Instant>,
    strikes: u32,
}

impl Gate {
    /// Waits for the next slot, or refuses while cooling down.
    pub async fn wait_turn(&mut self) -> Result<(), SpotifyError> {
        let now = Instant::now();
        if let Some(until) = self.blocked_until.filter(|until| *until > now) {
            return Err(SpotifyError::CoolingDown {
                secs: whole_secs(until - now),
            });
        }
        if let Some(next) = self.next_allowed {
            sleep_until(next).await;
        }
        self.next_allowed = Some(Instant::now() + MIN_INTERVAL + jitter(MAX_JITTER));
        Ok(())
    }

    /// Starts a cool-down and returns its length in seconds.
    pub fn rate_limited(&mut self, retry_after: Option<&str>) -> u64 {
        let wait = retry_after
            .and_then(|value| parse_retry_after(value, SystemTime::now()))
            .unwrap_or_else(|| {
                let factor = 2u32.saturating_pow(self.strikes);
                let base = DEFAULT_COOLDOWN.saturating_mul(factor).min(MAX_COOLDOWN);
                (base + jitter(base / 10)).min(MAX_COOLDOWN)
            });
        self.strikes = self.strikes.saturating_add(1);
        let now = Instant::now();
        let until = (now + wait).max(self.blocked_until.unwrap_or(now));
        self.blocked_until = Some(until);
        whole_secs(until - now)
    }

    /// A request went through, so any earlier cool-down is over.
    pub fn succeeded(&mut self) {
        self.strikes = 0;
        self.blocked_until = None;
    }
}

/// `Retry-After` as delta seconds or an HTTP date.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    let wait = match value.parse() {
        Ok(secs) => Duration::from_secs(secs),
        Err(_) => httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .unwrap_or_default(),
    };
    Some(wait.min(MAX_RETRY_AFTER))
}

/// Seconds, rounded up.
fn whole_secs(wait: Duration) -> u64 {
    wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

/// A random part of `max`, or all of it if the system gives no randomness.
fn jitter(max: Duration) -> Duration {
    let share = getrandom::u32().unwrap_or(u32::MAX);
    max.mul_f64(f64::from(share) / f64::from(u32::MAX))
}

#[cfg(test)]
mod tests {
    use tokio::time::advance;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn spaces_requests_with_jitter() {
        let mut gate = Gate::default();
        let start = Instant::now();
        gate.wait_turn().await.unwrap();
        assert_eq!(start.elapsed(), Duration::ZERO);
        gate.wait_turn().await.unwrap();
        let waited = start.elapsed();
        assert!(waited >= MIN_INTERVAL, "{waited:?}");
        // The paused clock rounds timers up to a millisecond.
        assert!(
            waited <= MIN_INTERVAL + MAX_JITTER + Duration::from_millis(1),
            "{waited:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stays_closed_for_retry_after() {
        let mut gate = Gate::default();
        assert_eq!(gate.rate_limited(Some("120")), 120);
        assert_eq!(
            gate.wait_turn().await,
            Err(SpotifyError::CoolingDown { secs: 120 })
        );
        advance(Duration::from_millis(119_500)).await;
        assert_eq!(
            gate.wait_turn().await,
            Err(SpotifyError::CoolingDown { secs: 1 })
        );
        advance(Duration::from_millis(500)).await;
        assert_eq!(gate.wait_turn().await, Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn backs_off_without_retry_after() {
        let mut gate = Gate::default();
        // Base 60 s doubling, plus up to a tenth of jitter, capped at an hour.
        for base in [60, 120, 240, 480, 960, 1920, 3600, 3600] {
            let secs = gate.rate_limited(None);
            assert!(
                (base..=(base + base / 10).min(3600)).contains(&secs),
                "{secs}"
            );
        }
        gate.succeeded();
        assert!(gate.rate_limited(None) <= 66);
    }

    #[tokio::test(start_paused = true)]
    async fn a_shorter_cooldown_never_replaces_a_longer_one() {
        let mut gate = Gate::default();
        gate.rate_limited(Some("300"));
        gate.rate_limited(Some("5"));
        assert_eq!(
            gate.wait_turn().await,
            Err(SpotifyError::CoolingDown { secs: 300 })
        );
    }

    #[test]
    fn reads_both_retry_after_forms() {
        let now = httpdate::parse_http_date("Thu, 26 Nov 2026 18:00:00 GMT").unwrap();
        let parse = |value| parse_retry_after(value, now);
        assert_eq!(parse(" 30 "), Some(Duration::from_secs(30)));
        assert_eq!(
            parse("Thu, 26 Nov 2026 18:02:00 GMT"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse("Thu, 26 Nov 2026 17:00:00 GMT"), Some(Duration::ZERO));
        assert_eq!(parse("soon"), None);
        // Absurd values must not overflow the clock.
        assert_eq!(
            parse("18446744073709551615"),
            Some(Duration::from_secs(86_400))
        );
    }
}
