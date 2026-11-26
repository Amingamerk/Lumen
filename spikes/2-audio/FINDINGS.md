# Spike #2: yt-dlp + Deno resolve, libmpv playback

Measured 2026-11-26 on Windows 11, yt-dlp 2026.11.23, Deno 2.9.8, libmpv (LGPL build, client API 2.7).

Run: `cargo run --release -- [url] [--seconds N] [--hook] [--break-url] [--clients SPEC | --default-clients]`
(`LIBMPV_DIR` overrides `%LOCALAPPDATA%\Programs\libmpv`).

## Results

| Measure | Value |
|---|---|
| `yt-dlp -J -f bestaudio` resolve time (default clients, `TVHTML5`) | 5.9 to 6.9 s |
| `yt-dlp -J -f bestaudio` resolve time (`default,-tv_samsung`, `VISIONOS`) | 3.7 to 4.2 s (median 3.8 s, 3 runs) |
| `loadfile` to audio, direct URL | 100 to 131 ms (median 111 ms, `VISIONOS`; 283 ms earlier on `TVHTML5`) |
| `loadfile` to audio, mpv `ytdl_hook` (includes resolving) | 6.4 to 9.2 s |
| Stream | format 251, Opus 48 kHz, about 129 kbps |
| Expiry | `expire=` query parameter, about 21,600 s (6 h) after resolve |
| Audio output | `wasapi`, position advanced for the whole run, 20 s cached |
| Playing RAM (`audio-spike.exe` + `libmpv-2.dll`, release, 3-run median) | 13.1 MB private working set (runs: 13.1, 13.4, 12.6 MB; up from 6.2 MB idle before playback) |
| Transient resolve RAM peak (`default,-tv_samsung`) | 57.2 MB median (runs: 57.2, 65.8, 56.4 MB; `yt-dlp` ~49 to 59 MB) |
| Transient resolve RAM peak (default clients + Deno JS challenge) | up to 216.8 MB for 1 to 2 s (`yt-dlp` ~74 MB + `deno` ~136 MB), drops back once resolved |

Audible output still needs a human to confirm on the speakers.

## Settings that worked

`vid=no`, `audio-display=no`, `demuxer-max-bytes=8MiB`, `demuxer-max-back-bytes=1MiB`, `cache-secs=20`.
Headers: `user-agent` option, then `change-list http-header-fields append` once per header
(values contain commas).

## Gotchas

- `libmpv-2.dll` must sit next to the exe; `build.rs` copies it. The `OUT_DIR` depth differs between
  cargo versions, so look for the `build` ancestor.
- An expired or altered URL gives HTTP 403 and mpv ends the file with error -13. It does not re-resolve.
- **Root cause of the fresh-URL 403:** on unauthenticated resolves, `yt-dlp`'s default client order tries
  `tv_samsung` (`c=TVHTML5`) first. Whenever YouTube attaches a pre-roll ad to `tv_samsung` (`Detected a Ns ad
  for tv_samsung` in `yt-dlp -v`), the returned `TVHTML5` format 251 URL is rejected by `googlevideo.com` with
  HTTP 403 on HEAD, Range, and GET alike (verified across 3 consecutive runs). Passing
  `--extractor-args youtube:player_client=default,-tv_samsung` selects `visionos` (`c=VISIONOS`), which
  returns format 251 with HTTP 206, resolves faster (3.8 s vs 6 s), and avoids the Deno RAM spike. Still,
  the real player must re-resolve once on 403 with backoff, and spike #4 must re-verify client behavior with
  authenticated cookies.
- mpv's error log prints the full stream URL, including the client IP. The app must not log it.
- Without a JS runtime yt-dlp only warns and still exits 0, with formats possibly missing.
- `ytdl_hook` works in this build but is as slow as resolving ourselves and cannot pre-resolve or
  re-resolve, so Lumen resolves itself.
- 3.8 to 6 s per resolve makes pre-resolving the next 1-2 tracks necessary.
- When Deno runs to solve a JS challenge, transient private working set spikes by ~210 MB for 1 to 2 s.
  Keep resolves serialized (never spawn multiple yt-dlp/deno resolves at once).
