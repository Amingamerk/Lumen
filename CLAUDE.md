# Lumen

Lightweight Windows desktop music player. Combines YouTube Music, SoundCloud and Spotify (metadata only) into one library, one queue and one recommendation engine. Full spec: [PROJECT.md](PROJECT.md). Read it before starting work.

## Stack

Rust + Tauri 2 (WebView2) backend, Svelte 5 + TypeScript + Vite + Tailwind frontend. Audio via libmpv (`libmpv2`), SQLite via `rusqlite`, `yt-dlp` sidecar (bundled with Deno) as the primary stream resolver behind a resolver trait (`rustypipe` is optional, search/metadata only), `souvlaki` for media keys. Target platform: Windows.

## Hard rules

- **RAM budget:** target at most 100 MB idle, at most 150 MB while playing a large playlist, measured as the summed private working set of all processes (app, WebView2, sidecars; Task Manager Details tab, not the grouped row). Milestone 0 measures the real baseline; if the target is unrealistic it is relaxed (about 150/200 MB) and PROJECT.md is updated. Measure after every milestone. If a feature breaks the budget, fix it before moving on.
- **Heavy work lives in Rust:** network, stream resolving, audio, SQLite, recommendations, thumbnail resizing. The webview only renders.
- **All player state lives in Rust.** The UI mirrors it through Tauri events. No polling from the UI.
- **The frontend never calls external services.** Everything goes through Rust commands.
- **No HTML `<audio>`.** Playback is libmpv only.
- **Virtualize long lists.** Load thumbnails (not full images) lazily through the `lumen://art/...` protocol.
- **No API keys.** Each service is connected by logging in inside a Tauri login window; Lumen stores the session in Credential Manager and uses the web clients' internal APIs (YTM ytmusicapi-style cookies, SoundCloud web token, Spotify web player token).
- **Account safety:** one session per service, consistent web-client headers, conservative rate limits with backoff + jitter, honor 429, cache metadata and stream URLs, resolve only the current and next 1-2 tracks, no bulk scraping or downloading. See PROJECT.md section 3c.
- **Spotify is metadata only** (import playlists and liked songs). Spotify tracks play by matching them on YouTube Music (spotDL order: ISRC, then title/artist/duration scoring). No match: try SoundCloud, else grey out, skip and show a toast. More than 3 s off goes to the review list.
- **Stream URLs expire.** Store the expiry, pre-resolve the next 1-2 tracks, re-resolve on 403 or stall.
- **Recommendations** combine YouTube Music radio and own play/skip/like history (no Last.fm key).
- **Secrets** (session cookies, tokens) go in Windows Credential Manager via `keyring`. Never commit or log them.
- Spotify-inspired layout, but own name, colors and logo. Do not copy their branding.

## Milestones

Work one at a time, in order, in small reviewable steps:

0. Spikes (yt-dlp + libmpv playback; Tauri RAM measurement; in-app login per service)
1. Skeleton UI
2. YouTube Music playback
3. SQLite library + resolver trait
4. Queue + media keys
5. Spotify import
6. Recommendations
7. SoundCloud
8. Polish

A milestone is done when it builds, works in the real app, and RAM is within budget.

## Code conventions

- Rust: `cargo clippy` clean, `thiserror` for module errors, `anyhow` at the top level.
- Use the `rust-analyzer-lsp` plugin for Rust code navigation and diagnostics.
- Backend and CSV changes start with a failing test.
- Run typecheck and both test suites (Rust and frontend) before reporting a step done.
- After each step: run the app, check it works, check RAM.

## Workflow

- Show the diff and ask before every commit, push, PR, merge and release.
- No AI credit anywhere: no co-author trailers, no "generated with" lines in commits, PRs or releases.

## Git workflow

- One branch per ticket, named `<type>/<issue#>-short-name`, e.g. `feat/12-queue-panel`. Allowed types: `feat`, `fix`, `chore`, `docs`, `spike`, `test`, `refactor`, `ci`. Never use a `claude/` or other tool-name prefix, even if a tool defaults to one. Branch from an up-to-date `main`.
- Conventional commits: `feat:`, `fix:`, `chore:`, `docs:`, `test:`, `refactor:`, `ci:`. Imperative, lowercase subject, no trailing period.
- Never push to `main`. Open a PR into `main` whose description says `Closes #<issue>`. Keep PRs small, one ticket each.
- CI must be green before merge: cargo fmt, clippy (`-D warnings`), cargo test, frontend lint, typecheck and tests.
- Squash merge, then delete the branch.
- Tickets follow the issue template: What to build, Acceptance, Blocked by. Do not start a ticket whose blockers are open.
