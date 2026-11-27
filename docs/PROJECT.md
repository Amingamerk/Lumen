# Lumen - Project Description

> A lightweight desktop music player that combines
> YouTube Music, SoundCloud and Spotify into one library, one queue and one recommendation engine.

Last updated: 2026-11-26 (plan review applied, see [research notes](#11-research-and-decisions)).

## 1. Goal

Build a Windows desktop music app that looks and feels like Spotify / YouTube Music desktop,
but uses a fraction of the memory of a browser tab or an Electron app.

- One search across YouTube Music and SoundCloud.
- Log in once per service inside the app (no API keys). Import Spotify playlists and liked songs, and play them via YouTube Music.
- One mixed queue: tracks from any source play back-to-back.
- Good "radio" / recommendations based on what I play, not tied to one service.

### Non-goals (for now)
- No playing Spotify audio directly (needs Spotify's own player; `librespot` breaks their terms and needs Premium).
- No mobile version, no web version, no multi-user accounts, no social features.
- Not a downloader. Caching is only for smooth playback.
- Personal use, not a commercial product, no redistribution of audio. Use own name, colors and logo:
  Spotify-*inspired* layout, not a copy of their branding.
- Do not host a hidden music.youtube.com web player (the Electron-wrapper approach). No control over
  gapless playback or memory.

## 2. Performance budget (the whole point of the project)

| Metric | Target |
|---|---|
| RAM, idle with home screen open | <= 100 MB total |
| RAM, playing + 5,000-track playlist open | <= 150 MB |
| Cold start to usable UI | <= 1.5 s |
| CPU while playing, window minimized | <= 1-2 % |
| Installer size | <= 20 MB, excluding the mpv DLL (the LGPL `libmpv-2.dll` is about 103 MB on disk, spike #2), yt-dlp and Deno |

**How RAM is measured:** the *summed private working set of all processes* (the app, every
`msedgewebview2` process and any sidecar such as yt-dlp), read from the Task Manager Details tab or with
[`scripts/measure-ram.ps1`](../scripts/measure-ram.ps1). Do not use the grouped Task Manager row, it hides
processes.

**Measured baseline (Milestone 0, 2026-11-26):** a release Tauri 2 + Svelte 5 spike used 72.2 MB idle on the
home screen and 74.2 MB with a 5,000-row virtualized list open (3-run medians, 7 processes). Adding the
13.1 MB that spike #2 measured for libmpv playback gives an estimate of about 87 MB while playing. The login
spikes (#4 to #6) idled between 72.9 and 76.6 MB (3-run medians for YouTube Music and Spotify, a single
73.5 MB reading for SoundCloud, whose earlier launch read 80.5 MB).
**Decision: the budget stays at 100 MB idle and 150 MB playing.** Full numbers and method:
[ram-baseline.md](ram-baseline.md).

**Transient over-budget peaks seen in Milestone 0** (the budget is not relaxed for them):

| Peak | Measured | Duration | Required handling |
|---|---|---|---|
| `yt-dlp` + Deno JS challenge during a resolve | up to about 217 MB for the spike #2 process tree; about 298 MB estimated on top of the playing app | 1 to 2 s | Keep resolves serialized; measure inside the real app in Milestone 2 |
| Third-party login window open | 178.6 MB (SoundCloud sign-in, an earlier reading 188.3 MB); 203.9 MB (Google sign-in, one reading, DevTools port on) | while open | Destroy the login window as soon as the session is captured; re-measure that the total returns to <= 100 MB |

After closing the SoundCloud login window the total read 157.7 MB at 20 s (earlier build) and 82.4 MB at
120 s. The return was not measured for YouTube Music or Spotify, and the Spotify login window was not
measured while open.

Measure at the end of every milestone. If a feature breaks the budget, fix it before moving on.

### Rules that keep it light
1. **All heavy work lives in Rust**: networking, stream resolving, audio, database, recommendations,
   image resizing. The webview only renders.
2. **Audio plays in Rust (libmpv)**, never via an HTML `<audio>` element.
3. **Lightweight frontend**: Svelte 5, no large component libraries.
4. **Virtualized lists** for anything that can be long (playlists, search results, library). Keep the full
   5,000 rows in Rust and send only the visible slice to a windowed list.
5. **Thumbnails, not full images**: Rust downloads, resizes (e.g. 300 px and 64 px) and caches
   album art on disk; the UI loads them lazily through a custom protocol (`lumen://art/...`).
6. No polling from the UI; Rust pushes state changes as Tauri events.
7. **Lean mpv settings**: `demuxer-max-bytes` about 8 MiB, low `cache-secs`, `vid=no`, `audio-display=no`.

## 3. Tech stack

| Layer | Choice |
|---|---|
| App shell | **Tauri 2** (WebView2 on Windows) |
| Frontend | **Svelte 5 + TypeScript + Vite + Tailwind CSS** |
| Async runtime | `tokio` |
| HTTP | `reqwest` |
| YouTube / YouTube Music stream resolving | **`yt-dlp` sidecar (primary)**, run as `yt-dlp -J`, bundled together with **Deno** (JS runtime yt-dlp needs since 2025.11.12). Pinned version, auto-updated. |
| YouTube Music account, search, library | In-app login; ytmusicapi-style browser auth (the logged-in web cookies + web client headers). The same cookies are passed to yt-dlp. `rustypipe` optional for search and metadata only (see decisions below) |
| SoundCloud | In-app login; the web login's OAuth token plus the scraped public `client_id` (same method yt-dlp uses) against the web client's internal API |
| Spotify | In-app login; web player token derived from the login cookie (`sp_dc`), **metadata only** (playlists, liked songs, followed artists). Most fragile part: Spotify changed token generation in 2025 (TOTP), expect breakage. No developer app. |
| Audio playback | **libmpv** via the `libmpv2` crate (streaming, buffering, gapless, codecs) |
| Database | SQLite via **`rusqlite`** (lighter and faster to build than `sqlx`) |
| Media keys / Windows media overlay | `souvlaki` |
| Discord status (later) | `discord-rich-presence` |
| Recommendations | YouTube Music radio + own history (ListenBrainz optional later). No Last.fm key. |
| Secrets | Session cookies / tokens stored in Windows Credential Manager via `keyring`. Never in plain files, never committed. |

### Stack decisions
- **yt-dlp is the primary resolver, not a fallback.** `rustypipe` has had no release since 2025-04-23, needs a
  separate PO-token binary (`rustypipe-botguard`) for the web client, and is GPL-3. Linking it makes the
  whole binary GPL-3, which is acceptable for personal use only. It stays optional and is not on the
  critical path.
- Spike mpv's own `ytdl_hook` too, but note it does not re-resolve expired URLs, so Lumen resolves itself
  and hands mpv a direct URL (or `edl://` for split audio/video streams). Prefer `bestaudio`.
- Resolving sits behind a **resolver trait with swappable backends**: yt-dlp first, native Rust second,
  something like Piped third.
- **License:** decide before publishing anything. Personal use means GPL-3 is fine.

## 3b. Accounts and login (no API keys)

Lumen has no developer apps and no API keys. Each service is connected by logging in once inside the app:

1. A separate Tauri window opens the service's normal site (music.youtube.com, soundcloud.com,
   open.spotify.com) in WebView2. I log in there like in a browser, including any 2FA.
2. Lumen reads the session cookies / tokens after login, stores them in Windows Credential Manager
   (`keyring`) and closes the window. It never sees or stores the password.
3. Rust uses the web clients' internal APIs with that session:
   - **YouTube Music:** ytmusicapi-style browser auth (cookies + web client headers). The same cookies are
     handed to yt-dlp for resolving.
   - **SoundCloud:** the OAuth token from the web login, plus the scraped `client_id`. Likes are read from
     `/users/{id}/track_likes`.
   - **Spotify:** the web player access token obtained from the `sp_dc` cookie (TOTP), plus a `client-token`.
     Library pages come from Pathfinder `fetchLibraryTracks`. Metadata only.
4. On 401/403 the session is marked expired and the app asks me to log in again. No silent retry loops.

Measured in Milestone 0 (2026-11-26, see [ADR 0002](adr/0002-in-app-login-no-api-keys.md)):

- **YouTube Music:** sign-in in an InPrivate WebView2 window worked with no Google embedded-browser block,
  followed by an authenticated `browse` call and an authenticated yt-dlp resolve (HTTP 206). The fallback
  (import cookies from a browser I am already logged in to) was not needed and was only tested offline.
- **SoundCloud:** email/password sign-in worked and likes were read via `/users/{id}/track_likes`. The Google
  social sign-in popup did not open in the default WebView2 window.
- **Spotify:** TOTP version 61 + `client-token` + Pathfinder `fetchLibraryTracks` worked, with ISRC in none of
  the 10 sampled tracks. `spclient` track metadata returned ISRC for 10 of 10. REST `/v1/me/tracks` returned
  HTTP 429 twice and triggered the 36 s cool-down pause.

Remaining risks: Google may start blocking embedded sign-in; Spotify's token flow changed in 2025 (TOTP
secret) and may change again; all three are unofficial and against the services' terms (see 3c and
section 9).

## 3c. Account safety

Goal: my own accounts should not get flagged. Nothing here can guarantee that, but the projects that live
with these services for years (yt-dlp, ytmusicapi, Spotube, spotDL) share the same habit: behave like one
ordinary user's client and stay quiet.

- **One user, one session per service.** One logged-in session each, no parallel logins, no account sharing,
  one device identity. One accepted exception: signed in, `yt-dlp` keeps its default clients, and two of them
  send their own User-Agent while extracting; the stream download uses the login one
  ([ADR 0003](adr/0003-ytdlp-client-set.md)).
- **Honest, consistent web-client behavior.** Send the headers a real web client sends (user agent, client
  name/version, language), taken from the same WebView2 build that did the login, and keep them stable.
  Keep yt-dlp updated, since it tracks current client versions.
- **Conservative rate limits** per service, with exponential backoff and jitter. Honor `429` and `Retry-After`
  immediately and pause the affected feature for a cool-down instead of retrying.
- **Cache everything cacheable:** metadata, search results, artwork, Spotify -> YouTube matches, stream URLs
  with their expiry. Never ask twice for something already known.
- **Resolve only what is needed:** the current track and the next 1-2. No resolving whole playlists up front.
- **Spotify import is paced.** Fetch pages slowly, once, and cache. Re-import only on request.
- **No bulk scraping, no downloading, no mass searching.** Matching 2,000 imported tracks runs as a slow
  background queue, not a burst.
- **Play like a listener:** only the playing track streams; no audio pre-fetch beyond mpv's small buffer.
- **Fail closed:** on repeated errors, captcha or "unusual traffic" responses, pause that service and tell me.

Personal use / terms of service: YouTube, SoundCloud and Spotify all forbid this kind of access. This is a real
risk to my own accounts (warnings, rate blocks, in the worst case a ban). Use accounts I can afford to lose.
Spotify audio is never played or downloaded.

## 4. Architecture

```
┌─────────────── Svelte UI (WebView2) ───────────────┐
│ Sidebar │ Main view (home / search / playlist)     │
│         │                                          │
│─────────┴──────── Player bar ──────────────────────│
└──────────────▲ Tauri commands  │ events ▼──────────┘
               │                 │
┌──────────────┴─── Rust core ───┴───────────────────┐
│  api/        Tauri commands + event emitters        │
│  player/     mpv wrapper, queue, shuffle/repeat     │
│  sources/    Source trait:                           │
│                youtube.rs  soundcloud.rs  spotify.rs │
│  resolver/   Resolver trait: yt-dlp first, others    │
│  recs/       recommendation engine                   │
│  library/    SQLite (rusqlite): tracks, playlists,   │
│              history, stream/match caches            │
│  art/        thumbnail download/resize/cache         │
│  media/      souvlaki media keys + OS overlay        │
└─────────────────────────────────────────────────────┘
```

### Core data model

```rust
enum SourceKind { YouTube, SoundCloud, Spotify }

struct SourceRef { kind: SourceKind, id: String }

struct Track {
    id: Uuid,                 // our own id
    title: String,
    artists: Vec<String>,
    album: Option<String>,
    duration_ms: u32,
    artwork_url: Option<String>,
    isrc: Option<String>,     // helps match Spotify -> YouTube; Spotify: from spclient track metadata, lazily
    sources: Vec<SourceRef>,  // same song can exist on several services
}
```

Metadata and audio are **separate roles**. Spotify is a metadata source only; it never provides audio.

```rust
#[async_trait]
trait Source {
    async fn search(&self, q: &str, limit: usize) -> Result<Vec<Track>>;
    async fn related(&self, r: &SourceRef) -> Result<Vec<Track>>;    // radio seeds
}

#[async_trait]
trait Resolver {
    async fn resolve(&self, r: &SourceRef) -> Result<StreamInfo>;    // url + expiry + available_at + headers
}
```

### Resolving playback
- YouTube / SoundCloud track -> resolver returns a direct stream URL with its **expiry time** -> hand to mpv.
- Wait until the stream's `available_at` time, when yt-dlp reports one, before handing the URL to mpv.
- Store each URL's expiry. **Pre-resolve the next 1-2 tracks** in the queue before the current one ends so
  there are no gaps. **Re-resolve on HTTP 403 or a stall**, mpv will not do this on its own.
- Recovery of the current track: a load or playback error, 10 s without progress while playing, or an end
  more than 5 s early drops the stream and resolves again through the gate, at least 2 s and then 10 s later.
  After those two attempts it stops with a manual retry and counts one gate failure. An expired URL is
  replaced before a resume or seek uses it. A debug build started with `--break-url` spoils the first stream.
- Gapless via mpv `playlist-append` of pre-resolved URLs (a suggestion, to be verified in a spike).
- YouTube needs PO tokens / SABR handling in some clients. Bundle Deno + yt-dlp-ejs, optionally a token
  provider later, and auto-update yt-dlp.
- Spotify track -> match on YouTube Music (see below) -> cache the mapping in SQLite.

### Spotify -> YouTube Music matching
Follows the spotDL order of operations. The Spotify library response carries no ISRC, so it is looked up from
`spclient` track metadata (`metadata/4/track/{gid}`, about one extra request per track) inside the paced
background matching queue and cached permanently; without it, matching falls back to title/artist/duration
scoring.

1. If the track has an **ISRC** and the provider supports it, search by ISRC. One verified hit wins.
2. Otherwise search `title artists` among YouTube Music **songs** (not plain videos). Return early at
   score >= 80 and verified, otherwise pool the results.
3. Keep the top 8, add a view-count bonus (max 15), cap at 100.
4. Score = average of artist match and title match (slugified, word-sorted, fuzzy ratio), -5 on an explicit
   mismatch.
5. Duration match = `100 * exp(-0.1 * |diff in s|)` (7 s off = 50, 14 s off = 25). Drop below 25, or below 50
   when the average is under 75.
6. Hard cuts: artist score < 70 or title match <= 60. Penalise "live", "remix", "cover", "sped up" (15 each) only
   when absent from the Spotify title; clamp the title score at 0. Prefer Topic / official-audio channels.
7. Weights live in data, not magic numbers. Cache the match together with its scores.

**Unmatched or low-confidence tracks:**
- No match on YouTube Music -> try SoundCloud -> otherwise grey the track out, skip it during playback and
  show a toast.
- A match that is more than 3 s off in duration, or below the confidence threshold, goes to the
  **review list** where I can fix it manually. Corrections are cached.

### Recommendations ("Radio" and "Home")
Score and mix candidates from several signals:
1. **YouTube Music radio** for the seed track (strong, ready-made).
2. **Own history**: play counts, skips (< 30 s = negative), likes, time of day.
3. **SoundCloud related tracks** when the seed is from SoundCloud.

Rules: de-duplicate across sources, max 2 tracks per artist in a row, never repeat something
played in the last N tracks, down-rank skipped tracks. Home screen = "Because you played X",
"More from artists you like", "Rediscover" (old favorites not played recently).

Note: Spotify's recommendation endpoints are not used; Spotify is only for importing my library.
Last.fm is dropped to avoid needing an API key; it can return later as an optional extra.

## 5. UI layout (Spotify / YouTube Music style)

- **Dark theme**, own accent color, rounded corners, subtle hover states.
- **Left sidebar**: Home, Search, Library, list of playlists (with source badges).
- **Main view**:
  - Home: horizontal rows of album/playlist cards (responsive grid).
  - Search: tabs or filter chips for All / YouTube / SoundCloud.
  - Playlist / album page: header with large artwork and a **background gradient taken from the
    artwork's dominant color**, then a virtualized track list.
- **Player bar (bottom)**: artwork + title, controls, seek bar, volume, queue button.
- **Queue panel** (right side, toggle) with drag-to-reorder.
- Small **source badge** (YT / SC / SP) on each track.
- Keyboard: Space play/pause, Ctrl+F search, Ctrl+Left/Right prev/next, Shift+Left/Right seek 5 s back or
  forward, Ctrl+Up/Down volume. Media keys work globally.
- Mini-player mode later.

## 6. Project structure

The target layout is documented in [CLAUDE.md](../CLAUDE.md) under "Repo layout". Rust code lives in
`src-tauri/src/` split into `ipc/`, `catalog/`, `audio/`, `queue/`, `sources/`, `resolver/`, `library/`,
`recs/`, `art/` and `media/`, plus `state.rs` for navigation, search and catalog state and `secrets.rs` for
sessions in Credential Manager (split into chunks); the Svelte app lives in `src/`; spikes in `spikes/`; developer scripts in `scripts/`; docs in `docs/`. `catalog/` holds the domain
shapes, the views built from them, search and Track slices (the playlist page gets only the rows it shows);
its data is a static fixture, including a generated 5,000-Track playlist that Search leaves out, until the
SQLite library replaces it in Milestone 3.

`sources/youtube/` holds the YouTube Music session (`account.rs`, `cookies.rs`, `session.rs`), the
account-safety gate (`gate/`: rules, saved state, clock) and the web client (`auth.rs`, `client.rs`,
`transport.rs`, `http.rs`, which follows the Windows system proxy like the login window). `service.rs` owns the
account, the gate and the transport and is the only way a request goes out. The gate moves up only when a second
service needs it.

## 7. Milestones

0. **Spikes** (before any UI, throwaway code in `spikes/`):
   - a. Bare Rust binary: `yt-dlp` (with Deno) resolves one YouTube Music track and libmpv plays it.
     Also try mpv's own `ytdl_hook` for comparison, and check lean mpv settings.
   - b. Tauri hello world plus a script that sums the **private working set of all processes**
     (app + every `msedgewebview2`). Records the real idle RAM baseline and decides whether the budget in
     section 2 stays or is relaxed.
   - c. Login spike: a Tauri login window for YouTube Music, SoundCloud and Spotify; confirm each login works
     inside WebView2, capture the session, and make one authenticated call per service (YTM library,
     SoundCloud likes, Spotify liked songs via the web token).
1. **Skeleton** - Tauri 2 + Svelte + Tailwind, dark layout with sidebar, main view and player bar
   (static data). Measure baseline RAM.
2. **YouTube Music playback** - search YouTube Music, click to play through libmpv, play/pause,
   seek, volume, progress events to the UI.
3. **SQLite library + resolver trait** - `rusqlite`: liked songs, local playlists, play history, artwork
   cache, stream-URL expiry and match caches. The `Source` and `Resolver` traits land here, before the queue.
4. **Queue + media keys** - add / play next / reorder / remove, shuffle, repeat, pre-resolve next 1-2
   tracks, re-resolve on 403, media keys + Windows media overlay.
5. **Spotify import** - in-app login (web token), paced import of playlists + liked songs, YouTube matching with cache,
   fallback chain (YTM -> SoundCloud -> grey out + toast) and the "couldn't match" review list.
6. **Recommendations** - radio from any track, Home screen rows, skip/like signals.
7. **SoundCloud** - search + playback in the same queue, source badges, combined search view.
   Last of the sources because it is the shakiest (scraped `client_id` + web login token).
8. **Polish** - artwork-color gradients, animations, settings page, mini-player, Discord status,
   installer, auto-update of yt-dlp.

Each milestone ends with: it builds, it works in the real app, and RAM is within budget.

## 8. Setup / requirements

- Rust (stable) with the MSVC toolchain, Node 20+, pnpm, Tauri CLI, Tauri 2 prerequisites for Windows
  (WebView2 is built into Windows 11).
- `libmpv` DLL (`libmpv-2.dll`) bundled next to the exe.
- `yt-dlp.exe` and `deno.exe` as Tauri sidecars; the app checks for yt-dlp updates.
- Accounts for the services I want to use (YouTube Music, SoundCloud, Spotify). No API keys, no developer apps:
  each service is connected by logging in inside Lumen (see section 3b).
- Session data is stored with `keyring` (Windows Credential Manager), never committed.

### Spotify notes
- Spotify is accessed through the web player token, metadata only (liked songs, playlists, followed artists).
  The token flow is unofficial and has changed before (TOTP, 2025). Only liked songs were read in Milestone 0.
- List the library through Pathfinder `fetchLibraryTracks`, not REST `/v1/me/tracks` (HTTP 429 with a web
  player token).
- ISRC is not in the Pathfinder response. `spclient` track metadata has it (10 of 10 sampled); fetch it lazily,
  cache it in SQLite and pause on 429.
- Read `clientVersion` at run time; it changed within 30 minutes during the spike.
- The official developer-mode limits (Premium owner, 5 users, `/items`) do not apply: there is no developer app.

## 9. Risks

| Risk | Mitigation |
|---|---|
| YouTube changes break stream extraction, or needs PO tokens / SABR | yt-dlp as primary with pinned version and auto-update, bundled Deno + yt-dlp-ejs, swappable resolver backends |
| Stream URLs expire mid-play | Store expiry, pre-resolve next 1-2 tracks, re-resolve on 403 or stall |
| RAM budget exceeded (WebView2, sidecars) | Milestone 0 kept the 100 / 150 MB budget ([ram-baseline.md](ram-baseline.md)). Open risks: transient yt-dlp / Deno resolve peaks (serialize, measure in Milestone 2) and login windows (destroy on capture, re-measure). Virtual lists, thumbnails only, lean mpv settings |
| SoundCloud `client_id` rotates, new official API credentials need a paid plan | Scrape and re-scrape automatically on 401/403 (once, then retry once), add retry on the stream-URL step; SoundCloud is last in the order |
| SoundCloud Google sign-in popup does not open in WebView2 | Email/password sign-in works; Milestone 7 handles OAuth popups or guides to email/password |
| Spotify web token flow changes again (TOTP, `client-token`, Pathfinder query hash) | Worked on 2026-11-26 (TOTP v61). Metadata only so playback never depends on it; isolate in one module; manual re-login |
| Spotify rate limits (REST `/v1/me/tracks` answered 429) | Use Pathfinder, fetch ISRCs lazily and paced, honor 429 with a cool-down |
| Account flagged or banned for unofficial access | Section 3c: one session per service, low rates, backoff, caching, resolve only current + next 1-2, pause on 429/captcha. Not a guarantee |
| Session cookies leaked or expired | Credential Manager only, re-login on 401/403, never log tokens |
| Google blocks login in embedded webviews | Not blocked in the Milestone 0 live check; fallback is importing cookies from a logged-in browser |
| Wrong Spotify -> YouTube matches | spotDL-style scoring, review list for > 3 s off, manual "fix match", cached corrections |
| GPL-3 if `rustypipe` is linked | Keep it optional; personal use only; decide the license before publishing |
| Unofficial APIs and terms of service (YouTube and SoundCloud forbid this) | Personal use only, no redistribution of audio. This is a real risk, not a formality |

## 10. Conventions for working with Claude

- One milestone at a time; small, reviewable steps.
- Rust: `cargo clippy` clean, `thiserror` for errors in modules, `anyhow` at the top level.
- Frontend never calls external services directly - always through Rust commands.
- All player state lives in Rust; the UI is a mirror updated by events.
- After each step: run the app, check it works, check RAM.

## 11. Research and decisions

The decisions above started from a plan review, a survey of how other well-starred projects resolve songs, and
the spotDL matching code (`spotdl/providers/audio/base.py`, `utils/matching.py`).

### Milestone 0 Verdict (2026-11-26)

| Question | Verdict | Record |
|---|---|---|
| RAM budget | Keep 100 MB idle / 150 MB playing; transient resolve and login-window peaks are open (section 2) | [ram-baseline.md](ram-baseline.md) |
| Playback and resolver | yt-dlp + Deno resolves and libmpv plays; yt-dlp behind the `Resolver` trait | [ADR 0001](adr/0001-ytdlp-primary-resolver.md) |
| yt-dlp client set, signed in | Defaults (Opus 251); `web_music,web` needs a PO token for audio on a free account | [ADR 0003](adr/0003-ytdlp-client-set.md) |
| YouTube Music login | Works in WebView2; authenticated library call and resolve passed | [ADR 0002](adr/0002-in-app-login-no-api-keys.md) |
| SoundCloud login | Works with email/password; Google popup does not open; likes call passed | [ADR 0002](adr/0002-in-app-login-no-api-keys.md) |
| Spotify login | Works; Pathfinder for liked songs, `spclient` metadata for ISRC, REST answers 429 | [ADR 0002](adr/0002-in-app-login-no-api-keys.md) |
| Domain model | Terms and boundaries for Milestone 1 onwards | [CONTEXT.md](CONTEXT.md) |

### Still unverified (check in later milestones)

- `souvlaki` quirks with Tauri.
- Cold start latency.
- Gapless playback through libmpv `playlist-append`.
- RAM returning to budget after the login window closes for YouTube Music and Spotify.
- The transient Deno resolve peak inside the real app.
- SABR streaming status.
- Licence terms of the libmpv build (the spike used a build published as LGPL).
