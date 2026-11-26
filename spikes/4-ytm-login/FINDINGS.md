# Spike #4: YouTube Music in-app login and one authenticated call

Measured 2026-11-26 on Windows 11 with Tauri 2.12.7, wry 0.57.4, WebView2 158.0.4370.33, yt-dlp
2026.11.23, Deno 2.9.8, `keyring` 4.2.0.

## Status

**No real login has happened yet.** The issue's acceptance ("works in WebView2, or documents Google's
embedded-login block and proves the cookie-import fallback") is open until the live check below is done.

| Claim | State |
|---|---|
| Login window opens `music.youtube.com`, then Google's sign-in page loads | Seen live, signed out |
| Google accepts the sign-in inside WebView2 | **Pending** |
| `cookies_for_url` reads the InPrivate login window | Seen live (signed-out cookies) |
| User-Agent read from the login window's page | Seen live |
| Session saved to and cleared from Credential Manager | Seen live in the app, with fake imported cookies |
| Authenticated `browse` call returns the library | **Pending** (unit tests and one anonymous call only) |
| Authenticated yt-dlp resolve, client, `available_at`, Range status | **Pending** (arguments and cookie file accepted by the real yt-dlp against a closed local port) |
| Cookie-import fallback works against YouTube | **Pending** |

## Run it

Needs `yt-dlp` and `deno` on `PATH`.

```powershell
cd spikes/4-ytm-login
pnpm install
pnpm tauri build --no-bundle
.\src-tauri\target\release\ytm-login-spike.exe
```

Checks: `pnpm run check`, `pnpm run build`, and in `src-tauri`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`. Two tests are ignored by default and run with
`cargo test -- --ignored`: a round trip of **fake** data through the real Credential Manager, and the real
yt-dlp against `127.0.0.1:1`.

## Pending Amin's live login check

This uses a real account against an unofficial API (PROJECT.md 3c). Press each test button once. Never
paste cookies, tokens or stream URLs into the issue, the PR or a chat.

1. **Open YouTube Music Login.** Answer the consent page, sign in, complete 2FA. Google may send a
   "new sign-in" notice, because the window is InPrivate.
   - [ ] Did Google block it ("This browser or app may not be secure" or similar)? At which step?
   - [ ] Did the window close by itself? If it stayed on `music.youtube.com`, press **Capture now** and
     note the message.
   - [ ] Saved session shows: cookie count, `LOGIN_INFO` present, a User-Agent without `Edg/`.
2. **Test Authenticated Call + yt-dlp** with **yt-dlp defaults**.
   - [ ] Playlist names and counts match the library.
   - [ ] Record: client, format, protocol, `available_at` and wait, expiry, Range status, resolve time,
     "Stream User-Agent equals login", warnings.
3. Wait at least 10 s, choose **only clients that use our User-Agent**, test once.
   - [ ] Record the same values, or the error (a Premium account and a free one may differ).
4. **Only if step 1 was blocked** (or to prove the fallback anyway): in a private window of a normal
   browser, sign in to `music.youtube.com`, copy the `Cookie` request header or export `cookies.txt`, copy
   `navigator.userAgent`, close the private window, then import in the app and repeat step 2.
   - [ ] Import accepted and the test passes. Delete any exported file.
5. **Clear Saved Session.**
   - [ ] `cmdkey /list | findstr lumen` prints nothing.
   - Clearing only removes Lumen's copy. To end the session itself, sign that device out in the Google
     account's security page.

If a run answers 401/403, "expired", a bot check or 429: stop, note the message, do not retry in a loop.

## Architecture

All logic is plain Rust in `src-tauri/src`, behind seams for the clock, HTTP, the credential store and the
yt-dlp process. Unit tests never reach YouTube.

| File | Job |
|---|---|
| `cookies.rs` | Netscape `cookies.txt` and `Cookie:` header parsing, youtube.com filter, rendering |
| `auth.rs` | `SAPISIDHASH <ts>_<sha1("<ts> <sapisid> <origin>")>`, from `SAPISID` or `__Secure-3PAPISID` |
| `session.rs` | The one session, chunked storage in Credential Manager |
| `jar.rs` | Temporary Netscape file for yt-dlp |
| `innertube.rs` | One `POST /youtubei/v1/browse` (`FEmusic_liked_playlists`, `WEB_REMIX`) |
| `resolver.rs` | yt-dlp run, safe report, `available_at` wait, one `Range: bytes=0-1023` request |
| `gate.rs`, `account.rs` | One limiter and one lock for every session write and YouTube call |
| `http.rs`, `transport.rs` | reqwest, no redirects with credentials, errors without URLs |
| `webview.rs`, `commands.rs`, `main.rs` | Tauri glue |

The UI (`src/App.svelte`) gets a session summary and the two reports. Cookie values only travel from the UI
to Rust (import), never back.

## Observed without logging in

- A fresh InPrivate window opening `https://music.youtube.com` lands on `consent.youtube.com` (EU cookie
  consent, in the system language).
- Its **Log ind** button leads to `accounts.google.com/v3/signin/identifier` with a working email field. No
  block message at that point. Google's embedded-browser refusal usually appears only after the email or
  password step, so **this proves nothing about the login itself**.
- `cookies_for_url("https://music.youtube.com")` returned 3 to 5 signed-out cookies.
- Tauri injects `window.__TAURI_INTERNALS__` into the remote page too. The capability file lists only the
  `main` window, so the login window has no permissions, but a page could detect it is inside Tauri.
- One anonymous `browse` request (curl, no cookies) answered **HTTP 200**, not 401, with
  `logged_in: "0"` in `responseContext.serviceTrackingParams`, a sign-in message and no playlist grid.
  The parser therefore treats `logged_in=0` as a rejected session.

## WebView2 sends YouTube a different User-Agent

In the same login window:

| Request to | `User-Agent` ends with | `sec-ch-ua` brands |
|---|---|---|
| `example.com` | `Chrome/158.0.0.0 Safari/537.36 Edg/158.0.0.0` | Microsoft Edge, Chromium, Microsoft Edge WebView2 |
| `music.youtube.com`, Google sign-in | `Chrome/158.0.0.0 Safari/537.36` | Chromium, Google Chrome |

`navigator.userAgent` matched the sent header in both cases. `ICoreWebView2Settings2::UserAgent`, read on the
YouTube consent page, still returned the **Edge** string. So the settings API (and the main window's
`navigator.userAgent`) give the wrong value for 3c's "same WebView2 build, same headers". The spike reads
`navigator.userAgent` from the login window while it is on `music.youtube.com`.

Follow-ups: the override is WebView2's own and may change with a runtime update, so capture it at every
login. The `browse` call does not send `sec-ch-ua` client hints, which a browser would.

For imported cookies the app **requires** the source browser's User-Agent, instead of guessing one.

## Tauri 2 cookie API notes

- `WebviewWindow::cookies_for_url` / `cookies` work with `tauri = { features = [] }`. The `unstable` feature
  is not needed in 2.12.7.
- They return HttpOnly and Secure cookies.
- They deadlock on Windows in synchronous commands and event handlers
  ([wry#583](https://github.com/tauri-apps/wry/issues/583)). Building a window does too. The spike uses
  async commands, and the page-load handler spawns a task.
- **`Cookie::domain()` strips the leading dot.** The `cookie` crate does this for every accessor, so
  `.youtube.com` and `youtube.com` look the same. Copied as is, the session cookies would become host-only
  and be sent neither to `music.youtube.com` nor by yt-dlp. The spike puts the dot back on `youtube.com`
  (except `__Host-` names). Other host names are kept as they come.
- Session cookies report `Expiration::Session`.
- `eval_with_callback` returns the script result as JSON.
- `incognito(true)` gives the login window an InPrivate profile: nothing of the login is meant to stay in
  the WebView2 profile, and no live page keeps rotating the captured cookies (yt-dlp recommends a private
  window for exactly that reason). The cost: Google sees a new browser at every login.

## Secrets

- Cookies and User-Agent are stored only in Credential Manager (`lumen-spike-ytm`).
- **One credential holds at most 2560 bytes** (`CRED_MAX_CREDENTIAL_BLOB_SIZE`), too small for a signed-in
  cookie set. The session is split into `session-chunk-N` entries plus a manifest written last.
- `keyring`'s default Windows persistence is `Enterprise`, which can roam with a domain profile. Fine on a
  home PC; Milestone 2 could set `Local` through `keyring-core`.
- Only youtube.com cookies are kept, also from an imported file that holds every site.
- yt-dlp gets a Netscape file in `%TEMP%\lumen-ytm-spike\`. A drop guard removes it, also on panic (so the
  release profile must not use `panic = "abort"`). Startup sweeps leftovers, and exit waits for a running
  test first, because yt-dlp rewrites the file when it ends. A killed process or power cut leaves the file
  until the next start.
- yt-dlp writes rotated cookies back into that file; the spike throws them away. Milestone 2 should decide
  whether to read them back.
- yt-dlp's output and the stream URL are never shown: errors are fixed categories.

## yt-dlp when signed in (read from source, not yet seen live)

From `yt_dlp/extractor/youtube` at tag 2026.11.23:

- It counts cookies as signed in only with `LOGIN_INFO` plus a SAPISID cookie. The login capture waits for
  both.
- Signed-in default clients are `tv_samsung`, `web_embedded`, `web` (Premium: `web_creator`, `tv_samsung`,
  `web`), plus `web_music` for `music.youtube.com` URLs.
- **`tv_samsung` and `web_embedded` have built-in User-Agents** (a Samsung TV and Safari on macOS), so with
  the defaults the account cookies go out under other device identities even with `--user-agent`. That
  conflicts with 3c ("one device identity").
- `web` and `web_music` have none, so `--user-agent` applies. But for both, HTTPS and DASH formats require a
  GVS PO token unless the account is Premium (HLS does not), so a free account may get only HLS or no usable
  format from them.

The app offers both choices so the live check can compare them. **Decision for Milestone 2:** use the
UA-consistent clients if they deliver, otherwise choose between a PO token provider and accepting the mixed
identities.

## Account-safety behaviour

| Signal | Reaction |
|---|---|
| 401, 403, `logged_in=0`, yt-dlp "sign in" or "cookies are no longer valid" | Session saved as expired; nothing more is sent until a new login or import |
| 429, yt-dlp rate-limit message | Cool-down: `Retry-After` as the minimum, else 60 s doubling to 1 h, plus jitter |
| Captcha, "unusual traffic", "not a bot" | Paused until the app restarts |
| Three failed runs in a row | Paused until the app restarts |
| Any two runs | At least 10 s apart, never in parallel |

Limits of the spike: cool-down and pause live in memory, so a restart clears them, and two app instances
would each have their own limiter (run only one). Nothing is cached, since a run is a manual button press. A refused Range request (403) is reported and counted, but does not expire
the session, because it comes from the stream host. The login window itself is a normal browser and is not
rate limited. yt-dlp's own requests inside one run are not paced by the app.

## Memory

Release build, `scripts/measure-ram.ps1`, summed private working set.

| Scenario | Result |
|---|---|
| Idle, login window never opened (3 launches: 77.3, 74.1, 76.6 MB) | median 76.6 MB |
| Login window open on Google's sign-in page (one reading, DevTools port on) | **203.9 MB**, 10 processes |

The login window pushes the total over the 150 MB budget while it is open. It is a short, rare, user-driven
state, but Milestone 2 should destroy the window as soon as the session is captured and measure that memory
returns. Executable: 13.3 MB.

## Scripting gotcha: do not use `CloseMainWindow()`

Tao creates a helper window (`Tao Thread Event Target`, empty title) that Windows reports as visible.
PowerShell's `Process.CloseMainWindow()` sometimes picks it. Closing it destroys the event loop's wake-up
window: the app stops answering close requests properly and the process never exits. Seen twice, then
reproduced on purpose. Sending `WM_CLOSE` to the window of class `Tauri Window` (what the X button does)
exited cleanly every time. Any automated RAM or smoke script has to target that window.

## How the signed-out check was driven

The release app was started with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>` and
driven over the DevTools protocol: open the login window, read the page and headers, press **Log ind**,
press **Capture now**, import obviously fake cookies, clear them, and press the test button with an invalid
track URL (rejected before anything is sent). Temporary diagnostic builds showed the two User-Agent values;
that code is not committed. No email or password was typed and no request carried the fake cookies. The idle
RAM runs used no debug port.
