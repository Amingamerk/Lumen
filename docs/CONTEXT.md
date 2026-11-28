# Lumen - domain context

Glossary and model boundaries for Milestone 1 onwards. Behaviour and numbers: [PROJECT.md](PROJECT.md).
Decisions: [adr/](adr/README.md). Use these names in code, tickets and tests.

## Glossary

### Track
One song as Lumen knows it: own `id`, title, artists, optional album, duration, artwork URL, optional ISRC,
and a list of `SourceRef`. The same song on several services is one Track with several refs. A Track is
metadata; it holds no Stream URL. Queue, playlists, likes and history refer to Tracks.

### Artwork
The picture of a Track (or of a playlist, drawn from its first song that has one). The webview never learns where
it lives: it asks for `lumen://art/<size>/<track id>`, with a size of 64 (rows, the player bar) or 300 (cards, page
heads), and Rust fetches it once from an allowlisted image host, resizes it and keeps it on disk. A picture that
cannot be had leaves the placeholder cover showing.

### SourceRef and SourceKind
`SourceRef` is a pointer to one item on one service: `kind` + the service's own `id`.
`SourceKind` is `YouTube` (YouTube Music), `SoundCloud` or `Spotify`.

### Playlist, Album and Collection
A playlist is an ordered list of Tracks that someone put together; an album is a release by its artists.
**Collection** is the word for either one: both are shown as cards. Albums and the sample playlists exist
only as test fixtures. What the user sees are **local playlists**: made, renamed and deleted by the user,
stored in the library, listed in the sidebar and in the Library view, and opened on the playlist route.

A playlist holds **entries**, each a place in the list that points to a Track. The same Track added twice is two
entries, and removing one moves the later ones up. Deleting a playlist removes its entries and nothing else: the
Tracks, likes and history stay. Playlists keep the order they were made in; reordering and importing the
YouTube Music library come later. A playlist has no source of its own; its songs do.

### Track slice
Some consecutive rows of one list (`offset`, `limit`, at most 200) with the list's total. The whole list
stays in Rust; the playlist route asks for the slices it is about to show.

### Shelf
One horizontal row of Collection cards on Home.

### Search
A query plus a source chip (`All`, `YouTube` or `SoundCloud`) and a type chip (`All`, `Songs`, `Albums`, `Artists`
or `Playlists`). Rust does the searching and remembers the last query and chips.

- With `All` or `YouTube`, YouTube Music is searched live. Only Enter and a type chip send a search, and only
  for a query that is not blank; changing the source chip sends nothing. One search makes at most one request
  through the Account-safety gate, and what it found is kept in memory for an hour (50 queries and chips at
  most), for that one Session. A type chip that was already answered shows its answer without a request.
- The `All` type chip finds a **top result** (the best match, with up to 3 quick-play songs), songs, albums,
  artists and playlists. The other type chips ask for one type only. Albums, artists and playlists are listed with
  their pictures; opening their pages comes later. Unknown shelves in a reply are skipped.
- With `SoundCloud`, nothing is searched: the page says SoundCloud arrives later (Milestone 7). The sample
  catalog is searched only by tests: every word must appear in a Track's title, artists or album, a
  playlist's name, or an album's title or artists. A blank query matches everything.

### Queue
The ordered list of what plays. Starting a Collection or a catalog search result replaces it with all of their
Tracks. A live YouTube Music result empties it and plays by itself. When Next or the end of a track passes the last entry, the **Recommender** appends an **autoplay**
pick and keeps a few upcoming. The history is the entries before the current one. The whole Queue stays in
Rust; the UI asks for the upcoming rows it shows.

### Queue entry
One place in the Queue: a Track plus a `key` that is unique and never reused, so the same Track twice is two
entries. Reordering moves an upcoming entry before another, by key.

### Engine
What makes sound, behind the `Engine` trait: load, play or pause, seek, volume, position. The app's engine is
libmpv; tests use stand-ins. An engine reports back only through engine events: loaded, duration, progress,
ended and failed.

### Media
What an Engine is asked to play: a local file, a Stream URL with its headers, or a catalog sample. libmpv
cannot play samples: choosing one says "sample track, cannot be played" and changes nothing. Resolving
happens before the Engine sees Media.

### Playback status
`idle`, `loading`, `playing`, `paused`, `ended` or `error`. Real audio (a local file or a live YouTube Music
song) is played by itself: the Queue is emptied, Previous and Next are off, and nothing advances when it ends. Queued playback of real audio, with
autoplay on by default, comes in Milestone 4.

### Library, like and history
The library is the SQLite file `library.db` in the app's local data folder. It keeps each Track it has met once,
found again by any of its `SourceRef`s, with the **liked songs** and the **history**. A like is one row per Track;
unliking removes it and keeps the Track. The history has one row per **play**: written when audio starts (not when
a song is chosen or resolved), with its start time, the time it actually sounded and why it ended (`finished`,
`skipped`, `replaced`, `failed`, `exit`, or `interrupted` after a crash). A pause, a seek, a stream recovery or
a failed load adds no row and no listened time. The Library view's **Recently played** lists each song once, the
latest play first. A file made by a newer Lumen, or a damaged one, is reported and left as it is; likes and
history stay off until it is dealt with. It also holds the **kept streams** of the Resolver and the **match** cache. Songs of the library, and the rows of a local playlist, play one at a
time, as a search result does, until Milestone 4. A run with `--list` opens a **throwaway library** in the system's
temp folder, holding one sample playlist of 5,000 songs, and never reads or writes the user's `library.db`.

### Notice
The small error message over the app, for a refused command or a failed load or playback. It never holds a
Stream URL, headers or the Engine's own words.

### Source
The metadata role of a service, behind the `Source` trait: `search` and `related` (radio seeds), returning
Tracks. YouTube Music is the first one. `related` answers "unsupported" without sending anything until
Milestone 6. Library import also belongs to the source module. A Source never produces audio.

### Resolver
The audio role: `SourceRef` -> `StreamInfo`. `yt-dlp` with Deno is the first backend
([ADR 0001](adr/0001-ytdlp-primary-resolver.md)). Only `YouTube` and `SoundCloud` refs can be resolved.
The `Resolver` trait (`resolve`, `forget`, `forget_all`) makes backends swappable; `YtDlpResolver` is the first.
A Spotify ref is never resolved: it is "unsupported" and nothing runs.

A resolve is one `yt-dlp` run through the Account-safety gate. Its `StreamInfo` is cached in memory and kept in
the library for the login that resolved it, and used again, even after a restart, while the URL has at least
10 minutes left. A new login, a disconnect and a failed load drop it. A selection that has been replaced before
its turn runs nothing.

### Stream URL and StreamInfo
The Stream URL is a short-lived direct audio URL handed to libmpv. `StreamInfo` is the Stream URL plus what
is needed to play it:

| Part | Meaning |
|---|---|
| `url` | The Stream URL. Sensitive (can include the client IP): never logged or sent to the UI. Kept unencrypted in the library, without any cookie or login header |
| expiry | When the URL stops working (about 6 h in the Milestone 0 YouTube resolves) |
| `available_at` | Optional earliest time to use the URL; wait for it before loading |
| headers | HTTP headers mpv must send, including the pinned `User-Agent` |

The current and next 1-2 queue entries are resolved (only the current one until queued playback of real audio
in Milestone 4). HTTP 403, an expired URL or a stall means resolve again; mpv does not do it.

### Match
A cached mapping from a Spotify Track to a playable `YouTube` (else `SoundCloud`) `SourceRef`, with its
scores (artist, title, duration and overall), kept in the library. ISRC search first, then title/artist/duration scoring. With no match anywhere, the Track is greyed
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

For YouTube Music, a Session with no unexpired cookie to sign a request with is also expired, and nothing is
sent. A late reply for a Session that has been replaced changes nothing. If the expired flag cannot be saved,
the Session is still not sent again and the gate pauses. A new login confirms the Session with one library
browse; there is no automatic retry.

### Verified
A Session that YouTube Music has answered as logged in since Lumen started (`logged_in=1`). Kept in memory
only and tied to that one Session, so after a restart the account reads Connected until a request succeeds.
The other outcomes of the check are unchecked, checking, offline and unexpected.

### Account-safety gate
The single per-service checkpoint every service request passes: serialization, minimum spacing, backoff with
jitter, `429` / `Retry-After` cool-down, pause on captcha or repeated errors, and no requests on an expired
Session. A `yt-dlp` run passes the gate as one unit; the requests inside it are not paced by Lumen. Rules:
PROJECT.md section 3c.

The YouTube gate (the only one so far) is ready, cooling down or paused:

- One run at a time, at least 1 s after the previous run ended, and 5 s between two `yt-dlp` runs.
- A `429` cools down for 60 s, doubling to 1 h while no request succeeds, or for the `Retry-After` time if that
  is longer. Jitter only lengthens a wait. A bot check cools down the same way and also pauses.
- The third failure in a row pauses. A connection that never opened (DNS, refused, unreachable) counts nothing.
- A pause ends only with Resume, which sends nothing and keeps a running cool-down.
- The state is saved in `%LOCALAPPDATA%\dev.lumen.player\youtube-gate.json` and survives a restart. A file
  that cannot be read or written pauses.
- A `Retry-After` can ask for a very long cool-down, and the UI has no way to shorten it. Deleting
  `youtube-gate.json` while Lumen is closed is the only override.

## Boundaries

| Area | Owns | Does not |
|---|---|---|
| UI (Svelte) | Rendering, user input | Call external services, hold player state or secrets |
| `ipc` | Commands in, events out | Business logic |
| `sources` | Service APIs, import, metadata -> Track | Play audio |
| `catalog` | Domain shapes, the static fixtures (tests and `--list` only), Home shelves, catalog search, Track slices | Call services, store data |
| `resolver` | `SourceRef` -> `StreamInfo` | Decide queue order |
| `queue` | Order, shuffle, repeat, pre-resolve timing | Talk to services directly |
| `recs` | Autoplay picks (static until Milestone 6) | Play audio |
| `audio` | Engine (libmpv), playback state, transport rules | Resolve URLs |
| `library` | SQLite: Tracks, likes, history, local playlists (caches and Matches later) | Store secrets |
| `art` | Artwork: fetch from allowlisted hosts, resize, cap and keep on disk, answer `lumen://art` | Send a remote address to the webview, send cookies, use the account gate |

## Invariants

- Spotify is metadata only. It never resolves or plays audio.
- Player state lives in Rust. The UI mirrors it through events and does not poll.
- Secrets stay in Rust and Windows Credential Manager. They are never sent to the UI, logged or committed.
  The only plain-file copy is the temporary cookie jar for `yt-dlp` (ADR 0001).
- Only one resolve runs at a time.
- A Spotify import failure never affects playback of what is already cached.
