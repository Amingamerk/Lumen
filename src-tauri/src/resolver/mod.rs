//! YouTube stream resolving with the bundled yt-dlp and Deno: one run at a
//! time through the YouTube gate, its answer cached until near expiry. There is
//! no resolver trait until Milestone 3.

mod args;
mod jar;
mod process;
mod stream;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use args::{CLIENTS, Paths, watch_url, ytdlp_args};
use jar::CookieJarFile;
pub use jar::default_dir as jar_dir;
pub use process::{Sidecars, YtDlp};
use stream::{MAX_WAIT_MS, StreamInfo};

use crate::audio::Media;
use crate::secrets::SecretStore;
use crate::sources::youtube::{
    GateClock, Generation, Kind, ServiceError, Session, Transport, Verdict, YouTube,
};

/// A cached URL is used again only with at least this much left of it.
const REUSE_MS: u64 = 600_000;

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
    #[error(transparent)]
    Service(#[from] ServiceError),
}

impl ResolveError {
    /// What a run that ended this way means to the gate.
    fn verdict(self) -> Verdict {
        match self {
            Self::AuthRejected => Verdict::Rejected,
            Self::RateLimited => Verdict::RateLimited { retry_after: None },
            Self::BotCheck => Verdict::BotCheck,
            // Nothing reached YouTube, or the song itself is the problem.
            Self::Unavailable | Self::CookieFile | Self::Spawn | Self::Closed => Verdict::Neutral,
            _ => Verdict::Failure,
        }
    }
}

/// Resolved streams of one account, by video id. Kept in memory only.
#[derive(Default)]
struct Cache {
    generation: Generation,
    streams: HashMap<String, StreamInfo>,
}

impl Cache {
    /// Drops what another session resolved.
    fn follow(&mut self, generation: Generation) {
        if self.generation != generation {
            self.generation = generation;
            self.streams.clear();
        }
    }
}

pub struct Resolver<Y, C> {
    ytdlp: Y,
    clock: C,
    jar_dir: PathBuf,
    /// yt-dlp's own cache folder, owned by Lumen.
    cache_dir: PathBuf,
    cache: Mutex<Cache>,
}

impl<Y: YtDlp, C: GateClock> Resolver<Y, C> {
    /// Removes cookie files an earlier run left behind.
    pub fn new(ytdlp: Y, clock: C, jar_dir: PathBuf, cache_dir: PathBuf) -> Self {
        // One that cannot be removed now is tried again at exit.
        let _ = jar::sweep(&jar_dir);
        Self {
            ytdlp,
            clock,
            jar_dir,
            cache_dir,
            cache: Mutex::default(),
        }
    }

    /// The stream of `video`, from the cache or one yt-dlp run, once it may be
    /// loaded. Runs nothing if `wanted` says no when its turn comes.
    pub async fn resolve<S: SecretStore, T: Transport, G: GateClock>(
        &self,
        youtube: &YouTube<S, T, G>,
        video: &str,
        wanted: impl Fn() -> bool,
    ) -> Result<Media, ResolveError> {
        let url = watch_url(video).ok_or(ResolveError::BadVideo)?;
        let generation = youtube.ready()?;
        let stream = match self.cached(generation, video) {
            Some(stream) => stream,
            None => {
                // Looked up first, so a missing sidecar costs nothing.
                let deno = self.ytdlp.deno()?;
                let (url, deno) = (&url, &deno);
                youtube
                    .through_gate(Kind::YtDlp, &wanted, |lease| async move {
                        // It may have been resolved while this run waited.
                        if let Some(stream) = self.cached(lease.generation, video) {
                            return (Verdict::Neutral, Ok(stream));
                        }
                        let outcome = self.run(&lease.session, url, deno).await;
                        match &outcome {
                            // An answer that was paid for is kept, wanted or not.
                            Ok(stream) => {
                                self.keep(lease.generation, video, stream.clone());
                                (Verdict::Success, outcome)
                            }
                            Err(error) => (error.verdict(), outcome),
                        }
                    })
                    .await??
            }
        };
        self.wait_for(&stream).await?;
        if youtube.ready()? != generation {
            return Err(ResolveError::AccountChanged);
        }
        Ok(stream.media())
    }

    /// Drops the stream of `video`, so it is resolved afresh next time.
    pub fn forget(&self, video: &str) {
        self.cache().streams.remove(video);
    }

    /// Drops every stream, as after an account change.
    pub fn forget_all(&self) {
        self.cache().streams.clear();
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

    fn cached(&self, generation: Generation, video: &str) -> Option<StreamInfo> {
        let needed = self.clock.wall_ms().saturating_add(REUSE_MS);
        let mut cache = self.cache();
        cache.follow(generation);
        cache
            .streams
            .get(video)
            .filter(|stream| {
                stream
                    .expires_at
                    .is_some_and(|at| at.saturating_mul(1_000) >= needed)
            })
            .cloned()
    }

    fn keep(&self, generation: Generation, video: &str, stream: StreamInfo) {
        let now = self.clock.wall_ms();
        let mut cache = self.cache();
        cache.follow(generation);
        // Expired ones go, so the map does not only grow.
        cache.streams.retain(|_, kept| {
            kept.expires_at
                .is_some_and(|at| at.saturating_mul(1_000) > now)
        });
        cache.streams.insert(video.to_owned(), stream);
    }

    /// A panic elsewhere must not lose resolving for good.
    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
