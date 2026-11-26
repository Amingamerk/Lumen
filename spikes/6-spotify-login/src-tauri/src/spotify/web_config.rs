//! The config block that open.spotify.com embeds in its HTML.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

use super::error::SpotifyError;

pub const WEB_PLAYER_URL: &str = "https://open.spotify.com/";

/// `clientVersion`, needed for the client token and `Spotify-App-Version`.
pub fn parse_client_version(html: &str) -> Result<String, SpotifyError> {
    client_version(html).ok_or(SpotifyError::Parse("web player config"))
}

fn client_version(html: &str) -> Option<String> {
    let (_, tag) = html.split_once(r#"id="appServerConfig""#)?;
    let (_, rest) = tag.split_once('>')?;
    let (encoded, _) = rest.split_once("</script>")?;
    let config: Value = serde_json::from_slice(&STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let version = config.get("clientVersion")?.as_str()?;
    // It becomes a header value.
    let usable = !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c));
    usable.then(|| version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(config: &str) -> String {
        format!(
            r#"<html><script id="other" type="text/plain">eA==</script><script id="appServerConfig" type="text/plain">{}</script></html>"#,
            STANDARD.encode(config)
        )
    }

    #[test]
    fn reads_the_client_version() {
        let html = page(r#"{"market":"DK","clientVersion":"1.3.12.2680.gfeb60d224989"}"#);
        assert_eq!(
            parse_client_version(&html).as_deref(),
            Ok("1.3.12.2680.gfeb60d224989")
        );
    }

    #[test]
    fn fails_when_the_block_or_the_version_is_missing() {
        for html in [
            "<html></html>".to_owned(),
            r#"<script id="appServerConfig" type="text/plain">!!</script>"#.to_owned(),
            page("{}"),
            page(r#"{"clientVersion":"bad\nvalue"}"#),
        ] {
            assert_eq!(
                parse_client_version(&html),
                Err(SpotifyError::Parse("web player config"))
            );
        }
    }
}
