//! Fakes shared by the unit tests. No test ever talks to SoundCloud.

use std::sync::Mutex;

use tokio::time::Instant;

use crate::{
    error::Error,
    http::{Http, Identity, Request, Response},
    secret::Secret,
    session::{Session, SessionStore},
};

pub const TOKEN: &str = "2-000000-000000000-FAKEtokenFAKE";
pub const CLIENT_ID: &str = "FAKEaaaaFAKEaaaaFAKEaaaaFAKE0001";
pub const NEW_CLIENT_ID: &str = "FAKEbbbbFAKEbbbbFAKEbbbbFAKE0002";

pub fn identity() -> Identity {
    Identity {
        user_agent: "TestAgent/1.0".to_string(),
        language: Some("en-US".to_string()),
    }
}

pub fn session(client_id: Option<&str>) -> Session {
    Session {
        oauth_token: Secret::new(TOKEN),
        client_id: client_id.map(Secret::new),
        identity: identity(),
        expired: false,
    }
}

pub fn reply(status: u16, body: impl Into<String>) -> Result<Response, Error> {
    Ok(Response {
        status,
        retry_after: None,
        body: body.into(),
    })
}

/// One recorded request.
pub struct Seen {
    pub url: String,
    pub authorization: Option<String>,
    pub user_agent: Option<String>,
    pub at: Instant,
}

type Handler = Box<dyn FnMut(&Request) -> Result<Response, Error> + Send>;

pub struct FakeHttp {
    handler: Mutex<Handler>,
    seen: Mutex<Vec<Seen>>,
}

impl FakeHttp {
    pub fn new(handler: impl FnMut(&Request) -> Result<Response, Error> + Send + 'static) -> Self {
        Self {
            handler: Mutex::new(Box::new(handler)),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Answers requests with the given replies, in order.
    pub fn sequence(replies: Vec<Result<Response, Error>>) -> Self {
        let mut replies = replies.into_iter();
        Self::new(move |request| {
            replies
                .next()
                .unwrap_or_else(|| panic!("unexpected request: {request:?}"))
        })
    }

    pub fn urls(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.url.clone())
            .collect()
    }

    pub fn seen<T>(&self, read: impl Fn(&Seen) -> T) -> Vec<T> {
        self.seen.lock().unwrap().iter().map(read).collect()
    }
}

impl Http for FakeHttp {
    async fn get(&self, request: Request) -> Result<Response, Error> {
        self.seen.lock().unwrap().push(Seen {
            url: request.url.to_string(),
            authorization: request.header("authorization").map(str::to_string),
            user_agent: request.header("user-agent").map(str::to_string),
            at: Instant::now(),
        });
        (self.handler.lock().unwrap())(&request)
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<Option<Session>>);

impl MemoryStore {
    pub fn with(session: Session) -> Self {
        Self(Mutex::new(Some(session)))
    }
}

impl SessionStore for MemoryStore {
    fn load(&self) -> Result<Option<Session>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, session: &Session) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }

    fn clear(&self) -> Result<(), Error> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}
