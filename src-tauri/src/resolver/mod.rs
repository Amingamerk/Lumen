//! Stream resolving behind the [`Resolver`] trait. The first backend runs the
//! bundled yt-dlp and Deno for YouTube: one run at a time through the YouTube
//! gate, its answer kept until near expiry, in memory and in the library, so a
//! replay after a restart starts no run.

mod args;
mod jar;
mod kept;
mod process;
mod stream;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use args::{CLIENTS, Paths, watch_url, ytdlp_args};
use jar::CookieJarFile;
pub use jar::default_dir as jar_dir;
#[cfg(test)]
pub use process::ProcessOutput;
pub use process::{Sidecars, YtDlp};
use stream::MAX_WAIT_MS;
pub use stream::StreamInfo;

use kept::Kept;

use crate::catalog::{SourceKind, SourceRef};
use crate::library::Library;
use crate::secrets::SecretStore;
use crate::sources::youtube::{
    GateClock, Generation, Kind, ServiceError, Session, Transport, Verdict, YouTube,
};

/// A cached URL is used again only with at least this much left of it.
pub const REUSE_MS: u64 = 600_000;

/// Why there is no stream. Fixed texts only: the user is shown them.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum ResolveError {
    #[error("this song has no valid video id")]
    BadVideo,
    #[error("yt-dlp is missing from Lumen's folder")]
    MissingYtDlp,
    #[error("Deno is missing from Lumen's folder")]
    MissingDeno,
    #[error("the temporary sign-in file could not be written")]
    CookieFile,
    #[error("yt-dlp could not be started")]
    Spawn,
    #[error("YouTube Music took too long to answer")]
    TimedOut,
    #[error("Lumen is closing")]
    Closed,
    #[error("YouTube Music refused the session. Log in again")]
    AuthRejected,
    #[error("YouTube Music asked Lumen to slow down")]
    RateLimited,
    #[error("YouTube Music answered with a bot check")]
    BotCheck,
    #[error("no audio format was available for this song")]
    NoFormat,
    #[error("this song is unavailable")]
    Unavailable,
    #[error("YouTube Music could not be asked for this song")]
    Failed,
    #[error("YouTube Music gave an unexpected answer")]
    UnexpectedOutput,
    #[error("the audio was on an unexpected server")]
    UnexpectedHost,
    #[error("the song is not available for more than a minute")]
    AvailableTooLate,
    #[error("the YouTube Music account changed")]
    AccountChanged,
    #[error("this song has no audio Lumen can play yet")]
    Unsupported,
    #[error(transparent)]
    Service(#[from] ServiceError),
}

impl ResolveError {
    /// Whether asking again for the same song could end better. A refusal, a
    /// block or a missing song would not.
    pub fn worth_retrying(self) -> bool {
        matches!(
            self,
            Self::CookieFile
                | Self::Spawn
                | Self::TimedOut
                | Self::NoFormat
                | Self::Failed
                | Self::UnexpectedOutput
                | Self::UnexpectedHost
        )
    }

    /// Whether the song is the problem, so that the next one may do better. A
    /// refusal, a block or a missing tool is not about the song.
    pub fn is_about_the_song(self) -> bool {
        !matches!(
            self,
            Self::MissingYtDlp
                | Self::MissingDeno
                | Self::Closed
                | Self::AuthRejected
                | Self::RateLimited
                | Self::BotCheck
                | Self::AccountChanged
                | Self::Service(_)
        )
    }

    /// What a run that ended this way means to the gate.
    fn verdict(self) -> Verdict {
        match self {
            Self::AuthRejected => Verdict::Rejected,
            Self::RateLimited => Verdict::RateLimited { retry_after: None },
            Self::BotCheck => Verdict::BotCheck,
            // Nothing reached YouTube, or the song itself is the problem.
            Self::Unavailable
            | Self::CookieFile
            | Self::Spawn
            | Self::Closed
            | Self::Unsupported => Verdict::Neutral,
            _ => Verdict::Failure,
        }
    }
}

/// Finds the audio of a song: [`SourceRef`] to [`StreamInfo`]. A backend keeps
/// its own rules for pacing and for what it stores.
pub trait Resolver: Send + Sync {
    /// The stream of `reference`, once it may be loaded. Runs nothing if `wanted`
    /// says no when its turn comes. A song of a service that has no audio here,
    /// Spotify included, is [`ResolveError::Unsupported`] and sends nothing.
    fn resolve(
        &self,
        reference: &SourceRef,
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> impl Future<Output = Result<StreamInfo, ResolveError>> + Send;

    /// Drops the stream of `reference`, so it is resolved afresh next time.
    fn forget(&self, reference: &SourceRef);

    /// Drops every stream, as after an account change.
    fn forget_all(&self);
}

/// Streams of one account in memory, by video id.
#[derive(Default)]
struct Cache {
    /// The session the streams belong to, once one has been seen.
    generation: Option<Generation>,
    streams: HashMap<String, StreamInfo>,
}

impl Cache {
    /// Drops what another session resolved, and says whether it did.
    fn follow(&mut self, generation: Generation) -> bool {
        match self.generation.replace(generation) {
            Some(known) if known != generation => {
                self.streams.clear();
                true
            }
            _ => false,
        }
    }
}

/// Whether `stream` still works at `needed_ms`.
fn lasts_until(stream: &StreamInfo, needed_ms: u64) -> bool {
    stream
        .expires_at
        .is_some_and(|at| at.saturating_mul(1_000) >= needed_ms)
}

/// The video of a YouTube ref. Only YouTube has audio here.
fn video_of(reference: &SourceRef) -> Result<&str, ResolveError> {
    match reference.kind {
        SourceKind::YouTube => Ok(&reference.id),
        SourceKind::SoundCloud | SourceKind::Spotify => Err(ResolveError::Unsupported),
    }
}

/// Resolves YouTube songs with yt-dlp, one run at a time through the gate of the
/// YouTube Music account.
pub struct YtDlpResolver<Y, C, S, T, G> {
    ytdlp: Y,
    clock: C,
    youtube: Arc<YouTube<S, T, G>>,
    jar_dir: PathBuf,
    /// yt-dlp's own cache folder, owned by Lumen.
    cache_dir: PathBuf,
    cache: Mutex<Cache>,
    /// Where streams outlive the app, if the library is open.
    kept: Option<Kept>,
}

impl<Y: YtDlp, C: GateClock, S: SecretStore, T: Transport, G: GateClock>
    YtDlpResolver<Y, C, S, T, G>
{
    /// Removes cookie files an earlier run left behind.
    pub fn new(
        ytdlp: Y,
        clock: C,
        youtube: Arc<YouTube<S, T, G>>,
        jar_dir: PathBuf,
        cache_dir: PathBuf,
    ) -> Self {
        // One that cannot be removed now is tried again at exit.
        let _ = jar::sweep(&jar_dir);
        Self {
            ytdlp,
            clock,
            youtube,
            jar_dir,
            cache_dir,
            cache: Mutex::default(),
            kept: None,
        }
    }

    /// Keeps streams in `library` too. A library that fails costs the saving and
    /// nothing else.
    #[must_use]
    pub fn keeping_in(mut self, library: Arc<Library>) -> Self {
        self.kept = Some(Kept::new(library));
        self
    }

    /// Ends a run in progress and removes the cookie files.
    pub fn shut_down(&self) {
        self.ytdlp.shut_down();
        // The run's own guard may never get to drop at exit. A process that is
        // still ending can hold its file for a moment.
        for _ in 0..20 {
            if jar::sweep(&self.jar_dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// One yt-dlp run. The cookie file lives exactly as long as it does.
    async fn run(
        &self,
        session: &Session,
        url: &str,
        deno: &Path,
    ) -> Result<StreamInfo, ResolveError> {
        let output = {
            let jar = CookieJarFile::create(&self.jar_dir, &session.cookies)
                .map_err(|_| ResolveError::CookieFile)?;
            let paths = Paths {
                deno,
                cache: &self.cache_dir,
                jar: jar.path(),
            };
            let args = ytdlp_args(&paths, &session.user_agent, url, CLIENTS);
            self.ytdlp.run(args).await?
        };
        stream::read(&output, self.clock.wall_ms())
    }

    /// Returns once the stream's URL works.
    async fn wait_for(&self, stream: &StreamInfo) -> Result<(), ResolveError> {
        let at_ms = stream.available_at.unwrap_or(0).saturating_mul(1_000);
        loop {
            let left = at_ms.saturating_sub(self.clock.wall_ms());
            if left == 0 {
                return Ok(());
            }
            if left > MAX_WAIT_MS {
                return Err(ResolveError::AvailableTooLate);
            }
            self.clock.sleep(Duration::from_millis(left)).await;
        }
    }

    /// A stream of this login with enough time left: from memory, else from the
    /// library.
    async fn cached(&self, generation: Generation, login: &str, video: &str) -> Option<StreamInfo> {
        let needed = self.clock.wall_ms().saturating_add(REUSE_MS);
        let (found, changed) = {
            let mut cache = self.cache();
            let changed = cache.follow(generation);
            let found = cache
                .streams
                .get(video)
                .filter(|stream| lasts_until(stream, needed))
                .cloned();
            (found, changed)
        };
        if changed {
            // Another session is signed in: nothing kept belongs to it.
            self.blocking(|kept| kept.clear()).await;
            return None;
        }
        if found.is_some() {
            return found;
        }
        let (login, owned) = (login.to_owned(), video.to_owned());
        let stored = self
            .blocking(move |kept| kept.load(&login, &owned, needed))
            .await??;
        self.cache()
            .streams
            .insert(video.to_owned(), stored.clone());
        Some(stored)
    }

    /// Keeps a stream that was paid for, in memory and in the library.
    async fn keep(&self, generation: Generation, login: &str, video: &str, stream: &StreamInfo) {
        let now = self.clock.wall_ms();
        {
            let mut cache = self.cache();
            cache.follow(generation);
            // Expired ones go, so the map does not only grow.
            cache.streams.retain(|_, kept| {
                kept.expires_at
                    .is_some_and(|at| at.saturating_mul(1_000) > now)
            });
            cache.streams.insert(video.to_owned(), stream.clone());
        }
        let (login, video, stream) = (login.to_owned(), video.to_owned(), stream.clone());
        self.blocking(move |kept| kept.save(&login, &video, &stream))
            .await;
    }

    /// Runs `work` on a blocking thread, where SQLite may wait for a lock.
    /// `None` without a library, or if the thread was lost.
    async fn blocking<R: Send + 'static>(
        &self,
        work: impl FnOnce(&Kept) -> R + Send + 'static,
    ) -> Option<R> {
        let kept = self.kept.clone()?;
        tokio::task::spawn_blocking(move || work(&kept)).await.ok()
    }

    /// A panic elsewhere must not lose resolving for good.
    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<Y, C, S, T, G> Resolver for YtDlpResolver<Y, C, S, T, G>
where
    Y: YtDlp,
    C: GateClock,
    S: SecretStore + Send + Sync,
    T: Transport,
    G: GateClock,
{
    async fn resolve(
        &self,
        reference: &SourceRef,
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> Result<StreamInfo, ResolveError> {
        let video = video_of(reference)?;
        let url = watch_url(video).ok_or(ResolveError::BadVideo)?;
        let (generation, login) = self.youtube.ready_login()?;
        let stream = match self.cached(generation, &login, video).await {
            Some(stream) => stream,
            None => {
                // Looked up first, so a missing sidecar costs nothing.
                let deno = self.ytdlp.deno()?;
                let (url, deno) = (&url, &deno);
                self.youtube
                    .through_gate(Kind::YtDlp, &wanted, |lease| async move {
                        let login = lease.session.login_id();
                        // It may have been resolved while this run waited.
                        if let Some(stream) = self.cached(lease.generation, &login, video).await {
                            return (Verdict::Neutral, Ok(stream));
                        }
                        let outcome = self.run(&lease.session, url, deno).await;
                        match &outcome {
                            // An answer that was paid for is kept, wanted or not.
                            Ok(stream) => {
                                self.keep(lease.generation, &login, video, stream).await;
                                (Verdict::Success, outcome)
                            }
                            Err(error) => (error.verdict(), outcome),
                        }
                    })
                    .await??
            }
        };
        self.wait_for(&stream).await?;
        if self.youtube.ready()? != generation {
            return Err(ResolveError::AccountChanged);
        }
        Ok(stream)
    }

    fn forget(&self, reference: &SourceRef) {
        if video_of(reference).is_err() {
            return;
        }
        self.cache().streams.remove(&reference.id);
        if let Some(kept) = &self.kept {
            kept.forget(&reference.id);
        }
    }

    fn forget_all(&self) {
        self.cache().streams.clear();
        if let Some(kept) = &self.kept {
            kept.clear();
        }
    }
}

/// A stand-in for a resolver, for the tests of the modules that use one.
#[cfg(test)]
pub(crate) mod testing {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;

    use super::*;

    type Answer = Result<StreamInfo, ResolveError>;

    enum Queued {
        Now(Answer),
        Later(oneshot::Receiver<Answer>),
    }

    /// Gives the queued answers in order and remembers what it was asked.
    #[derive(Default)]
    pub struct FakeResolver {
        answers: Mutex<VecDeque<Queued>>,
        resolved: Mutex<Vec<SourceRef>>,
        forgotten: Mutex<Vec<SourceRef>>,
        alive: AtomicUsize,
    }

    /// Counts one resolve for as long as its future lives.
    struct Alive<'a>(&'a AtomicUsize);

    impl Drop for Alive<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl FakeResolver {
        pub fn answer(&self, answer: Answer) {
            self.answers.lock().unwrap().push_back(Queued::Now(answer));
        }

        /// The next resolve stays in progress until it is answered here. The
        /// sender is closed once that resolve was dropped.
        pub fn answer_later(&self) -> oneshot::Sender<Answer> {
            let (answer, later) = oneshot::channel();
            self.answers.lock().unwrap().push_back(Queued::Later(later));
            answer
        }

        /// Resolves in progress now.
        pub fn alive(&self) -> usize {
            self.alive.load(Ordering::SeqCst)
        }

        pub fn resolved(&self) -> Vec<SourceRef> {
            self.resolved.lock().unwrap().clone()
        }

        pub fn forgotten(&self) -> Vec<SourceRef> {
            self.forgotten.lock().unwrap().clone()
        }
    }

    impl Resolver for FakeResolver {
        async fn resolve(
            &self,
            reference: &SourceRef,
            wanted: impl Fn() -> bool + Send + Sync,
        ) -> Result<StreamInfo, ResolveError> {
            self.resolved.lock().unwrap().push(reference.clone());
            if !wanted() {
                return Err(
                    ServiceError::Gate(crate::sources::youtube::GateError::Superseded).into(),
                );
            }
            self.alive.fetch_add(1, Ordering::SeqCst);
            let _alive = Alive(&self.alive);
            let answer = self.answers.lock().unwrap().pop_front();
            match answer {
                Some(Queued::Now(answer)) => answer,
                Some(Queued::Later(later)) => later.await.unwrap_or(Err(ResolveError::Failed)),
                None => Err(ResolveError::Failed),
            }
        }

        fn forget(&self, reference: &SourceRef) {
            self.forgotten.lock().unwrap().push(reference.clone());
        }

        fn forget_all(&self) {
            self.forgotten.lock().unwrap().clear();
        }
    }
}
