//! The resolver over a fake yt-dlp. Nothing here contacts YouTube.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::oneshot;
use tokio::task::yield_now;

use super::process::ProcessOutput;
use super::*;
use crate::secrets::fake::MemoryStore;
use crate::sources::youtube::testing::{
    FakeClock, FakeTransport, TempDir, music_page, signed_in_cookies,
};
use crate::sources::youtube::{Account, GateError, GateView};

const NOW_S: u64 = 1_700_000_000;
const VIDEO: &str = "dQw4w9WgXcQ";
const STREAM: &str = "https://rr1---sn-fake.googlevideo.com/videoplayback?expire=1700021600&ip=MARKER-IP&sig=MARKER-SIG";

type Service = YouTube<MemoryStore, FakeTransport, FakeClock>;

/// Answers every run the same way and records what it was given.
#[derive(Default)]
struct Fake {
    success: bool,
    stdout: String,
    stderr: String,
    missing: Option<ResolveError>,
    /// The arguments and the cookie file of each run.
    runs: Mutex<Vec<(Vec<OsString>, String)>>,
    /// Taken by the next run, which then waits for it.
    hold: Mutex<Option<oneshot::Receiver<()>>>,
}

impl YtDlp for Arc<Fake> {
    fn deno(&self) -> Result<PathBuf, ResolveError> {
        self.missing.map_or(Ok("deno.exe".into()), Err)
    }

    async fn run(&self, args: Vec<OsString>) -> Result<ProcessOutput, ResolveError> {
        let flag = args.iter().position(|a| a == "--cookies").unwrap();
        let jar = std::fs::read_to_string(&args[flag + 1]).unwrap();
        self.runs.lock().unwrap().push((args, jar));
        let hold = self.hold.lock().unwrap().take();
        if let Some(hold) = hold {
            hold.await.unwrap();
        }
        Ok(ProcessOutput {
            success: self.success,
            stdout: self.stdout.clone().into_bytes(),
            stderr: self.stderr.clone().into_bytes(),
        })
    }

    fn shut_down(&self) {}
}

fn answer(url: &str, available_at: Option<f64>) -> String {
    json!({
        "url": url, "available_at": available_at,
        "http_headers": {
            "User-Agent": "FakeAgent/1.0",
            "Accept": "*/*",
            "Cookie": "SAPISID=MARKER-COOKIE",
            "authorization": "MARKER-AUTH",
            "Range": "bytes=0-"
        }
    })
    .to_string()
}

fn printing(stdout: String) -> Fake {
    Fake {
        success: true,
        stdout,
        ..Fake::default()
    }
}

fn failing(stderr: &str) -> Fake {
    Fake {
        stderr: stderr.to_owned(),
        ..Fake::default()
    }
}

struct Fixture {
    _gate: TempDir,
    jars: tempfile::TempDir,
    clock: FakeClock,
    ytdlp: Arc<Fake>,
    youtube: Arc<Service>,
    resolver: Arc<Resolver<Arc<Fake>, FakeClock>>,
}

fn fixture(ytdlp: Fake) -> Fixture {
    let gate = TempDir::new();
    let jars = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(NOW_S * 1_000);
    let youtube = Arc::new(Service::new(
        Account::new(MemoryStore::default()),
        FakeTransport::default(),
        clock.clone(),
        gate.path().join("youtube-gate.json"),
        |_| {},
    ));
    sign_in(&youtube);
    let ytdlp = Arc::new(ytdlp);
    let resolver = Arc::new(Resolver::new(
        ytdlp.clone(),
        clock.clone(),
        jars.path().to_owned(),
        "cache".into(),
    ));
    Fixture {
        _gate: gate,
        jars,
        clock,
        ytdlp,
        youtube,
        resolver,
    }
}

fn sign_in(youtube: &Service) {
    let session = Session::capture(music_page(), signed_in_cookies()).unwrap();
    let attempt = youtube.account().begin_login();
    youtube.complete_login(attempt, &session, |_| {}).unwrap();
}

impl Fixture {
    async fn resolve(&self) -> Result<Media, ResolveError> {
        self.resolver.resolve(&self.youtube, VIDEO, || true).await
    }

    fn runs(&self) -> usize {
        self.ytdlp.runs.lock().unwrap().len()
    }

    fn jars_left(&self) -> usize {
        std::fs::read_dir(self.jars.path()).unwrap().count()
    }
}

async fn settle() {
    for _ in 0..20 {
        yield_now().await;
    }
}

#[tokio::test]
async fn gives_the_stream_without_the_secret_headers() {
    let f = fixture(printing(answer(STREAM, None)));

    let media = f.resolve().await.unwrap();

    assert!(
        media
            == Media::Stream {
                url: STREAM.to_owned(),
                headers: vec![
                    ("Accept".to_owned(), "*/*".to_owned()),
                    ("User-Agent".to_owned(), "FakeAgent/1.0".to_owned()),
                ],
                expires_at: Some(1_700_021_600),
            }
    );
    let runs = f.ytdlp.runs.lock().unwrap();
    let (args, jar) = &runs[0];
    let agent = args.iter().position(|a| a == "--user-agent").unwrap();
    assert_eq!(args[agent + 1], "FakeAgent/1.0");
    assert!(jar.contains("\tSAPISID\tfake-SAPISID"), "{jar}");
    assert_eq!(f.jars_left(), 0);
}

#[test]
fn debug_output_never_shows_the_url_or_a_header() {
    let output = ProcessOutput {
        success: true,
        stdout: answer(STREAM, None).into_bytes(),
        stderr: Vec::new(),
    };
    let info = stream::read(&output, NOW_S * 1_000).unwrap();

    let shown = format!("{info:?}{:?}", info.media());

    assert!(!shown.contains("MARKER"), "{shown}");
    assert!(!shown.contains("googlevideo"), "{shown}");
    assert_eq!(info.expires_at, Some(1_700_021_600));
}

#[tokio::test]
async fn nothing_is_handed_out_before_available_at() {
    let f = fixture(printing(answer(STREAM, Some(NOW_S as f64 + 5.5))));

    f.resolve().await.unwrap();

    assert_eq!(f.clock.wall_ms(), (NOW_S + 6) * 1_000);
    assert_eq!(f.clock.slept(), [Duration::from_secs(6)]);
}

#[tokio::test]
async fn a_wait_over_a_minute_is_an_error() {
    let f = fixture(printing(answer(STREAM, Some(NOW_S as f64 + 60.5))));

    assert!(f.resolve().await == Err(ResolveError::AvailableTooLate));
    assert!(f.clock.slept().is_empty());
}

#[tokio::test]
async fn only_https_googlevideo_is_loaded() {
    for url in [
        "https://example.org/videoplayback",
        "http://rr1.googlevideo.com/videoplayback",
        "https://googlevideo.com.example.org/videoplayback",
        "https://user@rr1.googlevideo.com/videoplayback",
        "https://rr1.googlevideo.com:8443/videoplayback",
        "file:///c:/x",
    ] {
        let f = fixture(printing(answer(url, None)));

        assert!(
            f.resolve().await == Err(ResolveError::UnexpectedHost),
            "{url}"
        );
    }
}

#[tokio::test]
async fn a_fresh_url_is_used_again_without_a_run() {
    let f = fixture(printing(answer(STREAM, None)));
    f.resolve().await.unwrap();

    // Exactly ten minutes left.
    f.clock.advance(Duration::from_secs(21_000));
    f.resolve().await.unwrap();
    assert_eq!(f.runs(), 1);

    f.clock.advance(Duration::from_secs(1));
    f.resolve().await.unwrap();
    assert_eq!(f.runs(), 2);
}

#[tokio::test]
async fn an_account_change_or_a_forget_drops_the_cache() {
    let f = fixture(printing(answer(STREAM, None)));
    f.resolve().await.unwrap();

    sign_in(&f.youtube);
    f.resolve().await.unwrap();
    f.resolver.forget(VIDEO);
    f.resolve().await.unwrap();
    f.resolver.forget_all();
    f.resolve().await.unwrap();

    assert_eq!(f.runs(), 4);
}

#[tokio::test]
async fn a_cached_url_is_not_given_without_a_session() {
    let f = fixture(printing(answer(STREAM, None)));
    f.resolve().await.unwrap();

    f.youtube.disconnect(|_| {}).unwrap();

    assert!(f.resolve().await == Err(ServiceError::Disconnected.into()));
    assert_eq!(f.runs(), 1);
}

#[tokio::test]
async fn a_missing_sidecar_runs_nothing_and_counts_nothing() {
    for missing in [ResolveError::MissingYtDlp, ResolveError::MissingDeno] {
        let f = fixture(Fake {
            missing: Some(missing),
            ..Fake::default()
        });

        for _ in 0..4 {
            assert!(f.resolve().await == Err(missing));
        }
        assert_eq!(f.runs(), 0);
        assert_eq!(f.youtube.status().gate, GateView::Ready);
        assert_eq!(f.jars_left(), 0);
    }
}

#[tokio::test]
async fn a_bad_video_id_runs_nothing() {
    let f = fixture(Fake::default());

    let outcome = f.resolver.resolve(&f.youtube, "--exec=calc", || true).await;

    assert!(outcome == Err(ResolveError::BadVideo));
    assert_eq!(f.runs(), 0);
}

#[tokio::test]
async fn three_quick_choices_run_twice_at_most_and_only_the_last_is_wanted() {
    let f = fixture(printing(answer(STREAM, None)));
    let (release, hold) = oneshot::channel();
    *f.ytdlp.hold.lock().unwrap() = Some(hold);
    let newest = Arc::new(Mutex::new(0));
    let mut choices = Vec::new();
    for (number, video) in ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"]
        .into_iter()
        .enumerate()
    {
        *newest.lock().unwrap() = number;
        let (resolver, youtube, newest) = (f.resolver.clone(), f.youtube.clone(), newest.clone());
        choices.push(tokio::spawn(async move {
            let wanted = move || *newest.lock().unwrap() == number;
            resolver.resolve(&youtube, video, wanted).await
        }));
        settle().await;
    }
    assert_eq!(f.runs(), 1);

    release.send(()).unwrap();

    let mut outcomes = Vec::new();
    for choice in choices {
        outcomes.push(choice.await.unwrap());
    }
    assert!(outcomes[1] == Err(ServiceError::Gate(GateError::Superseded).into()));
    assert!(outcomes[2].is_ok());
    let runs = f.ytdlp.runs.lock().unwrap();
    assert_eq!(runs.len(), 2);
    let last = runs[1].0.last().unwrap().to_str().unwrap();
    assert!(last.ends_with("ccccccccccc"));
}

/// Printed by yt-dlp on successful runs too. It must not count as a 403.
const PO_TOKEN_WARNING: &str = "WARNING: [youtube] x: web_music client https formats require a GVS PO Token which was not provided. They will be skipped as they may yield HTTP Error 403.";

#[tokio::test]
async fn failures_become_fixed_categories_that_quote_nothing() {
    use ResolveError::*;
    let cases = [
        ("ERROR: x: HTTP Error 429: Too Many Requests", RateLimited),
        ("ERROR: x: Sign in to confirm you're not a bot.", BotCheck),
        (
            "ERROR: [youtube] x: Unable to download API page: HTTP Error 401: Unauthorized",
            AuthRejected,
        ),
        (
            "ERROR: [youtube] x: Unable to download API page: HTTP Error 403: Forbidden",
            AuthRejected,
        ),
        (
            "ERROR: [youtube] x: Sign in to view this video",
            AuthRejected,
        ),
        (
            "ERROR: unable to download video data: HTTP Error 403: Forbidden",
            Failed,
        ),
        (
            "ERROR: [youtube] x: HTTP Error 403: Forbidden (https://rr1.googlevideo.com/videoplayback?ip=MARKER-IP)",
            Failed,
        ),
        ("ERROR: x: HTTP Error 403: Forbidden", Failed),
        ("ERROR: x: HTTP Error 401: Unauthorized", Failed),
        ("ERROR: x: Requested format is not available.", NoFormat),
        ("ERROR: x: Video unavailable", Unavailable),
        (
            "ERROR: x: Private video. Sign in if you have access",
            Unavailable,
        ),
        (
            "ERROR: x: Sign in to confirm your age. This video may be inappropriate for some users.",
            Unavailable,
        ),
        (
            "ERROR: MARKER https://rr1.googlevideo.com/?ip=MARKER-IP",
            Failed,
        ),
        // Warning signs end a run even though yt-dlp carried on.
        (
            "WARNING: The provided YouTube account cookies are no longer valid.",
            AuthRejected,
        ),
        (
            "WARNING: HTTP Error 429: Too Many Requests. Retrying",
            RateLimited,
        ),
        (
            "WARNING: Our systems have detected unusual traffic",
            BotCheck,
        ),
    ];
    for (line, expected) in cases {
        let f = fixture(Fake {
            success: line.starts_with("WARNING"),
            stdout: answer(STREAM, None),
            stderr: format!("{PO_TOKEN_WARNING}\n{line}\n"),
            ..Fake::default()
        });

        let error = f.resolve().await.err().unwrap();

        assert_eq!(error, expected, "{line}");
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("MARKER"), "{shown}");
        assert_eq!(f.jars_left(), 0);
    }
}

#[tokio::test]
async fn output_without_a_direct_url_is_refused() {
    for stdout in ["", "MARKER not json", r#"{"format_id": "251"}"#] {
        let f = fixture(printing(stdout.into()));

        assert!(
            f.resolve().await == Err(ResolveError::UnexpectedOutput),
            "{stdout}"
        );
    }
}

#[tokio::test]
async fn unavailable_songs_never_pause_but_three_failures_do() {
    let unavailable = fixture(failing("ERROR: x: Video unavailable"));
    let broken = fixture(failing("ERROR: something new"));

    for _ in 0..3 {
        let _ = unavailable.resolve().await;
        let _ = broken.resolve().await;
    }

    assert_eq!(unavailable.youtube.status().gate, GateView::Ready);
    assert_ne!(broken.youtube.status().gate, GateView::Ready);
}

#[tokio::test]
async fn each_serious_answer_gets_the_gates_reaction() {
    let limited = fixture(failing("ERROR: x: HTTP Error 429: Too Many Requests"));
    let rejected = fixture(failing(
        "ERROR: [youtube] x: Unable to download API page: HTTP Error 401: Unauthorized",
    ));

    let _ = limited.resolve().await;
    let _ = rejected.resolve().await;

    let gate = limited.youtube.status().gate;
    assert!(matches!(gate, GateView::CoolingDown { .. }));
    assert!(rejected.resolve().await == Err(ServiceError::Expired.into()));
    assert_eq!(rejected.runs(), 1);
}

#[test]
fn a_new_resolver_and_a_shutdown_sweep_leftover_jars() {
    let f = fixture(Fake::default());
    let leftover = f.jars.path().join("jar-leftover.txt");
    std::fs::write(&leftover, "x").unwrap();
    f.resolver.shut_down();
    assert!(!leftover.exists());

    std::fs::write(&leftover, "x").unwrap();
    Resolver::new(
        f.ytdlp.clone(),
        f.clock.clone(),
        f.jars.path().to_owned(),
        "cache".into(),
    );
    assert!(!leftover.exists());
}

#[test]
fn only_answers_that_could_change_are_worth_asking_again() {
    use ResolveError::*;
    for error in [TimedOut, Failed, NoFormat, UnexpectedOutput, Spawn] {
        assert!(error.worth_retrying(), "{error}");
    }
    for error in [
        AuthRejected,
        RateLimited,
        BotCheck,
        Unavailable,
        MissingYtDlp,
        MissingDeno,
        AvailableTooLate,
        AccountChanged,
        BadVideo,
        Closed,
        Service(ServiceError::Gate(GateError::Paused)),
        Service(ServiceError::Expired),
    ] {
        assert!(!error.worth_retrying(), "{error}");
    }
}
