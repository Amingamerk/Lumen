# Lumen - domain context

Glossary and model boundaries for Milestone 1 onwards. Behaviour and numbers: [PROJECT.md](PROJECT.md).
Decisions: [adr/](adr/README.md). Use these names in code, tickets and tests.

## Glossary

### Track
One song as Lumen knows it: own `id`, title, artists, optional album, duration, artwork URL, optional ISRC,
and a list of `SourceRef`. The same song on several services is one Track with several refs. A Track is
metadata; it holds no Stream URL. Queue, playlists, likes and history refer to Tracks.

### SourceRef and SourceKind
`SourceRef` is a pointer to one item on one service: `kind` + the service's own `id`.
`SourceKind` is `YouTube` (YouTube Music), `SoundCloud` or `Spotify`.

### Playlist, Album and Collection
A playlist is an ordered list of Tracks that someone put together; an album is a release by its artists. Each
comes from one `SourceKind`. **Collection** is the word for either one: both are shown as cards and opened on
the playlist route.

### Shelf
One horizontal row of Collection cards on Home.

### Search
A query plus a source chip: `All`, `YouTube` or `SoundCloud` (Spotify shows up under `All`). Every word must
appear in a Track's title, artists or album, a playlist's name, or an album's title or artists. A blank query
matches everything. Rust does the matching and remembers the last query and chip.

### Source
The metadata role of a service, behind the `Source` trait: `search` and `related` (radio seeds), returning
Tracks. Library import also belongs to the source module. A Source never produces audio.

### Resolver
The audio role, behind the `Resolver` trait: `SourceRef` -> `StreamInfo`. Backends are swappable;
`yt-dlp` is first ([ADR 0001](adr/0001-ytdlp-primary-resolver.md)). Only `YouTube` and `SoundCloud` refs can
be resolved.

### Stream URL and StreamInfo
The Stream URL is a short-lived direct audio URL handed to libmpv. `StreamInfo` is the Stream URL plus what
is needed to play it:

| Part | Meaning |
|---|---|
| `url` | The Stream URL. Sensitive (can include the client IP): never logged or sent to the UI |
| expiry | When the URL stops working (about 6 h in the Milestone 0 YouTube resolves) |
| `available_at` | Optional earliest time to use the URL; wait for it before loading |
| headers | HTTP headers mpv must send, including the pinned `User-Agent` |

The current and next 1-2 queue entries are resolved. HTTP 403, an expired URL or a stall means resolve
again; mpv does not do it.

### Match
A cached mapping from a Spotify Track to a playable `YouTube` (else `SoundCloud`) `SourceRef`, with its
scores. ISRC search first, then title/artist/duration scoring. With no match anywhere, the Track is greyed
out, skipped, and a toast is shown.

### Review list
Matches more than 3 s off in duration or below the confidence threshold, waiting for a manual fix. A manual
correction is cached.

### Session
The one logged-in state per service, captured from the in-app login window: cookies or tokens, the headers
that must stay consistent with that login (`User-Agent`, language) and an expired flag. Stored in Windows
Credential Manager. Rejected credentials (401, 403, logged-out or anonymous replies) mark it expired and
require a new login. A SoundCloud 401/403 first gets one `client_id` re-scrape and one retry. A 403 on a
Stream URL does not expire the Session.

### Account-safety gate
The single per-service checkpoint every service request passes: serialization, minimum spacing, backoff with
jitter, `429` / `Retry-After` cool-down, pause on captcha or repeated errors, and no requests on an expired
Session. A `yt-dlp` run passes the gate as one unit; the requests inside it are not paced by Lumen. Rules:
PROJECT.md section 3c.

## Boundaries

| Area | Owns | Does not |
|---|---|---|
| UI (Svelte) | Rendering, user input | Call external services, hold player state or secrets |
| `ipc` | Commands in, events out | Business logic |
| `sources` | Service APIs, import, metadata -> Track | Play audio |
| `catalog` | Domain shapes, the static fixtures, Home/Library/Search views | Call services, store data |
| `resolver` | `SourceRef` -> `StreamInfo` | Decide queue order |
| `queue` | Order, shuffle, repeat, pre-resolve timing | Talk to services directly |
| `audio` | libmpv, playback state | Resolve URLs |
| `library` | SQLite: Tracks, playlists, history, caches, Matches | Store secrets |

## Invariants

- Spotify is metadata only. It never resolves or plays audio.
- Player state lives in Rust. The UI mirrors it through events and does not poll.
- Secrets stay in Rust and Windows Credential Manager. They are never sent to the UI, logged or committed.
  The only plain-file copy is the temporary cookie jar for `yt-dlp` (ADR 0001).
- Only one resolve runs at a time.
- A Spotify import failure never affects playback of what is already cached.
