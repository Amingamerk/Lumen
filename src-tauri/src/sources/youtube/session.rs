//! The one captured YouTube Music session.

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use super::cookies::Cookie;

/// The one origin a session is captured on.
pub const ORIGIN: &str = "https://music.youtube.com";
const HOST: &str = "music.youtube.com";
/// Longer than any real User-Agent or language tag.
const MAX_HEADER_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// youtube.com cookies only.
    pub cookies: Vec<Cookie>,
    /// The `navigator.userAgent` of the login page: the one device identity of
    /// every later request.
    pub user_agent: String,
    /// The `navigator.language` of the login page.
    pub language: String,
    /// Set once YouTube rejects the session. It is never sent again.
    pub expired: bool,
}

/// What the login window's page says about itself, read in one script so the
/// values belong to the origin beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct PageIdentity {
    pub origin: String,
    pub user_agent: String,
    pub language: String,
}

impl Session {
    /// A name for this login that survives a restart: a digest of its cookies, so
    /// nothing of them can be read back. Another login has another one.
    pub fn login_id(&self) -> String {
        let mut digest = Sha1::new();
        for cookie in &self.cookies {
            for part in [&cookie.domain, &cookie.name, &cookie.value] {
                digest.update(part.as_bytes());
                digest.update([0]);
            }
        }
        digest
            .finalize()
            .iter()
            .fold(String::new(), |hex, byte| hex + &format!("{byte:02x}"))
    }

    /// The session of a login window, once sign-in is complete.
    pub fn capture(page: PageIdentity, cookies: Vec<Cookie>) -> Option<Self> {
        if page.origin != ORIGIN || !is_usable(&page.user_agent) || !is_usable(&page.language) {
            return None;
        }
        let cookies: Vec<Cookie> = cookies.into_iter().filter(Cookie::is_youtube).collect();
        is_signed_in(&cookies).then_some(Self {
            cookies,
            user_agent: page.user_agent,
            language: page.language,
            expired: false,
        })
    }
}

/// True once the cookies of a complete sign-in apply to music.youtube.com.
/// yt-dlp treats cookies without `LOGIN_INFO` as signed out, so both count.
pub fn is_signed_in(cookies: &[Cookie]) -> bool {
    let has = |name: &str| {
        cookies
            .iter()
            .any(|c| c.name == name && !c.value.is_empty() && c.applies_to(HOST))
    };
    (has("SAPISID") || has("__Secure-3PAPISID")) && has("LOGIN_INFO")
}

/// Both values end up in request headers.
fn is_usable(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_HEADER_BYTES
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
pub mod fake {
    use super::*;

    pub fn cookie(name: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.to_owned(),
            value: format!("fake-{name}"),
            domain: domain.to_owned(),
            path: "/".to_owned(),
            secure: true,
            http_only: false,
            expires: None,
        }
    }

    pub fn signed_in_cookies() -> Vec<Cookie> {
        vec![
            cookie("SAPISID", ".youtube.com"),
            cookie("LOGIN_INFO", ".youtube.com"),
        ]
    }

    pub fn music_page() -> PageIdentity {
        PageIdentity {
            origin: "https://music.youtube.com".to_owned(),
            user_agent: "FakeAgent/1.0".to_owned(),
            language: "da-DK".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::*, *};

    #[test]
    fn captures_the_cookies_and_the_identity_of_the_music_page() {
        let session = Session::capture(music_page(), signed_in_cookies());

        assert_eq!(
            session,
            Some(Session {
                cookies: signed_in_cookies(),
                user_agent: "FakeAgent/1.0".to_owned(),
                language: "da-DK".to_owned(),
                expired: false,
            })
        );
    }

    #[test]
    fn needs_both_a_sapisid_and_login_info() {
        let captured = |names: &[&str]| {
            let cookies = names.iter().map(|n| cookie(n, ".youtube.com")).collect();
            Session::capture(music_page(), cookies).is_some()
        };

        assert!(captured(&["SAPISID", "LOGIN_INFO"]));
        assert!(captured(&["__Secure-3PAPISID", "LOGIN_INFO"]));
        assert!(!captured(&["SAPISID"]));
        assert!(!captured(&["__Secure-3PAPISID"]));
        assert!(!captured(&["LOGIN_INFO"]));
        assert!(!captured(&["SID", "LOGIN_INFO"]));
        assert!(!captured(&[]));
    }

    #[test]
    fn an_empty_cookie_does_not_count() {
        for empty in ["SAPISID", "LOGIN_INFO"] {
            let mut cookies = signed_in_cookies();
            for cookie in &mut cookies {
                if cookie.name == empty {
                    cookie.value.clear();
                }
            }

            assert_eq!(Session::capture(music_page(), cookies), None, "{empty}");
        }
    }

    #[test]
    fn the_cookies_must_exist_on_music_youtube_com() {
        let captured = |domain: &str| {
            let cookies = vec![cookie("SAPISID", domain), cookie("LOGIN_INFO", domain)];
            Session::capture(music_page(), cookies).is_some()
        };

        assert!(captured(".youtube.com"));
        assert!(captured("music.youtube.com"));
        assert!(!captured("youtube.com"));
        assert!(!captured("www.youtube.com"));
        assert!(!captured(".google.com"));
    }

    #[test]
    fn captures_nothing_away_from_music_youtube_com() {
        for origin in [
            "https://accounts.google.com",
            "https://consent.youtube.com",
            "https://www.youtube.com",
            "http://music.youtube.com",
            "https://music.youtube.com.example.org",
            "null",
            "",
        ] {
            let page = PageIdentity {
                origin: origin.to_owned(),
                ..music_page()
            };

            assert_eq!(
                Session::capture(page, signed_in_cookies()),
                None,
                "{origin}"
            );
        }
    }

    #[test]
    fn keeps_only_youtube_com_cookies() {
        let mut cookies = signed_in_cookies();
        let kept = [
            cookie("PREF", "music.youtube.com"),
            cookie("__Host-1PLSID", "youtube.com"),
        ];
        cookies.extend(kept.clone());
        cookies.extend([
            cookie("SID", ".google.com"),
            cookie("x", "notyoutube.com"),
            cookie("y", ".youtube.com.example.org"),
            cookie("z", ""),
        ]);

        let session = Session::capture(music_page(), cookies).unwrap();

        assert_eq!(
            session.cookies,
            [signed_in_cookies(), kept.to_vec()].concat()
        );
    }

    #[test]
    fn refuses_an_unusable_user_agent_or_language() {
        let long = "a".repeat(513);
        for bad in ["", "  ", "Agent\r\nX-Evil: 1", long.as_str()] {
            for page in [
                PageIdentity {
                    user_agent: bad.to_owned(),
                    ..music_page()
                },
                PageIdentity {
                    language: bad.to_owned(),
                    ..music_page()
                },
            ] {
                assert_eq!(Session::capture(page, signed_in_cookies()), None, "{bad:?}");
            }
        }
    }

    #[test]
    fn debug_output_never_shows_cookie_values() {
        let session = Session::capture(music_page(), signed_in_cookies()).unwrap();

        assert!(!format!("{session:?}").contains("fake-"));
    }
}
