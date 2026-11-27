//! The real `libmpv-2.dll`, playing to a null audio output: no device, no sound.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::mpv::{MpvEngine, MpvEvents, Step};
use super::{Engine, EngineError, EngineEvent, Media};

const MARKER: &str = "SECRET-MARKER";
/// Generous: a loaded CI machine must not fail a test that works.
const PATIENCE: Duration = Duration::from_secs(20);

/// A directory of this test's own, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lumen-mpv-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A quiet 440 Hz tone: 16-bit mono PCM in a WAV file.
    fn tone(&self, name: &str, length: Duration) -> PathBuf {
        const RATE: u32 = 8_000;
        let samples = u32::try_from(length.as_millis()).unwrap() * (RATE / 1_000);
        let data = samples * 2;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes()); // PCM
        wav.extend(1u16.to_le_bytes()); // mono
        wav.extend(RATE.to_le_bytes());
        wav.extend((RATE * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data.to_le_bytes());
        for n in 0..samples {
            let angle = f64::from(n) * 440.0 * std::f64::consts::TAU / f64::from(RATE);
            wav.extend(((angle.sin() * 300.0) as i16).to_le_bytes());
        }
        self.write(name, &wav)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Left behind in the temp directory if mpv still holds a file.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn engine() -> (MpvEngine, MpvEvents) {
    MpvEngine::silent().expect("libmpv-2.dll starts")
}

fn play(engine: &mut MpvEngine, path: &Path) {
    engine
        .load(&Media::File(path.to_owned()), Duration::ZERO)
        .unwrap();
    engine.set_playing(true);
}

/// Events up to and including the first `wanted` one.
fn until(events: &mut MpvEvents, wanted: EngineEvent) -> Vec<EngineEvent> {
    let started = Instant::now();
    let mut seen = Vec::new();
    while started.elapsed() < PATIENCE {
        if let Step::Event(event) = events.step(Duration::from_millis(100)) {
            seen.push(event);
            if event == wanted {
                return seen;
            }
        }
    }
    panic!("no {wanted:?} after {PATIENCE:?}; saw {seen:?}");
}

/// Whatever arrives during `quiet`.
fn during(events: &mut MpvEvents, quiet: Duration) -> Vec<EngineEvent> {
    let started = Instant::now();
    let mut seen = Vec::new();
    while started.elapsed() < quiet {
        if let Step::Event(event) = events.step(Duration::from_millis(50)) {
            seen.push(event);
        }
    }
    seen
}

fn near(time: Duration, millis: u64) -> bool {
    time.abs_diff(Duration::from_millis(millis)) < Duration::from_millis(150)
}

#[test]
fn is_set_up_lean_and_cut_off_from_user_files() {
    let (engine, _events) = engine();

    for (option, value) in [
        ("vid", "no"),
        ("audio-display", "no"),
        ("config", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("tls-verify", "yes"),
        ("terminal", "no"),
    ] {
        assert_eq!(engine.option_text(option), value, "{option}");
    }
    assert_eq!(engine.option_number("demuxer-max-bytes"), 8.0 * 1_048_576.0);
    assert_eq!(engine.option_number("demuxer-max-back-bytes"), 1_048_576.0);
    assert_eq!(engine.option_number("cache-secs"), 20.0);
}

#[test]
fn plays_a_generated_file_to_its_end() {
    let scratch = Scratch::new("plays");
    let (mut engine, mut events) = engine();

    play(
        &mut engine,
        &scratch.tone("tone.wav", Duration::from_millis(1_500)),
    );
    until(&mut events, EngineEvent::Loaded);
    assert!(near(engine.duration(), 1_500), "{:?}", engine.duration());
    let rest = until(&mut events, EngineEvent::Ended);

    assert!(rest.contains(&EngineEvent::Progress), "{rest:?}");
    assert!(!rest.contains(&EngineEvent::Failed), "{rest:?}");
    // At most four a second.
    let progress = rest.iter().filter(|e| **e == EngineEvent::Progress).count();
    assert!(progress <= 7, "{progress} in 1.5 s");
    // Still known once mpv has let the file go.
    assert!(near(engine.duration(), 1_500));
    assert!(engine.position() > Duration::from_millis(900));
}

#[test]
fn a_paused_load_waits_where_it_was_asked_to_start() {
    let scratch = Scratch::new("paused");
    let tone = scratch.tone("tone.wav", Duration::from_secs(3));
    let (mut engine, mut events) = engine();

    engine
        .load(&Media::File(tone), Duration::from_secs(2))
        .unwrap();
    until(&mut events, EngineEvent::Loaded);
    let waiting = during(&mut events, Duration::from_millis(600));

    assert!(near(engine.position(), 2_000), "{:?}", engine.position());
    assert!(!waiting.contains(&EngineEvent::Ended), "{waiting:?}");
}

#[test]
fn seeking_and_volume_reach_mpv() {
    let scratch = Scratch::new("seek");
    let tone = scratch.tone("tone.wav", Duration::from_secs(5));
    let (mut engine, mut events) = engine();
    engine.load(&Media::File(tone), Duration::ZERO).unwrap();
    until(&mut events, EngineEvent::Loaded);

    engine.set_volume(35);
    engine.seek(Duration::from_secs(4));
    assert!(near(engine.position(), 4_000));
    during(&mut events, Duration::from_millis(400));

    assert!(near(engine.position(), 4_000), "{:?}", engine.position());
    assert_eq!(engine.option_number("volume"), 35.0);
    engine.set_playing(true);
    until(&mut events, EngineEvent::Ended);
}

#[test]
fn the_same_file_can_be_opened_again_after_its_end() {
    let scratch = Scratch::new("again");
    let tone = scratch.tone("tone.wav", Duration::from_millis(500));
    let (mut engine, mut events) = engine();
    play(&mut engine, &tone);
    until(&mut events, EngineEvent::Ended);

    play(&mut engine, &tone);

    until(&mut events, EngineEvent::Loaded);
    assert!(near(engine.duration(), 500), "{:?}", engine.duration());
    until(&mut events, EngineEvent::Ended);
}

#[test]
fn a_replaced_file_reports_nothing_more() {
    let scratch = Scratch::new("replace");
    let (mut engine, mut events) = engine();
    play(
        &mut engine,
        &scratch.tone("long.wav", Duration::from_secs(30)),
    );
    until(&mut events, EngineEvent::Loaded);

    play(
        &mut engine,
        &scratch.tone("short.wav", Duration::from_millis(700)),
    );
    assert_eq!(engine.duration(), Duration::ZERO);
    let seen = until(&mut events, EngineEvent::Loaded);

    assert!(!seen.contains(&EngineEvent::Ended), "{seen:?}");
    assert!(!seen.contains(&EngineEvent::Failed), "{seen:?}");
    assert!(near(engine.duration(), 700), "{:?}", engine.duration());
}

#[test]
fn a_file_that_is_not_audio_fails() {
    let scratch = Scratch::new("garbage");
    let path = scratch.write("broken.mp3", b"this is not audio at all");
    let (mut engine, mut events) = engine();

    play(&mut engine, &path);

    let seen = until(&mut events, EngineEvent::Failed);
    assert!(!seen.contains(&EngineEvent::Ended), "{seen:?}");
}

#[test]
fn refuses_what_is_not_a_file_on_this_computer() {
    let scratch = Scratch::new("refused");
    let (mut engine, _events) = engine();
    let mut load = |path: PathBuf| engine.load(&Media::File(path), Duration::ZERO);

    for path in [
        scratch.0.join("missing.wav"),
        scratch.0.clone(),
        PathBuf::from("https://media.invalid/a.mp3"),
        PathBuf::from(r"\\server\share\a.mp3"),
    ] {
        assert_eq!(load(path.clone()), Err(EngineError::NotFound), "{path:?}");
    }
}

#[test]
fn a_refused_load_stops_what_was_playing() {
    let scratch = Scratch::new("stops");
    let (mut engine, mut events) = engine();
    play(
        &mut engine,
        &scratch.tone("tone.wav", Duration::from_secs(30)),
    );
    until(&mut events, EngineEvent::Loaded);

    let refused = engine.load(&Media::File(scratch.0.join("gone.wav")), Duration::ZERO);

    assert_eq!(refused, Err(EngineError::NotFound));
    during(&mut events, Duration::from_millis(500));
    assert_eq!(engine.option_text_of("idle-active"), "yes");
}

#[test]
fn a_stream_gets_its_headers_and_a_failure_names_no_url() {
    let scratch = Scratch::new("stream");
    let (mut engine, mut events) = engine();
    let plain_agent = engine.option_text("user-agent");
    let stream = Media::Stream {
        // Nothing listens on port 1, and nothing leaves this machine.
        url: format!("http://127.0.0.1:1/{MARKER}"),
        headers: vec![
            ("User-Agent".to_owned(), "Lumen-Test/1.0".to_owned()),
            ("Accept".to_owned(), "audio/webm, audio/mp4".to_owned()),
            ("X-Marker".to_owned(), MARKER.to_owned()),
        ],
    };

    engine.load(&stream, Duration::ZERO).unwrap();
    assert_eq!(engine.option_text("user-agent"), "Lumen-Test/1.0");
    let fields = engine.option_text("http-header-fields");
    assert!(fields.contains("webm"), "{fields}");
    assert!(fields.contains(&format!("X-Marker: {MARKER}")), "{fields}");
    let seen = until(&mut events, EngineEvent::Failed);
    assert!(!format!("{seen:?}").contains(MARKER));

    // The next load carries none of it.
    play(
        &mut engine,
        &scratch.tone("tone.wav", Duration::from_millis(500)),
    );
    assert_eq!(engine.option_text("user-agent"), plain_agent);
    assert_eq!(engine.option_text("http-header-fields"), "");
    until(&mut events, EngineEvent::Ended);
}

#[test]
fn refuses_headers_that_could_smuggle_in_more_and_samples() {
    let (mut engine, _events) = engine();

    for (name, value) in [("X-A", "b\r\nCookie: c"), ("X-A\nB", "c"), ("", "c")] {
        let stream = Media::Stream {
            url: "http://127.0.0.1:1/a".to_owned(),
            headers: vec![(name.to_owned(), value.to_owned())],
        };
        assert_eq!(
            engine.load(&stream, Duration::ZERO),
            Err(EngineError::Unplayable)
        );
    }
    let sample = Media::Sample {
        duration: Duration::from_secs(1),
    };
    assert_eq!(
        engine.load(&sample, Duration::ZERO),
        Err(EngineError::Unplayable)
    );
    assert!(!engine.plays_samples());
}

#[test]
fn the_events_end_on_shutdown_while_the_engine_lives() {
    let (engine, mut events) = engine();
    let shutdown = engine.shutdown_handle();

    shutdown.shut_down();

    let waiter = std::thread::spawn(move || while events.wait().is_some() {});
    let started = Instant::now();
    while !waiter.is_finished() {
        assert!(started.elapsed() < PATIENCE, "mpv never shut down");
        std::thread::sleep(Duration::from_millis(20));
    }
    waiter.join().unwrap();
    drop(engine);
}

#[test]
fn the_events_end_when_the_engine_is_dropped() {
    let (engine, mut events) = engine();

    drop(engine);

    let waiter = std::thread::spawn(move || while events.wait().is_some() {});
    let started = Instant::now();
    while !waiter.is_finished() {
        assert!(started.elapsed() < PATIENCE, "mpv never shut down");
        std::thread::sleep(Duration::from_millis(20));
    }
    waiter.join().unwrap();
}
