//! The seam between the Spotify flow and the network.

use std::{future::Future, time::Duration};

use reqwest::{Url, header, redirect::Policy};
use serde_json::Value;

use super::secret::Secret;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub enum HeaderValue {
    Plain(String),
    Secret(Secret),
}

impl HeaderValue {
    pub fn expose(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::Secret(value) => value.expose(),
        }
    }
}

/// No `Debug`: the URL of the token request carries the TOTP codes.
pub struct HttpRequest {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(&'static str, HeaderValue)>,
    pub body: Option<Value>,
}

pub struct HttpResponse {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

/// A transport failure, described without the URL or any header.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TransportError(pub String);

pub trait Transport: Send + Sync {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send;
}

const TIMEOUT: Duration = Duration::from_secs(20);

/// HTTPS only, and no redirects, so credentials never follow one to another host.
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, TransportError> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .timeout(TIMEOUT)
            .build()
            .map_err(describe)?;
        Ok(Self { client })
    }
}

fn describe(error: reqwest::Error) -> TransportError {
    TransportError(error.without_url().to_string())
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let mut builder = match request.method {
            Method::Get => self.client.get(request.url),
            Method::Post => self.client.post(request.url),
        };
        for (name, value) in &request.headers {
            let mut header = header::HeaderValue::from_str(value.expose())
                .map_err(|_| TransportError(format!("invalid {name} header")))?;
            header.set_sensitive(matches!(value, HeaderValue::Secret(_)));
            builder = builder.header(*name, header);
        }
        if let Some(body) = request.body {
            builder = builder.body(body.to_string());
        }
        let response = builder.send().await.map_err(describe)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await.map_err(describe)?;
        Ok(HttpResponse {
            status,
            retry_after,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_plain_http_without_leaking_the_request() {
        let request = HttpRequest {
            method: Method::Get,
            url: Url::parse("http://spotify.invalid/api/token?totp=123456").unwrap(),
            headers: vec![("cookie", HeaderValue::Secret(Secret::new("sp_dc=FAKE")))],
            body: None,
        };
        let Err(error) = ReqwestTransport::new().unwrap().send(request).await else {
            panic!("plain http must be refused");
        };
        let shown = format!("{error} {error:?}");
        for hidden in ["123456", "totp", "FAKE", "spotify.invalid"] {
            assert!(!shown.contains(hidden), "{shown}");
        }
    }
}
