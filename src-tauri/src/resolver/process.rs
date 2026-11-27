//! Running the bundled yt-dlp. yt-dlp starts processes of its own (its own
//! bootloader and Deno), so ending a run early ends the whole process tree.

use std::ffi::OsString;
use std::future::Future;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use super::ResolveError;

const TIMEOUT: Duration = Duration::from_secs(90);
/// No console window for a child process.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct ProcessOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The process seam.
pub trait YtDlp: Send + Sync {
    /// The bundled Deno. An error if it or yt-dlp is missing, so nothing runs
    /// without both.
    fn deno(&self) -> Result<PathBuf, ResolveError>;
    fn run(
        &self,
        args: Vec<OsString>,
    ) -> impl Future<Output = Result<ProcessOutput, ResolveError>> + Send;
    /// Ends every run, and refuses new ones.
    fn shut_down(&self);
}

/// The sidecars shipped with the app.
#[derive(Default)]
pub struct Sidecars {
    /// Where to look, in order, each with the suffix file names carry there.
    places: Vec<(PathBuf, &'static str)>,
    running: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    closed: bool,
    pids: Vec<u32>,
}

impl Sidecars {
    /// Beside the app, where a build puts them. A development build also looks
    /// in the source tree, under Tauri's target-triple names.
    pub fn bundled() -> Self {
        let mut places = Vec::new();
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.to_owned()))
        {
            places.push((dir, ".exe"));
        }
        #[cfg(debug_assertions)]
        places.push((
            Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries"),
            "-x86_64-pc-windows-msvc.exe",
        ));
        Self {
            places,
            running: Mutex::default(),
        }
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        self.places
            .iter()
            .map(|(dir, suffix)| dir.join(format!("{name}{suffix}")))
            .find(|path| path.is_file())
    }

    /// Runs `program` to its end. A run that takes longer than `timeout`, or
    /// whose future is dropped, has its process tree killed.
    async fn execute(
        &self,
        program: &Path,
        args: Vec<OsString>,
        timeout: Duration,
    ) -> Result<ProcessOutput, ResolveError> {
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child;
        // Declared after the child, so it is dropped first: the tree can only
        // be walked while its root is alive.
        let tree;
        {
            // Held across the spawn, so a shutdown cannot miss the process.
            let mut running = self.running();
            if running.closed {
                return Err(ResolveError::Closed);
            }
            // Lets a development run count its runs. A fixed text, nothing else.
            if cfg!(debug_assertions) {
                eprintln!("youtube request: resolve");
            }
            child = command.spawn().map_err(|_| ResolveError::Spawn)?;
            let pid = child.id();
            running.pids.extend(pid);
            tree = Tree { owner: self, pid };
        }
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let finished = tokio::time::timeout(timeout, async {
            tokio::join!(child.wait(), read(stdout), read(stderr))
        })
        .await;
        if self.running().closed {
            return Err(ResolveError::Closed);
        }
        let Ok((status, stdout, stderr)) = finished else {
            return Err(ResolveError::TimedOut);
        };
        let output = ProcessOutput {
            success: status.map_err(|_| ResolveError::Failed)?.success(),
            stdout,
            stderr,
        };
        tree.finished();
        Ok(output)
    }

    /// A panic elsewhere must not leave processes unkillable.
    fn running(&self) -> MutexGuard<'_, Running> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the pid off the list. Whether it was still there.
    fn forget(running: &mut Running, pid: u32) -> bool {
        let before = running.pids.len();
        running.pids.retain(|known| *known != pid);
        before != running.pids.len()
    }
}

impl YtDlp for Sidecars {
    fn deno(&self) -> Result<PathBuf, ResolveError> {
        self.find("yt-dlp").ok_or(ResolveError::MissingYtDlp)?;
        self.find("deno").ok_or(ResolveError::MissingDeno)
    }

    async fn run(&self, args: Vec<OsString>) -> Result<ProcessOutput, ResolveError> {
        let program = self.find("yt-dlp").ok_or(ResolveError::MissingYtDlp)?;
        self.execute(&program, args, TIMEOUT).await
    }

    fn shut_down(&self) {
        let mut running = self.running();
        running.closed = true;
        for pid in std::mem::take(&mut running.pids) {
            kill_tree(pid);
        }
    }
}

/// Kills the process tree of one run unless the run finished by itself.
struct Tree<'a> {
    owner: &'a Sidecars,
    pid: Option<u32>,
}

impl Tree<'_> {
    fn finished(mut self) {
        if let Some(pid) = self.pid.take() {
            Sidecars::forget(&mut self.owner.running(), pid);
        }
    }
}

impl Drop for Tree<'_> {
    fn drop(&mut self) {
        let Some(pid) = self.pid else { return };
        // A shutdown that took the pid has killed the tree already.
        let ours = Sidecars::forget(&mut self.owner.running(), pid);
        if ours {
            kill_tree(pid);
        }
    }
}

/// Returns once the tree is gone.
fn kill_tree(pid: u32) {
    // By full path, so nothing else named taskkill can run in its place.
    let windows = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let _ = std::process::Command::new(Path::new(&windows).join(r"System32\taskkill.exe"))
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

async fn read(pipe: Option<impl AsyncRead + Unpin>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        // What arrived before a failure is judged as it is.
        let _ = pipe.read_to_end(&mut bytes).await;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn cmd() -> PathBuf {
        PathBuf::from(std::env::var_os("ComSpec").expect("Windows sets ComSpec"))
    }

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// A child whose own child writes `marker` after about two seconds.
    fn grandchild_writing(marker: &Path) -> Vec<OsString> {
        let script = format!("ping -n 3 127.0.0.1 >nul & echo x>{}", marker.display());
        args(&["/d", "/c", "cmd", "/d", "/c", &script])
    }

    async fn survived(marker: &Path) -> bool {
        tokio::time::sleep(Duration::from_secs(4)).await;
        marker.exists()
    }

    #[test]
    fn needs_both_sidecars_under_either_name() {
        let dir = tempfile::tempdir().unwrap();
        for suffix in [".exe", "-x86_64-pc-windows-msvc.exe"] {
            let sidecars = Sidecars {
                places: vec![(dir.path().to_owned(), suffix)],
                running: Mutex::default(),
            };
            assert_eq!(sidecars.deno(), Err(ResolveError::MissingYtDlp));
            let ytdlp = dir.path().join(format!("yt-dlp{suffix}"));
            std::fs::write(&ytdlp, "").unwrap();
            assert_eq!(sidecars.deno(), Err(ResolveError::MissingDeno));
            let deno = dir.path().join(format!("deno{suffix}"));
            std::fs::write(&deno, "").unwrap();
            assert_eq!(sidecars.deno(), Ok(deno.clone()));
            std::fs::remove_file(ytdlp).unwrap();
            std::fs::remove_file(deno).unwrap();
        }
    }

    #[tokio::test]
    async fn hands_back_the_output_of_a_finished_run() {
        let output = Sidecars::default()
            .execute(
                &cmd(),
                args(&["/d", "/c", "echo out& echo err>&2& exit 3"]),
                TIMEOUT,
            )
            .await
            .unwrap();

        assert!(!output.success);
        assert!(output.stdout.starts_with(b"out"));
        assert!(output.stderr.starts_with(b"err"));
    }

    #[tokio::test]
    async fn a_timeout_kills_the_whole_tree() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let sidecars = Sidecars::default();

        let outcome = sidecars
            .execute(
                &cmd(),
                grandchild_writing(&marker),
                Duration::from_millis(700),
            )
            .await;

        assert_eq!(outcome.err(), Some(ResolveError::TimedOut));
        assert!(!survived(&marker).await);
    }

    #[tokio::test]
    async fn a_dropped_run_kills_the_whole_tree() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let sidecars = Sidecars::default();

        let program = cmd();
        let run = sidecars.execute(&program, grandchild_writing(&marker), TIMEOUT);
        let dropped = tokio::time::timeout(Duration::from_millis(700), run).await;

        assert!(dropped.is_err());
        assert!(!survived(&marker).await);
    }

    #[tokio::test]
    async fn a_shutdown_kills_the_whole_tree_and_refuses_new_runs() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let sidecars = Arc::new(Sidecars::default());
        let running = sidecars.clone();
        let script = grandchild_writing(&marker);
        let run = tokio::spawn(async move { running.execute(&cmd(), script, TIMEOUT).await });
        tokio::time::sleep(Duration::from_millis(700)).await;

        sidecars.shut_down();

        assert_eq!(run.await.unwrap().err(), Some(ResolveError::Closed));
        let refused = sidecars.execute(&cmd(), Vec::new(), TIMEOUT).await;
        assert_eq!(refused.err(), Some(ResolveError::Closed));
        assert!(!survived(&marker).await);
    }
}
