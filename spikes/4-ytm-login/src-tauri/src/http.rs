//! The real transport. Errors are stripped of their URL, and redirects cannot carry a request
//! to a host we did not choose.

use std::time::Duration;

use reqwest::{header::RETRY_AFTER, redirect};

use crate::{
    resolver::is_googlevideo,
    transport::{Request, Response, Transport, TransportError},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 3;

pub struct HttpTransport {
    /// For credentialed calls: never follows a redirect.
    direct: reqwest::Client,
    /// For the stream probe: follows redirects between googlevideo hosts only.
    stream: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Result<Self, TransportError> {
        let client = |policy| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                .redirect(policy)
                // The browser headers are set per request, so add none of our own.
                .referer(false)
                .build()
                .map_err(safe)
        };
        let googlevideo_only = redirect::Policy::custom(|attempt| {
            if attempt.previous().len() <= MAX_REDIRECTS && is_googlevideo(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        Ok(Self {
            direct: client(redirect::Policy::none())?,
            stream: client(googlevideo_only)?,
        })
    }
}

/// reqwest errors print the URL unless it is removed.
fn safe(error: reqwest::Error) -> TransportError {
    TransportError(error.without_url().to_string())
}

async fn send(
    builder: reqwest::RequestBuilder,
    headers: Vec<(String, String)>,
) -> Result<reqwest::Response, TransportError> {
    headers
        .into_iter()
        .fold(builder, |builder, (name, value)| {
            builder.header(name, value)
        })
        .send()
        .await
        .map_err(safe)
}

fn head(response: &reqwest::Response) -> Response {
    Response {
        status: response.status().as_u16(),
        retry_after: response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        body: String::new(),
    }
}

impl Transport for HttpTransport {
    async fn post(&self, request: Request) -> Result<Response, TransportError> {
        let builder = self.direct.post(&request.url).body(request.body);
        let response = send(builder, request.headers).await?;
        let head = head(&response);
        let body = response.text().await.map_err(safe)?;
        Ok(Response { body, ..head })
    }

    async fn probe(&self, request: Request) -> Result<Response, TransportError> {
        let builder = self.stream.get(&request.url);
        let response = send(builder, request.headers).await?;
        // Dropping the response leaves the body unread.
        Ok(head(&response))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    use super::*;

    /// A one-shot local server: answers the first request with `reply` and hands back what it read.
    fn serve(reply: &'static str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 1024];
            // Read until the headers and the 4-byte test body have arrived.
            loop {
                let read = stream.read(&mut buffer).unwrap();
                received.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&received);
                let done = text
                    .split_once("\r\n\r\n")
                    .is_some_and(|(head, body)| head.starts_with("GET") || body.len() >= 4);
                if done || read == 0 {
                    break;
                }
            }
            stream.write_all(reply.as_bytes()).unwrap();
            sender
                .send(String::from_utf8_lossy(&received).into_owned())
                .unwrap();
        });
        (url, receiver)
    }

    fn request(url: &str, body: &str) -> Request {
        Request {
            url: url.into(),
            headers: vec![("X-Test".into(), "sent".into())],
            body: body.into(),
        }
    }

    const REDIRECT: &str = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\n\
        Content-Length: 0\r\nConnection: close\r\n\r\n";

    #[tokio::test]
    async fn posts_headers_and_body_and_returns_status_retry_after_and_body() {
        let (url, received) = serve(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 12\r\nContent-Length: 4\r\n\
             Connection: close\r\n\r\nslow",
        );

        let response = HttpTransport::new()
            .unwrap()
            .post(request(&url, "body"))
            .await
            .unwrap();

        assert_eq!(
            response,
            Response {
                status: 429,
                retry_after: Some("12".into()),
                body: "slow".into()
            }
        );
        let received = received.recv().unwrap().to_ascii_lowercase();
        assert!(received.starts_with("post / "), "{received}");
        assert!(received.contains("x-test: sent"), "{received}");
        assert!(received.ends_with("body"), "{received}");
    }

    #[tokio::test]
    async fn follows_no_redirect_with_credentials_or_off_googlevideo() {
        let transport = HttpTransport::new().unwrap();

        let (url, _) = serve(REDIRECT);
        let posted = transport.post(request(&url, "body")).await.unwrap();
        let (url, _) = serve(REDIRECT);
        let probed = transport.probe(request(&url, "")).await.unwrap();

        assert_eq!((posted.status, probed.status), (302, 302));
    }

    #[tokio::test]
    async fn a_probe_discards_the_body() {
        let (url, received) = serve(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nConnection: close\r\n\r\naudio",
        );

        let response = HttpTransport::new()
            .unwrap()
            .probe(request(&url, ""))
            .await
            .unwrap();

        assert_eq!((response.status, response.body.as_str()), (206, ""));
        assert!(received.recv().unwrap().starts_with("GET / "));
    }

    #[tokio::test]
    async fn errors_never_contain_the_url() {
        let transport = HttpTransport::new().unwrap();
        let url = "http://127.0.0.1:1/videoplayback?ip=203.0.113.7";

        for outcome in [
            transport.post(request(url, "x")).await,
            transport.probe(request(url, "")).await,
        ] {
            let error = outcome.unwrap_err();
            let shown = format!("{error} {error:?}");
            assert!(!shown.contains("203.0.113.7"), "{shown}");
            assert!(!shown.contains("videoplayback"), "{shown}");
        }
    }
}
