//! Which addresses artwork may be fetched from.

use reqwest::Url;

/// The hosts that serve artwork for the services Lumen knows. Exact names: a
/// wider match would let any subdomain someone controls through.
const HOSTS: &[&str] = &[
    // YouTube Music.
    "i.ytimg.com",
    "lh3.googleusercontent.com",
    "yt3.ggpht.com",
    "yt3.googleusercontent.com",
    // SoundCloud.
    "i1.sndcdn.com",
    // Spotify.
    "i.scdn.co",
];

/// Parses `address` and says whether artwork may be fetched from it: HTTPS, port
/// 443, no credentials and an allowlisted host.
pub fn allowed(address: &str) -> Option<Url> {
    let url = Url::parse(address).ok()?;
    is_allowed(&url).then_some(url)
}

/// The same check for an address already parsed, such as a redirect target.
pub fn is_allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
        && url.host_str().is_some_and(|host| HOSTS.contains(&host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_allowlisted_https_address_passes() {
        assert!(allowed("https://i.ytimg.com/vi/abc/hq720.jpg").is_some());
        assert!(allowed("https://lh3.googleusercontent.com/x=w544-h544").is_some());
        assert!(allowed("https://i.ytimg.com:443/vi/abc/hq720.jpg").is_some());
    }

    #[test]
    fn everything_else_is_refused() {
        for address in [
            "http://i.ytimg.com/vi/abc/hq720.jpg",
            "https://evil.example/a.jpg",
            "https://i.ytimg.com.evil.example/a.jpg",
            "https://evil.example/@i.ytimg.com/a.jpg",
            "https://i.ytimg.com@evil.example/a.jpg",
            "https://user:pass@i.ytimg.com/a.jpg",
            "https://i.ytimg.com:8443/a.jpg",
            "https://127.0.0.1/a.jpg",
            "https://localhost/a.jpg",
            "file:///C:/Windows/win.ini",
            "data:image/png;base64,AAAA",
            "//i.ytimg.com/a.jpg",
            "not a url",
            "",
        ] {
            assert!(allowed(address).is_none(), "{address}");
        }
    }
}
