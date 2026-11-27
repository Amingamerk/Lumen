---
name: libmpv-engine
description: Implement or modify Lumen audio playback in src-tauri/src/audio/ using libmpv2, mpv properties/events, Engine trait fakes, or libmpv-2.dll bundling on Windows.
---

# libmpv Audio Engine

Reference for `src-tauri/src/audio/` using `libmpv2`. Architecture and RAM rules live in `docs/PROJECT.md` (sections 2, 3, and 4); verified Windows measurements and gotchas live in `spikes/2-audio/FINDINGS.md`.

## Invariants

- Audio plays in Rust via `libmpv2` only. Never use HTML `<audio>`.
- All playback state lives in Rust and is pushed to the UI via Tauri events. No UI polling.
- Never log raw stream URLs or raw mpv error strings (mpv error logs include the full URL and client IP).
- Keep `ytdl=no`: Lumen resolves streams itself so it can pre-resolve, wait for `available_at`, and re-resolve on 403.

## Handle setup (`libmpv2 = { version = "7", default-features = false }`)

Configure lean options from `docs/PROJECT.md` section 2 rule 7 in `Mpv::with_initializer`:

```rust
const LEAN_OPTIONS: &[(&str, &str)] = &[
    ("vid", "no"),
    ("audio-display", "no"),
    ("demuxer-max-bytes", "8MiB"),
    ("demuxer-max-back-bytes", "1MiB"),
    ("cache-secs", "20"),
    ("ytdl", "no"),
];
```

Apply resolved stream headers before `loadfile`:
- Set `user-agent` via `mpv.set_property("user-agent", agent)`.
- Append every other header one at a time via `mpv.command("change-list", &["http-header-fields", "append", &format!("{name}: {value}")])`. Do not pass a comma-joined string; header values contain commas.

## Event thread and error handling

- Run `mpv.wait_event(timeout)` on a dedicated OS thread (never block Tokio worker threads).
- Observe playback properties (`time-pos`, `duration`, `pause`, `eof-reached`, `core-idle`) with `observe_property` and translate updates into throttled/deduplicated Tauri events.
- Handle both `Some(Ok(Event::EndFile(reason)))` and `Some(Err(err))`:
  - `EndFile` covers normal EOF, stop, redirect, and error reasons.
  - **Gotcha:** HTTP 403 on an expired or rejected URL ends with mpv error `-13`, which `libmpv2` surfaces as `Some(Err(_))` rather than `Event::EndFile(_)`. Treat playback `Err` as a recoverable stream failure and signal the player/resolver to re-resolve with backoff instead of letting it pass silently.

## Windows DLL placement and bundling

- LGPL `libmpv-2.dll` (~103 MB) and generated `mpv.lib` live in `LIBMPV_DIR` (default `%LOCALAPPDATA%\Programs\libmpv`).
- `build.rs` emits `cargo:rustc-link-search=native=<LIBMPV_DIR>` and copies `libmpv-2.dll` next to the built exe and `deps/` by locating the `build` ancestor of `OUT_DIR` (see `spikes/2-audio/build.rs`).
- For release bundles, ensure `libmpv-2.dll` ships beside `lumen.exe`.

## Testing without libmpv

Hide `libmpv2` behind an `Engine` trait (`load`, `play`, `pause`, `seek`, `set_volume`, event channel) so unit and integration tests run against an in-memory fake engine with deterministic events and zero audio hardware or network calls.
