//! Live check without any account: web player page, server time, anonymous
//! token and client token. Prints no token.
//!
//! `cargo run --example anonymous_check -- "<WebView2 user agent>" [language ...]`

use spotify_login_spike::spotify::{
    client::{SpotifyClient, system_clock},
    http::ReqwestTransport,
    session::Browser,
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let user_agent = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("pass the WebView2 user agent as the first argument"))?;
    let browser = Browser::new(&user_agent, &args.collect::<Vec<_>>())?;
    let client = SpotifyClient::new(ReqwestTransport::new()?, system_clock);
    let probe = tauri::async_runtime::block_on(client.anonymous_probe(&browser))?;
    println!("{}", serde_json::to_string_pretty(&probe)?);
    Ok(())
}
