//! The gate's state on disk, so a cool-down or pause survives a restart. It
//! holds no secrets. Anything unreadable loads as paused.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::policy::{GateState, PauseReason};

const VERSION: u32 = 1;

/// The file's content. Every field is required.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    version: u32,
    cooldown_until_ms: u64,
    rate_limits: u32,
    failures: u32,
    // Without this a missing `paused` would read as "not paused".
    #[serde(deserialize_with = "Option::deserialize")]
    paused: Option<PauseReason>,
}

pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// A missing file is a fresh state.
    pub fn load(&self) -> GateState {
        let content = match fs::read(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return GateState::default(),
            Err(_) => return unreadable(),
        };
        match serde_json::from_slice::<Stored>(&content) {
            Ok(stored) if stored.version == VERSION => GateState {
                cooldown_until_ms: stored.cooldown_until_ms,
                rate_limits: stored.rate_limits,
                failures: stored.failures,
                paused: stored.paused,
            },
            _ => unreadable(),
        }
    }

    /// Writes a temporary file and renames it over the old one, so a crash
    /// leaves one of the two whole.
    pub fn save(&self, state: &GateState) -> io::Result<()> {
        let content = serde_json::to_vec(&Stored {
            version: VERSION,
            cooldown_until_ms: state.cooldown_until_ms,
            rate_limits: state.rate_limits,
            failures: state.failures,
            paused: state.paused,
        })?;
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &self.path)
    }
}

fn unreadable() -> GateState {
    GateState {
        paused: Some(PauseReason::StateUnreadable),
        ..GateState::default()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A new empty directory, removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            static COUNT: AtomicU32 = AtomicU32::new(0);
            let count = COUNT.fetch_add(1, Ordering::Relaxed);
            let name = format!("lumen-test-{}-{count}", std::process::id());
            let path = std::env::temp_dir().join(name);
            // A leftover of an earlier run with the same process id.
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::TempDir, *};

    fn file(dir: &TempDir) -> StateFile {
        StateFile::new(dir.path().join("youtube-gate.json"))
    }

    #[test]
    fn a_missing_file_is_a_fresh_state() {
        let dir = TempDir::new();

        assert_eq!(file(&dir).load(), GateState::default());
    }

    #[test]
    fn everything_round_trips() {
        let dir = TempDir::new();
        for paused in [
            None,
            Some(PauseReason::BotCheck),
            Some(PauseReason::RepeatedFailures),
            Some(PauseReason::StateUnreadable),
            Some(PauseReason::StateUnwritable),
            Some(PauseReason::ExpiryUnsaved),
        ] {
            let state = GateState {
                cooldown_until_ms: 1_700_000_000_000,
                rate_limits: 3,
                failures: 2,
                paused,
            };

            file(&dir).save(&state).unwrap();

            assert_eq!(file(&dir).load(), state);
        }
    }

    #[test]
    fn the_file_has_the_documented_shape() {
        let dir = TempDir::new();
        let state = GateState {
            cooldown_until_ms: 5,
            rate_limits: 1,
            failures: 2,
            paused: Some(PauseReason::BotCheck),
        };

        file(&dir).save(&state).unwrap();

        let written = std::fs::read_to_string(dir.path().join("youtube-gate.json")).unwrap();
        assert_eq!(
            written,
            r#"{"version":1,"cooldownUntilMs":5,"rateLimits":1,"failures":2,"paused":"botCheck"}"#
        );
    }

    #[test]
    fn anything_unreadable_loads_as_paused() {
        let dir = TempDir::new();
        for content in [
            "",
            "garbage",
            "{}",
            r#"{"version":1,"cooldownUntilMs":0,"rateLimits":0,"#,
            r#"{"version":2,"cooldownUntilMs":0,"rateLimits":0,"failures":0,"paused":null}"#,
            // A missing pause must not read as "not paused".
            r#"{"version":1,"cooldownUntilMs":0,"rateLimits":0,"failures":0}"#,
            r#"{"version":1,"cooldownUntilMs":0,"rateLimits":0,"failures":0,"paused":"x"}"#,
        ] {
            std::fs::write(dir.path().join("youtube-gate.json"), content).unwrap();

            assert_eq!(file(&dir).load(), unreadable(), "{content}");
        }
        // A directory cannot be read as a file.
        assert_eq!(
            StateFile::new(dir.path().to_path_buf()).load(),
            unreadable()
        );
    }

    #[test]
    fn a_save_replaces_the_file_and_creates_its_folder() {
        let dir = TempDir::new();
        let nested = StateFile::new(dir.path().join("data").join("youtube-gate.json"));
        let mut state = unreadable();
        nested.save(&state).unwrap();
        state.paused = None;

        nested.save(&state).unwrap();

        assert_eq!(nested.load(), state);
        // No temporary file stays behind.
        assert_eq!(
            std::fs::read_dir(dir.path().join("data")).unwrap().count(),
            1
        );
    }

    #[test]
    fn a_failed_save_is_reported() {
        let dir = TempDir::new();
        let blocked = dir.path().join("blocked");
        std::fs::create_dir_all(blocked.join("in-the-way")).unwrap();

        assert!(StateFile::new(blocked).save(&GateState::default()).is_err());
    }
}
