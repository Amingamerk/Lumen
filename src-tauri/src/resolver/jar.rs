//! The short-lived Netscape cookie file handed to yt-dlp. It is the only
//! plain-text copy of the session, so it is removed as soon as the guard is
//! dropped, and stale ones are swept. It is never read back, so cookies yt-dlp
//! rotates are thrown away with it.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::sources::youtube::Cookie;

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
    std::env::temp_dir().join("lumen-cookies")
}

/// Removes jars left behind by a process that ended without running
/// destructors. One that cannot be removed does not keep the others.
pub fn sweep(dir: &Path) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut outcome = Ok(());
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(PREFIX)
            && let Err(error) = std::fs::remove_file(entry.path())
        {
            outcome = Err(error);
        }
    }
    outcome
}

/// `cookies.txt` content, as yt-dlp reads it with `--cookies`.
fn netscape_file(cookies: &[Cookie]) -> String {
    let flag = |on: bool| if on { "TRUE" } else { "FALSE" };
    // A tab or line break would shift the fields of a line.
    let fits = |c: &&Cookie| {
        [&c.name, &c.value, &c.domain, &c.path]
            .iter()
            .all(|field| !field.chars().any(char::is_control))
    };
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies.iter().filter(fits) {
        let prefix = if c.http_only { "#HttpOnly_" } else { "" };
        // Writing to a String cannot fail.
        let _ = writeln!(
            out,
            "{prefix}{}\t{}\t{}\t{}\t{}\t{}\t{}",
            c.domain,
            // Python's cookie jar rejects a file where this disagrees with the
            // leading dot.
            flag(c.domain.starts_with('.')),
            c.path,
            flag(c.secure),
            c.expires.unwrap_or(0).max(0),
            c.name,
            c.value,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::youtube::testing::cookie;

    #[test]
    fn writes_a_netscape_file_and_removes_it_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let mut site = cookie("SAPISID", ".youtube.com");
        site.http_only = true;
        let mut host = cookie("PREF", "music.youtube.com");
        host.expires = Some(1_900_000_000);
        let mut broken = cookie("BAD", ".youtube.com");
        broken.value = "a\tb".to_owned();
        let jar = CookieJarFile::create(&dir.path().join("jars"), &[site, host, broken]).unwrap();
        let path = jar.path().to_owned();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Netscape HTTP Cookie File\n\
             #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tfake-SAPISID\n\
             music.youtube.com\tFALSE\t/\tTRUE\t1900000000\tPREF\tfake-PREF\n"
        );
        drop(jar);
        assert!(!path.exists());
    }

    #[test]
    fn a_panic_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();

        let panicked = std::panic::catch_unwind(|| {
            let _jar = CookieJarFile::create(dir.path(), &[]).unwrap();
            panic!("midway");
        });

        assert!(panicked.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn sweeps_leftover_jars_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("jar-old.txt"), "x").unwrap();
        std::fs::write(dir.path().join("other.txt"), "x").unwrap();

        sweep(dir.path()).unwrap();
        sweep(&dir.path().join("missing")).unwrap();

        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["other.txt"]);
    }
}
