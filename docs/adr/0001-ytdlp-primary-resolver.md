# 0001: yt-dlp is the primary resolver

## Status

Accepted, 2026-11-26.

## Context

Lumen needs a direct audio URL for each YouTube Music (later SoundCloud) track. Options: `yt-dlp` as a
sidecar, mpv's own `ytdl_hook`, or the native Rust crate `rustypipe`.

Measured in [spike #2](../../spikes/2-audio/FINDINGS.md) (yt-dlp 2026.11.23, Deno 2.9.8, LGPL libmpv, signed
out) and [spike #4](../../spikes/4-ytm-login/FINDINGS.md) (signed in):

| Measure | Value |
|---|---|
| Stream | format 251, Opus 48 kHz, about 129 kbps |
| Expiry | about 6 h after resolve |
| Resolve, signed out, default clients | 5.9 to 6.9 s |
| Resolve, signed out, `default,-tv_samsung` | 3.7 to 4.2 s |
| Resolve, signed in, default clients (one run) | 12.3 s, `TVHTML5`, HTTP 206 |
| `loadfile` to audio, direct URL | 100 to 131 ms (one earlier run 283 ms) |
| `loadfile` to audio, `ytdl_hook` (includes resolving) | 6.4 to 9.2 s |
| Playing RAM, spike binary + `libmpv-2.dll` | 13.1 MB private working set |
| Resolve RAM peak, `default,-tv_samsung` (process tree) | 57.2 MB median |
| Resolve RAM peak with a Deno JS challenge (process tree) | up to 216.8 MB for 1 to 2 s |
| `libmpv-2.dll` on disk | about 103 MB |

- `ytdl_hook` works, but is about as slow and cannot pre-resolve or re-resolve.
- `rustypipe` has had no release since 2025-04-23, needs a separate PO-token binary for the web client and is
  GPL-3. It was not spiked.

## Decision

- `yt-dlp` (`yt-dlp -J`), bundled with Deno, is the primary backend behind the `Resolver` trait. Version
  pinned and auto-updated.
- Lumen resolves itself and gives libmpv a direct URL. `ytdl_hook` is not used.
- `rustypipe` stays optional, for search and metadata only.

## Consequences

- Resolving takes seconds, so the next 1-2 tracks are pre-resolved.
- Resolves are serialized: the Deno challenge adds about 210 MB. That peak breaks the 150 MB budget and must
  be measured inside the app in Milestone 2.
- Honor `available_at` before `loadfile`. Also re-resolve on 403 with backoff: mpv ends the file with error
  -13 and does not retry, and `libmpv2` reports it as an error result, not an `EndFile` event. Waiting is not
  proven to prevent the 403 seen once in testing.
- Pass `--user-agent` pinned to the User-Agent captured from the login window; the `yt-dlp` default is
  random per process. Run with `--ignore-config` and one item per call.
- Signed in, the default clients `tv_samsung` and `web_embedded` override that User-Agent, which conflicts
  with the one-device-identity rule. Milestone 2 tests `web_music,web` first (may need a PO token on free
  accounts); otherwise it chooses between a PO token provider and accepting mixed identities.
- Cookies reach `yt-dlp` through a temporary Netscape file removed by an RAII drop guard, with a startup sweep
  for leftovers. The release profile must therefore not use `panic = "abort"`. A killed process leaves the
  file until the next start. Whether to read rotated cookies back is open.
- Never log or show stream URLs, raw `yt-dlp` output or mpv error log lines that contain URLs.
- A missing JS runtime only produces a warning and exit code 0, so check that Deno is present.
- The signed-out `default,-tv_samsung` speed choice does not carry over: the `visionos` client it relies on
  does not accept cookies.
- The LGPL `libmpv-2.dll` ships next to the exe (about 103 MB). Building with MSVC needs an `mpv.lib`
  generated from the DLL exports.
- Extraction can break whenever YouTube changes. Other backends can be added behind the trait.
