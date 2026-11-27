//! YouTube Music: the signed-in session, where it is kept, and the one gate its
//! requests pass.

mod account;
mod auth;
mod client;
mod cookies;
// Not wired up yet.
#[allow(dead_code)]
mod gate;
// Not wired up yet.
#[allow(dead_code)]
mod http;
// Not wired up yet.
#[allow(dead_code)]
mod service;
mod session;
mod transport;

pub use account::{Account, AccountStatus, LoginAttempt};
pub use cookies::Cookie;
pub use session::{ORIGIN, PageIdentity, Session, is_signed_in};

/// The Credential Manager name the session is stored under.
pub const CREDENTIAL_SERVICE: &str = "lumen-youtube-music";
