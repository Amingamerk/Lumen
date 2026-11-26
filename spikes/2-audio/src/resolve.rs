//! Turns `yt-dlp -J` output into what mpv needs to play a stream.

use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("yt-dlp output is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yt-dlp returned no single direct stream URL (was -f bestaudio used?)")]
    NoDirectUrl,
}

#[derive(Debug, PartialEq)]
pub struct StreamInfo {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Unix seconds after which the URL stops working.
    pub expires_at: Option<u64>,
    /// Unix seconds before which the URL should not be requested. yt-dlp derives it
    /// from a pre-roll ad in the player response.
    pub available_at: Option<u64>,
}

impl StreamInfo {
    /// mpv takes the user agent through its own `user-agent` option.
    pub fn user_agent(&self) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
            .map(|(_, value)| value.as_str())
    }

    /// Remaining headers, one `http-header-fields` entry each. Append them one at a
    /// time: values contain commas, which the string form of the option splits on.
    pub fn header_fields(&self) -> impl Iterator<Item = String> {
        self.headers
            .iter()
            .filter(|(name, _)| !name.eq_ignore_ascii_case("user-agent"))
            .map(|(name, value)| format!("{name}: {value}"))
    }
}

#[derive(serde::Deserialize)]
struct YtDlp {
    url: Option<String>,
    #[serde(default)]
    http_headers: BTreeMap<String, String>,
    /// Fractional when the ad's skip offset is not a whole second.
    available_at: Option<f64>,
}

pub fn ytdlp_args(url: &str, player_clients: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-J".to_owned(),
        "-f".to_owned(),
        "bestaudio".to_owned(),
        "--no-playlist".to_owned(),
        // A user config file must not change what we send or how much we resolve.
        "--ignore-config".to_owned(),
        "--playlist-items".to_owned(),
        "1".to_owned(),
    ];
    if let Some(clients) = player_clients {
        args.push("--extractor-args".to_owned());
        args.push(format!("youtube:player_client={clients}"));
    }
    args.push("--".to_owned());
    args.push(url.to_owned());
    args
}

pub fn parse_ytdlp_json(json: &str) -> Result<StreamInfo, ResolveError> {
    let out: YtDlp = serde_json::from_str(json)?;
    let url = out.url.ok_or(ResolveError::NoDirectUrl)?;
    Ok(StreamInfo {
        expires_at: expiry_from_url(&url),
        headers: out.http_headers.into_iter().collect(),
        available_at: out.available_at.map(|at| at.ceil() as u64),
        url,
    })
}

/// How long to wait before requesting the URL, or `None` if it can be used now.
pub fn wait_duration(available_at: Option<u64>, now_unix_secs: u64) -> Option<Duration> {
    available_at?
        .checked_sub(now_unix_secs)
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
}

/// googlevideo URLs carry the expiry as `?expire=N` (HLS manifests as `/expire/N/`).
fn expiry_from_url(url: &str) -> Option<u64> {
    let from_query = url
        .split_once('?')
        .and_then(|(_, query)| query.split('&').find_map(|p| p.strip_prefix("expire=")));
    let from_path = || url.split_once("/expire/")?.1.split('/').next();
    from_query.or_else(from_path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUDIO: &str = r#"{
        "title": "A song", "format_id": "251", "acodec": "opus", "vcodec": "none",
        "url": "https://rr1.googlevideo.com/videoplayback?expire=1795728336&itag=251",
        "http_headers": {"User-Agent": "UA/1.0", "Accept-Language": "en-us,en;q=0.5"}
    }"#;

    #[test]
    fn reads_url_headers_and_expiry_of_an_audio_stream() {
        let info = parse_ytdlp_json(AUDIO).unwrap();

        assert_eq!(
            info,
            StreamInfo {
                url: "https://rr1.googlevideo.com/videoplayback?expire=1795728336&itag=251".into(),
                headers: vec![
                    ("Accept-Language".into(), "en-us,en;q=0.5".into()),
                    ("User-Agent".into(), "UA/1.0".into()),
                ],
                expires_at: Some(1_795_728_336),
                available_at: None,
            }
        );
    }

    #[test]
    fn reads_available_at_when_yt_dlp_reports_it() {
        let json = r#"{"url": "https://a", "available_at": 1795711721}"#;

        assert_eq!(
            parse_ytdlp_json(json).unwrap().available_at,
            Some(1_795_711_721)
        );
    }

    #[test]
    fn leaves_available_at_empty_when_absent_or_null() {
        for json in [
            r#"{"url": "https://a"}"#,
            r#"{"url": "https://a", "available_at": null}"#,
        ] {
            assert_eq!(parse_ytdlp_json(json).unwrap().available_at, None);
        }
    }

    #[test]
    fn rounds_a_fractional_available_at_up() {
        // A skippable ad with a fractional skip offset gives a fractional timestamp.
        let json = r#"{"url": "https://a", "available_at": 1795711721.5}"#;

        assert_eq!(
            parse_ytdlp_json(json).unwrap().available_at,
            Some(1_795_711_722)
        );
    }

    #[test]
    fn waits_until_a_future_available_at() {
        assert_eq!(
            wait_duration(Some(1_006), 1_000),
            Some(Duration::from_secs(6))
        );
    }

    #[test]
    fn does_not_wait_without_a_future_available_at() {
        assert_eq!(wait_duration(None, 1_000), None);
        assert_eq!(wait_duration(Some(999), 1_000), None);
        assert_eq!(wait_duration(Some(1_000), 1_000), None);
    }

    #[test]
    fn splits_headers_into_mpv_user_agent_and_header_fields() {
        let info = parse_ytdlp_json(AUDIO).unwrap();

        assert_eq!(info.user_agent(), Some("UA/1.0"));
        assert_eq!(
            info.header_fields().collect::<Vec<_>>(),
            ["Accept-Language: en-us,en;q=0.5"]
        );
    }

    #[test]
    fn rejects_output_without_a_single_direct_url() {
        let json = r#"{"requested_formats": [{"url": "https://a"}, {"url": "https://b"}]}"#;

        assert!(matches!(
            parse_ytdlp_json(json),
            Err(ResolveError::NoDirectUrl)
        ));
    }

    #[test]
    fn reads_expiry_from_a_manifest_path() {
        let json = r#"{"url": "https://manifest.googlevideo.com/api/manifest/hls_playlist/expire/1795728336/ei/x/index.m3u8"}"#;

        assert_eq!(
            parse_ytdlp_json(json).unwrap().expires_at,
            Some(1_795_728_336)
        );
    }

    #[test]
    fn builds_ytdlp_args_with_optional_player_clients() {
        let url = "https://music.youtube.com/watch?v=dQw4w9WgXcQ";
        assert_eq!(
            ytdlp_args(url, None),
            vec![
                "-J",
                "-f",
                "bestaudio",
                "--no-playlist",
                "--ignore-config",
                "--playlist-items",
                "1",
                "--",
                url,
            ]
        );
        assert_eq!(
            ytdlp_args(url, Some("default,-tv_samsung")),
            vec![
                "-J",
                "-f",
                "bestaudio",
                "--no-playlist",
                "--ignore-config",
                "--playlist-items",
                "1",
                "--extractor-args",
                "youtube:player_client=default,-tv_samsung",
                "--",
                url,
            ]
        );
    }
}
