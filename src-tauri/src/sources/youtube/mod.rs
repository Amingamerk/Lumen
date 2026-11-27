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
mod transport;

pub use account::{Account, AccountStatus, LoginAttempt};
pub use cookies::Cookie;
pub use gate::{GateError, SystemClock};
#[cfg(test)]
pub use gate::{GateView, PauseReason};
pub use http::HttpTransport;
pub use search::{Parsed, key as search_key};
#[cfg(test)]
pub use service::Check;
pub use service::{ServiceError, ServiceStatus, YouTube};
pub use session::{ORIGIN, PageIdentity, Session, is_signed_in};

/// The Credential Manager name the session is stored under.
pub const CREDENTIAL_SERVICE: &str = "lumen-youtube-music";
