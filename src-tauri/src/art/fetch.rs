//! Downloading one picture.

use std::future::Future;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use reqwest::Url;
use reqwest::header::{ACCEPT, RETRY_AFTER};
use reqwest::redirect::Policy;

use super::host;

/// The largest picture that is read; a cover is tens of kilobytes.
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("the image host answered 429")]
    RateLimited { retry_after: Option<Duration> },
    #[error("the image host answered {0}")]
    Status(u16),
    #[error("the picture is larger than Lumen reads")]
    TooLarge,
    #[error("the image host could not be reached")]
    Network,
}

/// Where pictures come from. A seam so tests need no network.
pub trait Fetch: Send + Sync + 'static {
    fn get(&self, url: Url) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send;
}

/// Fetches over HTTPS. It sends no cookies and no credentials, and follows a
/// redirect only to another allowlisted address.
#[derive(Default)]
pub struct HttpFetch {
    /// Built on the first request, so starting the app costs nothing.
    client: OnceLock<reqwest::Client>,
}

impl HttpFetch {
    fn client(&self) -> Result<&reqwest::Client, FetchError> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            // Redirects included.
            .https_only(true)
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS || !host::is_allowed(attempt.url()) {
                    attempt.error("artwork does not follow that redirect")
                } else {
                    attempt.follow()
                }
            }))
            .retry(reqwest::retry::never())
            .referer(false)
            // No cookie store: the feature is not even built in. The system
            // proxy stays on, like the account's requests.
            .build()
            .map_err(|_| FetchError::Network)?;
        Ok(self.client.get_or_init(|| client))
    }
}

impl Fetch for HttpFetch {
    async fn get(&self, url: Url) -> Result<Vec<u8>, FetchError> {
        let mut reply = self
            .client()?
            .get(url)
            .header(ACCEPT, "image/*")
            .send()
            .await
            .map_err(|_| FetchError::Network)?;
        let status = reply.status();
        if status.as_u16() == 429 {
            let retry_after = reply
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(retry_after);
            return Err(FetchError::RateLimited { retry_after });
        }
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        // The last stop must be allowlisted too.
        if !host::is_allowed(reply.url()) {
            return Err(FetchError::Network);
        }
        if reply
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(FetchError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = reply.chunk().await.map_err(|_| FetchError::Network)? {
            if body.len() + chunk.len() > MAX_BYTES {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

/// The wait a `Retry-After` header asks for, from seconds or an HTTP date.
fn retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(SystemTime::now()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_reads_seconds_and_dates() {
        assert_eq!(retry_after("90"), Some(Duration::from_secs(90)));
        assert_eq!(retry_after(" 5 "), Some(Duration::from_secs(5)));
        // A date in the past asks for no wait.
        assert_eq!(
            retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(Duration::ZERO)
        );
        assert_eq!(retry_after("soon"), None);
        assert_eq!(retry_after("-3"), None);
        assert_eq!(retry_after(""), None);
    }
}
