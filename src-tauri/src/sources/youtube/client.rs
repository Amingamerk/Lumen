//! The one place that knows how YouTube Music's web client asks, and what its
//! answers mean to the gate.

use serde_json::{Value, json};
use time::OffsetDateTime;

use super::auth::authorization;
use super::cookies::Cookie;
use super::gate::Verdict;
use super::session::{ORIGIN, Session};
use super::transport::{Request, Response, TransportError};

const HOST: &str = "music.youtube.com";
const CLIENT_NAME: &str = "WEB_REMIX";
/// `WEB_REMIX` as the number the headers use.
const CLIENT_NAME_ID: &str = "67";
/// The library's playlists.
const BROWSE_ID: &str = "FEmusic_liked_playlists";
/// Found in the answers of Google's bot checks.
const BOT_CHECK_MARKERS: [&str; 3] = ["captcha", "unusual traffic", "/sorry/"];

/// Why a reply gave nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    /// The request or its reply was lost on the way.
    Offline,
    /// The session was refused.
    Rejected,
    RateLimited,
    /// A captcha or "unusual traffic" answer.
    Blocked,
    /// A reply Lumen cannot use.
    Unexpected,
}

/// The library-playlists browse that confirms a session. `None` when no cookie
/// that applies to it can sign it.
pub fn browse(session: &Session, now_ms: u64) -> Option<Request> {
    request(session, "browse", json!({ "browseId": BROWSE_ID }), now_ms)
}

/// One `youtubei` call with the web client's headers and `context`. `body` holds
/// the rest of the fields.
fn request(session: &Session, endpoint: &str, mut body: Value, now_ms: u64) -> Option<Request> {
    let path = format!("/youtubei/v1/{endpoint}");
    let now_s = now_ms / 1_000;
    let cookies: Vec<&Cookie> = session
        .cookies
        .iter()
        .filter(|cookie| cookie.is_sent_to(HOST, &path, now_s))
        .collect();
    let authorization = authorization(&cookies, now_s)?;
    let cookie_header = cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ");
    // `da-DK` asks for `da-DK,da;q=0.9`, with `da` as the interface language.
    let language = session.language.as_str();
    let primary = language.split('-').next().unwrap_or(language);
    let accept_language = if primary == language {
        language.to_owned()
    } else {
        format!("{language},{primary};q=0.9")
    };
    let version = client_version(now_s);
    body["context"] = json!({
        "client": { "clientName": CLIENT_NAME, "clientVersion": version, "hl": primary },
        "user": {},
    });
    Some(Request {
        url: format!("{ORIGIN}{path}?prettyPrint=false"),
        headers: vec![
            ("Authorization", authorization),
            ("Cookie", cookie_header),
            ("User-Agent", session.user_agent.clone()),
            ("Accept-Language", accept_language),
            ("Origin", ORIGIN.to_owned()),
            ("X-Origin", ORIGIN.to_owned()),
            ("Referer", format!("{ORIGIN}/")),
            // The login window holds a single account.
            ("X-Goog-AuthUser", "0".to_owned()),
            ("Content-Type", "application/json".to_owned()),
            ("X-YouTube-Client-Name", CLIENT_NAME_ID.to_owned()),
            ("X-YouTube-Client-Version", version),
        ],
        body: body.to_string(),
    })
}

/// The web client versions itself by build date, as ytmusicapi mimics it.
fn client_version(now_s: u64) -> String {
    let date = i64::try_from(now_s)
        .ok()
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .date();
    let (year, month, day) = (date.year(), u8::from(date.month()), date.day());
    format!("1.{year:04}{month:02}{day:02}.01.00")
}

/// Accepts a browse reply that holds a page.
pub fn page(reply: &Value) -> Option<()> {
    let contents = reply["contents"].as_object()?;
    (!contents.is_empty()).then_some(())
}

/// What a reply means to the gate, and what `parse` made of it.
pub fn classify<T>(
    sent: Result<Response, TransportError>,
    parse: impl FnOnce(&Value) -> Option<T>,
) -> (Verdict, Result<T, Fault>) {
    let unusable = (Verdict::Failure, Err(Fault::Unexpected));
    let refused = (Verdict::Rejected, Err(Fault::Rejected));
    let response = match sent {
        Ok(response) => response,
        Err(TransportError::Connect) => return (Verdict::Neutral, Err(Fault::Offline)),
        Err(TransportError::Timeout | TransportError::Other) => {
            return (Verdict::Failure, Err(Fault::Offline));
        }
    };
    let reply: Option<Value> = (response.status == 200)
        .then(|| serde_json::from_str(&response.body).ok())
        .flatten();
    // A valid reply is never searched: a song title may hold those words.
    if reply.is_none() && is_bot_check(&response) {
        return (Verdict::BotCheck, Err(Fault::Blocked));
    }
    match response.status {
        429 => {
            let retry_after = response.retry_after;
            let verdict = Verdict::RateLimited { retry_after };
            (verdict, Err(Fault::RateLimited))
        }
        401 | 403 => refused,
        _ => {
            let Some(reply) = reply else { return unusable };
            match logged_in(&reply) {
                Some(false) => refused,
                Some(true) => match parse(&reply) {
                    Some(parsed) => (Verdict::Success, Ok(parsed)),
                    None => unusable,
                },
                None => unusable,
            }
        }
    }
}

fn is_bot_check(response: &Response) -> bool {
    [Some(&response.body), response.location.as_ref()]
        .into_iter()
        .flatten()
        .any(|text| {
            let text = text.to_ascii_lowercase();
            BOT_CHECK_MARKERS.iter().any(|marker| text.contains(marker))
        })
}

/// What the reply says about the session. One signed-out answer outweighs the
/// others.
fn logged_in(reply: &Value) -> Option<bool> {
    reply["responseContext"]["serviceTrackingParams"]
        .as_array()?
        .iter()
        .filter_map(|service| service["params"].as_array())
        .flatten()
        .filter(|param| param["key"] == "logged_in")
        .filter_map(|param| match param["value"].as_str()? {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        })
        .reduce(|all, one| all && one)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sources::youtube::cookies::Cookie;
    use crate::sources::youtube::session::fake::cookie;

    /// 2023-11-14 22:13:20 UTC.
    const NOW_MS: u64 = 1_700_000_000_000;

    fn session(language: &str) -> Session {
        let valued = |name: &str, value: &str| Cookie {
            value: value.to_owned(),
            ..cookie(name, ".youtube.com")
        };
        Session {
            cookies: vec![
                valued("SAPISID", "fakeSapisid/AbC123"),
                valued("LOGIN_INFO", "fake-login-info"),
                Cookie {
                    domain: "www.youtube.com".to_owned(),
                    ..valued("WWW_ONLY", "fake-www")
                },
                Cookie {
                    path: "/other".to_owned(),
                    ..valued("OTHER_PATH", "fake-path")
                },
                Cookie {
                    expires: Some(1_699_999_999),
                    ..valued("OLD", "fake-old")
                },
            ],
            user_agent: "FakeAgent/1.0".to_owned(),
            language: language.to_owned(),
            expired: false,
        }
    }

    #[test]
    fn browses_as_the_web_client_that_signed_in() {
        let request = browse(&session("da-DK"), NOW_MS).unwrap();

        assert_eq!(
            request.url,
            "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false"
        );
        let headers: Vec<_> = request
            .headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        assert_eq!(
            headers,
            [
                (
                    "Authorization",
                    "SAPISIDHASH 1700000000_fa90dc53295b9f3251e626355147af2d028548fd"
                ),
                (
                    "Cookie",
                    "SAPISID=fakeSapisid/AbC123; LOGIN_INFO=fake-login-info"
                ),
                ("User-Agent", "FakeAgent/1.0"),
                ("Accept-Language", "da-DK,da;q=0.9"),
                ("Origin", "https://music.youtube.com"),
                ("X-Origin", "https://music.youtube.com"),
                ("Referer", "https://music.youtube.com/"),
                ("X-Goog-AuthUser", "0"),
                ("Content-Type", "application/json"),
                ("X-YouTube-Client-Name", "67"),
                ("X-YouTube-Client-Version", "1.20231114.01.00"),
            ]
        );
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({
                "context": {
                    "client": {
                        "clientName": "WEB_REMIX",
                        "clientVersion": "1.20231114.01.00",
                        "hl": "da"
                    },
                    "user": {}
                },
                "browseId": "FEmusic_liked_playlists"
            })
        );
    }

    #[test]
    fn a_language_without_a_region_is_sent_as_it_is() {
        let request = browse(&session("da"), NOW_MS).unwrap();

        assert!(
            request
                .headers
                .contains(&("Accept-Language", "da".to_owned()))
        );
        assert!(request.body.contains(r#""hl":"da""#), "{}", request.body);
    }

    #[test]
    fn builds_no_request_without_an_applicable_sapisid() {
        for sapisid in [
            Cookie {
                domain: "www.youtube.com".to_owned(),
                ..cookie("SAPISID", "")
            },
            Cookie {
                path: "/other".to_owned(),
                ..cookie("SAPISID", ".youtube.com")
            },
            Cookie {
                expires: Some(1_700_000_000),
                ..cookie("SAPISID", ".youtube.com")
            },
        ] {
            let session = Session {
                cookies: vec![sapisid, cookie("LOGIN_INFO", ".youtube.com")],
                ..session("da-DK")
            };

            assert!(browse(&session, NOW_MS).is_none(), "{:?}", session.cookies);
        }
    }

    fn reply(status: u16, body: impl ToString) -> Response {
        Response {
            status,
            body: body.to_string(),
            ..Response::default()
        }
    }

    /// A page reply as YouTube Music shapes it.
    fn answered(logged_in: &str, title: &str) -> Value {
        json!({
            "responseContext": { "serviceTrackingParams": [
                { "service": "OTHER", "params": [{ "key": "c", "value": "WEB_REMIX" }] },
                { "service": "GFEEDBACK", "params": [{ "key": "logged_in", "value": logged_in }] }
            ]},
            "contents": { "title": title }
        })
    }

    fn judged(response: Response) -> (Verdict, Result<(), Fault>) {
        classify(Ok(response), page)
    }

    #[test]
    fn a_signed_in_page_is_a_success() {
        assert_eq!(
            judged(reply(200, answered("1", "Library"))),
            (Verdict::Success, Ok(()))
        );
    }

    #[test]
    fn a_refused_session_is_rejected() {
        for response in [
            reply(401, ""),
            reply(403, r#"{"error": {"code": 403}}"#),
            reply(200, answered("0", "Library")),
        ] {
            let status = response.status;

            assert_eq!(
                judged(response),
                (Verdict::Rejected, Err(Fault::Rejected)),
                "{status}"
            );
        }
    }

    #[test]
    fn a_429_is_rate_limited_and_keeps_retry_after() {
        let response = Response {
            retry_after: Some("90".to_owned()),
            ..reply(429, "")
        };

        assert_eq!(
            judged(response),
            (
                Verdict::RateLimited {
                    retry_after: Some("90".to_owned())
                },
                Err(Fault::RateLimited)
            )
        );
    }

    #[test]
    fn a_bot_check_is_found_in_the_body_or_the_location() {
        let redirected = |location: &str| Response {
            location: Some(location.to_owned()),
            ..reply(302, "")
        };
        for response in [
            reply(429, "Our systems have detected Unusual Traffic"),
            reply(403, "please solve the CAPTCHA"),
            reply(200, "<html><form action=\"/sorry/index\"></form></html>"),
            reply(200, "<html>unusual traffic from your network</html>"),
            reply(503, r#"{"reason": "captcha"}"#),
            redirected("https://www.google.com/sorry/index?continue=x"),
            redirected("https://www.google.com/recaptcha"),
        ] {
            let shown = format!(
                "{} {} {:?}",
                response.status, response.body, response.location
            );

            assert_eq!(
                judged(response),
                (Verdict::BotCheck, Err(Fault::Blocked)),
                "{shown}"
            );
        }
    }

    #[test]
    fn a_valid_page_may_name_those_words() {
        for title in ["Captcha", "Unusual Traffic", "x/sorry/y"] {
            assert_eq!(
                judged(reply(200, answered("1", title))),
                (Verdict::Success, Ok(())),
                "{title}"
            );
        }
    }

    #[test]
    fn any_other_reply_is_a_failure() {
        let signed_in_without_a_page = json!({ "responseContext": { "serviceTrackingParams": [
            { "params": [{ "key": "logged_in", "value": "1" }] }
        ]}});
        for response in [
            reply(500, ""),
            reply(503, "unavailable"),
            reply(302, ""),
            reply(204, ""),
            reply(404, answered("1", "Library")),
            reply(200, "<html></html>"),
            reply(200, ""),
            reply(200, "{}"),
            reply(200, json!({ "contents": {} })),
            reply(200, answered("yes", "Library")),
            reply(200, signed_in_without_a_page),
        ] {
            let shown = format!("{} {}", response.status, response.body);

            assert_eq!(
                judged(response),
                (Verdict::Failure, Err(Fault::Unexpected)),
                "{shown}"
            );
        }
    }

    #[test]
    fn only_a_request_that_never_arrived_counts_nothing() {
        for (error, verdict) in [
            (TransportError::Connect, Verdict::Neutral),
            (TransportError::Timeout, Verdict::Failure),
            (TransportError::Other, Verdict::Failure),
        ] {
            assert_eq!(
                classify(Err(error), page),
                (verdict, Err(Fault::Offline)),
                "{error:?}"
            );
        }
    }
}
