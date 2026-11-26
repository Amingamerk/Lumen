//! The real [`Http`] backend.

use std::time::Duration;

use reqwest::{Client, redirect::Policy};
use url::Url;

use crate::{
    error::Error,
    http::{Http, Request, Response},
};

const TIMEOUT: Duration = Duration::from_secs(30);

pub struct ReqwestHttp {
    public: Client,
    /// Follows no redirect, so the token only ever goes to the URL we built.
    authenticated: Client,
}

impl ReqwestHttp {
    pub fn new() -> Result<Self, Error> {
        let build = |policy| {
            Client::builder()
                .timeout(TIMEOUT)
                .referer(false)
                .redirect(policy)
                .build()
                .map_err(network)
        };
        Ok(Self {
            public: build(Policy::custom(|attempt| {
                if attempt.previous().len() <= 3 && may_follow(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))?,
            authenticated: build(Policy::none())?,
        })
    }
}

/// Whether an unauthenticated request may follow a redirect to this URL.
fn may_follow(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            ["soundcloud.com", "sndcdn.com"].iter().any(|domain| {
                host.strip_suffix(domain)
                    .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
            })
        })
}

/// reqwest errors name the URL, and ours carry the `client_id`, so it is removed.
fn network(error: reqwest::Error) -> Error {
    let error = error.without_url();
    let mut cause: &dyn std::error::Error = &error;
    while let Some(next) = cause.source() {
        cause = next;
    }
    let kind = if error.is_timeout() {
        "timed out".to_string()
    } else {
        cause.to_string()
    };
    Error::Network(kind)
}

impl Http for ReqwestHttp {
    async fn get(&self, request: Request) -> Result<Response, Error> {
        let client = if request.header("authorization").is_some() {
            &self.authenticated
        } else {
            &self.public
        };
        let mut builder = client.get(request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let response = builder.send().await.map_err(network)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response.text().await.map_err(network)?;
        Ok(Response {
            status,
            retry_after,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use super::*;
    use crate::{
        secret::Secret,
        testing::{CLIENT_ID, TOKEN, identity},
    };

    /// A one-shot local server. Returns its URL and the raw request it received.
    fn serve(reply: &'static str) -> (Url, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/path", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            sender.send(request).unwrap();
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (Url::parse(&url).unwrap(), receiver)
    }

    #[tokio::test]
    async fn sends_the_headers_and_maps_status_retry_after_and_body() {
        let (url, received) = serve(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 17\r\nContent-Length: 4\r\n\
             Connection: close\r\n\r\nslow",
        );
        let http = ReqwestHttp::new().unwrap();

        let response = http
            .get(Request::api(url, &identity(), &Secret::new(TOKEN)))
            .await
            .unwrap();

        assert_eq!(response.status, 429);
        assert_eq!(response.retry_after.as_deref(), Some("17"));
        assert_eq!(response.body, "slow");
        let sent = received.recv().unwrap().to_ascii_lowercase();
        assert!(sent.contains("user-agent: testagent/1.0"));
        assert!(sent.contains(&format!("authorization: oauth {TOKEN}").to_ascii_lowercase()));
    }

    #[tokio::test]
    async fn never_follows_a_redirect_with_the_token_or_to_a_foreign_host() {
        const REDIRECT: &str = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\n\
                                Content-Length: 0\r\nConnection: close\r\n\r\n";
        let http = ReqwestHttp::new().unwrap();
        let (url, _received) = serve(REDIRECT);
        let api = Request::api(url, &identity(), &Secret::new(TOKEN));
        assert_eq!(http.get(api).await.unwrap().status, 302);

        let (url, _received) = serve(REDIRECT);
        let public = Request::public(url, &identity());
        assert_eq!(http.get(public).await.unwrap().status, 302);
    }

    #[test]
    fn public_redirects_stay_on_https_soundcloud_hosts() {
        let follows = |u: &str| may_follow(&Url::parse(u).unwrap());
        assert!(follows("https://soundcloud.com/discover"));
        assert!(follows("https://www.soundcloud.com/"));
        assert!(follows("https://a-v2.sndcdn.com/assets/1.js"));
        assert!(!follows("http://soundcloud.com/"));
        assert!(!follows("https://soundcloud.com.evil.test/"));
        assert!(!follows("https://notsoundcloud.com/"));
    }

    #[tokio::test]
    async fn network_errors_do_not_name_the_url() {
        // Nothing listens on port 1, so this fails without leaving the machine.
        let url = Url::parse(&format!("http://127.0.0.1:1/me?client_id={CLIENT_ID}")).unwrap();
        let error = ReqwestHttp::new()
            .unwrap()
            .get(Request::api(url, &identity(), &Secret::new(TOKEN)))
            .await
            .unwrap_err();
        let text = format!("{error} {error:?}");
        assert!(matches!(error, Error::Network(_)));
        assert!(!text.contains(CLIENT_ID) && !text.contains(TOKEN), "{text}");
    }
}
