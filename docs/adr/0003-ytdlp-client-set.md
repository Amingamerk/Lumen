# 0003: yt-dlp keeps its default clients when signed in

## Status

Accepted, 2026-11-27. Supersedes the client-set lines of [ADR 0001](0001-ytdlp-primary-resolver.md)
("Signed in, the default clients `tv_samsung` and `web_embedded` override that User-Agent ... Milestone 2
tests `web_music,web` first ...").

## Context

ADR 0001 left open which YouTube clients `yt-dlp` may use with the account's cookies. The yt-dlp defaults
(signed in: `tv_samsung`, `web_embedded`, `web`, plus `web_music` for music.youtube.com) include two clients
with built-in User-Agents, so the account's cookies go out under other device identities during extraction.
That conflicts with [PROJECT.md 3c](../PROJECT.md#3c-account-safety) ("one device identity"). The
alternative, `--extractor-args youtube:player_client=web_music,web`, uses only clients that take the pinned
login User-Agent.

Issue #60 ran both modes with the pinned versions (yt-dlp 2026.11.23, Deno 2.9.8) on the owner's free
account, three tracks each, paced by hand ([spike #4 findings](../../spikes/4-ytm-login/FINDINGS.md#issue-60-client-sets-compared-2026-11-27)):

| Mode | Client picked | Format | Audio only | Bitrate | Resolve | Warnings |
|---|---|---|---|---|---|---|
| defaults | `TVHTML5` | 251, Opus | yes | 124.9 to 128.4 kbps | 6.3 to 6.9 s | none |
| `web_music,web` | `WEB_REMIX` | 18, AAC + 360p H.264 | no | 113.7 to 191.7 kbps total | 6.2 to 6.6 s | formats skipped: PO token required |

Both modes: HTTP 206 on the Range request, stream User-Agent equal to the login one, Deno started, expiry
about 6 h, `available_at` wait 3 to 5 s. libmpv played a defaults stream audibly.

## Decision

- Lumen passes no `player_client` extractor argument: yt-dlp picks from its default clients
  (`resolver::args::CLIENTS = Clients::Default`).
- The format selector stays `-f bestaudio`. Lumen does not fall back to combined video formats.
- `--user-agent` stays pinned to the login window's User-Agent, so every client without a built-in one, and
  the stream download itself, use it.
- Accepted trade-off: during extraction the TV and embedded clients send their own User-Agents with the
  account's cookies. That is the identity mix every signed-in yt-dlp user sends, which is a smaller signal
  than a PO-token provider or downloading video for audio.
- No PO token provider now.

## Consequences

- PROJECT.md 3c names this as the one exception to "one device identity".
- `Clients::PinnedUserAgent` stays in `resolver/args.rs` as the tested way back, if a PO token provider
  (for example a bundled `bgutil` server) is added later or yt-dlp's defaults change.
- Not tested on a Premium account, where yt-dlp's default set differs (`web_creator`, `tv_samsung`, `web`) and
  `web_music,web` may get audio formats without a PO token.
- yt-dlp updates can change the default clients. A resolve that returns no audio-only format surfaces as an
  error, not as a silent video download.
