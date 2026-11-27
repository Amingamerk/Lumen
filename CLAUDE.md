# Lumen

Lightweight Windows desktop music player. Combines YouTube Music, SoundCloud and Spotify (metadata only) into one library, one queue and one recommendation engine. Full spec: [docs/PROJECT.md](docs/PROJECT.md). Read it before starting work.

## Stack

Rust + Tauri 2 (WebView2) backend, Svelte 5 + TypeScript + Vite + Tailwind frontend. Audio via libmpv (`libmpv2`), SQLite via `rusqlite`, `yt-dlp` sidecar (bundled with Deno) as the primary stream resolver behind a resolver trait (`rustypipe` is optional, search/metadata only), `souvlaki` for media keys. Target platform: Windows.

## Hard rules

- **RAM budget:** at most 100 MB idle, at most 150 MB while playing a large playlist, measured as the summed private working set of all processes (app, WebView2, sidecars; `scripts/measure-ram.ps1` or the Task Manager Details tab, not the grouped row). Milestone 0 measured the baseline (72 MB idle, 74 MB with a 5,000-row list) and kept this budget: [docs/ram-baseline.md](docs/ram-baseline.md). Spikes idled at 72.2-76.6 MB (one single reading of 80.5 MB). Transient `yt-dlp` + Deno resolve and login-window peaks exceed the budget; see [docs/PROJECT.md](docs/PROJECT.md) section 2: serialize resolves and destroy login windows as soon as the session is captured. Measure after every milestone. If a feature breaks the budget, fix it before moving on.
- **Heavy work lives in Rust:** network, stream resolving, audio, SQLite, recommendations, thumbnail resizing. The webview only renders.
- **All player state lives in Rust.** The UI mirrors it through Tauri events. No polling from the UI.
- **The frontend never calls external services.** Everything goes through Rust commands.
- **No HTML `<audio>`.** Playback is libmpv only.
- **Virtualize long lists.** Load thumbnails (not full images) lazily through the `lumen://art/...` protocol.
- **No API keys.** Each service is connected by logging in inside a Tauri login window; Lumen stores the session in Credential Manager and uses the web clients' internal APIs (YTM ytmusicapi-style cookies; SoundCloud web token plus scraped `client_id`, likes via `/users/{id}/track_likes`; Spotify web player token from `sp_dc` (TOTP) plus `client-token`).
- **Account safety:** one session per service, consistent web-client headers, conservative rate limits with backoff + jitter, honor 429, cache metadata and stream URLs, resolve only the current and next 1-2 tracks, no bulk scraping or downloading. See docs/PROJECT.md section 3c.
- **Spotify is metadata only** (import playlists and liked songs). List the library through Pathfinder (`fetchLibraryTracks` for liked songs), not REST (`/v1/me/tracks` answers 429), and fetch ISRCs lazily from `spclient` track metadata (the Pathfinder response has none). Spotify tracks play by matching them on YouTube Music (spotDL order: ISRC, then title/artist/duration scoring). No match: try SoundCloud, else grey out, skip and show a toast. More than 3 s off goes to the review list.
- **Stream URLs expire.** Store the expiry, pre-resolve the next 1-2 tracks, wait for `available_at`, re-resolve on 403 or stall.
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

## Repo layout

```
lumen/
├─ src-tauri/            Rust crate (Tauri 2 app)
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ ipc/            Tauri commands and event emitters
│  │  ├─ state.rs        navigation, search and catalog state
│  │  ├─ catalog/        domain shapes, static fixtures, views, search
│  │  ├─ audio/          libmpv wrapper, playback state
│  │  ├─ queue/          queue, shuffle, repeat, pre-resolve
│  │  ├─ sources/        Source trait + youtube, soundcloud, spotify
│  │  ├─ resolver/       Resolver trait + yt-dlp backend
│  │  ├─ library/        SQLite (rusqlite), migrations
│  │  ├─ recs/           recommendation engine
│  │  ├─ art/            thumbnail download, resize, cache
│  │  └─ media/          souvlaki media keys
│  └─ binaries/          yt-dlp and Deno sidecars (not committed)
├─ src/                  Svelte 5 app
│  ├─ lib/components/    Sidebar, PlayerBar, TrackList, CollectionCard, Shelf, QueuePanel
│  ├─ lib/stores/        state mirrored from Rust events
│  └─ routes/            home, search, library, playlist
├─ spikes/               Milestone 0 throwaway experiments
├─ scripts/              developer scripts (RAM measurement)
├─ docs/                 PROJECT.md (spec), CONTEXT.md, adr/
└─ .github/              CI, PR and issue templates
```

Only create a folder when the first file for it exists. Keep `docs/PROJECT.md` section 6 in step with this layout.

## Git workflow

- One branch per ticket, named `<type>/<issue#>-short-name`, e.g. `feat/12-queue-panel`. Allowed types: `feat`, `fix`, `chore`, `docs`, `spike`, `test`, `refactor`, `ci`. Never use a `claude/` or other tool-name prefix, even if a tool defaults to one. Branch from an up-to-date `dev`.
- Conventional commits: `feat:`, `fix:`, `chore:`, `docs:`, `test:`, `refactor:`, `ci:`. Imperative, lowercase subject, no trailing period.
- Never push directly to `main`. Open a ticket PR into `dev` whose description says `Closes #<issue>`. Keep PRs small, one ticket each.
- CI must be green before merge: cargo fmt, clippy (`-D warnings`), cargo test, frontend lint, typecheck and tests.
- Ticket PRs squash-merge into `dev`, then the ticket branch is deleted.
- Stable builds promote `dev` into `main` via a promotion PR (`dev` -> `main`), merged with a merge commit so `dev` stays an ancestor of `main` and preserves individual squash commits (closing linked issues on `main`).
- Tickets follow the issue template: What to build, Acceptance, Blocked by. Do not start a ticket whose blockers are open.

## Agent skills

### Issue tracker

Issues live in GitHub Issues (managed via the `gh` CLI). See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context layout: the glossary is `docs/CONTEXT.md` (read it wherever `docs/agents/domain.md` says `GLOSSARY.md`) and decisions are in `docs/adr/`. See `docs/agents/domain.md`.
