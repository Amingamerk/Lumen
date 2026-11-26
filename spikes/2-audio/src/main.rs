//! Spike: resolve one track with yt-dlp and play it with libmpv.
//!
//! audio-spike <url> [--seconds N] [--hook] [--break-url] [--clients SPEC | --default-clients]

use std::{
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use audio_spike::resolve::{StreamInfo, parse_ytdlp_json, ytdlp_args};
use libmpv2::{Mpv, events::Event};

const DEFAULT_URL: &str = "https://music.youtube.com/watch?v=dQw4w9WgXcQ";
/// Excludes `tv_samsung`, whose ad-gated URLs return HTTP 403 on unauthenticated resolves.
const DEFAULT_CLIENTS: &str = "default,-tv_samsung";

/// Audio only, small buffers (PROJECT.md section 2, rule 7).
const LEAN_OPTIONS: &[(&str, &str)] = &[
    ("vid", "no"),
    ("audio-display", "no"),
    ("demuxer-max-bytes", "8MiB"),
    ("demuxer-max-back-bytes", "1MiB"),
    ("cache-secs", "20"),
    ("volume", "40"),
    ("terminal", "yes"),
    ("msg-level", "all=warn"),
];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let seconds = match args.iter().position(|a| a == "--seconds") {
        Some(i) => args
            .get(i + 1)
            .context("--seconds needs a number")?
            .parse()?,
        None => 15u64,
    };
    let url = args
        .iter()
        .find(|a| a.starts_with("https://"))
        .map_or(DEFAULT_URL, String::as_str);
    let hook = flag("--hook");
    let clients = if flag("--default-clients") {
        None
    } else if let Some(i) = args.iter().position(|a| a == "--clients") {
        Some(args.get(i + 1).context("--clients needs a value")?.as_str())
    } else {
        Some(DEFAULT_CLIENTS)
    };

    let mpv = Mpv::with_initializer(|init| {
        for (name, value) in LEAN_OPTIONS {
            init.set_option(name, *value)?;
        }
        init.set_option("ytdl", if hook { "yes" } else { "no" })?;
        if hook {
            init.set_option("ytdl-format", "bestaudio")?;
        }
        Ok(())
    })
    .map_err(|e| anyhow!("mpv init failed: {e}"))?;

    let target = if hook {
        url.to_owned()
    } else {
        let started = Instant::now();
        let mut info = resolve(url, clients)?;
        println!("resolve_ms={}", started.elapsed().as_millis());
        print_expiry(&info)?;
        apply_headers(&mpv, &info)?;
        if flag("--break-url") {
            info.url = info.url.replace("expire=", "expire=1");
        }
        info.url
    };

    let loaded = Instant::now();
    mpv.command("loadfile", &[&target])
        .map_err(|e| anyhow!("loadfile failed: {e}"))?;

    wait_for_playback(&mpv)?;
    println!("load_to_audio_ms={}", loaded.elapsed().as_millis());

    let playing = Instant::now();
    while playing.elapsed() < Duration::from_secs(seconds) {
        if let Some(Ok(Event::EndFile(_))) = mpv.wait_event(0.5) {
            break;
        }
    }
    report(&mpv);
    Ok(())
}

fn resolve(url: &str, player_clients: Option<&str>) -> Result<StreamInfo> {
    let out = Command::new("yt-dlp")
        .args(ytdlp_args(url, player_clients))
        .output()
        .context("could not start yt-dlp")?;
    if !out.status.success() {
        bail!("yt-dlp failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(parse_ytdlp_json(&String::from_utf8(out.stdout)?)?)
}

fn print_expiry(info: &StreamInfo) -> Result<()> {
    let now = std::time::UNIX_EPOCH.elapsed()?.as_secs();
    match info.expires_at {
        Some(at) => println!("expires_at={at} lifetime_s={}", at.saturating_sub(now)),
        None => println!("expires_at=unknown"),
    }
    Ok(())
}

fn apply_headers(mpv: &Mpv, info: &StreamInfo) -> Result<()> {
    if let Some(agent) = info.user_agent() {
        mpv.set_property("user-agent", agent)
            .map_err(|e| anyhow!("user-agent: {e}"))?;
    }
    for field in info.header_fields() {
        mpv.command("change-list", &["http-header-fields", "append", &field])
            .map_err(|e| anyhow!("http-header-fields: {e}"))?;
    }
    Ok(())
}

fn wait_for_playback(mpv: &Mpv) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(45) {
        match mpv.wait_event(0.5) {
            Some(Ok(Event::PlaybackRestart)) => return Ok(()),
            Some(Ok(Event::EndFile(reason))) => bail!("ended before playing, reason {reason:?}"),
            Some(Err(e)) => bail!("mpv error before playing: {e}"),
            _ => {}
        }
    }
    bail!("no audio after 45 s")
}

fn report(mpv: &Mpv) {
    for name in [
        "current-ao",
        "audio-codec-name",
        "audio-params/samplerate",
        "time-pos",
        "demuxer-cache-duration",
        "demuxer-cache-state/total-bytes",
    ] {
        let value = mpv
            .get_property::<String>(name)
            .unwrap_or_else(|e| format!("<{e}>"));
        println!("{name}={value}");
    }
}
