//! One page of liked songs, from the web player's pathfinder query or from the
//! public REST endpoint, plus the per-track metadata that carries the ISRC.

use reqwest::Url;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::error::SpotifyError;

/// One page only: this spike never paginates.
pub const PAGE_LIMIT: u32 = 10;
pub const PATHFINDER_URL: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
/// Persisted query hash from web-player.4c608093.js (2026-11-26).
pub const FETCH_LIBRARY_TRACKS_HASH: &str =
    "087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240";
const REST_LIKED_URL: &str = "https://api.spotify.com/v1/me/tracks";
const METADATA_URL: &str = "https://spclient.wg.spotify.com/metadata/4/track/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IsrcSource {
    /// Present in the liked songs response itself.
    LikedResponse,
    /// Needed one extra metadata request for the track.
    TrackMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LikedTrack {
    /// Base62 track id.
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub duration_ms: u64,
    pub isrc: Option<String>,
    pub isrc_source: Option<IsrcSource>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LikedPage {
    /// Size of the whole liked songs list, as reported by Spotify.
    pub total: Option<u64>,
    pub tracks: Vec<LikedTrack>,
    /// Items that are not a readable track (removed, restricted or local).
    pub skipped: usize,
    /// Field names (never values) of the first track object, for the findings.
    pub track_fields: Vec<String>,
}

pub fn pathfinder_body() -> Value {
    json!({
        "operationName": "fetchLibraryTracks",
        "variables": {"offset": 0, "limit": PAGE_LIMIT},
        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": FETCH_LIBRARY_TRACKS_HASH}}
    })
}

pub fn rest_url() -> Url {
    Url::parse_with_params(
        REST_LIKED_URL,
        [
            ("limit", PAGE_LIMIT.to_string()),
            ("offset", "0".to_owned()),
        ],
    )
    .expect("REST_LIKED_URL is valid")
}

/// The 32 hex digit form of a base62 track id.
pub fn track_gid(id: &str) -> Option<String> {
    if id.len() != 22 {
        return None;
    }
    let number = id.bytes().try_fold(0u128, |number, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'z' => byte - b'a' + 10,
            b'A'..=b'Z' => byte - b'A' + 36,
            _ => return None,
        };
        number.checked_mul(62)?.checked_add(u128::from(digit))
    })?;
    Some(format!("{number:032x}"))
}

pub fn metadata_url(id: &str) -> Option<Url> {
    let url = format!("{METADATA_URL}{}?market=from_token", track_gid(id)?);
    Url::parse(&url).ok()
}

pub fn parse_rest_page(body: &str) -> Result<LikedPage, SpotifyError> {
    let unreadable = || SpotifyError::Parse("REST liked songs");
    let response: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    let items = response["items"].as_array().ok_or_else(unreadable)?;
    Ok(page(response["total"].as_u64(), items, |item| {
        let track = item["track"].as_object()?;
        if track.get("is_local").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let id = track.get("id")?.as_str()?;
        let liked = liked_track(
            id,
            &item["track"]["name"],
            names(&item["track"]["artists"], "/name"),
            &item["track"]["duration_ms"],
            isrc_of(track),
        )?;
        Some((liked, track))
    }))
}

pub fn parse_pathfinder_page(body: &str) -> Result<LikedPage, SpotifyError> {
    let unreadable = || SpotifyError::Parse("pathfinder liked songs");
    let response: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    if let Some(errors) = response["errors"].as_array().filter(|e| !e.is_empty()) {
        // Server messages can echo the request, so only their kind is kept.
        let rotated = errors
            .iter()
            .any(|e| e["message"] == "PersistedQueryNotFound");
        return Err(if rotated {
            SpotifyError::QueryHashRotated
        } else {
            SpotifyError::GraphQl
        });
    }
    let tracks = &response["data"]["me"]["library"]["tracks"];
    let items = tracks["items"].as_array().ok_or_else(unreadable)?;
    Ok(page(tracks["totalCount"].as_u64(), items, |item| {
        let track = &item["track"];
        let data = track["data"].as_object()?;
        if data.get("__typename")? != "Track" {
            return None;
        }
        let uri = track["_uri"]
            .as_str()
            .or_else(|| track["uri"].as_str())
            .or_else(|| track["data"]["uri"].as_str())?;
        let liked = liked_track(
            uri.strip_prefix("spotify:track:")?,
            &track["data"]["name"],
            names(&track["data"]["artists"]["items"], "/profile/name"),
            &track["data"]["duration"]["totalMilliseconds"],
            isrc_of(data),
        )?;
        Some((liked, data))
    }))
}

pub fn parse_metadata_isrc(body: &str) -> Result<Option<String>, SpotifyError> {
    let unreadable = || SpotifyError::Parse("track metadata");
    let response: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    let track = response
        .as_object()
        .filter(|track| track.contains_key("gid"))
        .ok_or_else(unreadable)?;
    Ok(isrc_of(track))
}

/// Reads at most [`PAGE_LIMIT`] items. `read` returns the track and its raw object.
fn page<'a>(
    total: Option<u64>,
    items: &'a [Value],
    read: impl Fn(&'a Value) -> Option<(LikedTrack, &'a Map<String, Value>)>,
) -> LikedPage {
    let mut result = LikedPage {
        total,
        tracks: Vec::new(),
        skipped: 0,
        track_fields: Vec::new(),
    };
    for item in items.iter().take(PAGE_LIMIT as usize) {
        let Some((track, raw)) = read(item) else {
            result.skipped += 1;
            continue;
        };
        if result.track_fields.is_empty() {
            result.track_fields = raw.keys().cloned().collect();
            result.track_fields.sort();
        }
        result.tracks.push(track);
    }
    result
}

fn liked_track(
    id: &str,
    title: &Value,
    artists: Vec<String>,
    duration_ms: &Value,
    isrc: Option<String>,
) -> Option<LikedTrack> {
    Some(LikedTrack {
        id: id.to_owned(),
        title: title.as_str()?.to_owned(),
        artists,
        duration_ms: duration_ms.as_u64()?,
        isrc_source: isrc.is_some().then_some(IsrcSource::LikedResponse),
        isrc,
    })
}

fn names(artists: &Value, pointer: &str) -> Vec<String> {
    artists
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| artist.pointer(pointer)?.as_str().map(str::to_owned))
        .collect()
}

/// Looks only at the track's own ISRC fields, never at nested albums or alternatives.
fn isrc_of(track: &Map<String, Value>) -> Option<String> {
    track.iter().find_map(
        |(key, value)| match key.replace('_', "").to_ascii_lowercase().as_str() {
            "isrc" => normalize_isrc(value.as_str()?),
            "externalid" | "externalids" => external_isrc(value),
            _ => None,
        },
    )
}

/// `{"isrc": x}`, `{"items": [...]}` or `[{"type": "isrc", "id": x}]`.
fn external_isrc(ids: &Value) -> Option<String> {
    if let Some(list) = ids.as_array() {
        return list.iter().find_map(|entry| {
            if !entry["type"].as_str()?.eq_ignore_ascii_case("isrc") {
                return None;
            }
            normalize_isrc(entry["id"].as_str().or_else(|| entry["value"].as_str())?)
        });
    }
    match ids["isrc"].as_str() {
        Some(isrc) => normalize_isrc(isrc),
        None => ids.get("items").and_then(external_isrc),
    }
}

/// Two letters, three letters or digits, seven digits. Hyphens are dropped.
fn normalize_isrc(raw: &str) -> Option<String> {
    let isrc = raw.replace('-', "").to_ascii_uppercase();
    let bytes = isrc.as_bytes();
    let valid = bytes.len() == 12
        && bytes[..2].iter().all(u8::is_ascii_alphabetic)
        && bytes[2..5].iter().all(u8::is_ascii_alphanumeric)
        && bytes[5..].iter().all(u8::is_ascii_digit);
    valid.then_some(isrc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathfinder_body_asks_for_one_small_page() {
        assert_eq!(
            pathfinder_body(),
            json!({
                "operationName": "fetchLibraryTracks",
                "variables": {"offset": 0, "limit": 10},
                "extensions": {"persistedQuery": {
                    "version": 1,
                    "sha256Hash": FETCH_LIBRARY_TRACKS_HASH
                }}
            })
        );
    }

    #[test]
    fn rest_url_asks_for_one_small_page() {
        assert_eq!(
            rest_url().as_str(),
            "https://api.spotify.com/v1/me/tracks?limit=10&offset=0"
        );
    }

    #[test]
    fn converts_track_ids_to_gids() {
        assert_eq!(
            track_gid("4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("93bc414a606747b2b612491ef83d5a3e")
        );
        assert_eq!(
            track_gid("0000000000000000000010").as_deref(),
            Some("0000000000000000000000000000003e")
        );
        for bad in [
            "",
            "short",
            "4uLU6hMCjMI75M1A2tKUQ/",
            "ZZZZZZZZZZZZZZZZZZZZZZ",
        ] {
            assert_eq!(track_gid(bad), None, "{bad}");
        }
    }

    #[test]
    fn metadata_url_asks_for_the_account_market() {
        assert_eq!(
            metadata_url("4uLU6hMCjMI75M1A2tKUQC").unwrap().as_str(),
            "https://spclient.wg.spotify.com/metadata/4/track/93bc414a606747b2b612491ef83d5a3e?market=from_token"
        );
        assert_eq!(metadata_url("../../x"), None);
    }

    #[test]
    fn reads_isrc_from_a_rest_page() {
        let body = json!({"total": 42, "items": [
            {"track": {"id": "a1", "name": "One", "duration_ms": 1000,
                "artists": [{"name": "A"}, {"name": "B"}],
                "external_ids": {"isrc": "usum71703861"}}},
            {"track": {"id": "a2", "name": "Two", "duration_ms": 2000,
                "artists": [{"name": "C"}], "external_ids": {}}},
            {"track": {"id": "a3", "name": "Three", "duration_ms": 3000,
                "artists": [], "external_ids": {"isrc": "not an isrc"}}},
            {"track": null},
            {"track": {"id": null, "name": "Local file", "is_local": true}}
        ]})
        .to_string();
        let page = parse_rest_page(&body).unwrap();
        assert_eq!(page.total, Some(42));
        assert_eq!(page.skipped, 2);
        assert_eq!(page.tracks.len(), 3);
        assert_eq!(
            page.tracks[0],
            LikedTrack {
                id: "a1".into(),
                title: "One".into(),
                artists: vec!["A".into(), "B".into()],
                duration_ms: 1000,
                isrc: Some("USUM71703861".into()),
                isrc_source: Some(IsrcSource::LikedResponse),
            }
        );
        assert_eq!(page.tracks[1].isrc, None);
        assert_eq!(page.tracks[1].isrc_source, None);
        assert_eq!(page.tracks[2].isrc, None);
        assert_eq!(
            page.track_fields,
            ["artists", "duration_ms", "external_ids", "id", "name"]
        );
    }

    fn pathfinder_item(id: &str, extra: Value) -> Value {
        let mut data = json!({"__typename": "Track", "name": "One",
            "duration": {"totalMilliseconds": 1000},
            "artists": {"items": [{"profile": {"name": "A"}}]}});
        data.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"addedAt": {"isoString": "2026-01-01T00:00:00Z"},
            "track": {"_uri": format!("spotify:track:{id}"), "data": data}})
    }

    fn pathfinder_page(items: Vec<Value>) -> String {
        json!({"data": {"me": {"library": {"tracks": {
            "__typename": "UserLibraryTrackPage",
            "totalCount": 7,
            "pagingInfo": {"offset": 0, "limit": 10},
            "items": items
        }}}}})
        .to_string()
    }

    #[test]
    fn reads_a_pathfinder_page_without_isrc() {
        let body = pathfinder_page(vec![
            pathfinder_item("a1", json!({})),
            json!({"track": {"_uri": "spotify:track:gone", "data": {"__typename": "NotFound"}}}),
        ]);
        let page = parse_pathfinder_page(&body).unwrap();
        assert_eq!(page.total, Some(7));
        assert_eq!(page.skipped, 1);
        assert_eq!(
            page.tracks,
            [LikedTrack {
                id: "a1".into(),
                title: "One".into(),
                artists: vec!["A".into()],
                duration_ms: 1000,
                isrc: None,
                isrc_source: None,
            }]
        );
        assert_eq!(
            page.track_fields,
            ["__typename", "artists", "duration", "name"]
        );
    }

    #[test]
    fn finds_isrc_in_a_pathfinder_track_in_any_known_shape() {
        for extra in [
            json!({"isrc": "USUM71703861"}),
            json!({"externalIds": {"isrc": "USUM71703861"}}),
            json!({"externalIds": {"items": [
                {"type": "upc", "id": "1"},
                {"type": "isrc", "id": "US-UM7-17-03861"}
            ]}}),
        ] {
            let body = pathfinder_page(vec![pathfinder_item("a1", extra.clone())]);
            let track = &parse_pathfinder_page(&body).unwrap().tracks[0];
            assert_eq!(track.isrc.as_deref(), Some("USUM71703861"), "{extra}");
            assert_eq!(track.isrc_source, Some(IsrcSource::LikedResponse));
        }
    }

    #[test]
    fn an_album_isrc_lookalike_is_not_a_track_isrc() {
        let extra = json!({"albumOfTrack": {"isrc": "USUM71703861"}});
        let body = pathfinder_page(vec![pathfinder_item("a1", extra)]);
        assert_eq!(parse_pathfinder_page(&body).unwrap().tracks[0].isrc, None);
    }

    #[test]
    fn reports_a_rotated_query_hash() {
        let body = r#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#;
        assert_eq!(
            parse_pathfinder_page(body),
            Err(SpotifyError::QueryHashRotated)
        );
        let other = r#"{"errors":[{"message":"boom"}],"data":null}"#;
        assert_eq!(parse_pathfinder_page(other), Err(SpotifyError::GraphQl));
    }

    #[test]
    fn rejects_unexpected_shapes() {
        for body in ["<html>", "{}", r#"{"data":{"me":null}}"#] {
            assert_eq!(
                parse_pathfinder_page(body),
                Err(SpotifyError::Parse("pathfinder liked songs")),
                "{body}"
            );
            assert_eq!(
                parse_rest_page(body),
                Err(SpotifyError::Parse("REST liked songs")),
                "{body}"
            );
        }
    }

    #[test]
    fn reads_isrc_from_track_metadata() {
        let body = r#"{"gid":"00","name":"One","external_id":[
            {"type":"upc","id":"1"},{"type":"isrc","id":"USUM71703861"}],
            "alternative":[{"external_id":[{"type":"isrc","id":"GBAYE0601498"}]}]}"#;
        assert_eq!(
            parse_metadata_isrc(body),
            Ok(Some("USUM71703861".to_owned()))
        );
        assert_eq!(parse_metadata_isrc(r#"{"gid":"00"}"#), Ok(None));
        for body in ["<html>", "[]", "{}"] {
            assert_eq!(
                parse_metadata_isrc(body),
                Err(SpotifyError::Parse("track metadata")),
                "{body}"
            );
        }
    }
}
