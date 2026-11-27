<h1 align="center">
  <img src="docs/assets/lumen-logo.svg" alt="Lumen" width="360">
</h1>

A lightweight Windows desktop music player that combines YouTube Music, SoundCloud and Spotify into one library, one queue and one recommendation engine.

Lumen aims to look and feel like Spotify or YouTube Music on the desktop, while using a fraction of the memory of a browser tab or an Electron app. It is a personal project, not a commercial product.

## Features

Planned (see [Status](#status)):

- One search across YouTube Music and SoundCloud.
- One mixed queue: tracks from any source play back to back, with shuffle, repeat and gapless-style pre-resolving of upcoming tracks.
- Log in to each service inside the app. No API keys or developer apps needed.
- Import Spotify playlists and liked songs. Spotify is used for metadata only, and imported tracks play by matching them on YouTube Music.
- Recommendations and radio that combine YouTube Music radio and your own play, skip and like history.
- Local library in SQLite: liked songs, playlists, play history.
- Global media keys and the Windows media overlay.
- Dark, Spotify-inspired layout (sidebar, card grid, player bar, queue panel) with Lumen's own name, colors and logo.

## Stack

| Layer | Choice |
|---|---|
| App shell | Rust + Tauri 2 (WebView2) |
| Frontend | Svelte 5, TypeScript, Vite, Tailwind CSS |
| Audio | libmpv via the `libmpv2` crate |
| Stream resolving | `yt-dlp` sidecar (bundled with Deno), behind a swappable resolver trait |
| Database | SQLite via `rusqlite` |
| Media keys | `souvlaki` |
| Login / secrets | In-app login windows; sessions stored in Windows Credential Manager via `keyring` |

All heavy work (network, stream resolving, audio, database, recommendations, image resizing) lives in Rust. The webview only renders and mirrors player state from Rust events. The full spec is in [docs/PROJECT.md](docs/PROJECT.md).

## Status

Early stage. The app shell (sidebar, placeholder views and an inactive player bar) runs; there is no playback or sign-in yet. Work happens one milestone at a time:

0. Spikes: yt-dlp + libmpv playback, and measuring the real Tauri RAM baseline
1. Skeleton UI
2. YouTube Music playback
3. SQLite library + resolver trait
4. Queue + media keys
5. Spotify import
6. Recommendations
7. SoundCloud
8. Polish

Performance target: at most 100 MB RAM idle and 150 MB while playing a large playlist (summed private working set of all processes). Milestone 0 measured the baseline and kept this budget (100 MB idle, 150 MB playing): see [docs/ram-baseline.md](docs/ram-baseline.md).

## Prerequisites

Windows 10 or 11.

- Rust (stable) with the MSVC toolchain (Visual Studio Build Tools, "Desktop development with C++")
- Node.js 20.19 or newer and pnpm (the version is pinned in `package.json`)
- Tauri CLI and the other [Tauri 2 prerequisites for Windows](https://tauri.app/start/prerequisites/) (WebView2 is built into Windows 11)
- libmpv: `libmpv-2.dll` placed next to the executable (and the import library for linking)
- `yt-dlp.exe` and `deno.exe` (yt-dlp needs a JavaScript runtime for YouTube), used as Tauri sidecars in `src-tauri/binaries/`
- Accounts for the services you want to use (YouTube Music, SoundCloud, Spotify). You log in inside Lumen; there are no API keys or developer apps to set up.

Session data is stored in Windows Credential Manager. Never commit it.

## Build

libmpv, `yt-dlp` and Deno are not needed until playback lands in milestone 2.

```powershell
git clone https://github.com/Amingamerk/Lumen.git
cd Lumen
pnpm install
pnpm tauri dev      # run in development
pnpm tauri build --no-bundle    # release executable in src-tauri/target/release
```

Checks, as CI runs them:

```powershell
pnpm run lint
pnpm run check
pnpm run test
pnpm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Account safety

Lumen tries to behave like one ordinary user's client: one session per service, conservative rate limits with backoff, caching of metadata and stream URLs, and resolving only the current and next one or two tracks. It does no bulk scraping or downloading and pauses a service when it gets a 429 or captcha. This lowers the risk but cannot remove it. See section 3c of [docs/PROJECT.md](docs/PROJECT.md).

## Personal use and terms of service

Lumen is for personal use only. It talks to YouTube, SoundCloud and Spotify through unofficial means (your logged-in web session, yt-dlp, a scraped SoundCloud `client_id`), which their terms of service do not allow. That is a real risk to your own accounts (warnings, blocks, in the worst case a ban), not a formality. Use accounts you can afford to lose. Do not use it commercially and do not redistribute audio. Lumen is not a downloader; any caching exists only for smooth playback.

Spotify is used for metadata only. Lumen never plays Spotify audio.

Lumen is not affiliated with Spotify, YouTube or SoundCloud. The layout is inspired by Spotify, but the name, colors and logo are Lumen's own.
