//! What the login window yields: the token cookie and the WebView2 identity.

use crate::{http::Identity, secret::Secret, session::parse_token};

pub const LOGIN_URL: &str = "https://soundcloud.com/signin";
pub const COOKIE_URL: &str = "https://soundcloud.com/";
pub const TOKEN_COOKIE: &str = "oauth_token";
/// Run in a webview, its result is read by [`identity_from_eval`].
pub const IDENTITY_SCRIPT: &str = "[navigator.userAgent, navigator.language]";

/// The token among `(name, value)` cookie pairs.
pub fn token_from_cookies<'a>(
    cookies: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<Secret> {
    cookies
        .into_iter()
        .find(|(name, _)| *name == TOKEN_COOKIE)
        .and_then(|(_, value)| parse_token(value).ok())
}

pub fn identity_from_eval(json: &str) -> Option<Identity> {
    let (user_agent, language): (String, Option<String>) = serde_json::from_str(json).ok()?;
    let header_safe =
        |s: &String| !s.is_empty() && s.chars().all(|c| c == ' ' || c.is_ascii_graphic());
    Some(Identity {
        user_agent: Some(user_agent).filter(header_safe)?,
        language: language.filter(header_safe),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TOKEN;

    #[test]
    fn takes_the_oauth_token_cookie() {
        let cookies = [("sc_anonymous_id", "1-2"), ("oauth_token", TOKEN)];
        assert_eq!(token_from_cookies(cookies), Some(Secret::new(TOKEN)));
    }

    #[test]
    fn ignores_missing_empty_or_unusable_cookies() {
        for cookies in [
            vec![("sc_anonymous_id", TOKEN)],
            vec![("oauth_token", "")],
            vec![("oauth_token", "a b")],
            vec![("x_oauth_token", TOKEN)],
        ] {
            assert_eq!(token_from_cookies(cookies), None);
        }
    }

    #[test]
    fn reads_the_webview_identity() {
        assert_eq!(
            identity_from_eval(r#"["Mozilla/5.0 (Windows NT 10.0) Edg/1","da-DK"]"#),
            Some(Identity {
                user_agent: "Mozilla/5.0 (Windows NT 10.0) Edg/1".into(),
                language: Some("da-DK".into()),
            })
        );
        assert_eq!(
            identity_from_eval(r#"["Agent",null]"#).map(|i| i.language),
            Some(None)
        );
    }

    #[test]
    fn rejects_an_identity_that_cannot_be_a_header() {
        for json in ["null", "", r#"["",null]"#, r#"["a\r\nX: 1","en"]"#, "{}"] {
            assert_eq!(identity_from_eval(json), None, "{json}");
        }
    }
}
