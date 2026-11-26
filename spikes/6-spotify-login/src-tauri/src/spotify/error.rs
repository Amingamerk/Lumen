//! Errors carry endpoint names and status codes only: never a response body, a
//! URL with its query, a cookie or a token.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpotifyError {
    #[error("could not read the {0} response")]
    Parse(&'static str),
    #[error("network error calling {endpoint}: {detail}")]
    Network {
        endpoint: &'static str,
        detail: String,
    },
    #[error("{endpoint} answered HTTP {status}: the session is marked expired, log in again")]
    Unauthorized { endpoint: &'static str, status: u16 },
    #[error(
        "Spotify returned an anonymous token: the sp_dc cookie was not accepted, so the session is marked expired"
    )]
    AnonymousToken,
    #[error("{endpoint} answered HTTP 429: Spotify requests are paused for {retry_after_secs} s")]
    RateLimited {
        endpoint: &'static str,
        retry_after_secs: u64,
    },
    #[error("paused after an earlier HTTP 429: wait {secs} s")]
    CoolingDown { secs: u64 },
    #[error("{endpoint} answered HTTP {status}")]
    Status { endpoint: &'static str, status: u16 },
    #[error(
        "the token endpoint answered HTTP 400 for TOTP version {0}: Spotify has probably rotated the secret"
    )]
    TotpRejected(u32),
    #[error("Spotify asked for a client token challenge, which this spike does not solve")]
    ClientTokenChallenge,
    #[error("the persisted query hash was rejected: the web player has changed")]
    QueryHashRotated,
    #[error("the pathfinder query returned errors")]
    GraphQl,
    #[error("no saved Spotify session")]
    NoSession,
    #[error("the saved session is marked expired: log in again")]
    SessionExpired,
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("Credential Manager: {0}")]
    Store(String),
}

impl SpotifyError {
    /// True when the saved session must be marked expired.
    pub fn expires_session(&self) -> bool {
        matches!(self, Self::Unauthorized { .. } | Self::AnonymousToken)
    }
}
