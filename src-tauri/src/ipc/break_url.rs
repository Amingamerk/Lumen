//! `--break-url`, in debug builds only: the first Stream URL about to be played
//! is spoiled, so recovery can be watched in the real app.

use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::audio::Media;

const FLAG: &str = "--break-url";

/// Whether the next stream is still to be spoiled.
pub struct BreakUrl(AtomicBool);

impl BreakUrl {
    pub fn from_args<A: AsRef<OsStr>>(args: impl IntoIterator<Item = A>) -> Self {
        let asked = args.into_iter().any(|arg| arg.as_ref() == FLAG);
        Self(AtomicBool::new(asked))
    }

    /// Spoils the first stream it is given, if the app was started with the
    /// flag: its host then refuses it.
    pub fn apply(&self, mut media: Media) -> Media {
        if let Media::Stream { url, .. } = &mut media
            && self.0.swap(false, Ordering::SeqCst)
        {
            // The signature covers the expiry, so a changed one is refused.
            *url = url
                .replacen("expire=", "expire=1", 1)
                .replacen("/expire/", "/expire/1", 1);
        }
        media
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn stream(url: &str) -> Media {
        Media::Stream {
            url: url.to_owned(),
            headers: Vec::new(),
            expires_at: Some(7),
        }
    }

    #[test]
    fn spoils_the_first_stream_only() {
        let flag = BreakUrl::from_args(["lumen.exe", "--break-url"]);
        let sample = Media::Sample {
            duration: Duration::from_secs(1),
        };
        assert!(flag.apply(sample.clone()) == sample);

        let broken = flag.apply(stream("https://a.invalid/x?expire=17&sig=a"));
        let next = flag.apply(stream("https://a.invalid/x?expire=17&sig=a"));

        assert!(broken == stream("https://a.invalid/x?expire=117&sig=a"));
        assert!(next == stream("https://a.invalid/x?expire=17&sig=a"));
    }

    #[test]
    fn spoils_the_manifest_form_too() {
        let flag = BreakUrl::from_args(["--break-url"]);

        let broken = flag.apply(stream("https://a.invalid/expire/17/x"));

        assert!(broken == stream("https://a.invalid/expire/117/x"));
    }

    #[test]
    fn does_nothing_without_the_flag() {
        let flag = BreakUrl::from_args(["lumen.exe", "--break-urls"]);

        let media = flag.apply(stream("https://a.invalid/x?expire=17"));

        assert!(media == stream("https://a.invalid/x?expire=17"));
    }
}
