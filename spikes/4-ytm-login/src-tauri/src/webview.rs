//! The WebView2 side: the login window, reading its cookies and User-Agent.

use std::{sync::Mutex, time::Duration};

use tauri::{
    AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    webview::{Cookie as WebCookie, PageLoadEvent},
};
use tokio::sync::oneshot;
use ytm_login_spike::{
    auth::ORIGIN,
    clock::{Clock, SystemClock},
    cookies::Cookie,
    session::{CaptureMethod, SessionStatus, is_signed_in},
};

use crate::{App, SESSION_CHANGED};

pub const LOGIN_LABEL: &str = "ytm-login";
const NOT_SIGNED_IN: &str = "not signed in yet";
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(10);

fn is_music_page(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("music.youtube.com")
}

/// Decodes the JSON that a script evaluation hands back.
fn script_string(raw: &str) -> Result<String, String> {
    serde_json::from_str::<String>(raw)
        .ok()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "the login window returned no User-Agent".to_owned())
}

pub fn convert(cookie: &WebCookie<'_>) -> Cookie {
    let name = cookie.name();
    let domain = match cookie.domain() {
        // The `cookie` crate drops the leading dot from `domain()`. YouTube's site-wide cookies
        // are the ones on the bare domain, so give those the dot back.
        Some("youtube.com") if !name.starts_with("__Host-") => ".youtube.com",
        domain => domain.unwrap_or_default(),
    };
    Cookie {
        name: name.to_owned(),
        value: cookie.value().to_owned(),
        domain: domain.to_owned(),
        path: cookie.path().unwrap_or("/").to_owned(),
        secure: cookie.secure().unwrap_or(false),
        http_only: cookie.http_only().unwrap_or(false),
        expires: cookie
            .expires_datetime()
            .and_then(|at| u64::try_from(at.unix_timestamp()).ok()),
    }
}

/// Opens YouTube Music in its own InPrivate window: nothing of the login stays in the WebView2
/// profile, and no live browser keeps rotating the captured cookies.
///
/// Call from an async command: building a window in a synchronous one deadlocks on Windows.
pub fn open_login(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LOGIN_LABEL) {
        return window.set_focus();
    }
    let url = Url::parse(ORIGIN).map_err(tauri::Error::InvalidUrl)?;
    WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Log in to YouTube Music")
        .inner_size(1000.0, 800.0)
        .incognito(true)
        .on_page_load(|window, payload| {
            if is_music_page(payload.url()) && matches!(payload.event(), PageLoadEvent::Finished) {
                let app = window.app_handle().clone();
                // Reading cookies on this (main) thread deadlocks on Windows.
                tauri::async_runtime::spawn(async move {
                    match capture(&app).await {
                        Ok(_) => {}
                        Err(error) if error.starts_with(NOT_SIGNED_IN) => {}
                        Err(error) => {
                            let _ = app.emit("login-error", error);
                        }
                    }
                });
            }
        })
        .build()?;
    Ok(())
}

/// Saves the login window's session and closes the window, if the login is complete.
pub async fn capture(app: &AppHandle) -> Result<SessionStatus, String> {
    let window = app
        .get_webview_window(LOGIN_LABEL)
        .ok_or("the login window is not open")?;
    let cookies = youtube_cookies(window.clone()).await?;
    if !is_signed_in(&cookies, SystemClock.now()) {
        return Err(format!(
            "{NOT_SIGNED_IN}: {} youtube.com cookies, but no SAPISID with LOGIN_INFO",
            cookies.len()
        ));
    }
    if !is_music_page(&window.url().map_err(|e| e.to_string())?) {
        return Err("signed in: wait for the window to return to music.youtube.com".to_owned());
    }
    let user_agent = page_user_agent(&window).await?;
    let status = app
        .state::<App>()
        .save(cookies, &user_agent, CaptureMethod::LoginWindow)
        .await
        .map_err(|e| e.to_string())?;
    app.emit(SESSION_CHANGED, Some(&status))
        .map_err(|e| e.to_string())?;
    window.close().map_err(|e| e.to_string())?;
    Ok(status)
}

/// Only the cookies that apply to music.youtube.com, HttpOnly ones included.
async fn youtube_cookies(window: WebviewWindow) -> Result<Vec<Cookie>, String> {
    let url = Url::parse(ORIGIN).map_err(|e| e.to_string())?;
    let cookies = tauri::async_runtime::spawn_blocking(move || window.cookies_for_url(url))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(cookies.iter().map(convert).collect())
}

/// The User-Agent YouTube Music is actually sent. WebView2 replaces it per site (YouTube gets one
/// without the `Edg/` token), so the WebView2 settings value is wrong here: ask the page.
async fn page_user_agent(window: &WebviewWindow) -> Result<String, String> {
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));
    window
        .eval_with_callback("navigator.userAgent", move |result| {
            if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
                let _ = sender.send(result);
            }
        })
        .map_err(|e| e.to_string())?;
    let raw = tokio::time::timeout(SCRIPT_TIMEOUT, receiver)
        .await
        .map_err(|_| "the login window did not report its User-Agent".to_owned())?
        .map_err(|_| "the login window closed while reading its User-Agent".to_owned())?;
    script_string(&raw)
}

#[cfg(test)]
mod tests {
    use tauri::webview::cookie::{Expiration, time::OffsetDateTime};

    use super::*;

    #[test]
    fn keeps_the_attributes_of_a_webview_cookie() {
        let mut web = WebCookie::new("SAPISID", "fake");
        web.set_domain(".youtube.com");
        web.set_path("/");
        web.set_secure(true);
        web.set_http_only(true);
        web.set_expires(OffsetDateTime::from_unix_timestamp(1_900_000_000).unwrap());

        let cookie = convert(&web);

        // `Cookie::domain()` hides the leading dot, which decides whether subdomains match.
        assert_eq!(web.domain(), Some("youtube.com"));
        assert_eq!(cookie.domain, ".youtube.com");
        assert_eq!(
            (
                cookie.name.as_str(),
                cookie.value.as_str(),
                cookie.path.as_str()
            ),
            ("SAPISID", "fake", "/")
        );
        assert!(cookie.secure && cookie.http_only);
        assert_eq!(cookie.expires, Some(1_900_000_000));
    }

    #[test]
    fn a_host_prefixed_cookie_stays_host_only() {
        let mut web = WebCookie::new("__Host-Example", "fake");
        web.set_domain("youtube.com");

        assert_eq!(convert(&web).domain, "youtube.com");
    }

    #[test]
    fn only_youtube_music_over_https_counts_as_the_music_page() {
        let is_music = |url: &str| is_music_page(&Url::parse(url).unwrap());

        assert!(is_music("https://music.youtube.com/library"));
        assert!(!is_music("http://music.youtube.com/"));
        assert!(!is_music(
            "https://accounts.google.com/v3/signin/identifier"
        ));
        assert!(!is_music("https://music.youtube.com.example.org/"));
    }

    #[test]
    fn reads_the_user_agent_from_a_script_result() {
        assert_eq!(
            script_string(r#""Mozilla/5.0 Fake""#),
            Ok("Mozilla/5.0 Fake".to_owned())
        );
    }

    #[test]
    fn rejects_a_script_result_that_is_not_a_non_empty_string() {
        for raw in ["", "null", "{}", r#""""#] {
            assert!(script_string(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn a_session_cookie_has_no_expiry() {
        let mut web = WebCookie::new("YSC", "fake");
        web.set_domain("music.youtube.com");
        web.set_expires(Expiration::Session);

        let cookie = convert(&web);

        assert_eq!(cookie.domain, "music.youtube.com");
        assert_eq!(cookie.path, "/");
        assert_eq!(cookie.expires, None);
    }
}
