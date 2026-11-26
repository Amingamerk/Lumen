//! The one authenticated call: the first page of the signed-in user's liked tracks.

use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::time::sleep;
use url::Url;

use crate::{
    error::Error,
    http::{Http, Request, Response},
    scrape::{ScrapeReport, scrape_client_id},
    secret::Secret,
    session::{Session, SessionStore},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LikedTrack {
    pub title: String,
    pub artist: Option<String>,
}

/// Where the `client_id` used for the answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ClientIdSource {
    Cached,
    Scraped,
    /// Scraped again after a 401 or 403.
    Rescraped,
}

/// What the webview gets. No tokens, ids or links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Likes {
    pub display_name: Option<String>,
    pub tracks: Vec<LikedTrack>,
    pub has_more: bool,
    pub total_likes: Option<u64>,
    /// Which likes path answered.
    pub endpoint: &'static str,
    pub client_id_source: ClientIdSource,
    pub scrape: Option<ScrapeReport>,
}

const API: &str = "https://api-v2.soundcloud.com";
const LIKES_PATH: &str = "/me/track_likes";
const LIMIT: &str = "10";
/// Wait before the single retry after a 401 or 403, plus up to one second of jitter.
const RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Page {
    collection: Vec<Item>,
    next_href: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    track: Option<TrackBody>,
}

#[derive(Deserialize)]
struct TrackBody {
    title: Option<String>,
    user: Option<User>,
}

#[derive(Deserialize)]
struct User {
    username: Option<String>,
}

/// Only what is shown. The rest of the profile, such as the email, is never read.
#[derive(Deserialize)]
struct Me {
    id: u64,
    username: Option<String>,
    likes_count: Option<u64>,
}

/// One fetch, which may scrape a new `client_id` once.
struct Fetch<'a, H, S> {
    http: &'a H,
    store: &'a S,
    session: Session,
    source: ClientIdSource,
    scrape: Option<ScrapeReport>,
    rescraped: bool,
}

impl<H: Http, S: SessionStore> Fetch<'_, H, S> {
    async fn scrape_client_id(&mut self) -> Result<(), Error> {
        let (id, report) = scrape_client_id(self.http, &self.session.identity).await?;
        self.session.client_id = Some(id);
        self.scrape = Some(report);
        self.store.save(&self.session)
    }

    /// One authenticated request with no recovery.
    async fn send(&self, path: &str, limit: Option<&str>) -> Result<Response, Error> {
        let mut url = Url::parse(API).expect("constant URL is valid");
        url.set_path(path);
        let client_id = self.session.client_id.as_ref().map(Secret::expose);
        url.query_pairs_mut()
            .append_pair("client_id", client_id.unwrap_or_default());
        if let Some(limit) = limit {
            url.query_pairs_mut().append_pair("limit", limit);
        }
        let request = Request::api(url, &self.session.identity, &self.session.oauth_token);
        self.http.get(request).await
    }

    /// Sends a request. On 401 or 403 it scrapes the `client_id` again and retries, once
    /// per fetch; a further refusal marks the session expired.
    async fn get(&mut self, path: &str, limit: Option<&str>) -> Result<Response, Error> {
        loop {
            let response = self.send(path, limit).await?;
            if !matches!(response.status, 401 | 403) {
                return Ok(response);
            }
            if self.rescraped {
                return Err(self.expire());
            }
            self.rescraped = true;
            sleep(RETRY_DELAY + Duration::from_millis(fastrand::u64(0..1000))).await;
            match self.scrape_client_id().await {
                Ok(()) => self.source = ClientIdSource::Rescraped,
                // Being told to back off says nothing about the login.
                Err(error @ (Error::RateLimited { .. } | Error::Blocked)) => return Err(error),
                Err(_) => return Err(self.expire()),
            }
        }
    }

    fn expire(&mut self) -> Error {
        self.session.expired = true;
        match self.store.save(&self.session) {
            Ok(()) => Error::SessionExpired,
            Err(error) => error,
        }
    }
}

fn parse<T: DeserializeOwned>(
    response: &Response,
    what: &'static str,
    parse_error: &'static str,
) -> Result<T, Error> {
    if response.status != 200 {
        return Err(Error::Status {
            what,
            status: response.status,
        });
    }
    // serde errors can quote the body, so they are replaced.
    serde_json::from_str(&response.body).map_err(|_| Error::Parse(parse_error))
}

/// Fetches the first page of likes and, if that works, the display name.
pub async fn fetch_likes(http: &impl Http, store: &impl SessionStore) -> Result<Likes, Error> {
    let session = store.load()?.ok_or(Error::NotLoggedIn)?;
    if session.expired {
        return Err(Error::SessionExpired);
    }
    let mut fetch = Fetch {
        http,
        store,
        session,
        source: ClientIdSource::Cached,
        scrape: None,
        rescraped: false,
    };
    if fetch.session.client_id.is_none() {
        fetch.scrape_client_id().await?;
        fetch.source = ClientIdSource::Scraped;
    }

    let mut endpoint = LIKES_PATH;
    let mut me = None;
    let mut response = fetch.get(LIKES_PATH, Some(LIMIT)).await?;
    if response.status == 404 {
        // The web client also lists likes by user id.
        let profile = fetch.get("/me", None).await?;
        let profile: Me = parse(&profile, "profile", "profile response")?;
        let path = format!("/users/{}/track_likes", profile.id);
        response = fetch.get(&path, Some(LIMIT)).await?;
        endpoint = "/users/{id}/track_likes";
        me = Some(profile);
    }
    let page: Page = parse(&response, "track likes", "track likes response")?;

    // The name is a nicety: a failure here neither hides the likes nor expires the session.
    if me.is_none() {
        me = match fetch.send("/me", None).await {
            Ok(response) => parse(&response, "profile", "profile response").ok(),
            Err(_) => None,
        };
    }

    let tracks = page
        .collection
        .into_iter()
        .filter_map(|item| item.track)
        .filter_map(|track| {
            Some(LikedTrack {
                title: track.title?,
                artist: track.user.and_then(|user| user.username),
            })
        })
        .collect();
    let (display_name, total_likes) = me.map_or((None, None), |m| (m.username, m.likes_count));
    Ok(Likes {
        display_name,
        tracks,
        has_more: page.next_href.is_some(),
        total_likes,
        endpoint,
        client_id_source: fetch.source,
        scrape: fetch.scrape,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        scrape::HOME_URL,
        testing::{CLIENT_ID, FakeHttp, MemoryStore, NEW_CLIENT_ID, TOKEN, reply, session},
        throttle::{Limits, Throttle},
    };

    const BUNDLE: &str = "https://a-v2.sndcdn.com/assets/1-fake.js";
    const LIKES: &str = "api-v2.soundcloud.com/me/track_likes";
    const ME: &str = "api-v2.soundcloud.com/me";
    const USER_LIKES: &str = "api-v2.soundcloud.com/users/42/track_likes";

    const LIKES_BODY: &str = r#"{
        "collection": [
            {"kind": "like", "track": {"title": "First song", "user": {"username": "Artist A"}}},
            {"kind": "like", "track": {"title": "Second song"}},
            {"kind": "like"}
        ],
        "next_href": "https://api-v2.soundcloud.com/x?client_id=FAKEaaaaFAKEaaaaFAKEaaaaFAKE0001"
    }"#;
    const ME_BODY: &str = r#"{"id": 42, "username": "Test Listener", "likes_count": 57,
        "primary_email": "listener@example.test"}"#;

    /// Answers API requests by path and serves a home page whose bundle holds the new id.
    fn soundcloud(
        mut api: impl FnMut(&str, &str) -> Result<Response, Error> + Send + 'static,
    ) -> FakeHttp {
        FakeHttp::new(move |request: &Request| {
            let url = &request.url;
            match url.as_str() {
                HOME_URL => reply(200, format!(r#"<script src="{BUNDLE}"></script>"#)),
                BUNDLE => reply(200, format!(r#"{{client_id:"{NEW_CLIENT_ID}"}}"#)),
                full => {
                    let path = format!("{}{}", url.host_str().unwrap(), url.path());
                    api(&path, full)
                }
            }
        })
    }

    fn healthy() -> FakeHttp {
        soundcloud(|path, _| match path {
            LIKES => reply(200, LIKES_BODY),
            ME => reply(200, ME_BODY),
            other => panic!("unexpected path {other}"),
        })
    }

    /// Hosts and paths only, in request order.
    fn paths(http: &FakeHttp) -> Vec<String> {
        http.urls()
            .iter()
            .map(|u| {
                let u = u.trim_start_matches("https://");
                u.split('?').next().unwrap().to_string()
            })
            .collect()
    }

    const SCRAPE: [&str; 2] = ["soundcloud.com/", "a-v2.sndcdn.com/assets/1-fake.js"];

    fn saved(store: &MemoryStore) -> crate::session::Session {
        store.load().unwrap().unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn fetches_likes_once_with_the_cached_client_id() {
        let http = healthy();
        let store = MemoryStore::with(session(Some(CLIENT_ID)));

        let likes = fetch_likes(&http, &store).await.unwrap();

        assert_eq!(paths(&http), [LIKES, ME]);
        assert_eq!(
            http.urls()[0],
            format!("https://{LIKES}?client_id={CLIENT_ID}&limit=10")
        );
        assert_eq!(
            http.seen(|s| s.authorization.clone()),
            vec![Some(format!("OAuth {TOKEN}")); 2]
        );
        assert_eq!(likes.display_name.as_deref(), Some("Test Listener"));
        assert_eq!(
            likes.tracks,
            [
                LikedTrack {
                    title: "First song".into(),
                    artist: Some("Artist A".into())
                },
                LikedTrack {
                    title: "Second song".into(),
                    artist: None
                },
            ]
        );
        assert!(likes.has_more);
        assert_eq!(likes.total_likes, Some(57));
        assert_eq!(likes.endpoint, "/me/track_likes");
        assert_eq!(likes.client_id_source, ClientIdSource::Cached);
        assert_eq!(likes.scrape, None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_result_for_the_webview_holds_no_secrets_or_private_fields() {
        let http = healthy();
        let store = MemoryStore::with(session(None));
        let likes = fetch_likes(&http, &store).await.unwrap();
        let json = serde_json::to_string(&likes).unwrap();
        for private in [TOKEN, CLIENT_ID, NEW_CLIENT_ID, "listener@example.test"] {
            assert!(!json.contains(private));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn scrapes_and_caches_the_client_id_when_none_is_stored() {
        let http = healthy();
        let store = MemoryStore::with(session(None));

        let likes = fetch_likes(&http, &store).await.unwrap();

        assert_eq!(paths(&http), [SCRAPE[0], SCRAPE[1], LIKES, ME]);
        assert_eq!(saved(&store).client_id, Some(Secret::new(NEW_CLIENT_ID)));
        assert_eq!(likes.client_id_source, ClientIdSource::Scraped);
        assert_eq!(likes.scrape.map(|s| s.scripts_fetched), Some(1));
    }

    #[tokio::test(start_paused = true)]
    async fn makes_no_request_without_a_live_session() {
        let http = healthy();
        let mut expired = session(Some(CLIENT_ID));
        expired.expired = true;

        assert_eq!(
            fetch_likes(&http, &MemoryStore::default()).await,
            Err(Error::NotLoggedIn)
        );
        assert_eq!(
            fetch_likes(&http, &MemoryStore::with(expired)).await,
            Err(Error::SessionExpired)
        );
        assert!(http.urls().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_401_or_403_rescrapes_the_client_id_and_retries_once() {
        for status in [401, 403] {
            let http = soundcloud(move |path, url| match path {
                LIKES if url.contains(CLIENT_ID) => reply(status, ""),
                LIKES => reply(200, LIKES_BODY),
                _ => reply(200, ME_BODY),
            });
            let store = MemoryStore::with(session(Some(CLIENT_ID)));

            let likes = fetch_likes(&http, &store).await.unwrap();

            assert_eq!(paths(&http), [LIKES, SCRAPE[0], SCRAPE[1], LIKES, ME]);
            assert!(http.urls()[3].contains(NEW_CLIENT_ID));
            assert_eq!(likes.client_id_source, ClientIdSource::Rescraped);
            assert_eq!(saved(&store).client_id, Some(Secret::new(NEW_CLIENT_ID)));
            assert!(!saved(&store).expired);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn waits_before_the_retry() {
        let http = soundcloud(|path, url| match path {
            LIKES if url.contains(CLIENT_ID) => reply(401, ""),
            LIKES => reply(200, LIKES_BODY),
            _ => reply(200, ME_BODY),
        });
        fetch_likes(&http, &MemoryStore::with(session(Some(CLIENT_ID))))
            .await
            .unwrap();
        let times = http.seen(|s| s.at);
        assert!(times[1] - times[0] >= std::time::Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_refusal_marks_the_session_expired_and_stays_quiet() {
        let http = soundcloud(|_, _| reply(401, ""));
        let store = MemoryStore::with(session(Some(CLIENT_ID)));

        assert_eq!(fetch_likes(&http, &store).await, Err(Error::SessionExpired));
        assert_eq!(paths(&http), [LIKES, SCRAPE[0], SCRAPE[1], LIKES]);
        assert!(saved(&store).expired);

        assert_eq!(fetch_likes(&http, &store).await, Err(Error::SessionExpired));
        assert_eq!(http.urls().len(), 4, "an expired session sends nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_right_after_a_first_scrape_gets_one_rescrape_too() {
        let http = soundcloud(|_, _| reply(403, ""));
        let store = MemoryStore::with(session(None));
        assert_eq!(fetch_likes(&http, &store).await, Err(Error::SessionExpired));
        assert_eq!(
            paths(&http),
            [SCRAPE[0], SCRAPE[1], LIKES, SCRAPE[0], SCRAPE[1], LIKES]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn expires_the_session_when_the_rescrape_fails() {
        let http = FakeHttp::sequence(vec![reply(401, ""), reply(500, "")]);
        let store = MemoryStore::with(session(Some(CLIENT_ID)));
        assert_eq!(fetch_likes(&http, &store).await, Err(Error::SessionExpired));
        assert!(saved(&store).expired);
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_limited_rescrape_does_not_expire_the_session() {
        let http = FakeHttp::sequence(vec![reply(401, ""), Err(Error::RateLimited { secs: 30 })]);
        let store = MemoryStore::with(session(Some(CLIENT_ID)));
        assert_eq!(
            fetch_likes(&http, &store).await,
            Err(Error::RateLimited { secs: 30 })
        );
        assert!(!saved(&store).expired);
    }

    #[tokio::test(start_paused = true)]
    async fn a_429_or_bot_check_ends_the_fetch_without_retrying() {
        let limited = Response {
            status: 429,
            retry_after: Some("90".into()),
            body: String::new(),
        };
        let challenge = reply(403, r#"{"url":"https://geo.captcha-delivery.com/c"}"#);
        for answer in [Ok(limited), challenge] {
            let blocked = answer.as_ref().unwrap().status == 403;
            let http = Throttle::new(FakeHttp::sequence(vec![answer]), Limits::default());
            let store = MemoryStore::with(session(Some(CLIENT_ID)));

            let error = fetch_likes(&http, &store).await.unwrap_err();

            if blocked {
                assert_eq!(error, Error::Blocked);
            } else {
                assert!(matches!(error, Error::RateLimited { secs: 90..=91 }));
            }
            assert_eq!(http.inner().urls().len(), 1);
            assert!(!saved(&store).expired);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn falls_back_to_the_user_likes_path_when_me_track_likes_is_missing() {
        let http = soundcloud(|path, _| match path {
            LIKES => reply(404, ""),
            ME => reply(200, ME_BODY),
            USER_LIKES => reply(200, LIKES_BODY),
            other => panic!("unexpected path {other}"),
        });
        let store = MemoryStore::with(session(Some(CLIENT_ID)));

        let likes = fetch_likes(&http, &store).await.unwrap();

        assert_eq!(paths(&http), [LIKES, ME, USER_LIKES]);
        assert_eq!(likes.endpoint, "/users/{id}/track_likes");
        assert_eq!(likes.display_name.as_deref(), Some("Test Listener"));
        assert_eq!(likes.tracks.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_fallback_shares_the_single_rescrape() {
        let http = soundcloud(|path, url| match path {
            LIKES if url.contains(CLIENT_ID) => reply(401, ""),
            LIKES => reply(404, ""),
            _ => reply(401, ""),
        });
        let store = MemoryStore::with(session(Some(CLIENT_ID)));
        assert_eq!(fetch_likes(&http, &store).await, Err(Error::SessionExpired));
        assert_eq!(paths(&http), [LIKES, SCRAPE[0], SCRAPE[1], LIKES, ME]);
    }

    #[tokio::test(start_paused = true)]
    async fn likes_still_show_when_the_profile_call_fails() {
        let http = soundcloud(|path, _| match path {
            LIKES => reply(200, LIKES_BODY),
            _ => reply(401, ""),
        });
        let store = MemoryStore::with(session(Some(CLIENT_ID)));

        let likes = fetch_likes(&http, &store).await.unwrap();

        assert_eq!(likes.display_name, None);
        assert_eq!(likes.tracks.len(), 2);
        assert_eq!(paths(&http), [LIKES, ME]);
        assert!(!saved(&store).expired);
    }

    #[tokio::test(start_paused = true)]
    async fn other_failures_are_reported_without_the_body() {
        let store = MemoryStore::with(session(Some(CLIENT_ID)));

        let http = soundcloud(|_, _| reply(500, TOKEN));
        assert_eq!(
            fetch_likes(&http, &store).await,
            Err(Error::Status {
                what: "track likes",
                status: 500
            })
        );

        let http = soundcloud(|_, _| reply(200, format!(r#"{{"collection":"{TOKEN}"}}"#)));
        let error = fetch_likes(&http, &store).await.unwrap_err();
        assert_eq!(error, Error::Parse("track likes response"));
        assert!(!saved(&store).expired);
    }
}
