# Lumen — Project Description

> A lightweight desktop music player that combines
> YouTube Music, SoundCloud and Spotify into one library, one queue and one recommendation engine.

## 1. Goal

Build a Windows desktop music app that looks and feels like Spotify / YouTube Music desktop,
but uses a fraction of the memory of a browser tab or an Electron app.

- One search across YouTube Music and SoundCloud.
- Import Spotify playlists and liked songs, and play them via YouTube Music.
- One mixed queue: tracks from any source play back-to-back.
- Good "radio" / recommendations based on what I play, not tied to one service.

### Non-goals (for now)
- No playing Spotify audio directly (needs Spotify's own player; `librespot` breaks their terms).
- No mobile version, no web version, no multi-user accounts, no social features.
- Not a downloader. Caching is only for smooth playback.
- Personal use, not a commercial product. Use own name, colors and logo — Spotify-*inspired*
  layout, not a copy of their branding.

## 2. Performance budget (the whole point of the project)

| Metric | Target |
|---|---|
| RAM, idle with home screen open | ≤ 100 MB total (all processes incl. WebView2) |
| RAM, playing + 5,000-track playlist open | ≤ 150 MB |
| Cold start to usable UI | ≤ 1.5 s |
| CPU while playing, window minimized | ≤ 1–2 % |
| Installer size | ≤ 20 MB (excluding yt-dlp / mpv) |

Measure these at the end of every milestone (Task Manager, grouped processes). If a feature
breaks the budget, fix it before moving on.

### Rules that keep it light
1. **All heavy work lives in Rust**: networking, stream resolving, audio, database, recommendations,
   image resizing. The webview only renders.
2. **Audio plays in Rust (libmpv)**, never via an HTML `<audio>` element.
3. **Lightweight frontend**: Svelte 5, no large component libraries.
4. **Virtualized lists** for anything that can be long (playlists, search results, library).
5. **Thumbnails, not full images**: Rust downloads, resizes (e.g. 300 px and 64 px) and caches
   album art on disk; the UI loads them lazily through a custom protocol (`lumen://art/...`).
6. No polling from the UI; Rust pushes state changes as Tauri events.

## 3. Tech stack

| Layer | Choice |
|---|---|
| App shell | **Tauri 2** (WebView2 on Windows) |
| Frontend | **Svelte 5 + TypeScript + Vite + Tailwind CSS** |
| Async runtime | `tokio` |
| HTTP | `reqwest` |
| YouTube / YouTube Music | `rustypipe` (pure Rust) with **`yt-dlp` sidecar as fallback** for stream URLs |
| SoundCloud | Unofficial API (public `client_id`, same method other third-party clients use) |
| Spotify | Official Web API, OAuth PKCE, **metadata only** (playlists, liked songs, followed artists) |
| Audio playback | **libmpv** via the `libmpv2` crate (handles streaming, buffering, gapless, codecs) |
| Database | SQLite via `sqlx` (or `rusqlite`) |
| Media keys / Windows media overlay | `souvlaki` |
| Discord status (later) | `discord-rich-presence` |
| Recommendations | Last.fm API + YouTube Music radio + ListenBrainz (optional) + own history |
| Secrets | Windows Credential Manager via `keyring` |

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
│  resolver/   "find a playable stream for this Track" │
│  recs/       recommendation engine                   │
│  library/    SQLite: tracks, playlists, history      │
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
    isrc: Option<String>,     // helps match Spotify → YouTube
    sources: Vec<SourceRef>,  // same song can exist on several services
}
```

```rust
#[async_trait]
trait Source {
    async fn search(&self, q: &str, limit: usize) -> Result<Vec<Track>>;
    async fn stream_url(&self, r: &SourceRef) -> Result<StreamInfo>; // Err for Spotify
    async fn related(&self, r: &SourceRef) -> Result<Vec<Track>>;    // radio seeds
}
```

### Resolving playback
- YouTube / SoundCloud track → ask that source for a stream URL → hand to mpv.
- Spotify track → search YouTube Music with `artist + title`, pick the best match by duration
  (±3 s), "Topic"/official channel preference and title similarity → cache the mapping in SQLite.
- Stream URLs expire: resolve just before play, and pre-resolve the **next** track in the queue
  ~20 s before the current one ends so there are no gaps.

### Recommendations ("Radio" and "Home")
Score and mix candidates from several signals:
1. **YouTube Music radio** for the seed track (strong, ready-made).
2. **Last.fm** `track.getSimilar` and `artist.getSimilar`.
3. **Own history**: play counts, skips (< 30 s = negative), likes, time of day.
4. **SoundCloud related tracks** when the seed is from SoundCloud.

Rules: de-duplicate across sources, max 2 tracks per artist in a row, never repeat something
played in the last N tracks, down-rank skipped tracks. Home screen = "Because you played X",
"More from artists you like", "Rediscover" (old favorites not played recently).

Note: Spotify's recommendations / related-artists endpoints are closed to new apps (since late
2024), so Spotify is used only for importing my library.

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
- Keyboard: Space play/pause, Ctrl+F search, Ctrl+←/→ prev/next. Media keys work globally.
- Mini-player mode later.

## 6. Project structure

```
lumen/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ api/        # tauri commands + events
│  │  ├─ player/
│  │  ├─ sources/
│  │  ├─ resolver/
│  │  ├─ recs/
│  │  ├─ library/    # db + migrations
│  │  ├─ art/
│  │  └─ media/
│  ├─ binaries/      # yt-dlp sidecar
│  └─ tauri.conf.json
├─ src/              # Svelte app
│  ├─ lib/components/  # Sidebar, PlayerBar, TrackList, Card, QueuePanel
│  ├─ lib/stores/      # player state mirrored from Rust events
│  └─ routes/          # home, search, library, playlist
└─ PROJECT.md
```

## 7. Milestones

1. **Skeleton** — Tauri 2 + Svelte + Tailwind, dark layout with sidebar, main view and player bar
   (static data). Measure baseline RAM.
2. **YouTube Music playback** — search YouTube Music, click to play through libmpv, play/pause,
   seek, volume, progress events to the UI.
3. **Queue** — add / play next / reorder / remove, shuffle, repeat, pre-resolve next track,
   media keys + Windows media overlay.
4. **Library** — SQLite: liked songs, local playlists, play history, artwork cache.
5. **SoundCloud** — search + playback in the same queue, source badges, combined search view.
6. **Spotify import** — OAuth login, import playlists + liked songs, YouTube matching with cache,
   "couldn't match" review list.
7. **Recommendations** — radio from any track, Home screen rows, skip/like signals.
8. **Polish** — artwork-color gradients, animations, settings page, mini-player, Discord status,
   installer, auto-update of yt-dlp.

Each milestone ends with: it builds, it works in the real app, and RAM is within budget.

## 8. Setup / requirements

- Rust (stable), Node 20+, pnpm, Tauri 2 prerequisites for Windows (WebView2 is built into Windows 11).
- `libmpv` DLL (`libmpv-2.dll`) bundled next to the exe.
- `yt-dlp.exe` as a Tauri sidecar; the app checks for updates to it.
- API keys (stored with `keyring`, never committed):
  - Last.fm API key (free, register an API account).
  - Spotify app client ID (Spotify developer dashboard, PKCE, redirect `http://127.0.0.1:<port>/callback`).

## 9. Risks

| Risk | Mitigation |
|---|---|
| YouTube changes break stream extraction | yt-dlp fallback with self-update; keep `rustypipe` up to date |
| SoundCloud `client_id` rotates | Re-scrape automatically on 401/403 |
| Wrong Spotify → YouTube matches | Duration + title scoring, manual "fix match", cache corrections |
| RAM creep in the webview | Virtual lists, thumbnails only, measure every milestone |
| Unofficial APIs and terms of service | Personal use only, no redistribution of audio |

## 10. Conventions for working with Claude

- One milestone at a time; small, reviewable steps.
- Rust: `cargo clippy` clean, `thiserror` for errors in modules, `anyhow` at the top level.
- Frontend never calls external services directly — always through Rust commands.
- All player state lives in Rust; the UI is a mirror updated by events.
- After each step: run the app, check it works, check RAM.
