//! Authenticated resolve of one track with yt-dlp, then a small Range request on the result.
//! The signed stream URL (it holds the client IP) never leaves this module, and yt-dlp's own
//! output is reduced to fixed categories because it can quote that URL.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    future::Future,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::{
    clock::Clock,
    jar::CookieJarFile,
    session::Session,
    transport::{Request, Transport},
};

/// Longest `available_at` wait the spike accepts.
const MAX_WAIT_S: u64 = 60;
const YTDLP_TIMEOUT: Duration = Duration::from_secs(90);
/// No console window for the child process.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub enum Clients {
    /// yt-dlp picks. Signed in, some of its picks send their own fixed User-Agent.
    #[default]
    Default,
    /// Only clients without a built-in User-Agent, so `--user-agent` applies to them.
    PinnedUserAgent,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ResolveError {
    #[error("enter a track URL like https://music.youtube.com/watch?v=...")]
    BadUrl,
    #[error("could not write the temporary cookie file: {0}")]
    CookieFile(io::ErrorKind),
    #[error("could not run yt-dlp ({0}): is it on PATH?")]
    Spawn(io::ErrorKind),
    #[error("yt-dlp: YouTube rejected the session: log in again")]
    AuthRejected,
    #[error("yt-dlp: rate limited (HTTP 429)")]
    RateLimited { retry_after: Option<String> },
    #[error("yt-dlp: bot check or unusual-traffic response")]
    BotCheck,
    #[error("yt-dlp: no audio format was available for this account and client choice")]
    NoFormat,
    #[error("yt-dlp: this track is unavailable")]
    Unavailable,
    #[error("yt-dlp failed (exit code {0:?}); its output is not shown because it can hold URLs")]
    Failed(Option<i32>),
    #[error("yt-dlp returned no single direct stream URL")]
    UnexpectedOutput,
    #[error("the stream URL is not on https://*.googlevideo.com")]
    UnexpectedHost,
    #[error("the stream only becomes available in {0} s")]
    AvailableTooLate(u64),
    #[error("the range request failed: {0}")]
    Probe(String),
}

/// Everything about a resolved stream that is safe to show.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolveReport {
    pub format_id: String,
    pub codec: Option<String>,
    pub abr_kbps: Option<f64>,
    /// Total bitrate. Combined formats may give no audio bitrate.
    pub tbr_kbps: Option<f64>,
    /// `"none"` for an audio-only format.
    pub vcodec: Option<String>,
    pub audio_only: bool,
    pub protocol: Option<String>,
    /// The `c=` parameter of the stream URL, i.e. which YouTube client produced it.
    pub client: Option<String>,
    pub available_at: Option<u64>,
    pub waited_s: u64,
    pub expires_at: Option<u64>,
    pub resolve_ms: u64,
    pub user_agent_matches_login: bool,
    /// Whether yt-dlp ran Deno to solve a JavaScript challenge.
    pub deno_started: bool,
    /// HTTP status of `Range: bytes=0-1023`, normally 206.
    pub range_status: u16,
    /// Known yt-dlp warnings, as fixed labels.
    pub warnings: Vec<&'static str>,
}

/// The last playable stream, kept in memory for the player only. No `Debug` or `Serialize`, and
/// nothing outside this crate can read it.
#[derive(Clone)]
pub struct LastStream {
    url: String,
    /// Without credentials.
    headers: Vec<(String, String)>,
}

impl LastStream {
    pub(crate) fn new(url: String, headers: Vec<(String, String)>) -> Self {
        Self { url, headers }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
}

pub struct ProcessOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// The process seam.
pub trait YtDlp: Send + Sync {
    fn run(&self, args: Vec<OsString>) -> impl Future<Output = io::Result<ProcessOutput>> + Send;
}

/// The `yt-dlp` on PATH.
pub struct YtDlpProcess;

impl YtDlp for YtDlpProcess {
    async fn run(&self, args: Vec<OsString>) -> io::Result<ProcessOutput> {
        let mut command = tokio::process::Command::new("yt-dlp");
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        let output = tokio::time::timeout(YTDLP_TIMEOUT, command.output())
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        Ok(ProcessOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Accepts a YouTube Music track URL and rebuilds it with the video id only.
pub fn watch_url(input: &str) -> Result<String, ResolveError> {
    let url = Url::parse(input.trim()).map_err(|_| ResolveError::BadUrl)?;
    let on_music = url.scheme() == "https"
        && url.host_str() == Some("music.youtube.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/watch";
    let id = url
        .query_pairs()
        .find(|(name, _)| name == "v")
        .map(|(_, value)| value.into_owned())
        .filter(|id| {
            id.len() == 11
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        });
    match id {
        Some(id) if on_music => Ok(format!("https://music.youtube.com/watch?v={id}")),
        _ => Err(ResolveError::BadUrl),
    }
}

pub fn ytdlp_args(jar: &Path, user_agent: &str, url: &str, clients: Clients) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-J",
        // Debug lines say whether Deno ran. They stay in this module.
        "-v",
        "-f",
        // The fallback shows what a client set without audio-only formats would give.
        "bestaudio/best",
        "--no-playlist",
        // A user config file must not change what we send or how much we resolve.
        "--ignore-config",
        "--playlist-items",
        "1",
        "--cookies",
    ]
    .map(OsString::from)
    .into();
    args.push(jar.into());
    // Without this yt-dlp picks a random Chrome version for every process.
    args.extend(["--user-agent".into(), user_agent.into()]);
    if clients == Clients::PinnedUserAgent {
        args.extend([
            "--extractor-args".into(),
            "youtube:player_client=web_music,web".into(),
        ]);
    }
    args.extend(["--".into(), url.into()]);
    args
}

#[derive(Deserialize)]
struct YtDlpJson {
    format_id: String,
    url: String,
    acodec: Option<String>,
    abr: Option<f64>,
    tbr: Option<f64>,
    vcodec: Option<String>,
    protocol: Option<String>,
    /// Fractional when a pre-roll ad's skip offset is.
    available_at: Option<f64>,
    #[serde(default)]
    http_headers: BTreeMap<String, String>,
}

/// yt-dlp's output without the `[debug]` lines of `-v`: those quote arbitrary text.
fn without_debug_lines(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| !line.starts_with("[debug]"))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// yt-dlp 2026.11.23 logs this, with `-v`, each time it runs Deno on a challenge script.
/// Listing Deno as an available runtime does not count.
fn deno_ran(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|line| line.starts_with("[debug]") && line.contains("[jsc:deno] Running deno:"))
}

/// Warnings that end the run even when yt-dlp itself carried on.
fn warning_sign(stderr: &str) -> Option<ResolveError> {
    let all = stderr.to_ascii_lowercase();
    let has = |markers: &[&str]| markers.iter().any(|m| all.contains(m));
    if has(&["not a bot", "unusual traffic", "captcha challenge"]) {
        Some(ResolveError::BotCheck)
    } else if has(&["http error 429", "rate-limited by youtube"]) {
        Some(ResolveError::RateLimited { retry_after: None })
    } else if all.contains("account cookies are no longer valid") {
        Some(ResolveError::AuthRejected)
    } else {
        None
    }
}

/// Only `ERROR:` lines count: warnings mention HTTP 403 on healthy runs.
fn classify_failure(stderr: &str, code: Option<i32>) -> ResolveError {
    let errors: String = stderr
        .lines()
        .filter(|line| line.starts_with("ERROR:"))
        .map(|line| line.to_ascii_lowercase() + "\n")
        .collect();
    let has = |markers: &[&str]| markers.iter().any(|m| errors.contains(m));
    if has(&["captcha"]) {
        ResolveError::BotCheck
    } else if has(&["too many requests", "rate-limited"]) {
        ResolveError::RateLimited { retry_after: None }
    } else if has(&[
        "http error 401",
        "http error 403",
        "sign in to",
        "login_required",
    ]) {
        ResolveError::AuthRejected
    } else if has(&["requested format is not available"]) {
        ResolveError::NoFormat
    } else if has(&["video unavailable", "video is not available"]) {
        ResolveError::Unavailable
    } else {
        ResolveError::Failed(code)
    }
}

fn known_warnings(stderr: &str) -> Vec<&'static str> {
    let all = stderr.to_ascii_lowercase();
    [
        (
            "require a gvs po token",
            "formats skipped: PO token required",
        ),
        ("javascript runtime", "no JavaScript runtime found"),
        (
            "challenge solving failed",
            "JavaScript challenge solving failed",
        ),
        (
            "since it does not support cookies",
            "a client was skipped: no cookie support",
        ),
        ("sabr", "SABR-only streaming mentioned"),
    ]
    .into_iter()
    .filter_map(|(marker, label)| all.contains(marker).then_some(label))
    .collect()
}

/// A `name=value` query parameter, or the `/name/value/` form of manifest URLs.
fn url_field<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let from_query = url.split_once('?').and_then(|(_, query)| {
        query
            .split('&')
            .find_map(|p| p.strip_prefix(name)?.strip_prefix('='))
    });
    from_query.or_else(|| url.split_once(&format!("/{name}/"))?.1.split('/').next())
}

fn client_name(url: &str) -> Option<String> {
    url_field(url, "c")
        .filter(|c| {
            !c.is_empty()
                && c.len() <= 40
                && c.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_owned)
}

pub(crate) fn is_googlevideo(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host.ends_with(".googlevideo.com"))
    })
}

pub struct Resolver<'a, Y, T, C> {
    pub ytdlp: &'a Y,
    pub transport: &'a T,
    pub clock: &'a C,
    pub jar_dir: PathBuf,
}

impl<Y: YtDlp, T: Transport, C: Clock> Resolver<'_, Y, T, C> {
    /// One yt-dlp run and one Range request, no retries. The stream is returned only when the
    /// Range request accepted it.
    pub async fn resolve(
        &self,
        session: &Session,
        track_url: &str,
        clients: Clients,
    ) -> Result<(ResolveReport, Option<LastStream>), ResolveError> {
        let url = watch_url(track_url)?;
        let started = Instant::now();
        let output = {
            // The cookie file lives exactly as long as the process that reads it.
            let jar = CookieJarFile::create(&self.jar_dir, &session.cookies)
                .map_err(|e| ResolveError::CookieFile(e.kind()))?;
            let args = ytdlp_args(jar.path(), &session.user_agent, &url, clients);
            self.ytdlp
                .run(args)
                .await
                .map_err(|e| ResolveError::Spawn(e.kind()))?
        };
        let resolve_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let messages = without_debug_lines(&output.stderr);
        if let Some(error) = warning_sign(&messages) {
            return Err(error);
        }
        if !output.success {
            return Err(classify_failure(&output.stderr, output.code));
        }
        // The parser's message could quote the stream URL, so it is dropped.
        let stream: YtDlpJson =
            serde_json::from_str(&output.stdout).map_err(|_| ResolveError::UnexpectedOutput)?;
        if !is_googlevideo(&stream.url) {
            return Err(ResolveError::UnexpectedHost);
        }

        let available_at = stream.available_at.map(|at| at.ceil() as u64);
        let waited_s = available_at.map_or(0, |at| at.saturating_sub(self.clock.now()));
        if waited_s > MAX_WAIT_S {
            return Err(ResolveError::AvailableTooLate(waited_s));
        }
        if waited_s > 0 {
            self.clock.sleep(Duration::from_secs(waited_s)).await;
        }

        let user_agent_matches_login = stream.http_headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("user-agent") && *value == session.user_agent
        });
        let kept: Vec<(String, String)> = stream
            .http_headers
            .into_iter()
            .filter(|(name, value)| {
                let credential_or_range = ["cookie", "authorization", "range"]
                    .iter()
                    .any(|blocked| name.eq_ignore_ascii_case(blocked));
                !credential_or_range && !format!("{name}{value}").contains(['\r', '\n'])
            })
            .collect();
        let mut headers = kept.clone();
        headers.push(("Range".to_owned(), "bytes=0-1023".to_owned()));
        let response = self
            .transport
            .probe(Request {
                url: stream.url.clone(),
                headers,
                body: String::new(),
            })
            .await
            .map_err(|e| ResolveError::Probe(e.0))?;
        if response.status == 429 {
            return Err(ResolveError::RateLimited {
                retry_after: response.retry_after,
            });
        }

        let playable =
            matches!(response.status, 200 | 206).then(|| LastStream::new(stream.url.clone(), kept));
        let report = ResolveReport {
            format_id: stream.format_id,
            codec: stream.acodec,
            abr_kbps: stream.abr,
            tbr_kbps: stream.tbr,
            audio_only: stream.vcodec.as_deref() == Some("none"),
            vcodec: stream.vcodec,
            protocol: stream.protocol,
            client: client_name(&stream.url),
            available_at,
            waited_s,
            expires_at: url_field(&stream.url, "expire").and_then(|at| at.parse().ok()),
            resolve_ms,
            user_agent_matches_login,
            range_status: response.status,
            deno_started: deno_ran(&output.stderr),
            warnings: known_warnings(&messages),
        };
        Ok((report, playable))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use super::*;

    /// Gives one fixed answer and records the arguments and the cookie file it was given.
    #[derive(Default)]
    pub struct FakeYtDlp {
        pub success: bool,
        pub stdout: String,
        pub stderr: String,
        pub missing: bool,
        pub seen: Mutex<Vec<(Vec<OsString>, String)>>,
    }

    impl FakeYtDlp {
        pub fn printing(stdout: String) -> Self {
            Self {
                success: true,
                stdout,
                ..Self::default()
            }
        }

        pub fn failing(stderr: &str) -> Self {
            Self {
                stderr: stderr.into(),
                ..Self::default()
            }
        }

        pub fn runs(&self) -> usize {
            self.seen.lock().unwrap().len()
        }

        pub fn jar_path(&self) -> PathBuf {
            let seen = self.seen.lock().unwrap();
            let args = &seen[0].0;
            let flag = args.iter().position(|a| a == "--cookies").unwrap();
            PathBuf::from(&args[flag + 1])
        }
    }

    impl YtDlp for FakeYtDlp {
        async fn run(&self, args: Vec<OsString>) -> io::Result<ProcessOutput> {
            let flag = args.iter().position(|a| a == "--cookies").unwrap();
            let jar = std::fs::read_to_string(&args[flag + 1])?;
            self.seen.lock().unwrap().push((args, jar));
            if self.missing {
                return Err(io::ErrorKind::NotFound.into());
            }
            Ok(ProcessOutput {
                success: self.success,
                code: Some(if self.success { 0 } else { 1 }),
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use serde_json::json;

    use super::{fake::FakeYtDlp, *};
    use crate::{
        clock::fake::FakeClock,
        jar::CookieJarFile,
        session::fake::session,
        transport::{
            Request, Response, TransportError,
            fake::{FakeTransport, response},
        },
    };

    const NOW: u64 = 1_700_000_000;
    const TRACK: &str = "https://music.youtube.com/watch?v=dQw4w9WgXcQ";
    const STREAM: &str = "https://rr1---sn-fake.googlevideo.com/videoplayback?expire=1700021600&ip=203.0.113.7&c=WEB_REMIX&itag=251&sig=fake-signature";
    /// Printed by yt-dlp on successful runs too. It must not count as a 403.
    const PO_TOKEN_WARNING: &str = "WARNING: [youtube] x: web_music client https formats require a GVS PO Token which was not provided. They will be skipped as they may yield HTTP Error 403.";

    fn stream_json(url: &str, available_at: Option<f64>) -> String {
        json!({
            "format_id": "251", "acodec": "opus", "abr": 129.5, "protocol": "https",
            "vcodec": "none", "tbr": 130.5,
            "url": url, "available_at": available_at,
            "http_headers": {
                "User-Agent": "FakeAgent/1.0",
                "Accept": "*/*",
                "Cookie": "SAPISID=fake-sapisid"
            }
        })
        .to_string()
    }

    struct Fixture {
        ytdlp: FakeYtDlp,
        transport: FakeTransport,
        clock: FakeClock,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(ytdlp: FakeYtDlp, probe: &[Response]) -> Self {
            Self {
                ytdlp,
                transport: FakeTransport::answering(probe.to_vec()),
                clock: FakeClock::at(NOW),
                dir: tempfile::tempdir().unwrap(),
            }
        }

        async fn resolve(&self, url: &str) -> Result<ResolveReport, ResolveError> {
            self.resolve_both(url).await.map(|(report, _)| report)
        }

        async fn resolve_both(
            &self,
            url: &str,
        ) -> Result<(ResolveReport, Option<LastStream>), ResolveError> {
            Resolver {
                ytdlp: &self.ytdlp,
                transport: &self.transport,
                clock: &self.clock,
                jar_dir: self.dir.path().to_owned(),
            }
            .resolve(&session(), url, Clients::Default)
            .await
        }

        fn files_left(&self) -> usize {
            std::fs::read_dir(self.dir.path()).unwrap().count()
        }
    }

    #[test]
    fn accepts_a_music_track_url_and_keeps_only_the_video_id() {
        assert_eq!(
            watch_url(" https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVM&si=x "),
            Ok(TRACK.to_owned())
        );
    }

    #[test]
    fn rejects_every_other_url_so_cookies_go_nowhere_else() {
        for url in [
            "",
            "dQw4w9WgXcQ",
            "http://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com.example.org/watch?v=dQw4w9WgXcQ",
            "https://user@music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com:8443/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com/playlist?list=PLx",
            "https://music.youtube.com/watch?v=short",
            "https://music.youtube.com/watch?v=dQw4w9WgXc%22",
            "--exec=calc",
        ] {
            assert_eq!(watch_url(url), Err(ResolveError::BadUrl), "{url}");
        }
    }

    #[test]
    fn builds_the_arguments_with_the_pinned_user_agent_and_cookie_file() {
        let args = ytdlp_args(Path::new("jar.txt"), "Agent/1.0", TRACK, Clients::Default);

        assert_eq!(
            args,
            [
                "-J",
                "-v",
                "-f",
                "bestaudio/best",
                "--no-playlist",
                "--ignore-config",
                "--playlist-items",
                "1",
                "--cookies",
                "jar.txt",
                "--user-agent",
                "Agent/1.0",
                "--",
                TRACK
            ]
        );
    }

    #[test]
    fn can_limit_yt_dlp_to_clients_that_use_the_pinned_user_agent() {
        let args = ytdlp_args(
            Path::new("jar.txt"),
            "Agent/1.0",
            TRACK,
            Clients::PinnedUserAgent,
        );

        let flag = args.iter().position(|a| a == "--extractor-args").unwrap();
        assert_eq!(args[flag + 1], "youtube:player_client=web_music,web");
        assert_eq!(args[args.len() - 2..], ["--", TRACK]);
    }

    #[tokio::test]
    async fn reports_the_stream_without_its_url() {
        let fixture = Fixture::new(
            FakeYtDlp {
                stderr: PO_TOKEN_WARNING.into(),
                ..FakeYtDlp::printing(stream_json(STREAM, None))
            },
            &[response(206, "")],
        );

        let report = fixture.resolve(TRACK).await.unwrap();

        assert_eq!(
            report,
            ResolveReport {
                format_id: "251".into(),
                codec: Some("opus".into()),
                abr_kbps: Some(129.5),
                tbr_kbps: Some(130.5),
                vcodec: Some("none".into()),
                audio_only: true,
                protocol: Some("https".into()),
                client: Some("WEB_REMIX".into()),
                available_at: None,
                waited_s: 0,
                expires_at: Some(1_700_021_600),
                resolve_ms: report.resolve_ms,
                user_agent_matches_login: true,
                deno_started: false,
                range_status: 206,
                warnings: vec!["formats skipped: PO token required"],
            }
        );
        let shown = format!("{report:?}{}", serde_json::to_string(&report).unwrap());
        for secret in ["googlevideo", "203.0.113.7", "fake"] {
            assert!(!shown.contains(secret), "{secret} in {shown}");
        }
    }

    async fn report_for(stderr: &str) -> Result<ResolveReport, ResolveError> {
        let ytdlp = FakeYtDlp {
            stderr: stderr.into(),
            ..FakeYtDlp::printing(stream_json(STREAM, None))
        };
        Fixture::new(ytdlp, &[response(206, "")])
            .resolve(TRACK)
            .await
    }

    #[tokio::test]
    async fn reports_that_deno_ran_a_javascript_challenge() {
        let stderr = r"[debug] [youtube] [jsc:deno] Running deno: C:\tools\deno.exe run --ext=js -";

        assert!(report_for(stderr).await.unwrap().deno_started);
    }

    #[tokio::test]
    async fn a_listed_but_unused_deno_did_not_start() {
        let stderr = concat!(
            "[debug] JS runtimes: deno-2.9.8\n",
            "[debug] [youtube] [jsc] JS Challenge Providers: bun (unavailable), deno\n",
            "WARNING: [youtube] x: Running deno failed\n",
        );

        assert!(!report_for(stderr).await.unwrap().deno_started);
    }

    #[tokio::test]
    async fn verbose_debug_lines_are_not_warning_signs() {
        let stderr = concat!(
            "[debug] cache key abcaptcha challenge sabr\n",
            "[debug] HTTP Error 429 handling: javascript runtime\n",
        );

        let report = report_for(stderr).await.unwrap();

        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[tokio::test]
    async fn reports_a_combined_hls_fallback_as_not_audio_only() {
        let manifest = "https://manifest.googlevideo.com/api/manifest/hls_playlist/expire/1700021600/c/WEB/file/index.m3u8";
        let stdout = json!({
            "format_id": "95", "acodec": "mp4a.40.2", "vcodec": "avc1.4d401f", "tbr": 1500.0,
            "protocol": "m3u8_native", "url": manifest
        })
        .to_string();
        let fixture = Fixture::new(FakeYtDlp::printing(stdout), &[response(200, "")]);

        let report = fixture.resolve(TRACK).await.unwrap();

        assert!(!report.audio_only);
        assert_eq!(report.vcodec.as_deref(), Some("avc1.4d401f"));
        assert_eq!(report.abr_kbps, None);
        assert_eq!(report.tbr_kbps, Some(1500.0));
        assert_eq!(report.protocol.as_deref(), Some("m3u8_native"));
        assert_eq!(report.client.as_deref(), Some("WEB"));
        assert_eq!(report.expires_at, Some(1_700_021_600));
        assert_eq!(fixture.ytdlp.runs(), 1);
    }

    #[tokio::test]
    async fn an_unknown_video_codec_is_not_claimed_to_be_audio_only() {
        let stdout = json!({"format_id": "x", "url": STREAM}).to_string();
        let fixture = Fixture::new(FakeYtDlp::printing(stdout), &[response(206, "")]);

        let report = fixture.resolve(TRACK).await.unwrap();

        assert!(!report.audio_only);
        assert_eq!(report.vcodec, None);
    }

    #[tokio::test]
    async fn probes_the_first_kilobyte_without_cookies() {
        let fixture = Fixture::new(
            FakeYtDlp::printing(stream_json(STREAM, None)),
            &[response(403, "")],
        );

        let report = fixture.resolve(TRACK).await.unwrap();

        assert_eq!(report.range_status, 403);
        let probes = fixture.transport.probes.lock().unwrap();
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].url, STREAM);
        assert_eq!(probes[0].header("Range"), Some("bytes=0-1023"));
        assert_eq!(probes[0].header("User-Agent"), Some("FakeAgent/1.0"));
        assert_eq!(probes[0].header("Cookie"), None);
    }

    #[tokio::test]
    async fn keeps_a_playable_stream_with_sanitized_headers() {
        let stdout = json!({
            "format_id": "251", "url": STREAM,
            "http_headers": {
                "User-Agent": "FakeAgent/1.0", "Accept": "*/*",
                "cookie": "a=b", "Authorization": "x", "Range": "bytes=5-",
                "X-Split": "a\r\nCookie: c=d"
            }
        })
        .to_string();
        let fixture = Fixture::new(FakeYtDlp::printing(stdout), &[response(206, "")]);

        let stream = fixture.resolve_both(TRACK).await.unwrap().1.unwrap();

        assert_eq!(stream.url(), STREAM);
        assert_eq!(
            stream.headers(),
            [
                ("Accept".to_owned(), "*/*".to_owned()),
                ("User-Agent".to_owned(), "FakeAgent/1.0".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn keeps_no_stream_when_the_range_request_is_refused() {
        let fixture = Fixture::new(
            FakeYtDlp::printing(stream_json(STREAM, None)),
            &[response(403, "")],
        );

        assert!(fixture.resolve_both(TRACK).await.unwrap().1.is_none());
    }

    #[tokio::test]
    async fn hands_yt_dlp_the_session_cookies_and_deletes_the_file_afterwards() {
        let fixture = Fixture::new(
            FakeYtDlp::printing(stream_json(STREAM, None)),
            &[response(206, "")],
        );

        fixture.resolve(TRACK).await.unwrap();

        let jar = fixture.ytdlp.seen.lock().unwrap()[0].1.clone();
        assert_eq!(jar, crate::cookies::netscape_file(&session().cookies));
        assert!(!fixture.ytdlp.jar_path().exists());
        assert_eq!(fixture.files_left(), 0);
    }

    #[tokio::test]
    async fn deletes_the_cookie_file_when_yt_dlp_cannot_start() {
        let fixture = Fixture::new(
            FakeYtDlp {
                missing: true,
                ..FakeYtDlp::default()
            },
            &[],
        );

        assert_eq!(
            fixture.resolve(TRACK).await,
            Err(ResolveError::Spawn(io::ErrorKind::NotFound))
        );
        assert_eq!(fixture.files_left(), 0);
    }

    #[tokio::test]
    async fn does_not_start_yt_dlp_for_a_rejected_url() {
        let fixture = Fixture::new(FakeYtDlp::default(), &[]);

        assert_eq!(
            fixture.resolve("https://example.org/").await,
            Err(ResolveError::BadUrl)
        );
        assert!(fixture.ytdlp.seen.lock().unwrap().is_empty());
        assert_eq!(fixture.files_left(), 0);
    }

    /// Records the time of each probe.
    struct TimedProbe<'a> {
        clock: &'a FakeClock,
        at: Mutex<Vec<u64>>,
    }

    impl Transport for TimedProbe<'_> {
        async fn post(&self, _request: Request) -> Result<Response, TransportError> {
            unreachable!("a resolve never posts")
        }

        async fn probe(&self, _request: Request) -> Result<Response, TransportError> {
            self.at.lock().unwrap().push(self.clock.now());
            Ok(response(206, ""))
        }
    }

    #[tokio::test]
    async fn waits_for_available_at_before_requesting_the_stream() {
        let clock = FakeClock::at(NOW);
        let transport = TimedProbe {
            clock: &clock,
            at: Mutex::default(),
        };
        let ytdlp = FakeYtDlp::printing(stream_json(STREAM, Some(NOW as f64 + 5.5)));
        let dir = tempfile::tempdir().unwrap();

        let report = Resolver {
            ytdlp: &ytdlp,
            transport: &transport,
            clock: &clock,
            jar_dir: dir.path().to_owned(),
        }
        .resolve(&session(), TRACK, Clients::Default)
        .await
        .unwrap()
        .0;

        assert_eq!(report.available_at, Some(NOW + 6));
        assert_eq!(report.waited_s, 6);
        assert_eq!(*clock.slept.lock().unwrap(), [Duration::from_secs(6)]);
        assert_eq!(*transport.at.lock().unwrap(), [NOW + 6]);
    }

    #[tokio::test]
    async fn refuses_an_unreasonably_long_wait() {
        let fixture = Fixture::new(
            FakeYtDlp::printing(stream_json(STREAM, Some(NOW as f64 + 61.0))),
            &[],
        );

        assert_eq!(
            fixture.resolve(TRACK).await,
            Err(ResolveError::AvailableTooLate(61))
        );
        assert_eq!(fixture.transport.sent(), 0);
    }

    #[tokio::test]
    async fn requests_nothing_outside_googlevideo() {
        for url in [
            "https://example.org/videoplayback",
            "http://rr1.googlevideo.com/videoplayback",
            "https://googlevideo.com.example.org/videoplayback",
        ] {
            let fixture = Fixture::new(FakeYtDlp::printing(stream_json(url, None)), &[]);

            assert_eq!(
                fixture.resolve(TRACK).await,
                Err(ResolveError::UnexpectedHost),
                "{url}"
            );
            assert_eq!(fixture.transport.sent(), 0);
        }
    }

    #[tokio::test]
    async fn stops_on_a_rate_limited_range_request() {
        let limited = Response {
            retry_after: Some("30".into()),
            ..response(429, "")
        };
        let fixture = Fixture::new(FakeYtDlp::printing(stream_json(STREAM, None)), &[limited]);

        assert_eq!(
            fixture.resolve(TRACK).await,
            Err(ResolveError::RateLimited {
                retry_after: Some("30".into())
            })
        );
    }

    #[tokio::test]
    async fn rejects_output_without_a_direct_url() {
        for stdout in ["", "not json", r#"{"format_id": "251"}"#] {
            let fixture = Fixture::new(FakeYtDlp::printing(stdout.into()), &[]);

            assert_eq!(
                fixture.resolve(TRACK).await,
                Err(ResolveError::UnexpectedOutput),
                "{stdout}"
            );
        }
    }

    #[tokio::test]
    async fn reduces_a_yt_dlp_failure_to_a_category() {
        let limited = ResolveError::RateLimited { retry_after: None };
        let cases = [
            (
                "ERROR: [youtube] x: Unable to download API page: HTTP Error 429: Too Many Requests",
                limited,
            ),
            (
                "ERROR: [youtube] x: Sign in to confirm you\u{2019}re not a bot.",
                ResolveError::BotCheck,
            ),
            (
                "ERROR: [youtube] x: HTTP Error 401: Unauthorized",
                ResolveError::AuthRejected,
            ),
            (
                "ERROR: [youtube] x: HTTP Error 403: Forbidden",
                ResolveError::AuthRejected,
            ),
            (
                "ERROR: [youtube] x: Sign in to confirm your age",
                ResolveError::AuthRejected,
            ),
            (
                "ERROR: [youtube] x: Requested format is not available. Use --list-formats",
                ResolveError::NoFormat,
            ),
            (
                "ERROR: [youtube] x: Video unavailable",
                ResolveError::Unavailable,
            ),
            (
                "ERROR: something new https://rr1.googlevideo.com/?ip=203.0.113.7",
                ResolveError::Failed(Some(1)),
            ),
        ];
        for (line, expected) in cases {
            let stderr = format!("{PO_TOKEN_WARNING}\n{line}\n");
            let fixture = Fixture::new(FakeYtDlp::failing(&stderr), &[]);

            let error = fixture.resolve(TRACK).await.unwrap_err();

            assert_eq!(error, expected, "{line}");
            assert!(!error.to_string().contains("203.0.113.7"));
            assert_eq!(fixture.transport.sent(), 0);
            assert_eq!(fixture.files_left(), 0);
        }
    }

    #[tokio::test]
    async fn fails_closed_on_warning_signs_even_when_yt_dlp_succeeds() {
        let cases = [
            (
                "WARNING: [youtube] The provided YouTube account cookies are no longer valid.",
                ResolveError::AuthRejected,
            ),
            (
                "WARNING: [youtube] HTTP Error 429: Too Many Requests. Retrying (1/3)...",
                ResolveError::RateLimited { retry_after: None },
            ),
            (
                "WARNING: [youtube] Our systems have detected unusual traffic",
                ResolveError::BotCheck,
            ),
            (
                "WARNING: [youtube] x: Your account has been rate-limited by YouTube for up to an hour.",
                ResolveError::RateLimited { retry_after: None },
            ),
            (
                "WARNING: [youtube] x: YouTube is requiring a captcha challenge before playback",
                ResolveError::BotCheck,
            ),
        ];
        for (line, expected) in cases {
            let ytdlp = FakeYtDlp {
                stderr: line.into(),
                ..FakeYtDlp::printing(stream_json(STREAM, None))
            };
            let fixture = Fixture::new(ytdlp, &[]);

            assert_eq!(fixture.resolve(TRACK).await, Err(expected), "{line}");
            assert_eq!(fixture.transport.sent(), 0);
        }
    }

    /// Runs the real yt-dlp against a closed local port: checks that it accepts our arguments
    /// and cookie file. No request leaves the machine. Run by hand: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore = "needs yt-dlp on PATH"]
    async fn real_yt_dlp_accepts_the_arguments_and_cookie_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut cookies = session().cookies;
        cookies[0].http_only = true;
        cookies[1].expires = Some(1_900_000_000);
        let jar = CookieJarFile::create(dir.path(), &cookies).unwrap();

        let args = ytdlp_args(
            jar.path(),
            "FakeAgent/1.0",
            "http://127.0.0.1:1/track",
            Clients::PinnedUserAgent,
        );
        let output = YtDlpProcess.run(args).await.unwrap();

        let stderr = output.stderr.to_ascii_lowercase();
        assert!(!output.success);
        assert!(stderr.contains("127.0.0.1"), "{stderr}");
        for complaint in ["netscape", "skipping cookie", "no such option", "usage:"] {
            assert!(!stderr.contains(complaint), "{complaint}: {stderr}");
        }
    }
}
