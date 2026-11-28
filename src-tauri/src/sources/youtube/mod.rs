//! YouTube Music: the signed-in session, where it is kept, and the one gate its
//! requests pass.

mod account;
mod auth;
mod client;
mod cookies;
mod gate;
mod http;
mod search;
mod service;
mod session;
mod source;
mod transport;

pub use account::{Account, AccountStatus, Generation, LoginAttempt};
pub use cookies::Cookie;
pub use gate::{GateClock, GateError, Kind, SystemClock, Verdict};
#[cfg(test)]
pub use gate::{GateView, PauseReason};
pub use http::HttpTransport;
pub use search::{Parsed, key as search_key};
#[cfg(test)]
pub use service::Check;
pub use service::{ServiceError, ServiceStatus, YouTube};
pub use session::{ORIGIN, PageIdentity, Session, is_signed_in};
pub use transport::Transport;

/// The Credential Manager name the session is stored under.
pub const CREDENTIAL_SERVICE: &str = "lumen-youtube-music";

/// The stand-ins other modules' tests build a service from.
#[cfg(test)]
pub(crate) mod testing {
    pub(crate) use super::gate::{FakeClock, TempDir};
    pub use super::session::fake::{cookie, music_page, signed_in_cookies};
    pub(crate) use super::transport::fake::FakeTransport;
}
