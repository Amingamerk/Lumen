//! Rules for the login window.

use reqwest::Url;

/// Spotify's own login page, returning to the web player.
pub const LOGIN_URL: &str =
    "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F";
/// Cookies are read for this URL.
pub const COOKIE_URL: &str = "https://open.spotify.com/";

/// True when the login window may navigate there.
pub fn allows_navigation(url: &Url) -> bool {
    url.scheme() == "https" || url.as_str() == "about:blank"
}

/// True for the cookie that proves a finished login.
pub fn is_session_cookie(name: &str, domain: Option<&str>, value: &str) -> bool {
    let domain = domain.map(|domain| domain.trim_start_matches('.'));
    let spotify = domain.is_some_and(|d| d == "spotify.com" || d.ends_with(".spotify.com"));
    name == "sp_dc" && spotify && !value.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_stays_on_https() {
        let allowed = |url: &str| allows_navigation(&Url::parse(url).unwrap());
        assert!(allowed(LOGIN_URL));
        // Social logins leave spotify.com and come back.
        assert!(allowed("https://accounts.google.com/o/oauth2/auth"));
        assert!(allowed("about:blank"));
        assert!(!allowed("http://accounts.spotify.com/login"));
        assert!(!allowed("spotify:track:1"));
        assert!(!allowed("file:///C:/secret"));
    }

    #[test]
    fn finds_only_spotifys_own_sp_dc() {
        let found = is_session_cookie;
        assert!(found("sp_dc", Some(".spotify.com"), "FAKE"));
        assert!(found("sp_dc", Some("open.spotify.com"), "FAKE"));
        assert!(!found("sp_dc", Some(".spotify.com"), ""));
        assert!(!found("sp_key", Some(".spotify.com"), "FAKE"));
        assert!(!found("sp_dc", Some(".evilspotify.com"), "FAKE"));
        assert!(!found("sp_dc", None, "FAKE"));
    }
}
