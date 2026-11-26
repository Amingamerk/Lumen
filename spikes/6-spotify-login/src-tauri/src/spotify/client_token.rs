//! The `client-token` header the web player adds to calls on Spotify hosts.

use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{error::SpotifyError, secret::Secret, token::WEB_PLAYER_CLIENT_ID};

pub const CLIENT_TOKEN_URL: &str = "https://clienttoken.spotify.com/v1/clienttoken";

#[derive(Debug, Clone, PartialEq)]
pub struct ClientToken {
    pub token: Secret,
    /// Seconds after which the web player fetches a new one.
    pub refresh_after_secs: u64,
    /// Host suffixes the token may be sent to.
    pub domains: Vec<String>,
}

impl ClientToken {
    pub fn applies_to(&self, url: &Url) -> bool {
        let Some(host) = url.host_str().filter(|_| url.scheme() == "https") else {
            return false;
        };
        self.domains.iter().any(|domain| {
            host.strip_suffix(domain.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
        })
    }
}

pub fn request_body(client_version: &str, device_id: &str) -> Value {
    // The values the web player derives from a desktop Windows user agent.
    json!({"client_data": {
        "client_version": client_version,
        "client_id": WEB_PLAYER_CLIENT_ID,
        "js_sdk_data": {
            "device_brand": "unknown",
            "device_model": "unknown",
            "os": "windows",
            "os_version": "NT 10.0",
            "device_id": device_id,
            "device_type": "computer"
        }
    }})
}

#[derive(Deserialize)]
struct Granted {
    token: String,
    refresh_after_seconds: u64,
    domains: Vec<Domain>,
}

#[derive(Deserialize)]
struct Domain {
    domain: String,
}

pub fn parse_client_token(body: &str) -> Result<ClientToken, SpotifyError> {
    let unreadable = || SpotifyError::Parse("client token");
    let mut response: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    if response["response_type"] == "RESPONSE_CHALLENGES_RESPONSE" {
        return Err(SpotifyError::ClientTokenChallenge);
    }
    let granted = response
        .get_mut("granted_token")
        .map(Value::take)
        .and_then(|value| Granted::deserialize(value).ok())
        .filter(|granted| !granted.token.is_empty())
        .ok_or_else(unreadable)?;
    Ok(ClientToken {
        token: Secret::new(granted.token),
        refresh_after_secs: granted.refresh_after_seconds,
        domains: granted.domains.into_iter().map(|d| d.domain).collect(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const GRANTED: &str = r#"{"response_type":"RESPONSE_GRANTED_TOKEN_RESPONSE",
        "granted_token":{"token":"FAKE-CLIENT","expires_after_seconds":1209600,
        "refresh_after_seconds":1123200,
        "domains":[{"domain":"spotify.com"},{"domain":"spotify.net"}]}}"#;

    #[test]
    fn request_body_matches_the_web_player() {
        assert_eq!(
            request_body("1.2.3", "00ff"),
            json!({"client_data": {
                "client_version": "1.2.3",
                "client_id": "d8a5ed958d274c2e8ee717e6a4b0971d",
                "js_sdk_data": {
                    "device_brand": "unknown",
                    "device_model": "unknown",
                    "os": "windows",
                    "os_version": "NT 10.0",
                    "device_id": "00ff",
                    "device_type": "computer"
                }
            }})
        );
    }

    #[test]
    fn reads_a_granted_token() {
        let token = parse_client_token(GRANTED).unwrap();
        assert_eq!(token.token.expose(), "FAKE-CLIENT");
        assert_eq!(token.refresh_after_secs, 1_123_200);
        assert_eq!(token.domains, ["spotify.com", "spotify.net"]);
        assert!(!format!("{token:?}").contains("FAKE-CLIENT"));
    }

    #[test]
    fn is_only_sent_to_granted_hosts() {
        let token = parse_client_token(GRANTED).unwrap();
        let applies = |url: &str| token.applies_to(&Url::parse(url).unwrap());
        assert!(applies(
            "https://api-partner.spotify.com/pathfinder/v2/query"
        ));
        assert!(applies(
            "https://spclient.wg.spotify.com/metadata/4/track/00"
        ));
        assert!(applies("https://spotify.com/"));
        assert!(!applies("https://evilspotify.com/"));
        assert!(!applies("https://spotify.com.example.org/"));
    }

    #[test]
    fn a_challenge_fails_closed() {
        let body = r#"{"response_type":"RESPONSE_CHALLENGES_RESPONSE","challenges":{}}"#;
        assert_eq!(
            parse_client_token(body),
            Err(SpotifyError::ClientTokenChallenge)
        );
    }

    #[test]
    fn rejects_an_unreadable_response() {
        for body in ["<html>", "{}", r#"{"granted_token":{"token":""}}"#] {
            assert_eq!(
                parse_client_token(body),
                Err(SpotifyError::Parse("client token")),
                "{body}"
            );
        }
    }
}
