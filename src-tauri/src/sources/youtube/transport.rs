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

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use tokio::sync::oneshot;

    use super::*;

    type Answer = Result<Response, TransportError>;

    enum Queued {
        Now(Answer),
        Held(oneshot::Receiver<Answer>),
    }

    /// Replays queued answers and records what was sent. Clones share both.
    #[derive(Clone, Default)]
    pub struct FakeTransport {
        answers: Arc<Mutex<VecDeque<Queued>>>,
        sent: Arc<Mutex<Vec<Request>>>,
    }

    impl FakeTransport {
        pub fn answer(&self, answer: Answer) {
            self.answers.lock().unwrap().push_back(Queued::Now(answer));
        }

        /// Queues an answer that arrives when the test sends it.
        pub fn hold(&self) -> oneshot::Sender<Answer> {
            let (sender, receiver) = oneshot::channel();
            let held = Queued::Held(receiver);
            self.answers.lock().unwrap().push_back(held);
            sender
        }

        /// How many requests were sent.
        pub fn sent(&self) -> usize {
            self.sent.lock().unwrap().len()
        }

        /// The body of every request sent.
        pub fn bodies(&self) -> Vec<String> {
            let sent = self.sent.lock().unwrap();
            sent.iter().map(|request| request.body.clone()).collect()
        }

        pub fn urls(&self) -> Vec<String> {
            let sent = self.sent.lock().unwrap();
            sent.iter().map(|request| request.url.clone()).collect()
        }
    }

    impl Transport for FakeTransport {
        async fn post(&self, request: Request) -> Answer {
            self.sent.lock().unwrap().push(request);
            let queued = self.answers.lock().unwrap().pop_front();
            match queued.expect("the code sent more requests than the test allows") {
                Queued::Now(answer) => answer,
                Queued::Held(answer) => answer.await.expect("the test dropped the answer"),
            }
        }
    }
}
