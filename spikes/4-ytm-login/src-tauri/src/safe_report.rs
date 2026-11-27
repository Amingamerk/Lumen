//! A text copy of one run for the findings file. It takes only fixed labels and stream facts:
//! no playlist, track or account data, and anything shaped like a URL is replaced.

use std::fmt::Display;

use crate::{
    account::CheckReport,
    resolver::{Clients, ResolveReport},
};

pub(crate) fn text(clients: Clients, now: u64, report: &CheckReport) -> String {
    let mode = match clients {
        Clients::Default => "yt-dlp defaults",
        Clients::PinnedUserAgent => "web_music,web",
    };
    let mut lines = vec![format!("mode: {mode}"), format!("run_at: {now}")];
    if let Some(reason) = &report.blocked {
        lines.push(format!("blocked: {reason}"));
    }
    match &report.browse {
        Some(Ok(_)) => lines.push("browse: ok".to_owned()),
        Some(Err(error)) => lines.push(format!("browse: error: {error}")),
        None => {}
    }
    match &report.resolve {
        Some(Ok(stream)) => {
            lines.push("resolve: ok".to_owned());
            stream_lines(&mut lines, now, stream);
        }
        Some(Err(error)) => lines.push(format!("resolve: error: {error}")),
        None => {}
    }
    lines.iter().map(|line| without_urls(line) + "\n").collect()
}

fn stream_lines(lines: &mut Vec<String>, now: u64, stream: &ResolveReport) {
    let warnings = if stream.warnings.is_empty() {
        "none known".to_owned()
    } else {
        stream.warnings.join("; ")
    };
    let expires_at = stream.expires_at.map_or("unknown".to_owned(), |at| {
        format!("{at} (in {} s)", at.saturating_sub(now))
    });
    let protocol = stream
        .protocol
        .as_deref()
        .map_or("unknown".to_owned(), |name| {
            format!("{name} ({})", protocol_family(name))
        });
    let fields = [
        ("client", or_unknown(stream.client.as_ref())),
        ("format_id", stream.format_id.clone()),
        ("audio_only", yes_no(stream.audio_only)),
        ("vcodec", or_unknown(stream.vcodec.as_ref())),
        ("codec", or_unknown(stream.codec.as_ref())),
        ("abr_kbps", or_unknown(stream.abr_kbps)),
        ("tbr_kbps", or_unknown(stream.tbr_kbps)),
        ("protocol", protocol),
        (
            "available_at",
            stream
                .available_at
                .map_or("none".to_owned(), |at| at.to_string()),
        ),
        ("waited_s", stream.waited_s.to_string()),
        ("expires_at", expires_at),
        ("range_status", stream.range_status.to_string()),
        (
            "user_agent_matches_login",
            yes_no(stream.user_agent_matches_login),
        ),
        ("deno_started", yes_no(stream.deno_started)),
        ("warnings", warnings),
        ("resolve_ms", stream.resolve_ms.to_string()),
    ];
    lines.extend(fields.map(|(name, value)| format!("{name}: {value}")));
}

/// yt-dlp's protocol names, in the words of the findings.
fn protocol_family(protocol: &str) -> &'static str {
    if protocol.starts_with("m3u8") {
        "HLS"
    } else if protocol.contains("dash") {
        "DASH"
    } else if protocol == "https" {
        "HTTPS"
    } else {
        "other"
    }
}

fn or_unknown(value: Option<impl Display>) -> String {
    value.map_or("unknown".to_owned(), |value| value.to_string())
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

fn without_urls(line: &str) -> String {
    line.split(' ')
        .map(|word| if word.contains("://") { "[url]" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}
