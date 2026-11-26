# 0002: In-app login, no API keys

## Status

Accepted, 2026-11-26.

## Context

Lumen needs account access to YouTube Music, SoundCloud and Spotify. Official APIs need developer apps and
keys, with limits or paid plans. The web clients' internal APIs work with an ordinary logged-in session.

Live results in WebView2 158 on 2026-11-26:

| Service | Result | Findings |
|---|---|---|
| YouTube Music | InPrivate sign-in worked with no Google block; authenticated `browse` and `yt-dlp` resolve passed | [spike #4](../../spikes/4-ytm-login/FINDINGS.md) |
| SoundCloud | Email/password sign-in worked; the Google OAuth popup did not open; likes read via `/users/{id}/track_likes`; `client_id` scraped from the web bundles | [spike #5](../../spikes/5-soundcloud-login/FINDINGS.md) |
| Spotify | `sp_dc` captured; TOTP v61 token and `client-token` worked; Pathfinder `fetchLibraryTracks` returned no ISRC; `spclient` track metadata returned ISRC for 10 of 10; REST `/v1/me/tracks` answered 429 twice (36 s cool-down) | [spike #6](../../spikes/6-spotify-login/FINDINGS.md) |

## Decision

- No developer apps and no API keys. Each service is connected by logging in inside a separate Tauri WebView2
  window. Lumen never handles the password.
- One session per service, stored in Windows Credential Manager through `keyring`.
- All service calls come from Rust with the captured session, through one account-safety limiter per service.
- **YouTube Music:** youtube.com cookies, `SAPISIDHASH` auth, web client headers.
- **SoundCloud:** web `oauth_token` plus the scraped public `client_id`.
- **Spotify:** web player token from `sp_dc` (TOTP) plus `client-token`; library via Pathfinder; ISRC looked up
  lazily from `spclient` `metadata/4/track/{gid}` and cached. Metadata only.

## Consequences

Login window:

- It is listed in no Tauri capability, so remote pages cannot call into Rust.
- Cookie reads and window creation run off synchronous commands and event handlers (deadlock on Windows).
- It exceeds the RAM budget while open (178.6 MB SoundCloud, 203.9 MB Google sign-in). Destroy it as soon as
  the session is captured and re-measure the return to <= 100 MB.
- InPrivate keeps login cookies out of the profile (YouTube Music, Spotify), at the cost of a new browser at
  every login. The SoundCloud spike left cookies in the profile; Milestone 7 decides.

Secrets:

- One credential holds at most 2,560 bytes. Store bytes (`set_secret`, not UTF-16 `set_password`) and split
  larger sessions into chunks, with a manifest written last.
- `keyring` defaults to `Enterprise` persistence, which can roam; consider `Local`.

Per service:

- **YouTube Music:** `Cookie::domain()` strips the leading dot, so restore `.youtube.com` (except `__Host-`
  cookies). Read `navigator.userAgent` from the login window while on `music.youtube.com`: WebView2 sends
  YouTube a different User-Agent than its settings API reports. Capture it at every login. Fallback if Google
  blocks the window: import browser cookies with the source browser's User-Agent (tested offline only).
- **SoundCloud:** guide users to email/password or handle the OAuth popup. Re-scrape `client_id` once and retry
  once on 401/403. The scraper is very fragile.
- **Spotify:** an anonymous token for a cookie request means the session expired. Discover `clientVersion` at
  run time, and the persisted query hash instead of hardcoding it. A client token challenge is unsolved;
  manual `sp_dc` paste is the fallback if the login window never finishes. Do not use the REST library route.

Account safety and risk:

- Cool-downs and pauses must persist across restarts (the spikes kept them in memory).
- Rust requests differ from the browser (TLS stack, no client hints), so a bot challenge is possible. Fail
  closed.
- All of this is unofficial, against the services' terms and can break or get an account limited or banned.
  Each service stays isolated in its module so a break does not take down the others. Spotify audio is
  never played or downloaded.
