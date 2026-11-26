//! Finds the public web `client_id` in soundcloud.com's JavaScript bundles.

use std::sync::LazyLock;

use regex::Regex;
use tokio::time::Instant;
use url::Url;

use crate::{
    error::Error,
    http::{Http, Identity, Request},
    secret::Secret,
};

pub const HOME_URL: &str = "https://soundcloud.com/";

/// Which text pattern in a bundle carried the id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Pattern {
    /// `client_id:"..."`
    ObjectKey,
    /// `client_id=...`
    QueryString,
}

static SCRIPT_SRC: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)<script\b[^>]*?\ssrc\s*=\s*["']([^"']+)["']"#));
// The leading group keeps longer keys such as `oauth_client_id` from matching.
static OBJECT_KEY: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?:^|[^A-Za-z0-9_])client_id\s*:\s*"([A-Za-z0-9]{32})""#));
static QUERY_STRING: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?:^|[^A-Za-z0-9_])client_id=([A-Za-z0-9]{32})(?:[^A-Za-z0-9]|$)"));

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("constant pattern is valid")
}

/// Only SoundCloud's own asset bundles are fetched, never third-party scripts.
fn is_asset_bundle(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|h| h.ends_with(".sndcdn.com"))
        && url.path().starts_with("/assets/")
        && url.path().ends_with(".js")
}

/// The asset bundle URLs of a page, in document order.
pub fn script_urls(html: &str) -> Vec<Url> {
    SCRIPT_SRC
        .captures_iter(html)
        .filter_map(|c| Url::parse(&c[1]).ok())
        .filter(is_asset_bundle)
        .collect()
}

/// The first id in a bundle, preferring the object key form.
pub fn find_client_id(js: &str) -> Option<(Secret, Pattern)> {
    let first = |re: &Regex| re.captures(js).map(|c| Secret::new(&c[1]));
    first(&OBJECT_KEY)
        .map(|id| (id, Pattern::ObjectKey))
        .or_else(|| first(&QUERY_STRING).map(|id| (id, Pattern::QueryString)))
}

/// What a scrape did. Safe to show and log: it never holds the id.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ScrapeReport {
    /// Asset bundles listed by the home page.
    pub scripts_found: usize,
    pub scripts_fetched: usize,
    /// Position of the matching bundle in document order, starting at 0.
    pub matched_index: usize,
    pub pattern: Pattern,
    pub elapsed_ms: u64,
}

/// Bundles fetched at most per scrape.
const MAX_BUNDLES: usize = 20;
/// Failed bundles in a row that end the scrape.
const MAX_FAILURES: usize = 3;

/// Reads the home page, then its bundles from last to first until one holds an id.
/// The id usually sits in one of the last bundles, so this keeps the request count low.
pub async fn scrape_client_id(
    http: &impl Http,
    identity: &Identity,
) -> Result<(Secret, ScrapeReport), Error> {
    let started = Instant::now();
    let home = Url::parse(HOME_URL).expect("constant URL is valid");
    let page = http.get(Request::public(home, identity)).await?;
    if page.status != 200 {
        return Err(Error::Status {
            what: "soundcloud.com",
            status: page.status,
        });
    }

    let urls = script_urls(&page.body);
    let mut fetched = 0;
    let mut failures = 0;
    // A query string match can be some other service's id, so it is only a fallback.
    let mut fallback = None;
    let mut found = None;
    for (index, url) in urls.iter().enumerate().rev().take(MAX_BUNDLES) {
        fetched += 1;
        let failure = match http.get(Request::public(url.clone(), identity)).await {
            Ok(response) if response.status == 200 => {
                failures = 0;
                match find_client_id(&response.body) {
                    Some((id, Pattern::ObjectKey)) => {
                        found = Some((id, Pattern::ObjectKey, index));
                        break;
                    }
                    Some((id, pattern)) => {
                        fallback.get_or_insert((id, pattern, index));
                    }
                    None => {}
                }
                continue;
            }
            Ok(response) => {
                let error = Error::Status {
                    what: "script bundle",
                    status: response.status,
                };
                if matches!(response.status, 401 | 403 | 429) {
                    return Err(error);
                }
                error
            }
            Err(error @ Error::Network(_)) => error,
            Err(refusal) => return Err(refusal),
        };
        failures += 1;
        if failures == MAX_FAILURES {
            return Err(failure);
        }
    }

    let (id, pattern, matched_index) = found.or(fallback).ok_or(Error::ClientIdNotFound {
        found: urls.len(),
        fetched,
    })?;
    let report = ScrapeReport {
        scripts_found: urls.len(),
        scripts_fetched: fetched,
        matched_index,
        pattern,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    Ok((id, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::Response,
        testing::{FakeHttp, identity, reply},
    };

    // Fake 32 character ids. Real ones never appear in this repository.
    const ID_A: &str = "FAKEaaaaFAKEaaaaFAKEaaaaFAKE0001";
    const ID_B: &str = "FAKEbbbbFAKEbbbbFAKEbbbbFAKE0002";

    fn strings(urls: Vec<Url>) -> Vec<String> {
        urls.into_iter().map(String::from).collect()
    }

    #[test]
    fn lists_asset_bundles_in_document_order() {
        let html = r#"<html><head>
            <script crossorigin src="https://a-v2.sndcdn.com/assets/0-aaa.js"></script>
            <script type="text/javascript" src='https://a-v2.sndcdn.com/assets/49-bbb.js' defer></script>
            <script>window.x = 1;</script>
            </head></html>"#;
        assert_eq!(
            strings(script_urls(html)),
            [
                "https://a-v2.sndcdn.com/assets/0-aaa.js",
                "https://a-v2.sndcdn.com/assets/49-bbb.js",
            ]
        );
    }

    #[test]
    fn ignores_scripts_that_are_not_soundcloud_asset_bundles() {
        let html = r#"
            <script src="https://js.example-analytics.com/assets/tag.js"></script>
            <script src="http://a-v2.sndcdn.com/assets/plain-http.js"></script>
            <script src="https://a-v2.sndcdn.com.evil.test/assets/x.js"></script>
            <script src="https://evilsndcdn.com/assets/x.js"></script>
            <script src="https://a-v2.sndcdn.com/other/x.js"></script>
            <script src="https://a-v2.sndcdn.com/assets/x.css"></script>
            <script data-src="https://a-v2.sndcdn.com/assets/lazy.js"></script>
            <script src="/assets/relative.js"></script>"#;
        assert_eq!(strings(script_urls(html)), Vec::<String>::new());
    }

    #[test]
    fn finds_the_object_key_form() {
        let js = format!(r#"a.exports={{api:"x",client_id:"{ID_A}",env:"p"}}"#);
        assert_eq!(
            find_client_id(&js),
            Some((Secret::new(ID_A), Pattern::ObjectKey))
        );
        let spaced = format!(r#"{{ client_id : "{ID_A}" }}"#);
        assert_eq!(
            find_client_id(&spaced),
            Some((Secret::new(ID_A), Pattern::ObjectKey))
        );
    }

    #[test]
    fn finds_the_query_string_form() {
        let js = format!(r#"u="https://api-v2.soundcloud.com/x?client_id={ID_B}&app=1""#);
        assert_eq!(
            find_client_id(&js),
            Some((Secret::new(ID_B), Pattern::QueryString))
        );
    }

    #[test]
    fn prefers_the_object_key_form_within_one_bundle() {
        let js = format!(r#""?client_id={ID_B}";c={{client_id:"{ID_A}"}}"#);
        assert_eq!(
            find_client_id(&js),
            Some((Secret::new(ID_A), Pattern::ObjectKey))
        );
    }

    #[test]
    fn rejects_other_keys_and_other_lengths() {
        let long = format!("{ID_A}X");
        let short = &ID_A[..31];
        for js in [
            format!(r#"{{oauth_client_id:"{ID_A}"}}"#),
            format!(r#"{{client_id:"{long}"}}"#),
            format!(r#"{{client_id:"{short}"}}"#),
            format!("?client_id={long}&x=1"),
            format!("?client_id={short}&x=1"),
            format!("?google_client_id={ID_A}"),
            "client_id:e.clientId".to_string(),
        ] {
            assert_eq!(find_client_id(&js), None, "matched: {}", &js[..16]);
        }
    }

    fn bundle_url(i: usize) -> String {
        format!("https://a-v2.sndcdn.com/assets/{i}-fake.js")
    }

    /// A fake site whose home page lists one bundle per given reply.
    fn site(bundles: Vec<Result<Response, Error>>) -> FakeHttp {
        let html: String = (0..bundles.len())
            .map(|i| format!(r#"<script crossorigin src="{}"></script>"#, bundle_url(i)))
            .collect();
        FakeHttp::new(move |request| {
            let url = request.url.as_str();
            if url == HOME_URL {
                return reply(200, html.clone());
            }
            let index = (0..bundles.len())
                .find(|i| url == bundle_url(*i))
                .unwrap_or_else(|| panic!("unexpected request: {url}"));
            bundles[index].clone()
        })
    }

    fn with_key(id: &str) -> Result<Response, Error> {
        reply(200, format!(r#"e={{client_id:"{id}"}}"#))
    }

    fn empty() -> Result<Response, Error> {
        reply(200, "console.log(1)")
    }

    #[tokio::test]
    async fn scans_bundles_from_the_last_one_and_stops_at_the_first_key() {
        let http = site(vec![with_key(ID_B), with_key(ID_A), empty()]);

        let (id, report) = scrape_client_id(&http, &identity()).await.unwrap();

        assert_eq!(id, Secret::new(ID_A));
        assert_eq!(
            http.urls(),
            [HOME_URL.to_string(), bundle_url(2), bundle_url(1)]
        );
        assert_eq!(report.scripts_found, 3);
        assert_eq!(report.scripts_fetched, 2);
        assert_eq!(report.matched_index, 1);
        assert_eq!(report.pattern, Pattern::ObjectKey);
        assert!(!format!("{report:?}").contains(ID_A));
    }

    #[tokio::test]
    async fn never_sends_credentials_and_keeps_one_user_agent() {
        let http = site(vec![with_key(ID_A)]);
        scrape_client_id(&http, &identity()).await.unwrap();
        assert_eq!(http.seen(|s| s.authorization.clone()), [None, None]);
        assert_eq!(
            http.seen(|s| s.user_agent.clone()),
            [Some("TestAgent/1.0".into()), Some("TestAgent/1.0".into())]
        );
    }

    #[tokio::test]
    async fn uses_the_query_form_only_when_no_bundle_has_the_key_form() {
        let query = reply(200, format!(r#"u="/x?client_id={ID_B}""#));
        let http = site(vec![with_key(ID_A), query.clone()]);
        let (id, _) = scrape_client_id(&http, &identity()).await.unwrap();
        assert_eq!(id, Secret::new(ID_A));

        let http = site(vec![empty(), query]);
        let (id, report) = scrape_client_id(&http, &identity()).await.unwrap();
        assert_eq!(id, Secret::new(ID_B));
        assert_eq!(report.pattern, Pattern::QueryString);
        assert_eq!(report.matched_index, 1);
        assert_eq!(report.scripts_fetched, 2);
    }

    #[tokio::test]
    async fn skips_a_bundle_that_fails() {
        let http = site(vec![
            with_key(ID_A),
            reply(404, ""),
            Err(Error::Network("reset".into())),
        ]);
        let (id, report) = scrape_client_id(&http, &identity()).await.unwrap();
        assert_eq!(id, Secret::new(ID_A));
        assert_eq!(report.scripts_fetched, 3);
    }

    #[tokio::test]
    async fn gives_up_after_three_failed_bundles_in_a_row() {
        let http = site(vec![
            with_key(ID_A),
            // Scanned from the last one: 502, 404, then 500 ends the scrape.
            reply(500, ""),
            reply(404, ""),
            reply(502, ""),
        ]);
        assert_eq!(
            scrape_client_id(&http, &identity()).await.unwrap_err(),
            Error::Status {
                what: "script bundle",
                status: 500
            }
        );
        assert_eq!(http.urls().len(), 4);
    }

    #[tokio::test]
    async fn stops_at_once_when_soundcloud_refuses() {
        for (refusal, expected) in [
            (
                Err(Error::RateLimited { secs: 9 }),
                Error::RateLimited { secs: 9 },
            ),
            (Err(Error::Blocked), Error::Blocked),
            (
                reply(403, ""),
                Error::Status {
                    what: "script bundle",
                    status: 403,
                },
            ),
        ] {
            let http = site(vec![with_key(ID_A), refusal]);
            assert_eq!(
                scrape_client_id(&http, &identity()).await.unwrap_err(),
                expected
            );
            assert_eq!(http.urls().len(), 2, "no request after the refusal");
        }
    }

    #[tokio::test]
    async fn reports_how_far_it_looked_when_nothing_matches() {
        let http = site(vec![empty(), empty()]);
        assert_eq!(
            scrape_client_id(&http, &identity()).await.unwrap_err(),
            Error::ClientIdNotFound {
                found: 2,
                fetched: 2
            }
        );
    }

    #[tokio::test]
    async fn fetches_no_more_than_twenty_bundles() {
        let http = site((0..25).map(|_| empty()).collect());
        assert_eq!(
            scrape_client_id(&http, &identity()).await.unwrap_err(),
            Error::ClientIdNotFound {
                found: 25,
                fetched: 20
            }
        );
        assert_eq!(http.urls().len(), 21);
    }

    #[tokio::test]
    async fn fails_when_the_home_page_is_refused() {
        let http = FakeHttp::sequence(vec![reply(403, "")]);
        assert_eq!(
            scrape_client_id(&http, &identity()).await.unwrap_err(),
            Error::Status {
                what: "soundcloud.com",
                status: 403
            }
        );
    }
}
