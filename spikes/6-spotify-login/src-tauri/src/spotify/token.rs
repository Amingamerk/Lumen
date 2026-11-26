//! `server-time` and `/api/token`, as the web player calls them.

use reqwest::Url;
use serde::Deserialize;

use super::{error::SpotifyError, secret::Secret, totp};

pub const SERVER_TIME_URL: &str = "https://open.spotify.com/api/server-time";
pub const TOKEN_URL: &str = "https://open.spotify.com/api/token";
/// Client id of the web player. Public, and the same for every visitor.
pub const WEB_PLAYER_CLIENT_ID: &str = "d8a5ed958d274c2e8ee717e6a4b0971d";

#[derive(Debug, Clone, PartialEq)]
pub struct AccessToken {
    pub token: Secret,
    /// Unix seconds.
    pub expires_at: u64,
    pub is_anonymous: bool,
    pub client_id: String,
    /// `totpVerExpired` as sent by Spotify: the early warning of a rotation.
    pub totp_status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerTime {
    server_time: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenPayload {
    client_id: String,
    access_token: String,
    access_token_expiration_timestamp_ms: u64,
    is_anonymous: bool,
    totp_ver_expired: Option<String>,
}

pub fn parse_server_time(body: &str) -> Result<u64, SpotifyError> {
    match serde_json::from_str::<ServerTime>(body) {
        Ok(time) if time.server_time > 0 => Ok(time.server_time),
        // The serde error is dropped on purpose: it can quote the body.
        _ => Err(SpotifyError::Parse("server-time")),
    }
}

/// `totp` is computed from the local clock and `totpServer` from Spotify's.
pub fn token_url(local_secs: u64, server_secs: u64) -> Url {
    Url::parse_with_params(
        TOKEN_URL,
        [
            ("reason", "init"),
            ("productType", "web-player"),
            ("totp", &totp::spotify_code(local_secs)),
            ("totpServer", &totp::spotify_code(server_secs)),
            ("totpVer", &totp::VERSION.to_string()),
        ],
    )
    .expect("TOKEN_URL is valid")
}

pub fn parse_token(body: &str) -> Result<AccessToken, SpotifyError> {
    let payload: TokenPayload =
        serde_json::from_str(body).map_err(|_| SpotifyError::Parse("token"))?;
    if payload.access_token.is_empty() {
        return Err(SpotifyError::Parse("token"));
    }
    Ok(AccessToken {
        token: Secret::new(payload.access_token),
        expires_at: payload.access_token_expiration_timestamp_ms / 1000,
        is_anonymous: payload.is_anonymous,
        client_id: payload.client_id,
        totp_status: payload.totp_ver_expired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_server_time() {
        assert_eq!(
            parse_server_time(r#"{"serverTime":1795715854}"#),
            Ok(1_795_715_854)
        );
    }

    #[test]
    fn rejects_unreadable_server_time() {
        for body in [
            "<html>",
            "{}",
            r#"{"serverTime":"soon"}"#,
            r#"{"serverTime":0}"#,
        ] {
            assert_eq!(
                parse_server_time(body),
                Err(SpotifyError::Parse("server-time")),
                "{body}"
            );
        }
    }

    #[test]
    fn token_url_carries_both_codes_and_the_version() {
        let url = token_url(59, 1_111_111_109);
        assert_eq!(url.host_str(), Some("open.spotify.com"));
        assert_eq!(url.path(), "/api/token");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let expected = [
            ("reason", "init"),
            ("productType", "web-player"),
            ("totp", "332823"),
            ("totpServer", "611107"),
            ("totpVer", "61"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        assert_eq!(query, expected);
    }

    #[test]
    fn reads_a_token_payload() {
        let token = parse_token(
            r#"{"clientId":"abc","accessToken":"FAKE-ACCESS",
                "accessTokenExpirationTimestampMs":1795719454321,"isAnonymous":false,
                "_notes":"ignored"}"#,
        )
        .unwrap();
        assert_eq!(token.token.expose(), "FAKE-ACCESS");
        assert_eq!(token.expires_at, 1_795_719_454);
        assert!(!token.is_anonymous);
        assert_eq!(token.client_id, "abc");
        assert_eq!(token.totp_status, None);
        assert!(!format!("{token:?}").contains("FAKE-ACCESS"));
    }

    #[test]
    fn keeps_the_rotation_warning() {
        let token = parse_token(
            r#"{"clientId":"abc","accessToken":"FAKE-ACCESS",
                "accessTokenExpirationTimestampMs":1000,"isAnonymous":true,
                "totpVerExpired":"warning"}"#,
        )
        .unwrap();
        assert!(token.is_anonymous);
        assert_eq!(token.totp_status.as_deref(), Some("warning"));
    }

    #[test]
    fn bad_token_payloads_fail_without_echoing_values() {
        for body in [
            "FAKE-ACCESS",
            r#"{"clientId":"abc","isAnonymous":false,"accessTokenExpirationTimestampMs":1}"#,
            r#"{"clientId":"abc","accessToken":"","isAnonymous":false,"accessTokenExpirationTimestampMs":1}"#,
            r#"{"clientId":"abc","accessToken":"x","isAnonymous":false,"accessTokenExpirationTimestampMs":"FAKE-ACCESS"}"#,
            // Without isAnonymous a rejected cookie would look like a login.
            r#"{"clientId":"abc","accessToken":"x","accessTokenExpirationTimestampMs":1}"#,
        ] {
            let error = parse_token(body).unwrap_err();
            assert_eq!(error, SpotifyError::Parse("token"), "{body}");
            assert!(!format!("{error} {error:?}").contains("FAKE-ACCESS"));
        }
    }
}
