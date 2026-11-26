//! Turns `yt-dlp -J` output into what mpv needs to play a stream.

use std::collections::BTreeMap;

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
}

pub fn parse_ytdlp_json(json: &str) -> Result<StreamInfo, ResolveError> {
    let out: YtDlp = serde_json::from_str(json)?;
    let url = out.url.ok_or(ResolveError::NoDirectUrl)?;
    Ok(StreamInfo {
        expires_at: expiry_from_url(&url),
        headers: out.http_headers.into_iter().collect(),
        url,
    })
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
            }
        );
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
}
