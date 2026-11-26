//! The short-lived Netscape cookie file handed to yt-dlp. It is the only plain-text copy of the
//! session, so it is removed as soon as the guard is dropped, and stale ones are swept.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::cookies::{Cookie, netscape_file};

const PREFIX: &str = "jar-";

/// Deletes the file when dropped, also while unwinding from a panic.
pub struct CookieJarFile {
    path: tempfile::TempPath,
}

impl CookieJarFile {
    pub fn create(dir: &Path, cookies: &[Cookie]) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut file = tempfile::Builder::new()
            .prefix(PREFIX)
            .suffix(".txt")
            .tempfile_in(dir)?;
        file.write_all(netscape_file(cookies).as_bytes())?;
        // Close our handle, keep the delete-on-drop guard.
        Ok(Self {
            path: file.into_temp_path(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A folder of our own inside the user's temp folder.
pub fn default_dir() -> PathBuf {
    std::env::temp_dir().join("lumen-ytm-spike")
}

/// Removes jars left behind by a process that ended without running destructors.
pub fn sweep(dir: &Path) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(PREFIX) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::fake::session;

    #[test]
    fn writes_a_netscape_file_and_removes_it_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let jar = CookieJarFile::create(&dir.path().join("jars"), &session().cookies).unwrap();
        let path = jar.path().to_owned();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            crate::cookies::netscape_file(&session().cookies)
        );
        drop(jar);

        assert!(!path.exists());
    }

    #[test]
    fn removes_the_file_when_the_user_of_the_jar_panics() {
        let dir = tempfile::tempdir().unwrap();
        let mut path = PathBuf::new();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let jar = CookieJarFile::create(dir.path(), &session().cookies).unwrap();
            path = jar.path().to_owned();
            panic!("resolver blew up");
        }));

        assert!(outcome.is_err());
        assert!(path.starts_with(dir.path()));
        assert!(!path.exists());
    }

    #[test]
    fn sweep_removes_leftover_jars_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let leftover = dir.path().join("jar-abandoned.txt");
        let unrelated = dir.path().join("notes.txt");
        std::fs::write(&leftover, "old").unwrap();
        std::fs::write(&unrelated, "keep").unwrap();

        sweep(dir.path()).unwrap();

        assert!(!leftover.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn sweep_accepts_a_folder_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();

        assert!(sweep(&dir.path().join("missing")).is_ok());
    }
}
