//! The outbound HTTP seam. Tests fake it, the app plugs in reqwest.

use std::{fmt, future::Future};

use url::Url;

use crate::{error::Error, secret::Secret};

/// The one client identity: the WebView2 build that did the login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user_agent: String,
    pub language: Option<String>,
}

pub struct Request {
    pub url: Url,
    pub headers: Vec<(&'static str, String)>,
}

impl Request {
    /// A public page or script. Never carries credentials.
    pub fn public(url: Url, identity: &Identity) -> Self {
        let mut headers = vec![("User-Agent", identity.user_agent.clone())];
        if let Some(language) = &identity.language {
            headers.push(("Accept-Language", language.clone()));
        }
        headers.push(("Accept", "*/*".to_string()));
        Self { url, headers }
    }

    /// An authenticated request, shaped like the web client's.
    pub fn api(url: Url, identity: &Identity, token: &Secret) -> Self {
        let mut request = Self::public(url, identity);
        request.headers.extend([
            ("Origin", "https://soundcloud.com".to_string()),
            ("Referer", "https://soundcloud.com/".to_string()),
            ("Authorization", format!("OAuth {}", token.expose())),
        ]);
        request
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

// The query holds the client_id and the headers hold the token: print neither.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.headers.iter().map(|(name, _)| *name).collect();
        write!(
            f,
            "Request({}://{}{}, headers: {names:?})",
            self.url.scheme(),
            self.url.host_str().unwrap_or_default(),
            self.url.path()
        )
    }
}

#[derive(Clone, Default)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

// Bodies can hold account data.
impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Response({}, {} byte body)",
            self.status,
            self.body.len()
        )
    }
}

pub trait Http: Send + Sync {
    fn get(&self, request: Request) -> impl Future<Output = Result<Response, Error>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{CLIENT_ID, TOKEN, identity};

    fn api_url() -> Url {
        Url::parse(&format!(
            "https://api-v2.soundcloud.com/me?client_id={CLIENT_ID}"
        ))
        .unwrap()
    }

    #[test]
    fn only_api_requests_carry_the_token() {
        let public = Request::public(api_url(), &identity());
        let api = Request::api(api_url(), &identity(), &Secret::new(TOKEN));
        assert_eq!(public.header("authorization"), None);
        assert_eq!(
            api.header("authorization"),
            Some(&*format!("OAuth {TOKEN}"))
        );
        for request in [public, api] {
            assert_eq!(request.header("user-agent"), Some("TestAgent/1.0"));
            assert_eq!(request.header("accept-language"), Some("en-US"));
        }
    }

    #[test]
    fn debug_output_hides_the_query_and_header_values() {
        let api = Request::api(api_url(), &identity(), &Secret::new(TOKEN));
        let debug = format!("{api:?}");
        assert!(debug.contains("api-v2.soundcloud.com/me"));
        assert!(!debug.contains(TOKEN) && !debug.contains(CLIENT_ID));
    }
}
