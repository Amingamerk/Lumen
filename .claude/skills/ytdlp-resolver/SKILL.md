---
name: ytdlp-resolver
description: Implement or modify Lumen stream resolution in src-tauri/src/resolver/ or src-tauri/binaries/ using yt-dlp, Deno sidecars, stream expiry, available_at waits, or 403 re-resolve logic.
---

# yt-dlp Stream Resolver

Reference for `src-tauri/src/resolver/` and sidecar binaries. Full specs live in `docs/PROJECT.md` (sections 2, 3, 3c, and 4) and `spikes/2-audio/FINDINGS.md`.

## Sidecar bundling (Tauri `externalBin`)

- Sidecar binaries live under `src-tauri/binaries/` (gitignored, never committed) with Tauri's target-triple suffix:
  - `src-tauri/binaries/yt-dlp-x86_64-pc-windows-msvc.exe`
  - `src-tauri/binaries/deno-x86_64-pc-windows-msvc.exe`
- Ensure bundled Deno is discoverable by `yt-dlp` (same directory / PATH). Without a JS runtime, `yt-dlp` only warns and still exits 0 with missing formats.

## Invocation and JSON fields

Always invoke with isolated flags (see `spikes/2-audio/src/resolve.rs`):

```text
yt-dlp -J -f bestaudio --no-playlist --ignore-config --playlist-items 1 [--user-agent <wv2-ua>] -- <url>
```

- `--ignore-config` prevents a user's global `yt-dlp.conf` from altering requests.
- For authenticated YouTube Music resolves, pin `--user-agent` to the same WebView2 build used during login (`yt-dlp` otherwise picks a random Chrome major version per process).
- Parse only what playback needs into `StreamInfo`:
  - `url`: required direct stream URL (reject outputs with only `requested_formats` and no single `url`).
  - `http_headers`: split `User-Agent` from remaining headers for mpv.
  - `available_at`: optional fractional Unix timestamp from pre-roll ads; round up with `ceil()` and wait until `available_at` before handing the URL to mpv (`-J` does not sleep).
  - `expires_at`: parse `expire=<unix_secs>` from query string (`?expire=` / `&expire=`) or `/expire/<unix_secs>/` from HLS manifest paths.

## RAM and concurrency invariants

- **Serialize all resolves.** Never spawn parallel `yt-dlp`/Deno processes. A Deno JS challenge spikes transient private working set to ~217 MB standalone (~298 MB on top of the playing app) for 1-2 s (`docs/PROJECT.md` section 2, `docs/ram-baseline.md`).
- **Pre-resolve only the current and next 1-2 tracks.** Never resolve whole playlists up front.
- **Re-resolve on HTTP 403 or playback stall** with exponential backoff + jitter; mpv does not re-resolve expired URLs.

## Account safety (`docs/PROJECT.md` section 3c)

- One session per service, consistent web-client headers, conservative rate limits.
- Honor `429` and `Retry-After` immediately (pause the feature for a cool-down, no retry loops).
- Cache resolved `StreamInfo` with its `expires_at` and reuse until near expiry.
- Fail closed on repeated errors, captcha, or unusual-traffic responses.
- Never log cookies, tokens, or full stream URLs (stream URLs contain client IP).

## Testing

Put `yt-dlp` behind the `Resolver` trait (`docs/PROJECT.md` section 4). Unit-test argument building, JSON parsing, `expire` extraction, `available_at` rounding/waiting, and 403 retry logic with static fixtures and a fake clock. Never call YouTube or SoundCloud from `cargo test`.
