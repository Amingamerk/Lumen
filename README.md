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

Early stage. The app shell runs on a static sample catalog, you can sign in to YouTube Music, and libmpv plays a local audio file passed with `--play-file` (see [Build](#build)). Nothing is streamed from a service yet. Work happens one milestone at a time:

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

## How Lumen is built

Lumen is my own project. I write the spec ([docs/PROJECT.md](docs/PROJECT.md)), make the architecture decisions ([docs/adr/](docs/adr/)), split each milestone into small tickets and set the hard limits, such as the RAM budget and the account-safety rules.

Coding agents write much of the implementation, test-first, one ticket per pull request. Nothing reaches `dev` without passing CI (fmt, clippy, both test suites, lint, typecheck) and a review against the ticket and the spec. I run every milestone in the real app and measure its memory use before it is promoted to `main`. The agent setup lives in `.claude/` and [docs/agents/](docs/agents/).

## Prerequisites

Windows 10 or 11.

- Rust (stable) with the MSVC toolchain (Visual Studio Build Tools, "Desktop development with C++")
- Node.js 20.19 or newer and pnpm (the version is pinned in `package.json`)
- Tauri CLI and the other [Tauri 2 prerequisites for Windows](https://tauri.app/start/prerequisites/) (WebView2 is built into Windows 11)
- libmpv: `libmpv-2.dll` and its import library `mpv.lib` in `src-tauri/binaries/` (or the folder named by `LIBMPV_DIR`). The build copies the DLL next to the executable
- `yt-dlp.exe` and `deno.exe` (yt-dlp needs a JavaScript runtime for YouTube), used as Tauri sidecars in `src-tauri/binaries/`
- Accounts for the services you want to use (YouTube Music, SoundCloud, Spotify). You log in inside Lumen; there are no API keys or developer apps to set up.

Session data is stored in Windows Credential Manager. Never commit it.

## Build

Fetch the pinned libmpv, yt-dlp and Deno binaries before building. The command verifies
every cached file too, so it is safe to run again.

```powershell
git clone https://github.com/Amingamerk/Lumen.git
cd Lumen
pnpm install
./scripts/fetch-pinned-binaries.ps1
pnpm tauri dev      # run in development
pnpm tauri build --no-bundle    # release executable in src-tauri/target/release
```

To try playback, start the app with a local audio file (URLs and network shares are refused):

```powershell
./src-tauri/target/release/lumen.exe --play-file "C:\path\to\song.flac"
```

To update a pinned dependency, publish or select a permanent source URL, independently
verify its SHA-256, then update its version, URL and hash in
`scripts/pinned-binaries.json`. Update `THIRD-PARTY-NOTICES.md` for any version or
license change, run the fetcher twice from an empty `src-tauri/binaries/`, and run the
script tests before opening the change.

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
