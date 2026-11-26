//! Spike #4: YouTube Music login, one authenticated call, and an authenticated yt-dlp resolve.
//!
//! Everything here is plain Rust behind seams (clock, HTTP transport, credential store, process
//! runner), so the unit tests never touch the network or the real session.

pub mod account;
pub mod auth;
pub mod clock;
pub mod cookies;
pub mod gate;
pub mod http;
pub mod innertube;
pub mod jar;
pub mod resolver;
pub mod session;
pub mod transport;
