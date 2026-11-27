//! The network seam. Requests carry credentials and replies may carry account
//! data, so neither prints them, and errors carry nothing at all.

use std::future::Future;

/// One `POST`.
pub struct Request {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

/// Header names only: the values hold the cookies and the hash.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.headers.iter().map(|(name, _)| name).collect();
        f.debug_struct("Request")
            .field("header_names", &names)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<String>,
    pub location: Option<String>,
    pub body: String,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// How a request failed, without the address or anything that was sent.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum TransportError {
    /// Nothing reached the service: the name did not resolve, the connection
    /// was refused or there was no route.
    #[error("could not connect")]
    Connect,
    #[error("the request timed out")]
    Timeout,
    /// Everything else, a failed TLS handshake and a reset included.
    #[error("the request failed")]
    Other,
}

pub trait Transport: Send + Sync {
    /// Sends `request` once. Redirects are returned, never followed.
    fn post(
        &self,
        request: Request,
    ) -> impl Future<Output = Result<Response, TransportError>> + Send;
}
