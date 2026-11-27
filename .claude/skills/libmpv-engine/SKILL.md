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

The options are `OPTIONS` in `src-tauri/src/audio/mpv.rs`, set in `Mpv::with_initializer`: the lean ones from `docs/PROJECT.md` section 2 rule 7, user config, scripts and `ytdl` off, nothing loaded from the user's files, and `tls-verify=yes`. `mpv_tests.rs` reads them back.

Stream headers are applied before `loadfile`:
- Set `user-agent` via `mpv.set_property("user-agent", agent)`, restoring mpv's own for media without one.
- Clear `http-header-fields`, then append every other header one at a time with the `change-list` command. Do not pass a comma-joined string; header values contain commas.

## Event thread and error handling

- `MpvEvents::wait` blocks in `wait_event` on its own OS thread (`mpv-events`, started in `ipc/player.rs`), so a paused player never wakes. Dropping `MpvEngine` sends `quit` and ends it.
- Only `time-pos` and `duration` are observed. `mpv_events::translate` turns events into `EngineEvent`s and spaces progress at least 250 ms apart.
- Events are ignored until the new file's `StartFile`, so a replaced file reports nothing more. `PlaybackRestart` means loaded; the timeline is read then.
- Handle both `Ok(Event::EndFile(reason))` and `Err(_)`:
  - `EndFile` covers normal EOF, stop, redirect (treated as a failure) and error reasons.
  - **Gotcha:** HTTP 403 on an expired or rejected URL ends with mpv error `-13`, which `libmpv2` surfaces as `Some(Err(_))` rather than `Event::EndFile(_)`. Every `Err` is `EngineEvent::Failed`; the resolver tickets must re-resolve with backoff on it.

## Windows DLL placement and bundling

- `scripts/fetch-pinned-binaries.ps1` puts the LGPL `libmpv-2.dll` (~103 MB) and `mpv.lib` in `src-tauri/binaries/`. Run it in every new worktree before cargo. `LIBMPV_DIR` overrides the folder.
- `build.rs` adds the folder to the link search path and copies the DLL next to the built exe and into `deps/`.
- `tauri.conf.json` lists the DLL under `bundle.resources`, so it ships beside `lumen.exe`.

## Testing

- Player rules run against `FakeEngine` (and `SimulatedEngine` for samples): deterministic events, no audio device, no network.
- `mpv_tests.rs` loads the real DLL through `MpvEngine::silent()` (`ao=null`) and plays generated WAV files, locally and in CI.
