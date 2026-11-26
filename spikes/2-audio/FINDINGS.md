# Spike #2: yt-dlp + Deno resolve, libmpv playback

Measured 2026-11-26 on Windows 11, yt-dlp 2026.11.23, Deno 2.9.8, libmpv (LGPL build, client API 2.7).

Run: `cargo run --release -- [url] [--seconds N] [--hook] [--break-url] [--clients SPEC | --default-clients]`
(`LIBMPV_DIR` overrides `%LOCALAPPDATA%\Programs\libmpv`). The spike's own resolve runs yt-dlp with
`--ignore-config` and `--playlist-items 1`, so a user config cannot change it and one call resolves one
track. `--hook` ignores `--clients`, `--default-clients`, `--ignore-config` and `--playlist-items 1`: mpv
starts yt-dlp itself with default clients and reads any user yt-dlp config.

## libmpv build

- Source: LGPL Windows build `mpv-dev-lgpl-x86_64-20261126-git-1eab28e7b5` from
  `github.com/zhongfly/mpv-winbuild`.
- The MSVC linker needs `mpv.lib`, generated from the DLL exports: list them with
  `dumpbin /exports libmpv-2.dll`, put the `mpv_*` names under `EXPORTS` in `mpv.def` (first line
  `LIBRARY libmpv-2.dll`), then run `lib /def:mpv.def /machine:x64 /out:mpv.lib` with the Visual Studio
  x64 tools. Place `mpv.lib` in `LIBMPV_DIR` beside `libmpv-2.dll`, because `build.rs` sets that folder as
  the linker search path.
- `libmpv-2.dll` is about 103 MB on disk, far above the 25 to 30 MB estimate in `docs/PROJECT.md`
  section 2.

## Results

| Measure | Value |
|---|---|
| `yt-dlp -J -f bestaudio` resolve time (default clients, `TVHTML5`) | 5.9 to 6.9 s |
| `yt-dlp -J -f bestaudio` resolve time (`default,-tv_samsung`, `VISIONOS`) | 3.7 to 4.2 s (median 3.8 s, 3 runs) |
| `loadfile` to audio, direct URL | 100 to 131 ms (median 111 ms, `VISIONOS`; 283 ms earlier on `TVHTML5`) |
| `loadfile` to audio, mpv `ytdl_hook` (includes resolving, default clients) | 6.4 to 9.2 s |
| Stream | format 251, Opus 48 kHz, about 129 kbps |
| Expiry | `expire=` query parameter, about 21,600 s (6 h) after resolve |
| Audio output | `wasapi`, position advanced for the whole run, 20 s cached |
| Playing RAM (`audio-spike.exe` + `libmpv-2.dll`, release, 3-run median) | 13.1 MB private working set (runs: 13.1, 13.4, 12.6 MB; up from 6.2 MB idle before playback) |
| Transient resolve RAM peak (`default,-tv_samsung`) | 57.2 MB median (runs: 57.2, 65.8, 56.4 MB; `yt-dlp` ~49 to 59 MB) |
| Transient resolve RAM peak (default clients + Deno JS challenge) | up to 216.8 MB for 1 to 2 s (`yt-dlp` ~74 MB + `deno` ~136 MB), drops back once resolved |

The `ytdl_hook` time uses default clients, so compare it with the 5.9 to 6.9 s default-client resolve, not
with the 3.8 s `VISIONOS` one.

**How RAM was measured:** release build, `WorkingSetPrivate` from
`Win32_PerfFormattedData_PerfProc_Process`, matched by `IDProcess` across the process and all its
descendants from `Win32_Process`, median of 3 runs (the 216.8 MB figure is a peak, not a median).

**Over budget, carry into #7:** the transient 216.8 MB Deno + yt-dlp resolve peak exceeds the 150 MB playing
budget. It matters because the real player resolves the next track while one is playing.

Audible output confirmed on Amin's speakers during a 6 s release run on head `d96c532` (resolve 4.6 s,
`loadfile` to audio 266 ms, `wasapi`, Opus 48 kHz).

## Settings that worked

`vid=no`, `audio-display=no`, `demuxer-max-bytes=8MiB`, `demuxer-max-back-bytes=1MiB`, `cache-secs=20`.
Headers: `user-agent` option, then `change-list http-header-fields append` once per header
(values contain commas).

## Gotchas

- `libmpv-2.dll` must sit next to the exe; `build.rs` copies it. The `OUT_DIR` depth differs between
  cargo versions, so look for the `build` ancestor.
- An expired or altered URL gives HTTP 403 and mpv ends the file with error -13. It does not re-resolve.
  libmpv2 reports that as `Some(Err(_))`, not as an `EndFile` event, so the event loop must handle errors
  during playback too, or a mid-track failure passes silently.
- **Fresh-URL 403 and `available_at`:** on unauthenticated resolves, yt-dlp 2026.11.23 queries `tv_samsung`
  (`c=TVHTML5`) and `visionos` by default. When `tv_samsung` got a pre-roll ad (`Detected a Ns ad for
  tv_samsung` in `yt-dlp -v`), an earlier test saw its format 251 URL rejected with HTTP 403 on HEAD, Range
  and GET (3 consecutive runs). yt-dlp marks such formats with `available_at` (Unix seconds) and sleeps until
  then before downloading; `-J` returns immediately, so the caller has to wait.
- **Live check of that wait (one run, default clients):** a 6 s ad was detected, the URL was `c=TVHTML5`
  format 251, and `available_at` was 3.5 s ahead when the resolve returned. A Range request
  (`bytes=0-1023`, returned User-Agent) gave **HTTP 206 before** `available_at` (3.5 s early) and **HTTP 206
  after** it (same URL, 1.0 s late). The 403 did not reproduce, so waiting is **not proven** to turn 403 into
  206, and the cause of the earlier 403 stays unconfirmed. One sample cannot rule out that the server only
  enforces the wait some of the time.
- The spike honors `available_at` before `loadfile` anyway (`waiting_for_available_at_s=N`), as yt-dlp does.
  The value can be fractional (a skippable ad with a fractional skip offset), so it is rounded up. The parser
  and the wait helper have unit tests; the sleep itself has not been exercised in a live playback run.
- **Default clients:** the spike keeps `default,-tv_samsung` as a speed choice, not as a proven 403 fix. It
  selects `visionos` (`c=VISIONOS`, format 251, HTTP 206), resolves faster (3.8 s vs about 6 s), avoids the
  Deno RAM spike and avoids the `tv_samsung` ad wait on unauthenticated resolves. `available_at` can appear
  on any client, so the real player must honor it before `loadfile` **and** still re-resolve on 403 with
  backoff.
- **For spike #4:** `visionos` is a default client only without login, and it does not accept cookies. In
  yt-dlp 2026.11.23 the authenticated defaults are `tv_samsung`, `web_embedded` and `web` (Premium:
  `web_creator`, `tv_samsung`, `web`), plus `web_music` for `music.youtube.com` URLs. None of them is
  `visionos`, so spike #4 must test authenticated client behavior and `available_at`. Also note that yt-dlp
  2026.11.23 picks a random Chrome major version (151 to 157) for its default `User-Agent` per process; the
  real authenticated resolver must pin `--user-agent` to match the login WebView2 build (PROJECT.md 3c).
- mpv's error log prints the full stream URL, including the client IP. The app must not log it.
- Without a JS runtime yt-dlp only warns and still exits 0, with formats possibly missing.
- `ytdl_hook` works in this build but is about as slow as resolving ourselves with the same clients, and
  cannot pre-resolve or re-resolve, so Lumen resolves itself.
- 3.8 to 6 s per resolve makes pre-resolving the next 1-2 tracks necessary.
- When Deno runs to solve a JS challenge, transient private working set spikes by ~210 MB for 1 to 2 s.
  Keep resolves serialized (never spawn multiple yt-dlp/deno resolves at once).
