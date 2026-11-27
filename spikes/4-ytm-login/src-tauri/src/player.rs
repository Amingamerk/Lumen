//! Plays the last resolved stream for a moment with libmpv, to confirm that the format works.

use std::time::{Duration, Instant};

use libmpv2::{Mpv, events::Event};

use crate::resolver::LastStream;

const LISTEN: Duration = Duration::from_secs(5);
/// Longest wait for the first audio.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// Audio only, small buffers (PROJECT.md section 2, rule 7), and no yt-dlp hook: libmpv gets the
/// resolved URL and requests nothing else on its own.
const OPTIONS: &[(&str, &str)] = &[
    ("vid", "no"),
    ("audio-display", "no"),
    ("ytdl", "no"),
    ("demuxer-max-bytes", "8MiB"),
    ("demuxer-max-back-bytes", "1MiB"),
    ("cache-secs", "20"),
    ("volume", "40"),
];

/// Fixed messages: mpv's own output can quote the stream URL.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum PlayError {
    #[error("libmpv could not be set up")]
    Setup,
    #[error("libmpv could not play the stream: no audio started")]
    DidNotStart,
    #[error("libmpv stopped the stream with an error")]
    Failed,
}

/// Plays the stream aloud for 5 seconds, then stops. Blocks the calling thread.
pub fn play_five_seconds(stream: &LastStream) -> Result<(), PlayError> {
    play(stream, LISTEN, &[])
}

/// One load, no retries.
fn play(stream: &LastStream, listen: Duration, extra: &[(&str, &str)]) -> Result<(), PlayError> {
    let mpv = Mpv::with_initializer(|init| {
        for (name, value) in OPTIONS.iter().chain(extra) {
            init.set_option(name, *value)?;
        }
        Ok(())
    })
    .map_err(|_| PlayError::Setup)?;

    for (name, value) in stream.headers() {
        if name.eq_ignore_ascii_case("user-agent") {
            mpv.set_property("user-agent", value.as_str())
        } else {
            // `append` takes one item, so commas inside a value stay.
            let field = format!("{name}: {value}");
            mpv.command("change-list", &["http-header-fields", "append", &field])
        }
        .map_err(|_| PlayError::Setup)?;
    }
    mpv.command("loadfile", &[stream.url()])
        .map_err(|_| PlayError::DidNotStart)?;

    wait_for_audio(&mpv)?;
    let started = Instant::now();
    while let Some(left) = listen.checked_sub(started.elapsed()) {
        match mpv.wait_event(left.as_secs_f64().min(0.5)) {
            // A short track may end first.
            Some(Ok(Event::EndFile(_))) => break,
            Some(Err(_)) => return Err(PlayError::Failed),
            _ => {}
        }
    }
    Ok(())
}

fn wait_for_audio(mpv: &Mpv) -> Result<(), PlayError> {
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        match mpv.wait_event(0.5) {
            Some(Ok(Event::PlaybackRestart)) => return Ok(()),
            Some(Ok(Event::EndFile(_))) | Some(Err(_)) => break,
            _ => {}
        }
    }
    Err(PlayError::DidNotStart)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    use super::*;

    const SHORT: Duration = Duration::from_millis(200);
    /// No sound from tests.
    const SILENT: &[(&str, &str)] = &[("ao", "null")];

    /// One second of 8 kHz mono silence.
    fn wav() -> Vec<u8> {
        let data = 16_000u32;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(8_000u32.to_le_bytes());
        wav.extend(16_000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data.to_le_bytes());
        wav.resize(wav.len() + data as usize, 0);
        wav
    }

    /// A local server that answers one request with the WAV and hands back what it read.
    fn serve_wav() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/a.wav", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 1024];
            while !received.windows(4).any(|w| w == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..read]);
            }
            sender
                .send(String::from_utf8_lossy(&received).into_owned())
                .unwrap();
            let body = wav();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            // mpv may hang up as soon as it has enough.
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|()| stream.write_all(&body));
        });
        (url, receiver)
    }

    #[test]
    fn plays_the_stream_with_its_headers() {
        let (url, received) = serve_wav();
        let stream = LastStream::new(
            url,
            vec![
                ("User-Agent".into(), "FakeAgent/1.0".into()),
                ("Accept-Language".into(), "en-us,en;q=0.5".into()),
            ],
        );

        assert_eq!(play(&stream, SHORT, SILENT), Ok(()));

        let request = received.recv().unwrap().to_ascii_lowercase();
        assert!(
            request.contains("user-agent: fakeagent/1.0\r\n"),
            "{request}"
        );
        assert!(
            request.contains("accept-language: en-us,en;q=0.5\r\n"),
            "{request}"
        );
        assert!(!request.contains("cookie:"));
    }

    #[test]
    fn a_stream_that_cannot_open_fails_without_naming_its_url() {
        let stream = LastStream::new("http://127.0.0.1:1/secret-signature".into(), Vec::new());

        let error = play(&stream, SHORT, SILENT).unwrap_err();

        assert_eq!(error, PlayError::DidNotStart);
        assert!(!error.to_string().contains("127.0.0.1"));
    }
}
