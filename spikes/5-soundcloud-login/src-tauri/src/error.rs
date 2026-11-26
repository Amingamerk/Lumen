/// Messages are safe to show and log: no variant carries a token, an id, a URL query
/// or a response body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not signed in to SoundCloud")]
    NotLoggedIn,
    #[error("the SoundCloud session expired, sign in again")]
    SessionExpired,
    #[error("that does not look like an oauth_token value")]
    InvalidToken,
    #[error("SoundCloud requests are cooling down, wait {secs} s")]
    RateLimited { secs: u64 },
    #[error("SoundCloud showed a bot check, requests are paused until the app restarts")]
    Blocked,
    #[error("no client_id found ({found} bundles listed, {fetched} fetched)")]
    ClientIdNotFound { found: usize, fetched: usize },
    #[error("{what} returned HTTP {status}")]
    Status { what: &'static str, status: u16 },
    #[error("network error: {0}")]
    Network(String),
    #[error("unreadable {0}")]
    Parse(&'static str),
    #[error("credential store error: {0}")]
    Store(&'static str),
}
