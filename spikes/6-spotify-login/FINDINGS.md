# Spike #6: Spotify web player token, liked songs and ISRC

Measured 2026-11-26 on Windows 11, WebView2 158.0.4370.33, Rust 1.100.0, Tauri 2.12.7.

## Status

| Question | State |
|---|---|
| Does the TOTP secret, version and token endpoint work today? | **Yes**, confirmed live without an account |
| Does the in-app login window load and can the cookie be captured? | Login page loads; capture proven with a dummy cookie. **Real login pending** |
| Does the token from a real `sp_dc` read liked songs? | **Pending Amin's live run** |
| Is ISRC available for matching? | **Not confirmed. Pending Amin's live run.** Expectation below |

## Run it

```powershell
cd spikes/6-spotify-login
pnpm install
pnpm tauri dev

cd src-tauri
cargo test                    # no network
cargo test -- --ignored       # writes and deletes a test entry in Credential Manager
cargo run --example anonymous_check -- "<WebView2 user agent>" da-DK da
```

## Token flow today

Read from the live bundle `web-player.4c608093.js` and `vendor~web-player.4ac0a26d.js` on 2026-11-26.
The spike sends these in order, each once, at least 1 s apart.

1. **Web player page:** `GET https://open.spotify.com/`. A base64 JSON block,
   `<script id="appServerConfig" type="text/plain">`, holds `clientVersion`.
2. **Server time:** `GET https://open.spotify.com/api/server-time` returns `{"serverTime": <unix seconds>}`.
3. **Access token:** `GET https://open.spotify.com/api/token` with the cookie `sp_dc` and the query
   `reason=init`, `productType=web-player`, `totp`, `totpServer`, `totpVer=61`.
   - `totp` is computed from the local clock, `totpServer` from step 2. The web player sends
     `totpServer=unavailable` if step 2 fails; the spike stops instead.
   - The reply has `accessToken`, `accessTokenExpirationTimestampMs`, `clientId` and `isAnonymous`, and
     may have `totpVerExpired` (`warning` or `error`; the web player reacts to both).
   - A dead cookie does not necessarily fail: the reply can be an anonymous token. The spike treats
     `isAnonymous: true` for a cookie request as an expired session.
4. **Client token:** `POST https://clienttoken.spotify.com/v1/clienttoken` with `client_version`,
   `client_id` (`d8a5ed958d274c2e8ee717e6a4b0971d`, the public web player id) and `js_sdk_data` (device
   brand, model, OS, OS version, device id, device type). The reply grants a token, a refresh interval
   and the host suffixes it belongs on (`spotify.com`, `spotify.net`). A challenge reply stops the run.
5. **API calls:** `Authorization: Bearer`, `client-token` (granted hosts only), `app-platform: WebPlayer`,
   `spotify-app-version: <clientVersion>`.

### TOTP

Standard RFC 6238: HMAC-SHA1, 30 s step, 6 digits. Only the key is unusual. The bundle ships a list of
`{secret, version}` strings and uses the first (newest, 61; 60 and 59 are still listed). Each character
code is XORed with `index % 33 + 9`, the results are written as decimal numbers and joined, and the
**ASCII bytes of that text** are the HMAC key. The constant is public and the same for every visitor; it
is in [totp.rs](src-tauri/src/spotify/totp.rs), tested against the RFC vectors and against codes computed
with Node from the bundle's transform. The public scrapers
[TheNetsky/spotify-secrets](https://github.com/TheNetsky/spotify-secrets) and
[ThetaDev/spotify-secrets](https://code.thetadev.de/ThetaDev/spotify-secrets) listed the same three
versions with the same strings.

### Anonymous live check

One run at 2026-11-26 18:28 UTC, no cookie, token never printed:

```json
{
  "is_anonymous": true,
  "expires_in_secs": 3232,
  "client_id_is_web_player": true,
  "totp_status": null,
  "client_version": "1.3.12.2705.g3af3452a0d56",
  "client_token_domains": ["spotify.com", "spotify.net"]
}
```

So steps 1 to 4 **succeeded**: an anonymous token was issued with `totpVer=61` and no rotation warning, and
a client token was granted. Limits of this result:

- A success does not by itself prove the server checked the code (no deliberately wrong code was sent).
  The secret and version are confirmed separately, from the live bundle.
- Nothing about the cookie exchange, the library calls or ISRC is covered.
- The run used the code before `Accept-Language` was taken from the browser, so it sent `en`.

## Liked songs and ISRC

| Route | Request | ISRC expected? | Cost per 10 tracks |
|---|---|---|---|
| Pathfinder (web player's own) | `POST https://api-partner.spotify.com/pathfinder/v2/query`, `fetchLibraryTracks`, `{offset, limit}` | Probably not | 1 request |
| Track metadata | `GET https://spclient.wg.spotify.com/metadata/4/track/{gid}?market=from_token` | Yes (`external_id`, type `isrc`) | 10 requests |
| REST | `GET https://api.spotify.com/v1/me/tracks?limit=10` | Yes (`track.external_ids.isrc`) if it answers | 1 request |

What each expectation rests on:

- **Pathfinder:** the string `isrc` appears nowhere in either bundle, so the web player never reads an ISRC
  from any reply. The server decides the fields, so this is an inference until the live run. The spike
  checks the track's `isrc`, `externalId(s)` and `external_ids` fields and lists the field names it got.
- **Track metadata:** the web player calls this endpoint (it reads `has_lyrics` from it). That the JSON
  carries `external_id` comes from the endpoint's long-known shape and community projects, not from the
  bundle. `gid` is the base62 track id as 32 hex digits. The spike asks for JSON; a non-JSON reply is
  reported and stops the lookups.
- **REST:** `external_ids` is part of the public API's track object. Community projects report that
  `api.spotify.com` answers web player tokens with 429
  ([example](https://github.com/CHEYCKIT/lokal/pull/127)); not verified here. The web player does not use
  this route for the library, so it is a comparison only.

The persisted query hash is
`087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240`.

### What it means for Spotify to YouTube matching

The spotDL order in `docs/PROJECT.md` section 4 starts with an ISRC search.

- **REST answers with ISRC:** no extra requests. Importing 2,000 likes is 40 pages of 50.
- **Only track metadata has it:** one extra request per track, about 2,000 for 2,000 likes, which is 35 to
  50 minutes at this spike's pace. Look it up lazily in the background matching queue, only for the track
  being matched, and cache it for good. A 429 pauses that queue.
- **Neither works:** skip the ISRC step. Pathfinder still gives title, artists and duration (and album),
  which cover the scoring steps; expect more entries on the review list.

## How fragile this is

High. All of it is undocumented, against Spotify's terms, and can change without notice.

| What can change | Symptom | Fix |
|---|---|---|
| TOTP secret and `totpVer` (59, 60, 61 so far) | `totpVerExpired`, or HTTP 400 on `/api/token` (reported as a rotation) | Read the new list in the web player bundle, update `VERSION` and `CIPHER` |
| TOTP derivation, parameters or the endpoint itself (`get_access_token` was replaced by `/api/token` in 2025) | 400, 403 or 404 | Re-read the bundle |
| Persisted query hash | `PersistedQueryNotFound` | New hash from the bundle; the real app should discover it, not hardcode it |
| Web player page layout or `appServerConfig` | "could not read the web player config response" | Re-read the page |
| `clientVersion` | Two different values 30 minutes apart on 2026-11-26 (`1.3.12.2680`, then `1.3.12.2705`) | Always read it at run time, never hardcode |
| Client token challenge | "asked for a client token challenge" | Not solved here |
| Bot protection on the login page (reCAPTCHA) or social login in WebView2 | Login window never finishes | Manual `sp_dc` paste |
| `sp_dc` expiry or logout | Anonymous token or 401 | Log in again |

The real app should keep all of this in one module, treat import as optional and cache what it already has,
so a break never affects playback.

## Login window

- A separate window opens `accounts.spotify.com/login` with `continue=https://open.spotify.com/`.
- It runs **incognito**, so WebView2 does not write the login cookies to a profile on disk. The only saved
  copy of `sp_dc` is the Credential Manager entry `dev.lumen.spike.spotify-login`, with the user agent,
  `Accept-Language`, a random device id and an expired flag. The cost: every login is a fresh one, with any
  2FA, captcha or new-login e-mail that brings. Whether the real app should keep a profile is open.
- Rust reads the window's cookies once a second (local, no network) and closes it when `sp_dc` appears.
- The window is remote content and is in no capability. A command call from it was rejected live
  (`not allowed by ACL`).
- No other cookie is captured or sent.

## Account safety in this spike

- One limiter for every request Rust sends: 1 to 1.5 s apart, one run at a time, no retries.
- 429 closes all Spotify requests for `Retry-After`; without it, 60 s doubling up to 1 h, with jitter.
- 401, 403 or an anonymous token mark the saved session expired; nothing is sent until a new login.
- One page of 10 tracks, no pagination. Tokens, client version and ISRCs are cached in memory.
- The login window's user agent and languages are sent on every request. HTTPS only, no redirects.
- Secrets have no `Display` and a redacted `Debug`; errors carry endpoint names and status codes only.

Gaps the real app must close:

- The cool-down lives in memory, so restarting the app forgets it.
- Nothing pauses the service after repeated 5xx, network or unreadable (captcha page) replies. Here every
  run is a manual click and stops at the first failure.
- The requests do not look exactly like the browser's: Rust uses a different TLS stack, sends no
  `Sec-CH-UA` or `Sec-Fetch` headers, and no other Spotify cookies.

## Verified here

- `cargo test`: 64 passed, plus the ignored Credential Manager round trip run on this machine.
- Release app driven over WebView2's local debug port, with no credentials: UI renders; the login window
  loads Spotify's login page with the same user agent as the main window; closing it is reported; a pasted
  dummy value is saved and cleared without staying in the page; a dummy `sp_dc` planted in the login window
  (switched offline first, so it was never sent) was picked up, saved, and the window closed; clearing left
  no credential behind.
- Idle RAM of the release spike (login window closed), summed private working set via
  `scripts/measure-ram.ps1`: **72.9 MB** median of 3 runs (73.1, 72.9, 71.3), 7 processes. Measured before
  the `Accept-Language` change. RAM with the login window open was not measured.

## Pending Amin's live login + ISRC verification

Use an account you can afford to lose. **Never paste `sp_dc` or a token into a chat, issue or PR.**

1. `pnpm tauri dev` in `spikes/6-spotify-login`.
2. Click **Open Spotify Login** and log in. Expected: the window closes by itself and the status says
   `saved`. If it does not, note what the window showed. Fallback: paste `sp_dc` by hand from a logged-in
   browser (DevTools, Application, Cookies, `open.spotify.com`).
3. Click **Derive Token + Check Liked Songs & ISRC** once (about 15 s). Record the numbers below, the
   "Track fields" line and any red message.
4. Optional, once, after step 3: **Compare: REST /v1/me/tracks**. A 429 is a valid result. It pauses all
   Spotify requests; wait it out, do not restart the app to get around it.
5. Click **Clear Saved Session**.

| Result | Pathfinder | REST |
|---|---|---|
| Login window captured `sp_dc` | | n/a |
| Tracks returned | | |
| ISRC in the liked songs response | | |
| ISRC from track metadata | | n/a |
| Track fields | | |
| Errors (status codes only) | | |

Then update the Status table and the expectations above with what really happened, and only then mark the
PR ready.
