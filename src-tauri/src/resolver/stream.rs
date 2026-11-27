//! What one yt-dlp run gave: a stream to load, or a fixed reason why not. Raw
//! output never gets past here, because it can quote the stream URL.

use std::collections::BTreeMap;
use std::fmt;

use reqwest::Url;
use serde::Deserialize;

use super::ResolveError;
use super::process::ProcessOutput;
use crate::audio::Media;

/// Longest `available_at` wait that is accepted.
pub const MAX_WAIT_MS: u64 = 60_000;

/// One resolved stream.
#[derive(Clone, PartialEq)]
pub struct StreamInfo {
    url: String,
    headers: Vec<(String, String)>,
    /// Unix seconds before which the URL does not work.
    pub available_at: Option<u64>,
    /// Unix seconds after which it stops working.
    pub expires_at: Option<u64>,
}

/// Never shows the URL, which holds the client IP, or a header.
impl fmt::Debug for StreamInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamInfo")
            .field("available_at", &self.available_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl StreamInfo {
    pub fn media(&self) -> Media {
        Media::Stream {
            url: self.url.clone(),
            headers: self.headers.clone(),
        }
    }
}

#[derive(Deserialize)]
struct YtDlpJson {
    url: String,
    /// Fractional when a pre-roll ad's skip offset is.
    available_at: Option<f64>,
    #[serde(default)]
    http_headers: BTreeMap<String, String>,
}

/// Judges one finished run.
pub fn read(output: &ProcessOutput, now_ms: u64) -> Result<StreamInfo, ResolveError> {
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if let Some(error) = warning_sign(&stderr) {
        return Err(error);
    }
    if !output.success {
        return Err(classify_failure(&stderr));
    }
    // The parser's message could quote the stream URL, so it is dropped.
    let json: YtDlpJson =
        serde_json::from_slice(&output.stdout).map_err(|_| ResolveError::UnexpectedOutput)?;
    let url = googlevideo(&json.url).ok_or(ResolveError::UnexpectedHost)?;
    let available_at = json.available_at.map(|at| at.ceil() as u64);
    if available_at.is_some_and(|at| at.saturating_mul(1_000) > now_ms + MAX_WAIT_MS) {
        return Err(ResolveError::AvailableTooLate);
    }
    let expires_at = url_field(&url, "expire").and_then(|at| at.parse().ok());
    let headers = json
        .http_headers
        .into_iter()
        .filter(|(name, _)| {
            !["cookie", "authorization", "range"]
                .iter()
                .any(|blocked| name.eq_ignore_ascii_case(blocked))
        })
        .collect();
    Ok(StreamInfo {
        url,
        headers,
        available_at,
        expires_at,
    })
}

/// The URL as it was understood, if it is HTTPS on a `googlevideo.com` host.
/// mpv is given this form, so it cannot read another host into it.
fn googlevideo(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let allowed = url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".googlevideo.com"));
    allowed.then(|| url.into())
}

/// A `name=value` query parameter, or the `/name/value/` form of manifest URLs.
fn url_field<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let from_query = url.split_once('?').and_then(|(_, query)| {
        query
            .split('&')
            .find_map(|p| p.strip_prefix(name)?.strip_prefix('='))
    });
    from_query.or_else(|| url.split_once(&format!("/{name}/"))?.1.split('/').next())
}

/// Warnings that end the run even when yt-dlp itself carried on. `stderr` is in
/// lower case.
fn warning_sign(stderr: &str) -> Option<ResolveError> {
    let has = |markers: &[&str]| markers.iter().any(|m| stderr.contains(m));
    if has(&["not a bot", "unusual traffic", "captcha challenge"]) {
        Some(ResolveError::BotCheck)
    } else if has(&["http error 429", "rate-limited by youtube"]) {
        Some(ResolveError::RateLimited)
    } else if has(&["account cookies are no longer valid"]) {
        Some(ResolveError::AuthRejected)
    } else {
        None
    }
}

/// Only `error:` lines count: warnings mention HTTP 403 on healthy runs.
fn classify_failure(stderr: &str) -> ResolveError {
    let errors: String = stderr
        .lines()
        .filter(|line| line.starts_with("error:"))
        .flat_map(|line| [line, "\n"])
        .collect();
    let has = |markers: &[&str]| markers.iter().any(|m| errors.contains(m));
    if has(&["captcha"]) {
        ResolveError::BotCheck
    } else if has(&["too many requests", "rate-limited"]) {
        ResolveError::RateLimited
    } else if has(&[
        "http error 401",
        "http error 403",
        "sign in to",
        "login_required",
    ]) {
        ResolveError::AuthRejected
    } else if has(&["requested format is not available"]) {
        ResolveError::NoFormat
    } else if has(&[
        "video unavailable",
        "video is not available",
        "private video",
        "video has been removed",
    ]) {
        ResolveError::Unavailable
    } else {
        ResolveError::Failed
    }
}
