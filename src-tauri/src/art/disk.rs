//! The pictures kept on disk, under a cap. When the cap is passed, the picture
//! used longest ago goes first.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use super::{Size, lock};

pub struct Disk {
    dir: PathBuf,
    cap: u64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    files: HashMap<String, Entry>,
    total: u64,
    /// Counts every use, so the smallest `used` is the longest unused.
    clock: u64,
}

struct Entry {
    len: u64,
    used: u64,
}

const EXTENSION: &str = "jpg";
const UNFINISHED: &str = "tmp";

/// The file a picture is kept in. Keys are hex digests, so a name never holds a
/// separator or a dot.
fn file_name(key: &str, size: Size) -> String {
    format!("{key}-{}.{EXTENSION}", size.name())
}

impl Disk {
    /// Opens `dir`, making it if needed, and counts what is already there.
    pub fn open(dir: PathBuf, cap: u64) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let mut found = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                // A write that never finished.
                Some(UNFINISHED) => {
                    let _ = fs::remove_file(&path);
                }
                Some(EXTENSION) => {
                    let meta = entry.metadata()?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    found.push((meta.modified().ok(), name, meta.len()));
                }
                _ => {}
            }
        }
        // The oldest write counts as the longest unused.
        found.sort();
        let mut state = State::default();
        for (_, name, len) in found {
            state.clock += 1;
            state.total += len;
            state.files.insert(
                name,
                Entry {
                    len,
                    used: state.clock,
                },
            );
        }
        let disk = Self {
            dir,
            cap,
            state: Mutex::new(state),
        };
        disk.evict(None);
        Ok(disk)
    }

    /// The kept picture, if there is one.
    pub fn read(&self, key: &str, size: Size) -> Option<Vec<u8>> {
        let name = file_name(key, size);
        {
            let mut state = lock(&self.state);
            state.clock += 1;
            let clock = state.clock;
            state.files.get_mut(&name)?.used = clock;
        }
        match fs::read(self.dir.join(&name)) {
            Ok(bytes) => Some(bytes),
            Err(_) => {
                // Gone from under us: forget it, so it is fetched again.
                self.forget(&name);
                None
            }
        }
    }

    /// Keeps a picture, then drops the oldest ones until the cap holds.
    pub fn write(&self, key: &str, size: Size, bytes: &[u8]) -> io::Result<()> {
        let name = file_name(key, size);
        let unfinished = self.dir.join(format!("{name}.{UNFINISHED}"));
        fs::write(&unfinished, bytes)?;
        if let Err(error) = fs::rename(&unfinished, self.dir.join(&name)) {
            let _ = fs::remove_file(&unfinished);
            return Err(error);
        }
        let len = bytes.len() as u64;
        {
            let mut state = lock(&self.state);
            state.clock += 1;
            let used = state.clock;
            if let Some(old) = state.files.insert(name.clone(), Entry { len, used }) {
                state.total -= old.len;
            }
            state.total += len;
        }
        self.evict(Some(&name));
        Ok(())
    }

    fn forget(&self, name: &str) {
        let mut state = lock(&self.state);
        if let Some(entry) = state.files.remove(name) {
            state.total -= entry.len;
        }
    }

    /// Drops the longest unused files, never `keep`, until the cap holds.
    fn evict(&self, keep: Option<&str>) {
        let mut state = lock(&self.state);
        while state.total > self.cap {
            let oldest = state
                .files
                .iter()
                .filter(|(name, _)| Some(name.as_str()) != keep)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(name, _)| name.clone());
            let Some(name) = oldest else { break };
            if let Some(entry) = state.files.remove(&name) {
                state.total -= entry.len;
            }
            // A file that cannot be removed is forgotten anyway; the next start
            // counts it again.
            let _ = fs::remove_file(self.dir.join(&name));
        }
    }
}
