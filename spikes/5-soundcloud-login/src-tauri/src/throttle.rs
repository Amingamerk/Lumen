//! One limiter for every SoundCloud request: spacing with jitter, cool-downs after 429 and
//! repeated failures, and a full stop on a bot check (docs/PROJECT.md 3c).

use std::time::{Duration, SystemTime};

use tokio::{
    sync::Mutex,
    time::{Instant, sleep_until},
};

use crate::{
    error::Error,
    http::{Http, Request, Response},
};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Least time between two requests.
    pub min_gap: Duration,
    /// Random extra wait, up to this much.
    pub jitter: Duration,
    /// First cool-down when the server names none. It doubles on every further strike.
    pub cooldown: Duration,
    pub max_cooldown: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            min_gap: Duration::from_millis(750),
            jitter: Duration::from_millis(500),
            cooldown: Duration::from_secs(60),
            max_cooldown: Duration::from_secs(15 * 60),
        }
    }
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// Failures in a row (network or 5xx) that start a cool-down.
const FAILURE_LIMIT: u32 = 3;

pub struct Throttle<H> {
    inner: H,
    limits: Limits,
    // Held across each request, so requests never run in parallel.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    last: Option<Instant>,
    cooling_until: Option<Instant>,
    rate_limit_strikes: u32,
    failures: u32,
    blocked: bool,
}

impl<H> Throttle<H> {
    pub fn new(inner: H, limits: Limits) -> Self {
        Self {
            inner,
            limits,
            state: Mutex::default(),
        }
    }

    pub fn inner(&self) -> &H {
        &self.inner
    }

    fn jitter(&self) -> Duration {
        let max = u64::try_from(self.limits.jitter.as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(fastrand::u64(0..=max))
    }

    /// Exponential backoff with jitter, `strikes` starting at 1.
    fn backoff(&self, strikes: u32) -> Duration {
        let factor = 1u32 << strikes.saturating_sub(1).min(16);
        self.limits
            .cooldown
            .saturating_mul(factor)
            .min(self.limits.max_cooldown)
            + self.jitter()
    }

    fn count_failure(&self, state: &mut State) {
        state.failures += 1;
        if state.failures >= FAILURE_LIMIT {
            let wait = self.backoff(state.failures - FAILURE_LIMIT + 1);
            state.cooling_until = Some(Instant::now() + wait);
        }
    }
}

/// `Retry-After` is either seconds or an HTTP date.
fn retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return Some(value.parse().map_or(DAY, Duration::from_secs));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(SystemTime::now()).unwrap_or_default())
}

fn rate_limited(wait: Duration) -> Error {
    Error::RateLimited {
        secs: wait.as_secs() + u64::from(wait.subsec_nanos() > 0),
    }
}

/// SoundCloud's anti-bot service answers 401/403 with a captcha challenge.
fn is_bot_check(response: &Response) -> bool {
    matches!(response.status, 401 | 403) && {
        let body = response.body.to_ascii_lowercase();
        ["captcha", "datadome"].iter().any(|m| body.contains(m))
    }
}

impl<H: Http> Http for Throttle<H> {
    async fn get(&self, request: Request) -> Result<Response, Error> {
        let mut state = self.state.lock().await;
        if state.blocked {
            return Err(Error::Blocked);
        }
        let now = Instant::now();
        if let Some(until) = state.cooling_until.filter(|until| *until > now) {
            return Err(rate_limited(until - now));
        }
        if let Some(last) = state.last {
            sleep_until(last + self.limits.min_gap + self.jitter()).await;
        }
        state.last = Some(Instant::now());

        let response = match self.inner.get(request).await {
            Ok(response) => response,
            Err(error) => {
                self.count_failure(&mut state);
                return Err(error);
            }
        };

        if response.status == 429 {
            state.rate_limit_strikes += 1;
            // Honor the server's wait as given, only bounded so the clock cannot overflow.
            let wait = match response.retry_after.as_deref().and_then(retry_after) {
                Some(asked) => asked.min(DAY),
                None => self.backoff(state.rate_limit_strikes),
            };
            state.cooling_until = Some(Instant::now() + wait);
            return Err(rate_limited(wait));
        }
        if is_bot_check(&response) {
            state.blocked = true;
            return Err(Error::Blocked);
        }
        if response.status >= 500 {
            self.count_failure(&mut state);
        } else {
            state.failures = 0;
            if response.status < 400 {
                state.rate_limit_strikes = 0;
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::advance;
    use url::Url;

    use super::*;
    use crate::testing::{FakeHttp, identity, reply};

    const SEC: Duration = Duration::from_secs(1);

    fn limits() -> Limits {
        Limits::default()
    }

    async fn get(http: &impl Http) -> Result<u16, Error> {
        let url = Url::parse("https://api-v2.soundcloud.com/me").unwrap();
        http.get(Request::public(url, &identity()))
            .await
            .map(|r| r.status)
    }

    fn too_many(retry_after: Option<&str>) -> Result<Response, Error> {
        Ok(Response {
            status: 429,
            retry_after: retry_after.map(str::to_string),
            body: String::new(),
        })
    }

    fn assert_cooling(result: Result<u16, Error>, from: u64, to: u64) {
        match result {
            Err(Error::RateLimited { secs }) => {
                assert!((from..=to).contains(&secs), "cool-down was {secs} s")
            }
            other => panic!("expected a cool-down, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn spaces_requests_apart_with_bounded_jitter() {
        let http = Throttle::new(
            FakeHttp::sequence(vec![reply(200, ""), reply(200, ""), reply(200, "")]),
            limits(),
        );
        for _ in 0..3 {
            get(&http).await.unwrap();
        }
        let times = http.inner().seen(|s| s.at);
        for pair in times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap >= limits().min_gap, "gap {gap:?}");
            assert!(
                gap <= limits().min_gap + limits().jitter + Duration::from_millis(5),
                "gap {gap:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_429_pauses_for_retry_after_without_touching_the_network() {
        let http = Throttle::new(
            FakeHttp::sequence(vec![too_many(Some("120")), reply(200, "")]),
            limits(),
        );
        assert_cooling(get(&http).await, 120, 121);
        let started = Instant::now();
        assert_cooling(get(&http).await, 119, 121);
        assert_eq!(
            Instant::now(),
            started,
            "a cool-down must not sleep and retry"
        );
        assert_eq!(http.inner().urls().len(), 1);

        advance(122 * SEC).await;
        assert_eq!(get(&http).await, Ok(200));
        assert_eq!(http.inner().urls().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn honors_retry_after_given_as_a_date() {
        let later = httpdate::fmt_http_date(std::time::SystemTime::now() + 600 * SEC);
        let http = Throttle::new(FakeHttp::sequence(vec![too_many(Some(&later))]), limits());
        assert_cooling(get(&http).await, 595, 601);
    }

    #[tokio::test(start_paused = true)]
    async fn backs_off_exponentially_when_no_retry_after_is_sent() {
        let http = Throttle::new(
            FakeHttp::sequence(vec![too_many(None), too_many(Some("nonsense"))]),
            limits(),
        );
        assert_cooling(get(&http).await, 60, 61);
        advance(62 * SEC).await;
        assert_cooling(get(&http).await, 120, 121);
    }

    #[tokio::test(start_paused = true)]
    async fn an_absurd_retry_after_is_capped_at_a_day() {
        let huge = u64::MAX.to_string();
        let http = Throttle::new(FakeHttp::sequence(vec![too_many(Some(&huge))]), limits());
        assert_cooling(get(&http).await, 86_400, 86_401);
    }

    #[tokio::test(start_paused = true)]
    async fn a_bot_check_stops_all_requests() {
        let challenge = r#"{"url":"https://geo.captcha-delivery.com/captcha/?x=1"}"#;
        let http = Throttle::new(FakeHttp::sequence(vec![reply(403, challenge)]), limits());
        assert_eq!(get(&http).await, Err(Error::Blocked));
        advance(86_400 * SEC).await;
        assert_eq!(get(&http).await, Err(Error::Blocked));
        assert_eq!(http.inner().urls().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn plain_401_403_and_pages_that_mention_datadome_pass_through() {
        let http = Throttle::new(
            FakeHttp::sequence(vec![
                reply(401, ""),
                reply(403, "forbidden"),
                reply(200, "var datadome = 'captcha-delivery.com'"),
                reply(200, ""),
            ]),
            limits(),
        );
        for status in [401, 403, 200, 200] {
            assert_eq!(get(&http).await, Ok(status));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn three_failures_in_a_row_start_a_cool_down() {
        let http = Throttle::new(
            FakeHttp::sequence(vec![
                reply(500, ""),
                Err(Error::Network("timed out".into())),
                reply(503, ""),
            ]),
            limits(),
        );
        assert_eq!(get(&http).await, Ok(500));
        assert!(get(&http).await.is_err());
        assert_eq!(get(&http).await, Ok(503));
        assert_cooling(get(&http).await, 59, 61);
        assert_eq!(http.inner().urls().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_success_resets_the_failure_count() {
        let http = Throttle::new(
            FakeHttp::sequence(vec![
                reply(500, ""),
                reply(500, ""),
                reply(200, ""),
                reply(500, ""),
                reply(500, ""),
                reply(200, ""),
            ]),
            limits(),
        );
        for status in [500, 500, 200, 500, 500, 200] {
            assert_eq!(get(&http).await, Ok(status));
        }
    }
}
