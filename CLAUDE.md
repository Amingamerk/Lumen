# Lumen

Lightweight Windows desktop music player. Combines YouTube Music, SoundCloud and Spotify (metadata only) into one library, one queue and one recommendation engine. Full spec: [PROJECT.md](PROJECT.md). Read it before starting work.

## Stack

Rust + Tauri 2 (WebView2) backend, Svelte 5 + TypeScript + Vite + Tailwind frontend. Audio via libmpv (`libmpv2`), SQLite, `rustypipe` with a `yt-dlp` sidecar fallback, `souvlaki` for media keys. Target platform: Windows.

## Hard rules

- **RAM budget:** at most 100 MB idle, at most 150 MB while playing a large playlist (all processes including WebView2). Measure in Task Manager after every milestone. If a feature breaks the budget, fix it before moving on.
- **Heavy work lives in Rust:** network, stream resolving, audio, SQLite, recommendations, thumbnail resizing. The webview only renders.
- **All player state lives in Rust.** The UI mirrors it through Tauri events. No polling from the UI.
- **The frontend never calls external services.** Everything goes through Rust commands.
- **No HTML `<audio>`.** Playback is libmpv only.
- **Virtualize long lists.** Load thumbnails (not full images) lazily through the `lumen://art/...` protocol.
- **Spotify is metadata only** (import playlists and liked songs). Spotify tracks play by matching them on YouTube Music.
- **Recommendations** combine YouTube Music radio, Last.fm similar tracks and own play/skip/like history.
- **Secrets** go in Windows Credential Manager via `keyring`. Never commit keys.
- Spotify-inspired layout, but own name, colors and logo. Do not copy their branding.

## Milestones

Work one at a time, in order, in small reviewable steps:

1. Skeleton UI
2. YouTube Music playback
3. Queue + media keys
4. SQLite library
5. SoundCloud
6. Spotify import
7. Recommendations
8. Polish

A milestone is done when it builds, works in the real app, and RAM is within budget.

## Code conventions

- Rust: `cargo clippy` clean, `thiserror` for module errors, `anyhow` at the top level.
- Backend and CSV changes start with a failing test.
- Run typecheck and both test suites (Rust and frontend) before reporting a step done.
- After each step: run the app, check it works, check RAM.

## Workflow

- Show the diff and ask before every commit, push, PR, merge and release.
- No AI credit anywhere: no co-author trailers, no "generated with" lines in commits, PRs or releases.
