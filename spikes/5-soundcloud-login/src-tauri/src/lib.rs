//! Spike #5: SoundCloud in-app login, `client_id` scrape and one authenticated likes call.

pub mod app;
pub mod error;
pub mod http;
pub mod likes;
pub mod login;
pub mod scrape;
pub mod secret;
pub mod session;
#[cfg(test)]
mod testing;
pub mod throttle;
pub mod transport;
