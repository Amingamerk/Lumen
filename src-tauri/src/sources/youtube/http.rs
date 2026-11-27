//! The real transport: no redirects, no retries, no cookie store, no referrer.

use std::error::Error as StdError;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{HeaderName, LOCATION, RETRY_AFTER};

use super::transport::{Request, Response, Transport, TransportError};

const TIMEOUT: Duration = Duration::from_secs(20);

/// Resolves a host name.
type Lookup = fn(&str) -> io::Result<Vec<SocketAddr>>;

fn system_lookup(host: &str) -> io::Result<Vec<SocketAddr>> {
    // The port is replaced by the one in the URL.
    Ok((host, 0).to_socket_addrs()?.collect())
}

/// A name lookup failure, marked so it can be told apart from a TLS failure:
/// reqwest calls both "connect" errors.
#[derive(Debug, thiserror::Error)]
#[error("name lookup failed")]
struct LookupFailed;

struct Resolver(Lookup);

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let (lookup, host) = (self.0, name.as_str().to_owned());
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || lookup(&host)).await {
                Ok(Ok(found)) => Ok(Box::new(found.into_iter()) as Addrs),
                _ => Err(LookupFailed.into()),
            }
        })
    }
}

pub struct HttpTransport {
    timeout: Duration,
    lookup: Lookup,
    /// Built on the first request, so starting the app costs nothing.
    client: OnceLock<reqwest::Client>,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::with(TIMEOUT, system_lookup)
    }
}

impl HttpTransport {
    fn with(timeout: Duration, lookup: Lookup) -> Self {
        Self {
            timeout,
            lookup,
            client: OnceLock::new(),
        }
    }

    fn client(&self) -> Result<&reqwest::Client, TransportError> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            // Credentials go to the address asked for and nowhere else.
            .redirect(reqwest::redirect::Policy::none())
            // Every request is one the gate allowed.
            .retry(reqwest::retry::never())
            // The web-client headers are set per request, so add none here.
            .referer(false)
            .dns_resolver(Resolver(self.lookup))
            // The system proxy stays on (no `no_proxy()`), so requests leave
            // by the same path as the login.
            .build()
            .map_err(|_| TransportError::Other)?;
        Ok(self.client.get_or_init(|| client))
    }
}

/// Keeps the kind of a failure and drops the rest: reqwest errors hold the URL.
fn classify(error: reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout
    } else if error.is_connect() && never_arrived(&error) {
        TransportError::Connect
    } else {
        TransportError::Other
    }
}

/// Whether the failure proves that nothing reached the service.
fn never_arrived(error: &reqwest::Error) -> bool {
    std::iter::successors(error.source(), |&cause| cause.source()).any(|cause| {
        cause.is::<LookupFailed>()
            || cause.downcast_ref::<io::Error>().is_some_and(|io| {
                matches!(
                    io.kind(),
                    ErrorKind::ConnectionRefused
                        | ErrorKind::HostUnreachable
                        | ErrorKind::NetworkUnreachable
                        | ErrorKind::NetworkDown
                )
            })
    })
}

impl Transport for HttpTransport {
    async fn post(&self, request: Request) -> Result<Response, TransportError> {
        let builder = request.headers.into_iter().fold(
            self.client()?.post(&request.url),
            |builder, (name, value)| builder.header(name, value),
        );
        let reply = builder.body(request.body).send().await.map_err(classify)?;
        let header = |name: HeaderName| {
            let value = reply.headers().get(name)?.to_str().ok()?;
            Some(value.to_owned())
        };
        let status = reply.status().as_u16();
        let (retry_after, location) = (header(RETRY_AFTER), header(LOCATION));
        let body = match reply.text().await {
            Ok(body) => body,
            // A refusal still counts when its text is lost.
            Err(_) if status != 200 => String::new(),
            Err(error) => return Err(classify(error)),
        };
        Ok(Response {
            status,
            retry_after,
            location,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    use super::*;

    /// A one-shot local server. `answer` gets the connection; the address and
    /// what `answer` returns come back.
    fn serve<T: Send + 'static>(
        answer: impl FnOnce(TcpStream) -> T + Send + 'static,
    ) -> (String, mpsc::Receiver<T>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = sender.send(answer(stream));
        });
        (address, receiver)
    }

    /// Reads the headers and the test body, answers `reply` and hands back what
    /// it read.
    fn replying(reply: &'static str) -> (String, mpsc::Receiver<String>) {
        serve(move |mut stream| {
            let mut received = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                received.extend_from_slice(&buffer[..read]);
                let done = String::from_utf8_lossy(&received).ends_with("marker-body");
                if done || read == 0 {
                    break;
                }
            }
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&received).into_owned()
        })
    }

    fn request(url: String) -> Request {
        Request {
            url,
            headers: vec![("Cookie", "marker-cookie".to_owned())],
            body: "marker-body".to_owned(),
        }
    }

    #[test]
    fn a_request_prints_only_its_header_names() {
        let printed = format!(
            "{:?}",
            Request {
                url: "https://marker.invalid/".to_owned(),
                headers: vec![
                    ("Cookie", "marker-cookie".to_owned()),
                    ("Authorization", "marker-hash".to_owned()),
                ],
                body: "marker-body".to_owned(),
            }
        );

        assert!(printed.contains("Cookie"), "{printed}");
        assert!(printed.contains("Authorization"), "{printed}");
        assert!(!printed.contains("marker"), "{printed}");
    }

    #[test]
    fn a_response_prints_only_its_status() {
        let printed = format!(
            "{:?}",
            Response {
                status: 302,
                retry_after: None,
                location: Some("marker-location".to_owned()),
                body: "marker-body".to_owned(),
            }
        );

        assert!(printed.contains("302"), "{printed}");
        assert!(!printed.contains("marker"), "{printed}");
    }

    #[test]
    fn the_client_follows_the_system_proxy_like_the_login_window() {
        let transport = HttpTransport::default();

        // `no_proxy()` would leave the client without any proxy matcher.
        let client = format!("{:?}", transport.client().unwrap());
        assert!(client.contains("proxies"), "{client}");
        // Without this feature reqwest only reads the proxy environment
        // variables, not the Windows settings.
        let reqwest = include_str!("../../../Cargo.toml")
            .lines()
            .find(|line| line.starts_with("reqwest = "))
            .unwrap();
        assert!(reqwest.contains("\"system-proxy\""), "{reqwest}");
    }

    #[tokio::test]
    async fn posts_headers_and_body_and_returns_the_reply() {
        let (address, received) = replying(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 12\r\nLocation: /later\r\n\
             Content-Length: 4\r\nConnection: close\r\n\r\nslow",
        );

        let response = HttpTransport::default()
            .post(request(format!("http://{address}/path?q=1")))
            .await
            .unwrap();

        assert!(
            response
                == Response {
                    status: 429,
                    retry_after: Some("12".to_owned()),
                    location: Some("/later".to_owned()),
                    body: "slow".to_owned(),
                }
        );
        let received = received.recv().unwrap();
        assert!(
            received.starts_with("POST /path?q=1 HTTP/1.1"),
            "{received}"
        );
        assert!(received.contains("cookie: marker-cookie"), "{received}");
        assert!(!received.contains("referer:"), "{received}");
        assert!(received.ends_with("\r\n\r\nmarker-body"), "{received}");
    }

    #[tokio::test]
    async fn returns_a_redirect_without_following_it() {
        let elsewhere = TcpListener::bind("127.0.0.1:0").unwrap();
        elsewhere.set_nonblocking(true).unwrap();
        let target = format!("http://{}/elsewhere", elsewhere.local_addr().unwrap());
        let reply = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
        );
        let (address, _received) = replying(reply.leak());

        let response = HttpTransport::default()
            .post(request(format!("http://{address}/")))
            .await
            .unwrap();

        assert_eq!(response.status, 302);
        assert_eq!(response.location, Some(target));
        assert!(elsewhere.accept().is_err(), "the redirect was followed");
    }

    #[tokio::test]
    async fn a_refusal_whose_body_breaks_off_still_reports_its_status() {
        let (address, _received) = replying(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 30\r\nContent-Length: 50\r\n\
             Connection: close\r\n\r\ncut",
        );

        let response = HttpTransport::default()
            .post(request(format!("http://{address}/")))
            .await
            .unwrap();

        assert_eq!(response.status, 429);
        assert_eq!(response.retry_after.as_deref(), Some("30"));
    }

    /// The error of one post, after checking that it says nothing about it.
    async fn failure(transport: HttpTransport, url: String) -> TransportError {
        let error = transport.post(request(url.clone())).await.unwrap_err();
        let shown = format!("{error} {error:?}");
        for secret in ["marker", "127.0.0.1", "http", url.as_str()] {
            assert!(!shown.contains(secret), "{shown}");
        }
        error
    }

    #[tokio::test]
    async fn a_refused_connection_never_reached_the_service() {
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();

        let error = failure(
            HttpTransport::default(),
            format!("http://{closed}/marker-path"),
        )
        .await;

        assert_eq!(error, TransportError::Connect);
    }

    #[tokio::test]
    async fn a_name_that_does_not_resolve_never_reached_the_service() {
        let transport = HttpTransport::with(TIMEOUT, |_| Err(std::io::Error::other("marker")));

        let error = failure(transport, "https://marker.invalid/".to_owned()).await;

        assert_eq!(error, TransportError::Connect);
    }

    #[tokio::test]
    async fn a_server_that_never_answers_is_a_timeout() {
        let (address, _held) = serve(|stream| stream);
        let transport = HttpTransport::with(Duration::from_millis(200), system_lookup);

        let error = failure(transport, format!("http://{address}/")).await;

        assert_eq!(error, TransportError::Timeout);
    }

    #[tokio::test]
    async fn a_failed_tls_handshake_is_not_a_connect_error() {
        let (address, _received) = serve(|mut stream| {
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(b"this is not a TLS server\r\n\r\n");
        });

        let error = failure(HttpTransport::default(), format!("https://{address}/")).await;

        assert_eq!(error, TransportError::Other);
    }

    #[tokio::test]
    async fn a_dropped_connection_is_not_a_connect_error() {
        for scheme in ["http", "https"] {
            // Closing with unread bytes resets the connection.
            let (address, _received) = serve(|mut stream| {
                let _ = stream.read(&mut [0; 1]);
            });

            let error = failure(HttpTransport::default(), format!("{scheme}://{address}/")).await;

            assert_eq!(error, TransportError::Other, "{scheme}");
        }
    }
}
