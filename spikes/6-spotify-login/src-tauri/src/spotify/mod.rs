//! Spotify web player access: token, liked songs and ISRC. Metadata only.

pub mod client;
pub mod client_token;
pub mod error;
pub mod gate;
pub mod http;
pub mod liked;
pub mod login;
pub mod secret;
pub mod session;
pub mod token;
pub mod totp;
pub mod web_config;
