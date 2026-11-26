//! One live, signed-out `client_id` scrape. Prints counts and timing, never the id.
//!
//! `cargo run --example live_scrape -- "<WebView2 user agent>"`

use soundcloud_spike::{
    http::Identity,
    scrape::scrape_client_id,
    throttle::{Limits, Throttle},
    transport::ReqwestHttp,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let user_agent = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("pass the WebView2 user agent as the first argument"))?;
    let identity = Identity {
        user_agent,
        language: Some("en-US".to_string()),
    };
    let http = Throttle::new(ReqwestHttp::new()?, Limits::default());
    let (id, report) = scrape_client_id(&http, &identity).await?;
    println!("client_id: {}", id.masked());
    println!("{report:#?}");
    Ok(())
}
