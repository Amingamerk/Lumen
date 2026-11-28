//! The play log's way into the library: a thread of its own does the writing,
//! so playback never waits for a database.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{EndReason, HistoryChange, Library, LibraryError, PlayId, PlayLog};
use crate::catalog::Track;

/// How long [`HistoryWriter::finish`] waits for the writes in line.
const PATIENCE: Duration = Duration::from_secs(2);

enum Job {
    Begin {
        id: PlayId,
        track: Box<Track>,
        started_at_ms: u64,
    },
    End {
        id: PlayId,
        listened: Duration,
        reason: EndReason,
    },
    Stop,
}

pub struct HistoryWriter {
    next: AtomicU64,
    jobs: Mutex<Sender<Job>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl HistoryWriter {
    /// Starts the writer. `changed` hears of each play that starts, on the
    /// writer's thread.
    pub fn start(
        library: Arc<Library>,
        changed: impl Fn(HistoryChange) + Send + 'static,
    ) -> Result<Self, LibraryError> {
        let next = AtomicU64::new(library.next_play_id()?);
        let (jobs, queue) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("history-writer".to_owned())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    match job {
                        Job::Begin {
                            id,
                            track,
                            started_at_ms,
                        } => {
                            // A write that fails costs one history row, never playback.
                            if let Ok(change) = library.begin_play(id, &track, started_at_ms) {
                                changed(change);
                            }
                        }
                        Job::End {
                            id,
                            listened,
                            reason,
                        } => {
                            let _ = library.end_play(id, listened, reason);
                        }
                        Job::Stop => break,
                    }
                }
            })?;
        Ok(Self {
            next,
            jobs: Mutex::new(jobs),
            thread: Mutex::new(Some(thread)),
        })
    }

    fn send(&self, job: Job) {
        // The writer ended: the app is closing, and nobody is left to tell.
        let _ = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(job);
    }

    /// Writes what is in line and stops. Whether the writer ended in time.
    pub fn finish(&self) -> bool {
        self.send(Job::Stop);
        let Some(thread) = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return true;
        };
        let started = Instant::now();
        while !thread.is_finished() {
            if started.elapsed() >= PATIENCE {
                return false;
            }
            thread::sleep(Duration::from_millis(5));
        }
        thread.join().is_ok()
    }
}

impl PlayLog for HistoryWriter {
    fn started(&self, track: &Track, started_at_ms: u64) -> PlayId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.send(Job::Begin {
            id,
            track: Box::new(track.clone()),
            started_at_ms,
        });
        id
    }

    fn ended(&self, play: PlayId, listened: Duration, reason: EndReason) {
        self.send(Job::End {
            id: play,
            listened,
            reason,
        });
    }
}

impl PlayLog for Arc<HistoryWriter> {
    fn started(&self, track: &Track, started_at_ms: u64) -> PlayId {
        HistoryWriter::started(self, track, started_at_ms)
    }

    fn ended(&self, play: PlayId, listened: Duration, reason: EndReason) {
        HistoryWriter::ended(self, play, listened, reason);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::library::testing::{START_MS, library_at, song};

    #[test]
    fn plays_reach_the_library_and_are_closed_when_the_writer_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let (library, _) = library_at(&dir.path().join("lumen.db"));
        let library = Arc::new(library);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        let writer = HistoryWriter::start(Arc::clone(&library), move |change| {
            sink.lock().unwrap().push(change);
        })
        .unwrap();

        let first = writer.started(&song(1), START_MS);
        writer.ended(first, Duration::from_secs(30), EndReason::Skipped);
        let second = writer.started(&song(2), START_MS + 31_000);
        writer.ended(second, Duration::from_secs(7), EndReason::Exit);
        assert!(writer.finish());

        assert_eq!((first, second), (1, 2));
        let plays = library.plays();
        assert_eq!(plays.len(), 2);
        assert_eq!(plays[0].listened_ms, 30_000);
        assert_eq!(plays[1].end_reason.as_deref(), Some("exit"));
        assert_eq!(
            *heard.lock().unwrap(),
            [
                HistoryChange { played_count: 1 },
                HistoryChange { played_count: 2 }
            ]
        );
    }

    #[test]
    fn ids_go_on_after_the_plays_of_an_earlier_run() {
        let dir = tempfile::tempdir().unwrap();
        let (library, _) = library_at(&dir.path().join("lumen.db"));
        library.begin_play(41, &song(1), START_MS).unwrap();
        let writer = HistoryWriter::start(Arc::new(library), |_| {}).unwrap();

        assert_eq!(writer.started(&song(2), START_MS), 42);
        assert!(writer.finish());
    }
}
