//! The HTTP seam. Requests carry credentials or signed URLs, so `Debug` redacts them.

use std::future::Future;

#[derive(Clone, PartialEq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Empty for a `GET`.
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Header names only: values hold cookies, and a stream URL holds the client IP.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("Request")
            .field("header_names", &names)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

/// The message must not contain the request URL or any header value.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("network request failed: {0}")]
pub struct TransportError(pub String);

pub trait Transport: Send + Sync {
    /// One `POST`. Redirects are not followed, so credentials go nowhere else.
    fn post(
        &self,
        request: Request,
    ) -> impl Future<Output = Result<Response, TransportError>> + Send;

    /// One `GET` whose body is thrown away: only the status matters.
    fn probe(
        &self,
        request: Request,
    ) -> impl Future<Output = Result<Response, TransportError>> + Send;
}

#[cfg(test)]
pub(crate) mod fake {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    /// Replays queued answers and records what was sent.
    #[derive(Default)]
    pub struct FakeTransport {
        pub answers: Mutex<VecDeque<Result<Response, TransportError>>>,
        pub posts: Mutex<Vec<Request>>,
        pub probes: Mutex<Vec<Request>>,
    }

    impl FakeTransport {
        pub fn answering(answers: impl IntoIterator<Item = Response>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().map(Ok).collect()),
                ..Self::default()
            }
        }

        pub fn sent(&self) -> usize {
            self.posts.lock().unwrap().len() + self.probes.lock().unwrap().len()
        }

        fn next(&self) -> Result<Response, TransportError> {
            let answer = self.answers.lock().unwrap().pop_front();
            answer.expect("the code sent more requests than the test allows")
        }
    }

    pub fn response(status: u16, body: &str) -> Response {
        Response {
            status,
            retry_after: None,
            body: body.into(),
        }
    }

    impl Transport for FakeTransport {
        async fn post(&self, request: Request) -> Result<Response, TransportError> {
            self.posts.lock().unwrap().push(request);
            self.next()
        }

        async fn probe(&self, request: Request) -> Result<Response, TransportError> {
            self.probes.lock().unwrap().push(request);
            self.next()
        }
    }
}
