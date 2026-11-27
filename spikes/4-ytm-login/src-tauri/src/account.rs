//! The one YouTube Music account: saving and clearing its session, and the single test run.
//! Every session write and every call to YouTube goes through one lock and one [`Gate`].

use std::{path::PathBuf, sync::PoisonError};

use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    clock::Clock,
    cookies::Cookie,
    gate::{Gate, parse_retry_after},
    innertube::{BrowseError, PlaylistPage, liked_playlists},
    resolver::{Clients, LastStream, ResolveError, ResolveReport, Resolver, YtDlp, watch_url},
    safe_report,
    session::{CaptureMethod, SecretStore, Session, SessionError, SessionStatus, SessionVault},
    transport::Transport,
};

/// What a failed call means for the account (PROJECT.md 3c).
enum Reaction {
    ExpireSession,
    CoolDown(Option<String>),
    Pause,
    CountFailure,
    /// A local problem: YouTube never saw it.
    Nothing,
}

impl From<&BrowseError> for Reaction {
    fn from(error: &BrowseError) -> Self {
        match error {
            BrowseError::NotSignedIn | BrowseError::Unauthorized(_) | BrowseError::SignedOut => {
                Self::ExpireSession
            }
            BrowseError::RateLimited { retry_after } => Self::CoolDown(retry_after.clone()),
            BrowseError::Blocked(_) => Self::Pause,
            BrowseError::Status(_)
            | BrowseError::UnexpectedResponse
            | BrowseError::Transport(_) => Self::CountFailure,
        }
    }
}

impl From<&ResolveError> for Reaction {
    fn from(error: &ResolveError) -> Self {
        match error {
            ResolveError::AuthRejected => Self::ExpireSession,
            ResolveError::RateLimited { retry_after } => Self::CoolDown(retry_after.clone()),
            ResolveError::BotCheck => Self::Pause,
            ResolveError::NoFormat
            | ResolveError::Unavailable
            | ResolveError::Failed(_)
            | ResolveError::UnexpectedOutput
            | ResolveError::UnexpectedHost
            | ResolveError::AvailableTooLate(_)
            | ResolveError::Probe(_) => Self::CountFailure,
            ResolveError::BadUrl | ResolveError::CookieFile(_) | ResolveError::Spawn(_) => {
                Self::Nothing
            }
        }
    }
}

/// Outcome of one run, safe to show. `blocked` says why nothing was sent.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct CheckReport {
    pub blocked: Option<String>,
    pub browse: Option<Result<PlaylistPage, String>>,
    pub resolve: Option<Result<ResolveReport, String>>,
    /// The same run as text that is safe to copy into the findings.
    pub safe_report: String,
}

pub struct Account<S, T, Y, C> {
    vault: SessionVault<S>,
    transport: T,
    ytdlp: Y,
    clock: C,
    jar_dir: PathBuf,
    gate: Mutex<Gate>,
    /// Memory only: dropped with the session and at the start of the next run.
    last_stream: std::sync::Mutex<Option<LastStream>>,
}

impl<S: SecretStore, T: Transport, Y: YtDlp, C: Clock> Account<S, T, Y, C> {
    pub fn new(store: S, transport: T, ytdlp: Y, clock: C, jar_dir: PathBuf) -> Self {
        Self {
            vault: SessionVault::new(store),
            transport,
            ytdlp,
            clock,
            jar_dir,
            gate: Mutex::default(),
            last_stream: Default::default(),
        }
    }

    pub async fn status(&self) -> Result<Option<SessionStatus>, SessionError> {
        let _writes_done = self.gate.lock().await;
        Ok(self.vault.load()?.map(|session| session.status()))
    }

    /// Replaces the stored session.
    pub async fn save(
        &self,
        cookies: Vec<Cookie>,
        user_agent: &str,
        method: CaptureMethod,
    ) -> Result<SessionStatus, SessionError> {
        let session = Session::new(cookies, user_agent, method, self.clock.now())?;
        let _only_writer = self.gate.lock().await;
        self.keep(None);
        self.vault.save(&session)?;
        Ok(session.status())
    }

    pub async fn clear(&self) -> Result<(), SessionError> {
        let _only_writer = self.gate.lock().await;
        self.keep(None);
        self.vault.clear()
    }

    /// The stream of the latest run, if that run resolved a playable one.
    pub fn last_stream(&self) -> Option<LastStream> {
        self.last_stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn keep(&self, stream: Option<LastStream>) {
        *self
            .last_stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = stream;
    }

    /// Returns once no run is in flight, so its cookie file is gone.
    pub async fn wait_until_idle(&self) {
        drop(self.gate.lock().await);
    }

    /// One browse call, then one resolve. Stops at the first failure and never retries.
    pub async fn check(&self, track_url: &str, clients: Clients) -> CheckReport {
        let mut report = self.run_check(track_url, clients).await;
        report.safe_report = safe_report::text(clients, self.clock.now(), &report);
        report
    }

    async fn run_check(&self, track_url: &str, clients: Clients) -> CheckReport {
        let blocked = |reason: String| CheckReport {
            blocked: Some(reason),
            ..CheckReport::default()
        };
        let Ok(mut gate) = self.gate.try_lock() else {
            return blocked("a check is already running".to_owned());
        };
        if let Err(error) = watch_url(track_url) {
            return blocked(error.to_string());
        }
        let mut session = match self.vault.load() {
            Ok(Some(session)) if !session.expired => session,
            Ok(Some(_)) => return blocked("the session is expired: log in again".to_owned()),
            Ok(None) => return blocked("no saved session: log in or import first".to_owned()),
            Err(error) => return blocked(error.to_string()),
        };
        if let Err(error) = gate.enter(self.clock.now()) {
            return blocked(error.to_string());
        }

        self.keep(None);
        let mut report = CheckReport::default();
        let browse = liked_playlists(&self.transport, &session, self.clock.now()).await;
        match browse {
            Ok(page) => report.browse = Some(Ok(page)),
            Err(error) => {
                let message = self.react(&mut gate, &mut session, (&error).into(), &error);
                report.browse = Some(Err(message));
                return report;
            }
        }

        let resolver = Resolver {
            ytdlp: &self.ytdlp,
            transport: &self.transport,
            clock: &self.clock,
            jar_dir: self.jar_dir.clone(),
        };
        report.resolve = Some(match resolver.resolve(&session, track_url, clients).await {
            Ok((resolved, stream)) => {
                self.keep(stream);
                if matches!(resolved.range_status, 200 | 206) {
                    gate.success();
                } else {
                    gate.failure();
                }
                Ok(resolved)
            }
            Err(error) => Err(self.react(&mut gate, &mut session, (&error).into(), &error)),
        });
        report
    }

    /// Applies the reaction and returns the message to show.
    fn react(
        &self,
        gate: &mut Gate,
        session: &mut Session,
        reaction: Reaction,
        error: &dyn std::error::Error,
    ) -> String {
        let mut message = error.to_string();
        match reaction {
            Reaction::ExpireSession => {
                session.expired = true;
                if let Err(store) = self.vault.save(session) {
                    gate.pause("the session could not be marked expired");
                    message = format!("{message} (not marked expired: {store})");
                }
            }
            Reaction::CoolDown(retry_after) => {
                let now = self.clock.now();
                let retry_after = retry_after.and_then(|value| parse_retry_after(&value, now));
                let wait = gate.rate_limited(now, retry_after, self.clock.jitter());
                message = format!("{message}: cooling down for {wait} s");
            }
            Reaction::Pause => gate.pause("bot check or unusual-traffic response"),
            Reaction::CountFailure => gate.failure(),
            Reaction::Nothing => {}
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        clock::fake::FakeClock,
        resolver::fake::FakeYtDlp,
        session::fake::{MemoryStore, cookie, session},
        transport::{
            Response,
            fake::{FakeTransport, response},
        },
    };

    const NOW: u64 = 1_700_000_000;
    const TRACK: &str = "https://music.youtube.com/watch?v=dQw4w9WgXcQ";

    type TestAccount<'a> = Account<&'a MemoryStore, FakeTransport, FakeYtDlp, FakeClock>;

    fn library() -> Response {
        response(200, r#"{"contents": {"gridRenderer": {"items": []}}}"#)
    }

    fn stream() -> FakeYtDlp {
        FakeYtDlp::printing(
            json!({"format_id": "251", "url": "https://r.googlevideo.com/v?c=WEB_REMIX"})
                .to_string(),
        )
    }

    async fn account<'a>(
        store: &'a MemoryStore,
        answers: &[Response],
        ytdlp: FakeYtDlp,
        dir: &tempfile::TempDir,
    ) -> TestAccount<'a> {
        let account = Account::new(
            store,
            FakeTransport::answering(answers.to_vec()),
            ytdlp,
            FakeClock::at(NOW),
            dir.path().to_owned(),
        );
        let signed_in = session();
        account
            .save(signed_in.cookies, &signed_in.user_agent, signed_in.method)
            .await
            .unwrap();
        account
    }

    fn pass_time(account: &TestAccount<'_>, seconds: u64) {
        *account.clock.now.lock().unwrap() += seconds;
    }

    async fn run(account: &TestAccount<'_>) -> CheckReport {
        account.check(TRACK, Clients::Default).await
    }

    async fn is_expired(account: &TestAccount<'_>) -> bool {
        account.status().await.unwrap().unwrap().expired
    }

    #[tokio::test]
    async fn saves_reports_and_clears_the_session() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[], FakeYtDlp::default(), &dir).await;

        let status = account.status().await.unwrap().unwrap();
        assert_eq!(status.captured_at, NOW);
        assert_eq!(status.cookie_count, 2);

        account.clear().await.unwrap();

        assert_eq!(account.status().await, Ok(None));
        assert!(store.entries.borrow().is_empty());
    }

    #[tokio::test]
    async fn keeps_the_old_session_when_the_new_cookies_are_not_signed_in() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[], FakeYtDlp::default(), &dir).await;

        let saved = account
            .save(
                vec![cookie("PREF", "x")],
                "Agent/1.0",
                CaptureMethod::Import,
            )
            .await;

        assert_eq!(saved, Err(SessionError::NotSignedIn));
        assert_eq!(account.status().await.unwrap().unwrap().cookie_count, 2);
    }

    #[tokio::test]
    async fn runs_one_browse_call_and_one_resolve() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[library(), response(206, "")], stream(), &dir).await;

        let report = run(&account).await;

        assert_eq!(report.blocked, None);
        assert!(report.browse.unwrap().is_ok());
        assert_eq!(report.resolve.unwrap().unwrap().range_status, 206);
        assert_eq!(account.transport.posts.lock().unwrap().len(), 1);
        assert_eq!(account.transport.probes.lock().unwrap().len(), 1);
        assert_eq!(account.ytdlp.runs(), 1);
        assert!(!is_expired(&account).await);
    }

    #[tokio::test]
    async fn keeps_the_last_resolved_stream_until_the_session_is_cleared_or_replaced() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let answers = [library(), response(206, ""), library(), response(206, "")];
        let account = account(&store, &answers, stream(), &dir).await;
        assert!(account.last_stream().is_none());

        run(&account).await;
        assert!(account.last_stream().is_some());
        account.clear().await.unwrap();
        assert!(account.last_stream().is_none());

        let fresh = session();
        account
            .save(fresh.cookies.clone(), &fresh.user_agent, fresh.method)
            .await
            .unwrap();
        pass_time(&account, 60);
        run(&account).await;
        assert!(account.last_stream().is_some());
        account
            .save(fresh.cookies, &fresh.user_agent, fresh.method)
            .await
            .unwrap();
        assert!(account.last_stream().is_none());
    }

    #[tokio::test]
    async fn a_failed_run_drops_the_earlier_stream_but_a_blocked_one_keeps_it() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let answers = [library(), response(206, ""), response(429, "")];
        let account = account(&store, &answers, stream(), &dir).await;
        run(&account).await;

        assert!(run(&account).await.blocked.is_some());
        assert!(account.last_stream().is_some());
        pass_time(&account, 60);
        run(&account).await;

        assert!(account.last_stream().is_none());
    }

    #[tokio::test]
    async fn the_safe_report_records_the_run_without_urls_cookies_or_account_data() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let library = response(
            200,
            r#"{"contents": {"gridRenderer": {"items": [{"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": "My private mix"}]},
                "navigationEndpoint": {"browseEndpoint": {"browseId": "VLPLsecret"}}}}]}}}"#,
        );
        let ytdlp = FakeYtDlp {
            stderr: "[debug] [youtube] [jsc:deno] Running deno: deno run -".into(),
            ..FakeYtDlp::printing(
                json!({
                    "format_id": "251", "acodec": "opus", "vcodec": "none", "abr": 129.5,
                    "tbr": 130.5, "protocol": "https",
                    "url": "https://r.googlevideo.com/v?expire=1700021600&c=WEB_REMIX&sig=fake-signature",
                    "http_headers": {"User-Agent": "StreamAgent/1.0"}
                })
                .to_string(),
            )
        };
        let account = account(&store, &[library, response(206, "")], ytdlp, &dir).await;

        let report = account.check(TRACK, Clients::PinnedUserAgent).await;

        assert!(
            report
                .browse
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .playlists
                .len()
                == 1
        );
        let lines: Vec<&str> = report.safe_report.lines().collect();
        assert_eq!(
            lines[..lines.len() - 1],
            [
                "mode: web_music,web",
                "run_at: 1700000000",
                "browse: ok",
                "resolve: ok",
                "client: WEB_REMIX",
                "format_id: 251",
                "audio_only: yes",
                "vcodec: none",
                "codec: opus",
                "abr_kbps: 129.5",
                "tbr_kbps: 130.5",
                "protocol: https (HTTPS)",
                "available_at: none",
                "waited_s: 0",
                "expires_at: 1700021600 (in 21600 s)",
                "range_status: 206",
                "user_agent_matches_login: no",
                "deno_started: yes",
                "warnings: none known",
            ]
        );
        assert!(lines.last().unwrap().starts_with("resolve_ms: "));
        let signed_in = session();
        for secret in signed_in
            .cookies
            .iter()
            .map(|cookie| cookie.value.as_str())
            .chain([
                signed_in.user_agent.as_str(),
                "googlevideo",
                "://",
                "signature",
            ])
            .chain(["private mix", "PLsecret", "dQw4w9WgXcQ"])
        {
            assert!(!report.safe_report.contains(secret), "{secret}");
        }
    }

    #[tokio::test]
    async fn the_safe_report_names_failures_and_drops_urls_from_messages() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let ytdlp = FakeYtDlp::failing("ERROR: Requested format is not available");
        let account = account(&store, &[library()], ytdlp, &dir).await;

        let bad_url = account.check("https://example.org", Clients::Default).await;
        let failed = run(&account).await;

        assert!(
            bad_url.safe_report.contains("mode: yt-dlp defaults\n"),
            "{}",
            bad_url.safe_report
        );
        assert!(
            bad_url
                .safe_report
                .contains("blocked: enter a track URL like [url]")
        );
        assert!(!bad_url.safe_report.contains("://"));
        assert!(
            failed
                .safe_report
                .contains("resolve: error: yt-dlp: no audio format was available")
        );
    }

    #[tokio::test]
    async fn sends_nothing_without_a_session_or_with_a_bad_url() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[], FakeYtDlp::default(), &dir).await;

        let bad_url = account.check("https://example.org", Clients::Default).await;
        account.clear().await.unwrap();
        let no_session = run(&account).await;

        assert!(bad_url.blocked.unwrap().contains("watch?v="));
        assert!(no_session.blocked.unwrap().contains("no saved session"));
        assert_eq!(account.transport.sent() + account.ytdlp.runs(), 0);
    }

    #[tokio::test]
    async fn a_rejected_browse_call_expires_the_session_and_nothing_else_is_sent() {
        let signed_out = response(
            200,
            r#"{"responseContext": {"serviceTrackingParams": [{"params": [{"key": "logged_in", "value": "0"}]}]}}"#,
        );
        for rejection in [response(401, ""), response(403, ""), signed_out] {
            let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
            let account = account(&store, &[rejection], stream(), &dir).await;

            let first = run(&account).await;
            pass_time(&account, 3_600);
            let second = run(&account).await;

            assert!(first.browse.unwrap().is_err());
            assert_eq!(first.resolve, None);
            assert!(is_expired(&account).await);
            assert!(second.blocked.unwrap().contains("expired"));
            assert_eq!(account.transport.sent(), 1);
            assert_eq!(account.ytdlp.runs(), 0);
        }
    }

    #[tokio::test]
    async fn a_new_login_replaces_an_expired_session() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[response(401, "")], stream(), &dir).await;
        run(&account).await;

        let fresh = session();
        account
            .save(fresh.cookies, &fresh.user_agent, fresh.method)
            .await
            .unwrap();

        assert!(!is_expired(&account).await);
    }

    #[tokio::test]
    async fn a_session_rejected_by_yt_dlp_expires_too() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let ytdlp = FakeYtDlp::failing("ERROR: HTTP Error 403: Forbidden");
        let account = account(&store, &[library()], ytdlp, &dir).await;

        let report = run(&account).await;

        assert!(report.resolve.unwrap().is_err());
        assert!(is_expired(&account).await);
    }

    #[tokio::test]
    async fn a_429_cools_down_for_retry_after_without_expiring_the_session() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let limited = Response {
            retry_after: Some("600".into()),
            ..response(429, "")
        };
        let account = account(&store, &[limited], stream(), &dir).await;

        run(&account).await;
        pass_time(&account, 599);
        let during = run(&account).await;

        assert!(during.blocked.unwrap().contains("cooling down for 1 s"));
        assert!(!is_expired(&account).await);
        assert_eq!(account.transport.sent(), 1);
        assert_eq!(account.ytdlp.runs(), 0);
    }

    #[tokio::test]
    async fn a_bot_check_pauses_every_later_run() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let ytdlp = FakeYtDlp::failing("ERROR: Sign in to confirm you're not a bot");
        let account = account(&store, &[library()], ytdlp, &dir).await;

        run(&account).await;
        pass_time(&account, 1_000_000);
        let later = run(&account).await;

        assert!(later.blocked.unwrap().contains("paused"));
        assert_eq!(account.transport.sent(), 1);
    }

    #[tokio::test]
    async fn a_refused_range_request_is_reported_but_keeps_the_session() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[library(), response(403, "")], stream(), &dir).await;

        let report = run(&account).await;

        assert_eq!(report.resolve.unwrap().unwrap().range_status, 403);
        assert!(!is_expired(&account).await);
    }

    #[tokio::test]
    async fn pauses_after_three_failed_runs() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let failures = [response(500, ""), response(500, ""), response(500, "")];
        let account = account(&store, &failures, stream(), &dir).await;

        for _ in 0..3 {
            assert!(run(&account).await.browse.is_some());
            pass_time(&account, 60);
        }
        let fourth = run(&account).await;

        assert!(fourth.blocked.unwrap().contains("paused"));
        assert_eq!(account.transport.sent(), 3);
    }

    #[tokio::test]
    async fn spaces_runs_apart() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[library(), response(206, "")], stream(), &dir).await;
        run(&account).await;

        let again = run(&account).await;

        assert!(again.blocked.unwrap().contains("too soon"));
        assert_eq!(account.transport.sent(), 2);
    }

    #[tokio::test]
    async fn is_idle_only_after_the_running_check_ends() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[], stream(), &dir).await;
        let short = std::time::Duration::from_millis(20);

        let running = account.gate.lock().await;
        let busy = tokio::time::timeout(short, account.wait_until_idle()).await;
        drop(running);
        let idle = tokio::time::timeout(short, account.wait_until_idle()).await;

        assert!(busy.is_err());
        assert!(idle.is_ok());
    }

    #[tokio::test]
    async fn refuses_a_second_run_while_one_is_in_progress() {
        let (store, dir) = (MemoryStore::default(), tempfile::tempdir().unwrap());
        let account = account(&store, &[], stream(), &dir).await;
        let _running = account.gate.lock().await;

        let second = run(&account).await;

        assert!(second.blocked.unwrap().contains("already running"));
        assert_eq!(account.transport.sent(), 0);
    }
}
