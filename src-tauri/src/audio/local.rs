//! Audio files on this computer, and the `--play-file` launch option.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf, Prefix};

use super::Media;
use crate::catalog::TrackRow;

/// The path after `--play-file`, empty when none was given.
pub fn play_file_arg<I>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter().skip_while(|arg| arg != "--play-file");
    args.next()?;
    let path = args
        .next()
        .filter(|next| !next.to_string_lossy().starts_with("--"));
    Some(path.map(PathBuf::from).unwrap_or_default())
}

/// The full path of a file on a drive of this computer. `None` for anything mpv
/// would fetch from elsewhere: a URL or a network share.
pub fn local_path(path: &Path) -> Option<PathBuf> {
    if path.to_string_lossy().contains("://") {
        return None;
    }
    let full = std::path::absolute(path).ok()?;
    let on_a_drive = matches!(
        full.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    );
    on_a_drive.then_some(full)
}

/// The media and the row to show for a file. Only a file on this computer gets
/// its name shown: anything else could be a URL.
pub fn file_media(path: PathBuf) -> (Media, TrackRow) {
    let title = local_path(&path)
        .and_then(|full| Some(full.file_stem()?.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "that file".to_owned());
    (Media::File(path), TrackRow::untagged(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<PathBuf> {
        play_file_arg(args.iter().map(OsString::from))
    }

    #[test]
    fn reads_the_path_after_the_option() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&["--list"]), None);
        assert_eq!(
            parse(&["--list", "--play-file", r"C:\a b\tone.wav", "--x"]),
            Some(PathBuf::from(r"C:\a b\tone.wav"))
        );
        for args in [&["--play-file"][..], &["--play-file", "--list"]] {
            assert_eq!(parse(args), Some(PathBuf::new()), "{args:?}");
        }
    }

    #[test]
    fn a_relative_path_is_made_full() {
        let full = local_path(Path::new("tone.wav")).unwrap();

        assert!(full.is_absolute());
        assert!(full.ends_with("tone.wav"));
    }

    #[test]
    fn urls_shares_and_devices_are_not_local() {
        for path in [
            "https://media.invalid/a.mp3",
            "file://C:/a.mp3",
            "edl://a.mp3",
            r"\\server\share\a.mp3",
            "//server/share/a.mp3",
            r"\\?\UNC\server\share\a.mp3",
            r"\\.\pipe\a",
            "",
        ] {
            assert_eq!(local_path(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn the_row_shows_a_local_file_by_name_and_never_a_url() {
        let (media, row) = file_media(PathBuf::from(r"C:\music\Harbor Lights.flac"));
        assert_eq!(
            media,
            Media::File(PathBuf::from(r"C:\music\Harbor Lights.flac"))
        );
        assert_eq!(row.title, "Harbor Lights");

        let (_, row) = file_media(PathBuf::from("https://media.invalid/SECRET?sig=1"));
        assert_eq!(row.title, "that file");
    }
}
