# Third-party notices

Lumen does not commit these binaries. `scripts/fetch-pinned-binaries.ps1` downloads the exact
versions pinned in `scripts/pinned-binaries.json` and checks their SHA-256 hashes.

## libmpv (LGPL build, 2026-11-26)

- Archive: `mpv-dev-lgpl-x86_64-20261126-git-1eab28e7b5.7z`, mpv commit `1eab28e7b5`.
- Built by: https://github.com/zhongfly/mpv-winbuild (build scripts in that repository).
- Mirror: the `deps-libmpv-20261126` release of this repository, because the builder keeps
  builds for only 30 days. The mirror carries the licence texts.
- Source: https://github.com/mpv-player/mpv
- Licence: LGPL-2.1-or-later for mpv. FFmpeg and the other libraries are statically linked into
  `libmpv-2.dll`; the FFmpeg parts are under LGPL-3.0-or-later. The builder does not guarantee
  that its list of components and their sources is complete.
- Lumen loads `libmpv-2.dll` dynamically, so it can be replaced with another compatible build.

## yt-dlp 2026.11.23 (`yt-dlp.exe`)

- Source: https://github.com/yt-dlp/yt-dlp
- Licence: the yt-dlp source is Unlicense, but the official `yt-dlp.exe` build bundles GPL
  components, so the executable as a combined work is GPL-3.0-or-later.
- Lumen runs it as a separate process (sidecar) and does not link to it.

## Deno v2.9.8

- Source: https://github.com/denoland/deno
- Licence: MIT.
- Used by yt-dlp as its JavaScript runtime; runs as a separate process.
