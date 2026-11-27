# Spike #4: YouTube Music in-app login and one authenticated call

Measured 2026-11-26 on Windows 11 with Tauri 2.12.7, wry 0.57.4, WebView2 158.0.4370.33, yt-dlp
2026.11.23, Deno 2.9.8, `keyring` 4.2.0.

## Status

**Accepted (live check passed 2026-11-26).** In-app WebView2 login worked without a Google
embedded-browser block, the authenticated `browse` call returned the user's library playlists, and
authenticated `yt-dlp` resolve + Range check succeeded with HTTP 206.

| Claim | State |
|---|---|
| Login window opens `music.youtube.com`, then Google's sign-in page loads | Seen live, signed out |
| Google accepts the sign-in inside WebView2 | **Worked** (live 2026-11-26, no embedded-browser block) |
| `cookies_for_url` reads the InPrivate login window | Seen live (signed-out cookies) |
| User-Agent read from the login window's page | Seen live |
| Session saved to and cleared from Credential Manager | Seen live in the app (fake import and real login) |
| Authenticated `browse` call returns the library | **Worked** (12 playlists on first page, more exist, matched real library) |
| Authenticated yt-dlp resolve, client, `available_at`, Range status | **Worked** (yt-dlp defaults: TVHTML5, format 251 opus 128.93 kbps https, HTTP 206, 0 s wait, ~6 h expiry, stream UA matched login) |
| Cookie-import fallback works against YouTube | Not needed (WebView2 login succeeded directly; import path verified offline with fake cookies) |

## Issue #60: client sets compared (2026-11-27)

Run by the owner with the spike build at `8866185`, yt-dlp 2026.11.23 and Deno 2.9.8 (the versions #61
pins), on a **free** account (not Premium). Three different tracks per mode, runs alternating, 35 s or
more apart, no retries, no 429, captcha or rejected session. Every run: `browse` ok, Deno started, HTTP 206
on the Range request, stream User-Agent equal to the login one, expiry about 6 h (21,591 to 21,594 s).
Times are CET.

| Run | Mode | Client | Format | Codec | Audio only | Bitrate | Protocol | `available_at` wait | Resolve | Warnings |
|---|---|---|---|---|---|---|---|---|---|---|
| 22:23:35 | defaults | `TVHTML5` | 251 | opus | yes | 124.9 kbps | HTTPS | 3 s | 6,875 ms | none known |
| 22:24:43 | `web_music,web` | `WEB_REMIX` | 18 | mp4a.40.2 + avc1.42001E | no | 191.7 kbps total | HTTPS | 4 s | 6,556 ms | formats skipped: PO token required |
| 22:26:22 | defaults | `TVHTML5` | 251 | opus | yes | 128.4 kbps | HTTPS | 4 s | 6,745 ms | none known |
| 22:26:57 | `web_music,web` | `WEB_REMIX` | 18 | mp4a.40.2 + avc1.42001E | no | 121.9 kbps total | HTTPS | 3 s | 6,229 ms | formats skipped: PO token required |
| 22:27:50 | `web_music,web` | `WEB_REMIX` | 18 | mp4a.40.2 + avc1.42001E | no | 113.7 kbps total | HTTPS | 5 s | 6,258 ms | formats skipped: PO token required |
| 22:28:30 | defaults | `TVHTML5` | 251 | opus | yes | 125.3 kbps | HTTPS | 4 s | 6,332 ms | none known |

- **`web_music,web` offers no `bestaudio` format** on a free account: every audio-only format is skipped
  for a missing GVS PO token. What is left is format 18, a combined 360p H.264 + AAC stream (113.7 to 191.7
  kbps total, AAC share unknown). It plays, but downloads video for nothing, at lower audio quality. No HLS
  format was chosen by `bestaudio/best`.
- **The defaults deliver**: Opus 251 at about 125 kbps, audio only, no warnings, the same as spike #4 but
  about half its resolve time (6.3 to 6.9 s against 12.3 s).
- libmpv played a defaults stream (format 251) audibly through "Play 5 s in libmpv".
- The stream fetch carried the login User-Agent in both modes. With the defaults, the extraction requests
  of the TV and embedded clients still use their built-in User-Agents (see "yt-dlp when signed in").

Decision: keep the yt-dlp default clients and accept the mixed identities during extraction.
[ADR 0003](../../docs/adr/0003-ytdlp-client-set.md).

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

## Extensions for issue #60 (client sets)

- Building also needs libmpv in `%LOCALAPPDATA%\Programs\libmpv` (or `LIBMPV_DIR`); `build.rs` copies
  `libmpv-2.dll` beside the exe.
- The format selector is now `bestaudio/best`, so a client set without an audio-only format shows its
  HLS or combined fallback (`audio_only`, `vcodec`, total bitrate) instead of failing. Results above
  were measured with `bestaudio`.
- yt-dlp runs with `-v`. `deno_started` is true when a `[jsc:deno] Running deno:` debug line appears;
  debug lines are ignored for warnings and never leave `resolver.rs`.
- "Play 5 s in libmpv" plays the stream of the latest successful run. The URL stays in Rust memory.
- "Copy safe report" copies the run as text without URLs, cookies or account data. Premium status is
  not detected: note it by hand.

## Live login check (2026-11-26)

| Check | Result |
|---|---|
| In-app WebView2 login | Succeeded (no Google block, session captured and saved) |
| Authenticated `browse` (`FEmusic_liked_playlists`) | Succeeded: 12 playlists on first page (more exist), matched user's library |
| `yt-dlp` resolve mode tested | `yt-dlp defaults` (`web_music,web` compared later, see issue #60 section) |
| Client | `TVHTML5` |
| Format | `251`, `opus`, `128.93 kbps`, `https` |
| `available_at` | `2026-11-26 23:13:43 CET` (waited `0 s`) |
| Expires | `2026-11-27 05:13:40 CET` (~6 h) |
| Range request (`bytes=0-1023`) | `HTTP 206` |
| Resolve time | `12,286 ms` |
| Stream User-Agent equals login | `yes` |
| Warnings | `none known` |
| Cookie-import fallback | Not needed (WebView2 login succeeded directly; import tested offline with fake cookies) |
| Clear Saved Session | Verified clean (`cmdkey /list | findstr lumen` printed nothing) |

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

## yt-dlp when signed in

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

Live check (2026-11-26): `yt-dlp defaults` resolved cleanly via `TVHTML5` (format 251 opus 128.93 kbps https,
HTTP 206, 0 s wait, ~6 h expiry) and the stream Range request used the login User-Agent. The
UA-consistent `web_music,web` option was compared in issue #60 (above): on a free account it gets no
audio-only format without a PO token, so Lumen keeps the defaults ([ADR 0003](../../docs/adr/0003-ytdlp-client-set.md)).

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
