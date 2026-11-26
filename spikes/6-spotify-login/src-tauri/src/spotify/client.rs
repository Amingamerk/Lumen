//! The check itself: one paced pass over the web player's endpoints, stopping at
//! the first refusal. Nothing here retries.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{sync::Mutex, time::Instant};

use super::{
    client_token::{self, CLIENT_TOKEN_URL, ClientToken},
    error::SpotifyError,
    gate::Gate,
    http::{HeaderValue, HttpRequest, Method, Transport},
    liked::{self, IsrcSource, LikedPage, LikedTrack, PATHFINDER_URL},
    secret::Secret,
    session::{Browser, Session, SessionStore, random_device_id},
    token::{self, AccessToken, SERVER_TIME_URL, WEB_PLAYER_CLIENT_ID},
    totp,
    web_config::{self, WEB_PLAYER_URL},
};

const ORIGIN: &str = "https://open.spotify.com";
const APP_PLATFORM: &str = "WebPlayer";
const JSON: &str = "application/json";
/// A cached access token is replaced this long before it expires.
const EXPIRY_MARGIN_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// What the web player itself does.
    Pathfinder,
    /// The public `/v1/me/tracks`, for comparison.
    Rest,
}

/// Safe to show and log: it holds no cookie or token.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub route: Route,
    pub items_returned: usize,
    pub track_count: usize,
    pub skipped: usize,
    pub total_liked: Option<u64>,
    pub isrc_in_liked_response: usize,
    pub isrc_from_metadata: usize,
    pub tracks_with_isrc: usize,
    pub track_fields: Vec<String>,
    pub tracks: Vec<LikedTrack>,
    /// Why the per-track ISRC lookups stopped early.
    pub lookup_error: Option<String>,
    pub session_expired: bool,
    pub totp_status: Option<String>,
}

/// Result of the token flow without a cookie.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnonymousProbe {
    pub is_anonymous: bool,
    pub expires_in_secs: u64,
    pub client_id_is_web_player: bool,
    pub totp_status: Option<String>,
    pub client_version: String,
    pub client_token_domains: Vec<String>,
}

pub struct SpotifyClient<T> {
    transport: T,
    /// Unix seconds, for the TOTP and token expiry.
    clock: fn() -> u64,
    /// Held for a whole run, so runs never overlap.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    gate: Gate,
    /// The access token and the cookie it came from.
    access: Option<(Secret, AccessToken)>,
    client_version: Option<String>,
    client_token: Option<CachedClientToken>,
    /// ISRC lookups by track id.
    isrc: HashMap<String, Option<String>>,
}

#[derive(Clone)]
struct CachedClientToken {
    token: ClientToken,
    device_id: String,
    refresh_at: Instant,
}

/// What every call after the login needs.
struct Credentials {
    browser: Browser,
    client_version: String,
    access: AccessToken,
    client_token: ClientToken,
}

impl Credentials {
    fn request(&self, method: Method, url: Url, body: Option<Value>) -> HttpRequest {
        let mut headers = web_headers(&self.browser, JSON);
        let bearer = format!("Bearer {}", self.access.token.expose());
        headers.push(("authorization", HeaderValue::Secret(Secret::new(bearer))));
        if self.client_token.applies_to(&url) {
            headers.push((
                "client-token",
                HeaderValue::Secret(self.client_token.token.clone()),
            ));
        }
        headers.extend(app_headers(&self.client_version));
        if body.is_some() {
            headers.push(("content-type", plain("application/json;charset=UTF-8")));
        }
        HttpRequest {
            method,
            url,
            headers,
            body,
        }
    }
}

fn plain(value: &str) -> HeaderValue {
    HeaderValue::Plain(value.to_owned())
}

/// The headers a script on open.spotify.com sends.
fn web_headers(browser: &Browser, accept: &str) -> Vec<(&'static str, HeaderValue)> {
    vec![
        ("user-agent", plain(&browser.user_agent)),
        ("accept", plain(accept)),
        ("accept-language", plain(&browser.accept_language)),
        ("origin", plain(ORIGIN)),
        ("referer", plain(WEB_PLAYER_URL)),
    ]
}

fn app_headers(client_version: &str) -> [(&'static str, HeaderValue); 2] {
    [
        ("app-platform", plain(APP_PLATFORM)),
        ("spotify-app-version", plain(client_version)),
    ]
}

fn url(address: &str) -> Url {
    Url::parse(address).expect("a constant URL is valid")
}

impl<T: Transport> SpotifyClient<T> {
    pub fn new(transport: T, clock: fn() -> u64) -> Self {
        Self {
            transport,
            clock,
            state: Mutex::default(),
        }
    }

    /// Reads one page of liked songs and finds each track's ISRC.
    pub async fn check_liked_songs(
        &self,
        session: &Session,
        route: Route,
    ) -> Result<Report, SpotifyError> {
        let mut state = self.state.lock().await;
        let credentials = self.log_in(&mut state, session).await?;
        let (method, address, body) = match route {
            Route::Pathfinder => (
                Method::Post,
                url(PATHFINDER_URL),
                Some(liked::pathfinder_body()),
            ),
            Route::Rest => (Method::Get, liked::rest_url(), None),
        };
        let request = credentials.request(method, address, body);
        let response = self.send(&mut state.gate, "liked songs", request).await?;
        let mut page = match route {
            Route::Pathfinder => liked::parse_pathfinder_page(&response)?,
            Route::Rest => liked::parse_rest_page(&response)?,
        };
        // The REST route is only here to show what its own response carries.
        let refusal = match route {
            Route::Pathfinder => self
                .look_up_isrcs(&mut state, &credentials, &mut page)
                .await
                .err(),
            Route::Rest => None,
        };
        Ok(report(route, page, refusal, credentials.access.totp_status))
    }

    /// Runs the token flow without a cookie, to see that the TOTP secret,
    /// version and endpoints are current.
    pub async fn anonymous_probe(&self, browser: &Browser) -> Result<AnonymousProbe, SpotifyError> {
        let mut state = self.state.lock().await;
        let gate = &mut state.gate;
        let client_version = self.fetch_client_version(gate, browser).await?;
        let access = self
            .fetch_access_token(gate, browser, &client_version, None)
            .await?;
        let client_token = self
            .fetch_client_token(gate, browser, &client_version, &random_device_id()?)
            .await?;
        Ok(AnonymousProbe {
            is_anonymous: access.is_anonymous,
            expires_in_secs: access.expires_at.saturating_sub((self.clock)()),
            client_id_is_web_player: access.client_id == WEB_PLAYER_CLIENT_ID,
            totp_status: access.totp_status,
            client_version,
            client_token_domains: client_token.domains,
        })
    }

    /// Drops everything tied to a login. A running cool-down stays.
    pub async fn forget(&self) {
        let mut state = self.state.lock().await;
        *state = State {
            gate: std::mem::take(&mut state.gate),
            ..State::default()
        };
    }

    /// Client version, access token and client token, in the order a browser
    /// asks for them. Each is only requested when the cached one is unusable.
    async fn log_in(
        &self,
        state: &mut State,
        session: &Session,
    ) -> Result<Credentials, SpotifyError> {
        let browser = &session.browser;
        let client_version = match &state.client_version {
            Some(version) => version.clone(),
            None => {
                let version = self.fetch_client_version(&mut state.gate, browser).await?;
                state.client_version.insert(version).clone()
            }
        };

        let usable_after = (self.clock)() + EXPIRY_MARGIN_SECS;
        let access = match &state.access {
            Some((cookie, access))
                if *cookie == session.sp_dc && access.expires_at > usable_after =>
            {
                access.clone()
            }
            _ => {
                let access = self
                    .fetch_access_token(
                        &mut state.gate,
                        browser,
                        &client_version,
                        Some(&session.sp_dc),
                    )
                    .await?;
                // A cookie Spotify does not accept still gets a token: an anonymous one.
                if access.is_anonymous {
                    return Err(SpotifyError::AnonymousToken);
                }
                state.access = Some((session.sp_dc.clone(), access.clone()));
                access
            }
        };

        let client_token = match &state.client_token {
            Some(cached)
                if cached.device_id == session.device_id && cached.refresh_at > Instant::now() =>
            {
                cached.token.clone()
            }
            _ => {
                let token = self
                    .fetch_client_token(
                        &mut state.gate,
                        browser,
                        &client_version,
                        &session.device_id,
                    )
                    .await?;
                let refresh_in = Duration::from_secs(token.refresh_after_secs.min(30 * 86_400));
                state.client_token = Some(CachedClientToken {
                    token: token.clone(),
                    device_id: session.device_id.clone(),
                    refresh_at: Instant::now() + refresh_in,
                });
                token
            }
        };

        Ok(Credentials {
            browser: browser.clone(),
            client_version,
            access,
            client_token,
        })
    }

    async fn fetch_client_version(
        &self,
        gate: &mut Gate,
        browser: &Browser,
    ) -> Result<String, SpotifyError> {
        // A page load, so no origin or referer.
        let request = HttpRequest {
            method: Method::Get,
            url: url(WEB_PLAYER_URL),
            headers: vec![
                ("user-agent", plain(&browser.user_agent)),
                ("accept", plain("text/html")),
                ("accept-language", plain(&browser.accept_language)),
            ],
            body: None,
        };
        let page = self.send(gate, "web player", request).await?;
        web_config::parse_client_version(&page)
    }

    async fn fetch_access_token(
        &self,
        gate: &mut Gate,
        browser: &Browser,
        client_version: &str,
        sp_dc: Option<&Secret>,
    ) -> Result<AccessToken, SpotifyError> {
        let request = HttpRequest {
            method: Method::Get,
            url: url(SERVER_TIME_URL),
            headers: web_headers(browser, JSON),
            body: None,
        };
        let server_time = self.send(gate, "server-time", request).await?;
        let server_secs = token::parse_server_time(&server_time)?;

        let mut headers = web_headers(browser, JSON);
        headers.extend(app_headers(client_version));
        if let Some(sp_dc) = sp_dc {
            let cookie = format!("sp_dc={}", sp_dc.expose());
            headers.push(("cookie", HeaderValue::Secret(Secret::new(cookie))));
        }
        let request = HttpRequest {
            method: Method::Get,
            url: token::token_url((self.clock)(), server_secs),
            headers,
            body: None,
        };
        match self.send(gate, "token", request).await {
            Ok(body) => token::parse_token(&body),
            Err(SpotifyError::Status { status: 400, .. }) => {
                Err(SpotifyError::TotpRejected(totp::VERSION))
            }
            Err(error) => Err(error),
        }
    }

    async fn fetch_client_token(
        &self,
        gate: &mut Gate,
        browser: &Browser,
        client_version: &str,
        device_id: &str,
    ) -> Result<ClientToken, SpotifyError> {
        let mut headers = web_headers(browser, JSON);
        headers.push(("content-type", plain(JSON)));
        let request = HttpRequest {
            method: Method::Post,
            url: url(CLIENT_TOKEN_URL),
            headers,
            body: Some(client_token::request_body(client_version, device_id)),
        };
        let body = self.send(gate, "client token", request).await?;
        client_token::parse_client_token(&body)
    }

    /// Fills in the ISRC of tracks that came without one, one paced request per
    /// track. Stops at the first failure.
    async fn look_up_isrcs(
        &self,
        state: &mut State,
        credentials: &Credentials,
        page: &mut LikedPage,
    ) -> Result<(), SpotifyError> {
        for track in page.tracks.iter_mut().filter(|t| t.isrc.is_none()) {
            let found = match state.isrc.get(&track.id) {
                Some(cached) => cached.clone(),
                None => {
                    let Some(address) = liked::metadata_url(&track.id) else {
                        continue;
                    };
                    let request = credentials.request(Method::Get, address, None);
                    let found = match self.send(&mut state.gate, "track metadata", request).await {
                        Ok(body) => liked::parse_metadata_isrc(&body)?,
                        Err(SpotifyError::Status { status: 404, .. }) => None,
                        Err(error) => return Err(error),
                    };
                    state.isrc.insert(track.id.clone(), found.clone());
                    found
                }
            };
            track.isrc_source = found.is_some().then_some(IsrcSource::TrackMetadata);
            track.isrc = found;
        }
        Ok(())
    }

    /// The only way out to the network: paced, and never retried.
    async fn send(
        &self,
        gate: &mut Gate,
        endpoint: &'static str,
        request: HttpRequest,
    ) -> Result<String, SpotifyError> {
        gate.wait_turn().await?;
        let response =
            self.transport
                .send(request)
                .await
                .map_err(|error| SpotifyError::Network {
                    endpoint,
                    detail: error.0,
                })?;
        match response.status {
            200..=299 => {
                gate.succeeded();
                Ok(response.body)
            }
            status @ (401 | 403) => Err(SpotifyError::Unauthorized { endpoint, status }),
            429 => Err(SpotifyError::RateLimited {
                endpoint,
                retry_after_secs: gate.rate_limited(response.retry_after.as_deref()),
            }),
            status => Err(SpotifyError::Status { endpoint, status }),
        }
    }
}

fn report(
    route: Route,
    page: LikedPage,
    refusal: Option<SpotifyError>,
    totp_status: Option<String>,
) -> Report {
    let from = |source| {
        page.tracks
            .iter()
            .filter(|track| track.isrc_source == Some(source))
            .count()
    };
    Report {
        route,
        items_returned: page.tracks.len() + page.skipped,
        track_count: page.tracks.len(),
        skipped: page.skipped,
        total_liked: page.total,
        isrc_in_liked_response: from(IsrcSource::LikedResponse),
        isrc_from_metadata: from(IsrcSource::TrackMetadata),
        tracks_with_isrc: page.tracks.iter().filter(|t| t.isrc.is_some()).count(),
        track_fields: page.track_fields,
        lookup_error: refusal.as_ref().map(ToString::to_string),
        session_expired: refusal.is_some_and(|error| error.expires_session()),
        totp_status,
        tracks: page.tracks,
    }
}

/// The clock the app runs on, in Unix seconds.
pub fn system_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Checks with the saved session, and marks it expired when Spotify refuses it.
pub async fn run_check<T: Transport>(
    client: &SpotifyClient<T>,
    store: &dyn SessionStore,
    route: Route,
) -> Result<Report, SpotifyError> {
    let session = store.load()?.ok_or(SpotifyError::NoSession)?;
    if session.expired {
        return Err(SpotifyError::SessionExpired);
    }
    let result = client.check_liked_songs(&session, route).await;
    let refused = match &result {
        Ok(report) => report.session_expired,
        Err(error) => error.expires_session(),
    };
    if refused {
        // Only the session that was refused: a new login may have replaced it.
        let saved = store.load()?.filter(|saved| saved.sp_dc == session.sp_dc);
        if let Some(mut saved) = saved {
            saved.expired = true;
            store.save(&saved)?;
        }
        client.forget().await;
    }
    result
}

#[cfg(test)]
mod tests;
