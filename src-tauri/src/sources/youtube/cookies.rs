//! The youtube.com cookies of a session. Their values are secrets: they never
//! appear in `Debug` output.

use serde::{Deserialize, Serialize};
use tauri::webview::Cookie as WebCookie;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// A leading dot means the cookie also applies to subdomains.
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds. `None` for a session cookie.
    pub expires: Option<i64>,
}

impl Cookie {
    /// Whether it belongs to youtube.com or one of its subdomains.
    pub fn is_youtube(&self) -> bool {
        let domain = self.domain.strip_prefix('.').unwrap_or(&self.domain);
        domain == "youtube.com" || domain.ends_with(".youtube.com")
    }

    /// Whether a browser would send it to `host`.
    pub fn applies_to(&self, host: &str) -> bool {
        match self.domain.strip_prefix('.') {
            Some(suffix) => host
                .strip_suffix(suffix)
                .is_some_and(|sub| sub.is_empty() || sub.ends_with('.')),
            None => host == self.domain,
        }
    }
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

impl From<&WebCookie<'_>> for Cookie {
    fn from(cookie: &WebCookie<'_>) -> Self {
        let name = cookie.name();
        // `domain()` drops the leading dot. YouTube's site-wide cookies are the
        // ones on the bare domain, so those get it back. A `__Host-` cookie is
        // bound to one host by definition.
        let domain = match cookie.domain() {
            Some("youtube.com") if !name.starts_with("__Host-") => ".youtube.com",
            domain => domain.unwrap_or_default(),
        };
        Self {
            name: name.to_owned(),
            value: cookie.value().to_owned(),
            domain: domain.to_owned(),
            path: cookie.path().unwrap_or("/").to_owned(),
            secure: cookie.secure().unwrap_or(false),
            http_only: cookie.http_only().unwrap_or(false),
            expires: cookie.expires_datetime().map(|at| at.unix_timestamp()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tauri::webview::cookie::Expiration;
    use tauri::webview::cookie::time::OffsetDateTime;

    use super::*;

    fn web(name: &'static str, domain: &'static str) -> WebCookie<'static> {
        let mut web = WebCookie::new(name, "fake-value");
        web.set_domain(domain);
        web
    }

    #[test]
    fn keeps_the_attributes_of_a_webview_cookie() {
        let mut cookie = web("SAPISID", ".youtube.com");
        cookie.set_path("/youtubei");
        cookie.set_secure(true);
        cookie.set_http_only(true);
        cookie.set_expires(OffsetDateTime::from_unix_timestamp(1_900_000_000).unwrap());

        let kept = Cookie::from(&cookie);

        assert!(
            kept == Cookie {
                name: "SAPISID".to_owned(),
                value: "fake-value".to_owned(),
                domain: ".youtube.com".to_owned(),
                path: "/youtubei".to_owned(),
                secure: true,
                http_only: true,
                expires: Some(1_900_000_000),
            },
            "{kept:?} expires {:?}",
            kept.expires
        );
    }

    #[test]
    fn a_session_cookie_has_no_expiry() {
        let mut cookie = web("YSC", "music.youtube.com");
        cookie.set_expires(Expiration::Session);

        let kept = Cookie::from(&cookie);

        assert_eq!(kept.expires, None);
        assert_eq!(kept.domain, "music.youtube.com");
        assert_eq!(kept.path, "/");
    }

    #[test]
    fn restores_the_leading_dot_of_site_wide_cookies_only() {
        // `Cookie::domain()` hides the dot, which decides whether subdomains match.
        assert_eq!(web("SAPISID", ".youtube.com").domain(), Some("youtube.com"));

        for (name, domain, kept) in [
            ("SAPISID", ".youtube.com", ".youtube.com"),
            ("LOGIN_INFO", "youtube.com", ".youtube.com"),
            ("__Secure-3PAPISID", ".youtube.com", ".youtube.com"),
            ("__Host-1PLSID", "youtube.com", "youtube.com"),
            ("PREF", "music.youtube.com", "music.youtube.com"),
            ("SID", ".google.com", "google.com"),
        ] {
            let cookie = Cookie::from(&web(name, domain));

            assert_eq!(cookie.domain, kept, "{name} on {domain}");
        }
    }

    #[test]
    fn debug_output_never_shows_the_value() {
        let printed = format!("{:?}", Cookie::from(&web("SAPISID", "youtube.com")));

        assert!(printed.contains("SAPISID"), "{printed}");
        assert!(!printed.contains("fake-value"), "{printed}");
    }
}
