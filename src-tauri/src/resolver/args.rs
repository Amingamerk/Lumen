//! What yt-dlp is asked to do. Nothing else decides its arguments.

use std::ffi::OsString;
use std::path::Path;

/// Which YouTube clients yt-dlp may pick from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clients {
    /// yt-dlp picks. Signed in, some of its picks send their own fixed
    /// User-Agent.
    Default,
    /// Only clients without a built-in User-Agent, so `--user-agent` applies to
    /// them.
    #[cfg_attr(not(test), allow(dead_code))]
    PinnedUserAgent,
}

/// The one place the choice is made. The defaults are what spike #4 proved on a
/// signed-in account; #60 settles whether `web_music,web` works too.
pub const CLIENTS: Clients = Clients::Default;

/// The paths one run needs.
pub struct Paths<'a> {
    pub deno: &'a Path,
    pub cache: &'a Path,
    pub jar: &'a Path,
}

/// The track's page, rebuilt from nothing but a well-formed video id.
pub fn watch_url(video: &str) -> Option<String> {
    let well_formed = video.len() == 11
        && video
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
    well_formed.then(|| format!("https://music.youtube.com/watch?v={video}"))
}

pub fn ytdlp_args(paths: &Paths, user_agent: &str, url: &str, clients: Clients) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-J",
        "-f",
        "bestaudio",
        // No config file or plugin of the user's changes what is sent or run.
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-playlist",
        "--playlist-items",
        "1",
        // Every attempt passes the gate; yt-dlp must not retry on its own.
        "--retries",
        "0",
        "--extractor-retries",
        "0",
        "--fragment-retries",
        "0",
        "--file-access-retries",
        "0",
        // Only the bundled Deno, never one found on this computer.
        "--no-js-runtimes",
        "--js-runtimes",
    ]
    .map(OsString::from)
    .into();
    let mut runtime = OsString::from("deno:");
    runtime.push(paths.deno);
    args.push(runtime);
    args.extend(["--cache-dir".into(), paths.cache.into()]);
    // Without this yt-dlp picks a random Chrome version for every process.
    args.extend(["--user-agent".into(), user_agent.into()]);
    args.extend(["--cookies".into(), paths.jar.into()]);
    if clients == Clients::PinnedUserAgent {
        args.extend([
            "--extractor-args".into(),
            "youtube:player_client=web_music,web".into(),
        ]);
    }
    args.extend(["--".into(), url.into()]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = "https://music.youtube.com/watch?v=dQw4w9WgXcQ";

    fn args(clients: Clients) -> Vec<OsString> {
        let paths = Paths {
            deno: Path::new("deno.exe"),
            cache: Path::new("cache"),
            jar: Path::new("jar.txt"),
        };
        ytdlp_args(&paths, "Agent/1.0", TRACK, clients)
    }

    #[test]
    fn builds_the_exact_isolated_invocation() {
        assert_eq!(
            args(Clients::Default),
            [
                "-J",
                "-f",
                "bestaudio",
                "--ignore-config",
                "--no-plugin-dirs",
                "--no-playlist",
                "--playlist-items",
                "1",
                "--retries",
                "0",
                "--extractor-retries",
                "0",
                "--fragment-retries",
                "0",
                "--file-access-retries",
                "0",
                "--no-js-runtimes",
                "--js-runtimes",
                "deno:deno.exe",
                "--cache-dir",
                "cache",
                "--user-agent",
                "Agent/1.0",
                "--cookies",
                "jar.txt",
                "--",
                TRACK
            ]
        );
    }

    #[test]
    fn the_app_uses_the_default_clients() {
        assert_eq!(CLIENTS, Clients::Default);
    }

    #[test]
    fn can_limit_yt_dlp_to_clients_that_use_the_pinned_user_agent() {
        let args = args(Clients::PinnedUserAgent);

        let flag = args.iter().position(|a| a == "--extractor-args").unwrap();
        assert_eq!(args[flag + 1], "youtube:player_client=web_music,web");
        assert_eq!(args[args.len() - 2..], ["--", TRACK]);
    }

    #[test]
    fn rebuilds_the_url_from_a_valid_video_id_only() {
        assert_eq!(watch_url("dQw4w9WgXcQ").as_deref(), Some(TRACK));
        for id in [
            "",
            "short",
            "dQw4w9WgXcQ&list=x",
            "dQw4w9WgXc\"",
            "--exec=calc",
        ] {
            assert_eq!(watch_url(id), None, "{id}");
        }
    }
}
