//! The one authenticated call: YouTube Music's own `browse` endpoint for the library playlists,
//! with the headers its web client sends.

use serde::Serialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::{
    auth::{ORIGIN, sapisid, sapisidhash},
    cookies::{header_value, matching},
    session::Session,
    transport::{Request, Transport, TransportError},
};

const HOST: &str = "music.youtube.com";
const PATH: &str = "/youtubei/v1/browse";
const BROWSE_ID: &str = "FEmusic_liked_playlists";

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum BrowseError {
    #[error("the session has no SAPISID cookie that applies to music.youtube.com")]
    NotSignedIn,
    #[error("YouTube Music rejected the session (HTTP {0}): log in again")]
    Unauthorized(u16),
    #[error("YouTube Music answered as signed out (logged_in=0): log in again")]
    SignedOut,
    #[error("rate limited (HTTP 429)")]
    RateLimited { retry_after: Option<String> },
    #[error("bot check or unusual-traffic page (HTTP {0})")]
    Blocked(u16),
    #[error("unexpected HTTP status {0}")]
    Status(u16),
    #[error("the response did not contain a playlist grid")]
    UnexpectedResponse,
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    /// Absent for automatic playlists such as Liked Music.
    pub track_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaylistPage {
    pub playlists: Vec<Playlist>,
    /// More pages exist. The spike fetches only the first.
    pub has_more: bool,
}

pub fn liked_playlists_request(session: &Session, now: u64) -> Result<Request, BrowseError> {
    let cookies = matching(&session.cookies, HOST, PATH, now);
    let sapisid = sapisid(&cookies).ok_or(BrowseError::NotSignedIn)?;
    let headers = [
        ("Authorization", sapisidhash(sapisid, now, ORIGIN)),
        ("Cookie", header_value(&cookies)),
        ("User-Agent", session.user_agent.clone()),
        ("Accept", "*/*".to_owned()),
        ("Accept-Language", "en-US,en;q=0.9".to_owned()),
        ("Content-Type", "application/json".to_owned()),
        ("Origin", ORIGIN.to_owned()),
        ("X-Origin", ORIGIN.to_owned()),
        ("Referer", format!("{ORIGIN}/")),
        // The login window holds a single account.
        ("X-Goog-AuthUser", "0".to_owned()),
    ];
    let body = json!({
        "context": {
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": client_version(now),
                "hl": "en",
            },
            "user": {},
        },
        "browseId": BROWSE_ID,
    });
    Ok(Request {
        url: format!("{ORIGIN}{PATH}?prettyPrint=false"),
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
        body: body.to_string(),
    })
}

/// The web client versions itself by build date, as ytmusicapi mimics it.
fn client_version(now: u64) -> String {
    let date = i64::try_from(now)
        .ok()
        .and_then(|secs| OffsetDateTime::from_unix_timestamp(secs).ok())
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .date();
    format!(
        "1.{:04}{:02}{:02}.01.00",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

pub fn parse_liked_playlists(body: &str) -> Result<PlaylistPage, BrowseError> {
    let json: Value = serde_json::from_str(body).map_err(|_| BrowseError::UnexpectedResponse)?;
    if signed_out(&json) {
        return Err(BrowseError::SignedOut);
    }
    let grid = find_key(&json, "gridRenderer").ok_or(BrowseError::UnexpectedResponse)?;
    let items = grid["items"]
        .as_array()
        .ok_or(BrowseError::UnexpectedResponse)?;
    Ok(PlaylistPage {
        playlists: items.iter().filter_map(playlist).collect(),
        has_more: grid.get("continuations").is_some()
            || items
                .iter()
                .any(|item| item.get("continuationItemRenderer").is_some()),
    })
}

fn signed_out(json: &Value) -> bool {
    let services = json["responseContext"]["serviceTrackingParams"].as_array();
    services
        .into_iter()
        .flatten()
        .filter_map(|service| service["params"].as_array())
        .flatten()
        .any(|param| param["key"] == "logged_in" && param["value"] == "0")
}

fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|child| find_key(child, key))),
        Value::Array(list) => list.iter().find_map(|child| find_key(child, key)),
        _ => None,
    }
}

/// Playlists have a `VL<id>` browse id. The "New playlist" tile and albums do not.
fn playlist(item: &Value) -> Option<Playlist> {
    let renderer = &item["musicTwoRowItemRenderer"];
    let title = renderer["title"]["runs"].as_array()?;
    let id = title
        .first()?
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .or_else(|| renderer.pointer("/navigationEndpoint/browseEndpoint/browseId"))?
        .as_str()?
        .strip_prefix("VL")?;
    Some(Playlist {
        id: id.to_owned(),
        title: title
            .iter()
            .filter_map(|run| run["text"].as_str())
            .collect(),
        track_count: renderer["subtitle"]["runs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|run| run["text"].as_str())
            .find_map(track_count),
    })
}

/// Reads "1,204 songs" (the request pins `hl=en`).
fn track_count(text: &str) -> Option<u32> {
    let (number, unit) = text.trim().split_once(' ')?;
    if !matches!(unit, "song" | "songs" | "track" | "tracks") {
        return None;
    }
    number.replace(',', "").parse().ok()
}

/// Sends exactly one request. It never retries.
pub async fn liked_playlists(
    transport: &impl Transport,
    session: &Session,
    now: u64,
) -> Result<PlaylistPage, BrowseError> {
    let response = transport
        .post(liked_playlists_request(session, now)?)
        .await?;
    if response.status == 200 {
        return parse_liked_playlists(&response.body);
    }
    // Checked first: a bot check must pause everything, whatever status it came with.
    let body = response.body.to_ascii_lowercase();
    if ["unusual traffic", "captcha", "/sorry/"]
        .iter()
        .any(|marker| body.contains(marker))
    {
        return Err(BrowseError::Blocked(response.status));
    }
    Err(match response.status {
        401 | 403 => BrowseError::Unauthorized(response.status),
        429 => BrowseError::RateLimited {
            retry_after: response.retry_after,
        },
        status => BrowseError::Status(status),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        cookies::Cookie,
        session::fake::{cookie, session},
        transport::{
            Response,
            fake::{FakeTransport, response},
        },
    };

    const NOW: u64 = 1_700_000_000;

    fn signed_in_session() -> Session {
        let mut signed_in = session();
        signed_in.cookies = vec![
            cookie("SAPISID", "fakeSapisid/AbC123"),
            cookie("LOGIN_INFO", "fake-login-info"),
            Cookie {
                domain: "www.youtube.com".into(),
                ..cookie("WWW_ONLY", "fake-www")
            },
        ];
        signed_in
    }

    fn page(logged_in: &str, items: Vec<Value>, continuations: bool) -> String {
        let mut grid = json!({ "items": items });
        if continuations {
            grid["continuations"] = json!([{ "nextContinuationData": {} }]);
        }
        json!({
            "responseContext": { "serviceTrackingParams": [
                { "service": "GFEEDBACK", "params": [{ "key": "logged_in", "value": logged_in }] }
            ]},
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [ { "tabRenderer": {
                "content": { "sectionListRenderer": { "contents": [ { "gridRenderer": grid } ] } }
            } } ] } }
        })
        .to_string()
    }

    fn item(title: &str, browse_id: &str, subtitle: &[&str]) -> Value {
        let runs: Vec<Value> = subtitle.iter().map(|t| json!({ "text": t })).collect();
        json!({ "musicTwoRowItemRenderer": {
            "title": { "runs": [ { "text": title, "navigationEndpoint": {
                "browseEndpoint": { "browseId": browse_id } } } ] },
            "subtitle": { "runs": runs }
        } })
    }

    fn library() -> String {
        let new_playlist = json!({ "musicTwoRowItemRenderer": {
            "title": { "runs": [ { "text": "New playlist" } ] },
            "navigationEndpoint": { "createPlaylistEndpoint": {} }
        } });
        page(
            "1",
            vec![
                new_playlist,
                item("Liked Music", "VLLM", &["Auto playlist"]),
                item(
                    "Road trip",
                    "VLPLfake1",
                    &[
                        "Playlist",
                        " \u{2022} ",
                        "Someone",
                        " \u{2022} ",
                        "1,204 songs",
                    ],
                ),
                item("Solo", "VLPLfake2", &["1 song"]),
                item("An album", "MPREb_fake", &["Album"]),
            ],
            false,
        )
    }

    #[test]
    fn sends_the_web_client_headers_and_the_hashed_authorization() {
        let request = liked_playlists_request(&signed_in_session(), NOW).unwrap();

        assert_eq!(
            request.url,
            "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false"
        );
        let expected = [
            (
                "Authorization",
                "SAPISIDHASH 1700000000_fa90dc53295b9f3251e626355147af2d028548fd",
            ),
            (
                "Cookie",
                "SAPISID=fakeSapisid/AbC123; LOGIN_INFO=fake-login-info",
            ),
            ("User-Agent", "FakeAgent/1.0"),
            ("Origin", "https://music.youtube.com"),
            ("X-Origin", "https://music.youtube.com"),
            ("Referer", "https://music.youtube.com/"),
            ("Content-Type", "application/json"),
            ("Accept-Language", "en-US,en;q=0.9"),
            ("X-Goog-AuthUser", "0"),
        ];
        for (name, value) in expected {
            assert_eq!(request.header(name), Some(value), "{name}");
        }
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({
                "browseId": "FEmusic_liked_playlists",
                "context": {
                    "client": {
                        "clientName": "WEB_REMIX",
                        // 1700000000 is 2023-11-14 UTC.
                        "clientVersion": "1.20231114.01.00",
                        "hl": "en"
                    },
                    "user": {}
                }
            })
        );
    }

    #[test]
    fn a_request_never_prints_its_credentials() {
        let request = liked_playlists_request(&signed_in_session(), NOW).unwrap();

        let printed = format!("{request:?}");

        assert!(!printed.contains("fake"), "{printed}");
        assert!(!printed.contains("fa90dc53"), "{printed}");
    }

    #[test]
    fn refuses_to_build_a_request_without_an_applicable_sapisid() {
        let mut no_sapisid = session();
        no_sapisid.cookies = vec![cookie("PREF", "x")];

        assert_eq!(
            liked_playlists_request(&no_sapisid, NOW),
            Err(BrowseError::NotSignedIn)
        );
    }

    #[test]
    fn lists_playlists_with_titles_and_counts() {
        let parsed = parse_liked_playlists(&library()).unwrap();

        let playlist = |id: &str, title: &str, track_count| Playlist {
            id: id.into(),
            title: title.into(),
            track_count,
        };
        assert_eq!(
            parsed,
            PlaylistPage {
                playlists: vec![
                    playlist("LM", "Liked Music", None),
                    playlist("PLfake1", "Road trip", Some(1_204)),
                    playlist("PLfake2", "Solo", Some(1)),
                ],
                has_more: false,
            }
        );
    }

    #[test]
    fn notes_that_more_pages_exist() {
        let parsed = parse_liked_playlists(&page("1", vec![], true)).unwrap();

        assert!(parsed.has_more);
        assert!(parsed.playlists.is_empty());
    }

    #[test]
    fn treats_a_logged_out_answer_as_signed_out() {
        assert_eq!(
            parse_liked_playlists(&page("0", vec![], false)),
            Err(BrowseError::SignedOut)
        );
    }

    #[test]
    fn does_not_report_an_unknown_layout_as_an_empty_library() {
        for body in ["<html></html>", "{}", r#"{"contents": {}}"#] {
            assert_eq!(
                parse_liked_playlists(body),
                Err(BrowseError::UnexpectedResponse),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn makes_exactly_one_call() {
        let transport = FakeTransport::answering([response(200, &library())]);

        let parsed = liked_playlists(&transport, &signed_in_session(), NOW)
            .await
            .unwrap();

        assert_eq!(parsed.playlists.len(), 3);
        assert_eq!(transport.sent(), 1);
    }

    #[tokio::test]
    async fn classifies_error_statuses_without_retrying() {
        let limited = Response {
            retry_after: Some("90".into()),
            ..response(429, "")
        };
        let cases = [
            (response(401, ""), BrowseError::Unauthorized(401)),
            (response(403, ""), BrowseError::Unauthorized(403)),
            (
                limited,
                BrowseError::RateLimited {
                    retry_after: Some("90".into()),
                },
            ),
            (
                response(429, "Our systems have detected unusual traffic"),
                BrowseError::Blocked(429),
            ),
            (
                response(302, "https://www.google.com/sorry/index"),
                BrowseError::Blocked(302),
            ),
            (
                response(403, "please solve the CAPTCHA"),
                BrowseError::Blocked(403),
            ),
            (response(500, ""), BrowseError::Status(500)),
        ];
        for (answer, expected) in cases {
            let transport = FakeTransport::answering([answer]);

            let outcome = liked_playlists(&transport, &signed_in_session(), NOW).await;

            assert_eq!(outcome, Err(expected));
            assert_eq!(transport.sent(), 1);
        }
    }
}
