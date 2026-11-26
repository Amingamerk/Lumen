# Spike #2: yt-dlp + Deno resolve, libmpv playback

Measured 2026-11-26 on Windows 11, yt-dlp 2026.11.23, Deno 2.9.8, libmpv (LGPL build, client API 2.7).

Run: `cargo run --release -- [url] [--seconds N] [--hook] [--break-url]`
(`LIBMPV_DIR` overrides `%LOCALAPPDATA%\Programs\libmpv`).

## Results

| Measure | Value |
|---|---|
| `yt-dlp -J -f bestaudio` resolve time | 5.9 to 6.9 s |
| `loadfile` to audio, direct URL | 283 ms |
| `loadfile` to audio, mpv `ytdl_hook` (includes resolving) | 6.4 s |
| Stream | format 251, Opus 48 kHz, about 129 kbps, client `TVHTML5` |
| Expiry | `expire=` query parameter, about 21,600 s (6 h) after resolve |
| Audio output | `wasapi`, position advanced for the whole run, 20 s cached |
| yt-dlp private working set while resolving | about 59 MB (second process), spike binary 6 MB before playback |

Not measured: RAM of the spike binary while playing. Audible output still needs a human to confirm.

## Settings that worked

`vid=no`, `audio-display=no`, `demuxer-max-bytes=8MiB`, `demuxer-max-back-bytes=1MiB`, `cache-secs=20`.
Headers: `user-agent` option, then `change-list http-header-fields append` once per header
(values contain commas).

## Gotchas

- `libmpv-2.dll` must sit next to the exe; `build.rs` copies it. The `OUT_DIR` depth differs between
  cargo versions, so look for the `build` ancestor.
- An expired or altered URL gives HTTP 403 and mpv ends the file with error -13. It does not re-resolve.
- One freshly resolved, unaltered URL also returned 403 (after about six resolves within five minutes).
  Cause unknown. The player must re-resolve once on 403, with backoff.
- mpv's error log prints the full stream URL, including the client IP. The app must not log it.
- Without a JS runtime yt-dlp only warns and still exits 0, with formats possibly missing.
- `ytdl_hook` works in this build but is as slow as resolving ourselves and cannot pre-resolve or
  re-resolve, so Lumen resolves itself.
- About 6 s per resolve makes pre-resolving the next 1-2 tracks necessary.
