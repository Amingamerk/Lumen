//! YouTube Music: the signed-in session and where it is kept. Nothing here sends
//! a request.

mod account;
mod cookies;
mod session;

pub use account::{Account, AccountStatus, LoginAttempt};
pub use cookies::Cookie;
pub use session::{ORIGIN, PageIdentity, Session, is_signed_in};

/// The Credential Manager name the session is stored under.
pub const CREDENTIAL_SERVICE: &str = "lumen-youtube-music";
