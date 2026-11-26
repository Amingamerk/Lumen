use std::{collections::VecDeque, sync::Mutex as StdMutex, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::time::advance;

use super::*;
use crate::spotify::{
    http::{HttpRequest, HttpResponse, Method, TransportError},
    liked::IsrcSource,
    session::MemoryStore,
};

const NOW: u64 = 1_795_715_000;
const SERVER_NOW: u64 = 1_111_111_109;
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/158.0.0.0";
const COOKIE: &str = "FAKE-SP-DC-cookie";
const ID_1: &str = "4uLU6hMCjMI75M1A2tKUQC";
const ID_2: &str = "0000000000000000000010";

#[derive(Clone)]
struct Seen {
    method: Method,
    url: String,
    headers: Vec<(&'static str, String)>,
    body: Option<Value>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

type Reply = Result<(u16, Option<&'static str>, String), &'static str>;

/// Replays canned responses and records what was sent.
#[derive(Default)]
struct Fake {
    replies: StdMutex<VecDeque<Reply>>,
    seen: StdMutex<Vec<Seen>>,
}

impl Transport for &Fake {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.seen.lock().unwrap().push(Seen {
            method: request.method,
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(n, v)| (*n, v.expose().to_owned()))
                .collect(),
            body: request.body,
        });
        let reply = self.replies.lock().unwrap().pop_front();
        match reply.expect("a request the test did not expect") {
            Ok((status, retry_after, body)) => Ok(HttpResponse {
                status,
                retry_after: retry_after.map(str::to_owned),
                body,
            }),
            Err(detail) => Err(TransportError(detail.to_owned())),
        }
    }
}

impl Fake {
    fn with(replies: impl IntoIterator<Item = Reply>) -> Self {
        let fake = Self::default();
        fake.push(replies);
        fake
    }

    fn push(&self, replies: impl IntoIterator<Item = Reply>) {
        self.replies.lock().unwrap().extend(replies);
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn urls(&self) -> Vec<String> {
        self.seen()
            .iter()
            .map(|s| s.url.split('?').next().unwrap().to_owned())
            .collect()
    }
}

fn ok(body: impl ToString) -> Reply {
    Ok((200, None, body.to_string()))
}

fn status(code: u16) -> Reply {
    Ok((code, None, "FAKE-BODY".to_owned()))
}

fn html() -> Reply {
    let config = STANDARD.encode(r#"{"clientVersion":"1.2.3"}"#);
    ok(format!(
        r#"<script id="appServerConfig" type="text/plain">{config}</script>"#
    ))
}

fn server_time() -> Reply {
    ok(json!({"serverTime": SERVER_NOW}))
}

fn token(anonymous: bool) -> Reply {
    ok(json!({
        "clientId": "d8a5ed958d274c2e8ee717e6a4b0971d",
        "accessToken": "FAKE-ACCESS",
        "accessTokenExpirationTimestampMs": (NOW + 3600) * 1000,
        "isAnonymous": anonymous
    }))
}

fn client_token() -> Reply {
    ok(json!({
        "response_type": "RESPONSE_GRANTED_TOKEN_RESPONSE",
        "granted_token": {"token": "FAKE-CLIENT", "refresh_after_seconds": 1000,
            "domains": [{"domain": "spotify.com"}]}
    }))
}

/// Everything before the first API call.
fn login() -> [Reply; 4] {
    [html(), server_time(), token(false), client_token()]
}

fn pathfinder() -> Reply {
    let items: Vec<Value> = [ID_1, ID_2]
        .iter()
        .map(|id| {
            json!({"track": {"_uri": format!("spotify:track:{id}"), "data": {
                "__typename": "Track", "name": "Song",
                "duration": {"totalMilliseconds": 1000},
                "artists": {"items": [{"profile": {"name": "Artist"}}]}}}})
        })
        .collect();
    ok(json!({"data": {"me": {"library": {"tracks": {"totalCount": 2, "items": items}}}}}))
}

fn metadata(isrc: Option<&str>) -> Reply {
    let ids: Vec<Value> = isrc
        .map(|id| json!({"type": "isrc", "id": id}))
        .into_iter()
        .collect();
    ok(json!({"gid": "00", "external_id": ids}))
}

fn client(fake: &Fake) -> SpotifyClient<&Fake> {
    SpotifyClient::new(fake, || NOW)
}

const LANGUAGES: &str = "da-DK,da;q=0.9";

fn browser() -> Browser {
    Browser {
        user_agent: UA.to_owned(),
        accept_language: LANGUAGES.to_owned(),
    }
}

fn session() -> Session {
    Session {
        sp_dc: Secret::new(COOKIE),
        browser: browser(),
        device_id: "00ff".to_owned(),
        expired: false,
    }
}

fn assert_no_secrets(text: &str) {
    assert!(!text.contains("FAKE"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn follows_the_web_player_order_and_looks_up_missing_isrcs() {
    let fake = Fake::with(login());
    fake.push([pathfinder(), metadata(Some("USUM71703861")), metadata(None)]);
    let report = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();

    assert_eq!(
        fake.urls(),
        [
            "https://open.spotify.com/",
            "https://open.spotify.com/api/server-time",
            "https://open.spotify.com/api/token",
            "https://clienttoken.spotify.com/v1/clienttoken",
            "https://api-partner.spotify.com/pathfinder/v2/query",
            "https://spclient.wg.spotify.com/metadata/4/track/93bc414a606747b2b612491ef83d5a3e",
            "https://spclient.wg.spotify.com/metadata/4/track/0000000000000000000000000000003e",
        ]
    );
    assert_eq!(
        (report.items_returned, report.track_count, report.skipped),
        (2, 2, 0)
    );
    assert_eq!(report.total_liked, Some(2));
    assert_eq!(report.isrc_in_liked_response, 0);
    assert_eq!(report.isrc_from_metadata, 1);
    assert_eq!(report.tracks_with_isrc, 1);
    assert_eq!(
        report.tracks[0].isrc_source,
        Some(IsrcSource::TrackMetadata)
    );
    assert_eq!(report.lookup_error, None);
    assert!(!report.session_expired);
    assert_no_secrets(&serde_json::to_string(&report).unwrap());
}

#[tokio::test(start_paused = true)]
async fn sends_each_credential_only_where_it_belongs() {
    let fake = Fake::with(login());
    fake.push([pathfinder(), metadata(None), metadata(None)]);
    let start = Instant::now();
    client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();
    // Seven requests, at least a second apart.
    assert!(start.elapsed() >= Duration::from_secs(6));

    let seen = fake.seen();
    for (index, request) in seen.iter().enumerate() {
        assert_eq!(request.header("user-agent"), Some(UA), "request {index}");
        assert_eq!(request.header("accept-language"), Some(LANGUAGES));
        let cookie = request.header("cookie");
        if index == 2 {
            assert_eq!(cookie, Some("sp_dc=FAKE-SP-DC-cookie"));
        } else {
            assert_eq!(cookie, None, "request {index}");
        }
        let api = index >= 4;
        assert_eq!(
            request.header("authorization"),
            api.then_some("Bearer FAKE-ACCESS"),
            "request {index}"
        );
        assert_eq!(
            request.header("client-token"),
            api.then_some("FAKE-CLIENT"),
            "request {index}"
        );
        if api {
            assert_eq!(request.header("app-platform"), Some("WebPlayer"));
            assert_eq!(request.header("spotify-app-version"), Some("1.2.3"));
        }
    }
    // totp from the local clock, totpServer from Spotify's.
    let local = crate::spotify::totp::spotify_code(NOW);
    assert!(seen[2].url.contains(&format!("totp={local}")));
    assert!(seen[2].url.contains("totpServer=611107"));
    assert_eq!(seen[3].method, Method::Post);
    assert_eq!(
        seen[3].body.as_ref().unwrap()["client_data"]["js_sdk_data"]["device_id"],
        "00ff"
    );
    assert_eq!(seen[4].method, Method::Post);
    assert_eq!(seen[4].body.as_ref().unwrap()["variables"]["limit"], 10);
}

#[tokio::test(start_paused = true)]
async fn a_second_run_only_repeats_the_page_request() {
    let fake = Fake::with(login());
    fake.push([pathfinder(), metadata(Some("USUM71703861")), metadata(None)]);
    let client = client(&fake);
    let first = client
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();
    fake.push([pathfinder()]);
    let second = client
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(fake.seen().len(), 8);
}

#[tokio::test(start_paused = true)]
async fn another_cookie_or_forget_drops_the_cached_tokens() {
    let fake = Fake::with(login());
    fake.push([ok(json!({"items": []}))]);
    let client = client(&fake);
    client
        .check_liked_songs(&session(), Route::Rest)
        .await
        .unwrap();

    let mut other = session();
    other.sp_dc = Secret::new("FAKE-OTHER-cookie");
    fake.push([server_time(), token(false), ok(json!({"items": []}))]);
    client.check_liked_songs(&other, Route::Rest).await.unwrap();

    client.forget().await;
    fake.push(login());
    fake.push([ok(json!({"items": []}))]);
    client.check_liked_songs(&other, Route::Rest).await.unwrap();
    assert!(fake.replies.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn rest_route_reads_isrc_without_lookups() {
    let fake = Fake::with(login());
    fake.push([ok(json!({"total": 1, "items": [{"track": {
        "id": ID_1, "name": "Song", "duration_ms": 1000, "artists": [],
        "external_ids": {"isrc": "USUM71703861"}}}]}))]);
    let report = client(&fake)
        .check_liked_songs(&session(), Route::Rest)
        .await
        .unwrap();
    assert_eq!(report.route, Route::Rest);
    assert_eq!(report.isrc_in_liked_response, 1);
    assert_eq!(report.tracks_with_isrc, 1);
    let seen = fake.seen();
    assert_eq!(seen.len(), 5);
    assert_eq!(
        seen[4].url,
        "https://api.spotify.com/v1/me/tracks?limit=10&offset=0"
    );
}

#[tokio::test(start_paused = true)]
async fn stops_at_the_first_refusal_of_any_step() {
    let steps = [
        "web player",
        "server-time",
        "token",
        "client token",
        "liked songs",
    ];
    for (failing, endpoint) in steps.into_iter().enumerate() {
        for code in [401, 403] {
            let mut replies: Vec<Reply> = login().into_iter().take(failing).collect();
            replies.push(status(code));
            let fake = Fake::with(replies);
            let error = client(&fake)
                .check_liked_songs(&session(), Route::Pathfinder)
                .await
                .unwrap_err();
            assert_eq!(
                error,
                SpotifyError::Unauthorized {
                    endpoint,
                    status: code
                }
            );
            assert!(error.expires_session());
            assert_eq!(fake.seen().len(), failing + 1, "no retry");
            assert_no_secrets(&error.to_string());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn an_anonymous_token_for_a_cookie_means_the_cookie_is_dead() {
    let fake = Fake::with([html(), server_time(), token(true)]);
    let error = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap_err();
    assert_eq!(error, SpotifyError::AnonymousToken);
    assert!(error.expires_session());
    assert_eq!(fake.seen().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_rejected_totp_is_reported_as_a_rotation() {
    let fake = Fake::with([html(), server_time(), status(400)]);
    let error = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap_err();
    assert_eq!(error, SpotifyError::TotpRejected(61));
    assert!(!error.expires_session());
}

#[tokio::test(start_paused = true)]
async fn server_errors_and_network_errors_are_not_retried() {
    let fake = Fake::with([html(), status(503)]);
    let error = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SpotifyError::Status {
            endpoint: "server-time",
            status: 503
        }
    );
    assert_eq!(fake.seen().len(), 2);
    assert_no_secrets(&error.to_string());

    let fake = Fake::with([Err("connection reset")]);
    let error = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SpotifyError::Network {
            endpoint: "web player",
            detail: "connection reset".to_owned()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_429_closes_every_request_until_retry_after_has_passed() {
    let fake = Fake::with([html(), server_time(), Ok((429, Some("120"), String::new()))]);
    let client = client(&fake);
    let error = client
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SpotifyError::RateLimited {
            endpoint: "token",
            retry_after_secs: 120
        }
    );
    assert!(!error.expires_session());

    // Neither another run nor a new login gets past the cool-down.
    client.forget().await;
    let error = client
        .check_liked_songs(&session(), Route::Rest)
        .await
        .unwrap_err();
    assert!(matches!(error, SpotifyError::CoolingDown { .. }), "{error}");
    assert_eq!(fake.seen().len(), 3);

    advance(Duration::from_secs(120)).await;
    fake.push(login());
    fake.push([ok(json!({"items": []}))]);
    client
        .check_liked_songs(&session(), Route::Rest)
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn lookups_stop_at_the_first_429_and_keep_the_page() {
    let fake = Fake::with(login());
    fake.push([pathfinder(), Ok((429, Some("60"), String::new()))]);
    let report = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();
    assert_eq!(report.track_count, 2);
    assert_eq!(report.tracks_with_isrc, 0);
    assert!(report.lookup_error.unwrap().contains("429"));
    assert!(!report.session_expired);
    assert_eq!(fake.seen().len(), 6);
}

#[tokio::test(start_paused = true)]
async fn lookups_skip_a_missing_track_but_stop_on_anything_else() {
    let fake = Fake::with(login());
    fake.push([pathfinder(), status(404), metadata(Some("USUM71703861"))]);
    let report = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();
    assert_eq!(report.tracks_with_isrc, 1);
    assert_eq!(report.lookup_error, None);

    let fake = Fake::with(login());
    fake.push([pathfinder(), ok("\u{1}protobuf")]);
    let report = client(&fake)
        .check_liked_songs(&session(), Route::Pathfinder)
        .await
        .unwrap();
    assert!(report.lookup_error.is_some());
    assert_eq!(fake.seen().len(), 6);
}

#[tokio::test(start_paused = true)]
async fn run_check_needs_a_saved_session() {
    let fake = Fake::default();
    let store = MemoryStore::default();
    let result = run_check(&client(&fake), &store, Route::Pathfinder).await;
    assert_eq!(result, Err(SpotifyError::NoSession));
}

#[tokio::test(start_paused = true)]
async fn run_check_marks_a_refused_session_expired_and_then_stays_quiet() {
    let fake = Fake::with([html(), server_time(), status(401)]);
    let store = MemoryStore::default();
    store.save(&session()).unwrap();
    let client = client(&fake);

    let first = run_check(&client, &store, Route::Pathfinder).await;
    assert!(matches!(first, Err(SpotifyError::Unauthorized { .. })));
    assert!(store.load().unwrap().unwrap().expired);

    let second = run_check(&client, &store, Route::Pathfinder).await;
    assert_eq!(second, Err(SpotifyError::SessionExpired));
    assert_eq!(fake.seen().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn run_check_marks_expired_when_only_a_lookup_is_refused() {
    let fake = Fake::with(login());
    fake.push([pathfinder(), status(403)]);
    let store = MemoryStore::default();
    store.save(&session()).unwrap();
    let report = run_check(&client(&fake), &store, Route::Pathfinder)
        .await
        .unwrap();
    assert!(report.session_expired);
    assert!(store.load().unwrap().unwrap().expired);
}

#[tokio::test(start_paused = true)]
async fn run_check_leaves_the_session_alone_after_other_failures() {
    let fake = Fake::with([status(503)]);
    let store = MemoryStore::default();
    store.save(&session()).unwrap();
    let result = run_check(&client(&fake), &store, Route::Pathfinder).await;
    assert!(result.is_err());
    assert_eq!(store.load().unwrap(), Some(session()));
}

#[tokio::test(start_paused = true)]
async fn anonymous_probe_sends_no_cookie() {
    let fake = Fake::with([html(), server_time(), token(true), client_token()]);
    let probe = client(&fake).anonymous_probe(&browser()).await.unwrap();
    assert_eq!(
        probe,
        AnonymousProbe {
            is_anonymous: true,
            expires_in_secs: 3600,
            client_id_is_web_player: true,
            totp_status: None,
            client_version: "1.2.3".to_owned(),
            client_token_domains: vec!["spotify.com".to_owned()],
        }
    );
    assert!(fake.seen().iter().all(|s| s.header("cookie").is_none()));
    assert_no_secrets(&serde_json::to_string(&probe).unwrap());
}
