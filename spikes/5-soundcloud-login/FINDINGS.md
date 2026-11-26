# Spike #5: SoundCloud in-app login, token and likes call

Measured 2026-11-26 on Windows 11, WebView2 158.0.4370.33, tauri 2.12.7, wry 0.57.4, keyring 4.2.0,
reqwest 0.13.5 (native TLS).

## Status

| Check | Result |
|---|---|
| Signed-out `client_id` scrape, live | **worked** (one run) |
| `soundcloud.com/signin` renders in WebView2 | **yes**, no bot check seen |
| Token paste, Credential Manager save and clear, in the real app (fake token) | **worked** |
| Sign in inside WebView2 and capture `oauth_token` | **pending Amin's live login check** |
| One authenticated likes call | **pending Amin's live login check** |

The ticket is not accepted until the two pending rows pass. Everything about the signed-in path is only
tested against fakes.

## Run

```powershell
cd spikes/5-soundcloud-login
pnpm install
pnpm tauri build --no-bundle      # or: pnpm tauri dev (has developer tools, port 1425)
.\src-tauri\target\release\soundcloud-login-spike.exe
```

Signed-out scrape on its own (2 requests today, never prints the id):

```powershell
cd spikes/5-soundcloud-login/src-tauri
cargo run --example live_scrape -- "<WebView2 user agent>"
```

Tests: `cargo test` in `src-tauri` (no test contacts SoundCloud; the transport tests use a local socket,
and two tests write fake values to throwaway Credential Manager entries and delete them).

## Pending Amin's live login check

Use an account you can afford to lose (PROJECT.md 3c). Never paste the token or the `client_id` anywhere.

1. Build and run as above. The window should show `Session none`.
2. Press **Open SoundCloud Login** and sign in with email. The window should close by itself and show
   `oauth_token present (len=N)` and a user agent.
3. Press **Scrape client_id + Fetch Likes** once. Note the name, the number of tracks, the `Path` line
   (`/me/track_likes` or the fallback) and whether the `client_id` was scraped or rescraped.
4. Press **Clear Saved Session**, then run `cmdkey /list | findstr lumen`. It should print nothing.
5. Optional: repeat step 2 with the social sign-in button you normally use (Google, Facebook or Apple).

If something fails:

- The window stays open after sign-in: no `oauth_token` cookie was seen, or its value has characters other
  than letters, digits, `-`, `_` and `.`. In `pnpm tauri dev`, check the cookie name in developer tools
  (Application, Cookies). The paste field takes the cookie value as a fallback.
- `bot check`: stop. Requests stay paused until the app restarts. Do not keep retrying.
- `cooling down`: wait the stated time.
- `session expired`: sign in again once. If it expires again at once, stop and record the message.

| Live result | Value |
|---|---|
| Sign-in method and outcome | pending |
| Token captured, window closed | pending |
| Likes call outcome and path used | pending |
| Clear removed the credential | pending |

## client_id scrape

How it works (`src-tauri/src/scrape.rs`):

1. `GET https://soundcloud.com/`.
2. Take every `<script src="...">` that is `https`, on a `*.sndcdn.com` host, under `/assets/` and ends in
   `.js`. Other scripts are never fetched.
3. Fetch them **from last to first**, at most 20, and stop at the first match.

Patterns, in order of trust:

| Pattern | Regex |
|---|---|
| Object key | `(?:^\|[^A-Za-z0-9_])client_id\s*:\s*"([A-Za-z0-9]{32})"` |
| Query string (fallback) | `(?:^\|[^A-Za-z0-9_])client_id=([A-Za-z0-9]{32})(?:[^A-Za-z0-9]\|$)` |

The leading group stops keys such as `oauth_client_id` from matching. A query string match is only used when
no fetched bundle has the object key form, because it can belong to another service.

### Live signed-out run (2026-11-26 18:14 UTC, one run, no login)

| Measure | Value |
|---|---|
| Result | success, 32 characters (value not logged or stored) |
| Asset bundles listed | 9 |
| Bundles fetched | 1 |
| Matching bundle (document order, 0-based) | 8, the last one |
| Pattern | object key (the fallback was not exercised live) |
| Latency | 1,353 ms, including the wait that starts the two requests 0.75 to 1.25 s apart |

### How fragile it is

Very. Nothing here is a documented interface, and one successful run says nothing about next week.

- It breaks if SoundCloud moves the bundles off `*.sndcdn.com/assets/`, renames the key, changes the id
  length, builds the object differently (single quotes, a variable instead of a literal) or loads the id at
  run time. Each of these ends in `no client_id found (N bundles listed, M fetched)`.
- Bundle file names are hashed and change on deploys. The scraper does not depend on names, only on order
  for speed: if the id moves to an early bundle, up to 9 fetches at today's count (about 1 s apart), at
  most 20.
- The id rotates at unknown intervals. A 401 or 403 triggers one new scrape and one retry.
- No bot check appeared in this run, but one can. Our requests come from reqwest, not Chromium: the TLS
  fingerprint differs and no `sec-ch-ua` or `sec-fetch` headers are sent, so a challenge is possible even
  with the right user agent. A 401 or 403 mentioning `captcha` or `datadome` stops all requests. A
  challenge served as HTTP 200 would instead show up as `no client_id found`.
- Web API rate limits are not published. They were not probed.

## Likes call

- `GET https://api-v2.soundcloud.com/me/track_likes?client_id=...&limit=10` with
  `Authorization: OAuth <token>`, then `GET /me` for the display name.
- **`/me/track_likes` is unconfirmed.** A web search found no source for it, and it was not probed live. If
  it answers 404, the spike calls `/me` and then `/users/{id}/track_likes`, and shows which path answered.
  Drop whichever path turns out to be wrong.
- The response fields read (`collection[].track.title`, `track.user.username`, `next_href`, `/me` `id`,
  `username`, `likes_count`) are assumptions tested only against fake bodies. `likes_count` is optional.

## Login in WebView2

- The sign-in page rendered cleanly in a second Tauri window: Facebook, Google and Apple buttons and the
  email field. No bot check appeared. Social sign-in was not tried; Google may refuse embedded webviews.
  Popups use the WebView2 default.
- At about 880 px wide the page scrolled sideways, so the window now opens 1040 px wide.
- While signed out no `oauth_token` cookie was captured, so nothing was saved.
- The window is named in no capability, and Tauri rejects commands from remote pages, so the SoundCloud page
  cannot call into Rust.
- Rust checks the cookie store once a second (local only, no requests) for up to 10 minutes.
- The user agent and language are read from the login window itself. The pasted-token path reads them from
  the main window, the same WebView2 build:
  `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0 Safari/537.36 Edg/158.0.0.0`.

## Account safety and secrets

| Situation | Behaviour |
|---|---|
| Any request | one at a time, at least 750 ms apart plus up to 500 ms jitter |
| 429 | no retry; pause for `Retry-After` (seconds or date, capped at 24 h), else 60 s doubling to 15 min, plus jitter |
| 3 network or 5xx failures in a row | same doubling pause |
| 401 or 403 on an API call | wait 2 to 3 s, scrape the `client_id` once, retry once; then mark the session expired and send nothing more |
| The new scrape fails (not rate limit or bot check) | session marked expired |
| Bot check | all requests stop until restart; the session is not marked expired |
| Redirects | never followed with the token; otherwise only to `https` SoundCloud hosts |

- `oauth_token`, `client_id`, user agent, language and the expiry flag are one JSON entry in Credential
  Manager (`session.dev.lumen.spike.soundcloud`). The webview only gets `absent` or `present (len=N)`.
- Error messages carry no URL, header or body. reqwest errors include the URL with the `client_id`, so it
  is stripped (`without_url`).

## Gotchas

- A Windows credential holds 2,560 bytes. `set_password` stores UTF-16, which halves that, and a long token
  did not fit. The spike stores bytes (`set_secret`).
- `cookies_for_url` deadlocks in synchronous commands on Windows; it runs on a blocking thread.

## RAM

Release build, summed private working set of all processes, `scripts/measure-ram.ps1`. One reading each.

| State | MB | Processes |
|---|---|---|
| Idle, main window, 30 s after launch | 73.5 (an earlier launch: 80.5) | 7 |
| Login window open, 25 s | 178.6 (earlier: 188.3) | 9 |
| 20 s after closing the login window (earlier build) | 157.7 | 9 |
| 120 s after closing it | 82.4 | 8 |

**The app exceeds both budgets while the login window is open** (100 MB idle, 150 MB playing): the
SoundCloud page's renderer alone is about 92 MB. In the first run it still held 65 MB 20 s after the window
closed; in the second it was gone at 120 s. This happens only around sign-in, but Milestone 7 should decide
whether that is acceptable.

## Open questions for the real implementation (Milestone 7)

- The captured cookies **stay in the WebView2 profile** (`%LOCALAPPDATA%\dev.lumen.spike.soundcloud-login`)
  until Clear Saved Session wipes all browsing data. Decide whether Lumen deletes them right after capture,
  so Credential Manager holds the only copy, at the cost of a fresh web sign-in next time.
- Cool-downs and the bot-check pause live in memory and reset on restart. Persist them.
- `Accept-Language` is the single `navigator.language` value, not the weighted list a browser sends.
- A failed re-scrape after a 401, for example during a network outage, marks the session expired. That is
  strict; revisit if it asks for logins too often.
