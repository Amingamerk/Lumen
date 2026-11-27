# RAM baseline

Measured with [`scripts/measure-ram.ps1`](../scripts/measure-ram.ps1) (summed `WorkingSetPrivate` across the app and every descendant process). MB means 1,048,576 bytes.

## Milestone 1 (app shell, static catalog, virtualized playlist, player bar)

Measured 2026-11-27 on the release build of [`src-tauri/target/release/lumen.exe`](../src-tauri/Cargo.toml) (`chore/43-m1-ram-baseline`).

### Summary and verdict

**Pass.** Both Milestone 1 budget targets pass on 3-run medians: **93.1 MB** idle on Home (budget 100 MB, 6.9 MB headroom) and **93.2 MB** with the 5,000-row virtualized playlist open via `--list` (budget 150 MB, 56.8 MB headroom).

| Scenario | Run 1 | Run 2 | Run 3 | Median | Budget | Headroom | Verdict |
|---|---|---|---|---|---|---|---|
| Idle, home screen (30 s, no DevTools) | 93.1 MB | 95.0 MB | 92.7 MB | **93.1 MB** | 100 MB | 6.9 MB | pass |
| 5,000-row virtualized playlist (`--list`, 30 s, no DevTools) | 93.2 MB | 84.6 MB | 93.5 MB | **93.2 MB** | 150 MB | 56.8 MB | pass |
| `--list` scrolled end-to-end (immediate, CDP on) | 99.1 MB | 99.2 MB | 99.3 MB | **99.2 MB** | 150 MB | 50.8 MB | pass |
| `--list` scrolled end-to-end (30 s settle, CDP on) | 96.8 MB | 97.4 MB | 97.2 MB | **97.2 MB** | 150 MB | 52.8 MB | pass |
| Page browsing `Home -> Search -> Library -> Playlist` (immediate, UIA, no DevTools) | 108.7 MB | 108.5 MB | 111.4 MB | **108.7 MB** | 150 MB | 41.3 MB | pass |
| Page browsing `Home -> Search -> Library -> Playlist` (30 s settle, UIA, no DevTools) | 104.5 MB | 104.6 MB | 108.0 MB | **104.6 MB** | 150 MB | 45.4 MB | pass |

Compared to the Milestone 0 spike (72.2 MB idle), Milestone 1 idle uses +20.9 MB (+10.2 MB renderer for bundled fonts, Svelte 5 views, Tailwind CSS, and SVG/Phosphor assets; +5.5 MB GPU process for backdrop gradients and composited surfaces; +4.0 MB `lumen.exe` for the catalog, player state, and bundled icon resources; +1.0 MB browser process). Idle headroom is now **6.9 MB**, so Milestones 2 and 3 (libmpv, SQLite, HTTP client, artwork) must keep idle Home allocations tight.

### Idle, home screen

3 fresh launches, no DevTools port, read 30 s after launch. Private working set in MB.

| Process | Role | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| `msedgewebview2.exe` | browser | 25.9 | 26.1 | 26.2 | 26.1 |
| `msedgewebview2.exe` | gpu-process | 24.9 | 24.1 | 24.3 | 24.3 |
| `msedgewebview2.exe` | renderer | 23.1 | 25.5 | 22.8 | 23.1 |
| `lumen.exe` | app (catalog, state, player) | 8.2 | 8.2 | 8.3 | 8.2 |
| `msedgewebview2.exe` | utility, network service | 6.2 | 6.3 | 6.3 | 6.3 |
| `msedgewebview2.exe` | utility, storage service | 2.7 | 2.7 | 2.7 | 2.7 |
| `msedgewebview2.exe` | crashpad-handler | 2.0 | 2.0 | 2.0 | 2.0 |
| **Total (7 processes)** | | **93.1** | **95.0** | **92.7** | **93.1** |

### 5,000-row virtualized playlist (`--list`)

3 fresh launches with `lumen.exe --list`, no DevTools port, read 30 s after launch.

| Process | Role | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| `msedgewebview2.exe` | browser | 26.0 | 23.0 | 26.2 | 26.0 |
| `msedgewebview2.exe` | gpu-process | 24.8 | 23.1 | 24.9 | 24.8 |
| `msedgewebview2.exe` | renderer | 23.1 | 21.6 | 23.2 | 23.1 |
| `lumen.exe` | app (holds 5,000 tracks) | 8.3 | 7.3 | 8.2 | 8.2 |
| `msedgewebview2.exe` | utility, network service | 6.3 | 5.5 | 6.3 | 6.3 |
| `msedgewebview2.exe` | utility, storage service | 2.7 | 2.5 | 2.8 | 2.7 |
| `msedgewebview2.exe` | crashpad-handler | 2.0 | 1.6 | 2.0 | 2.0 |
| **Total (7 processes)** | | **93.2** | **84.6** | **93.5** | **93.2** |

### Scrolling the 5,000-row playlist (`--list`)

3 fresh launches with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9229` and `--list`. After 4 s, `[data-scroller]` was scrolled over CDP from top (`15` DOM rows, `#1` to `#15`) to middle (`scrollTop = 140000`, `29` DOM rows, `#2,487` to `#2,515`) to bottom (`scrollTop = 279692`, `20` DOM rows, `#4,981` to `#5,000`) with `0` console/runtime errors, then measured immediately (`WaitSeconds 0`) and after a 30 s settle (`WaitSeconds 30`).

Per-process breakdown (30 s settle after scrolling):

| Process | Role | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| `msedgewebview2.exe` | gpu-process | 27.2 | 26.8 | 27.1 | 27.1 |
| `msedgewebview2.exe` | renderer | 25.5 | 25.5 | 25.4 | 25.5 |
| `msedgewebview2.exe` | browser | 24.9 | 25.8 | 25.4 | 25.4 |
| `lumen.exe` | app | 8.3 | 8.3 | 8.4 | 8.3 |
| `msedgewebview2.exe` | utility, network service | 6.2 | 6.2 | 6.2 | 6.2 |
| `msedgewebview2.exe` | utility, storage service | 2.7 | 2.7 | 2.7 | 2.7 |
| `msedgewebview2.exe` | crashpad-handler | 2.0 | 2.0 | 2.0 | 2.0 |
| **Total (7 processes, 30 s settle)** | | **96.8** | **97.4** | **97.2** | **97.2** |
| **Total (7 processes, immediate)** | | **99.1** | **99.2** | **99.3** | **99.2** |

### Page browsing (`Home -> Search -> Library -> Playlist`) and #46 reading

3 fresh launches on Home with no DevTools port. After 5 s, sidebar links `Search`, `Library`, and `The long haul` (`long-haul`, 5,000 tracks) were invoked in sequence via Windows UI Automation `InvokePattern` (2 s per page), then measured immediately on arrival (`WaitSeconds 0`) and after a 30 s settle (`WaitSeconds 30`).

Per-process breakdown (30 s settle after browsing):

| Process | Role | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| `msedgewebview2.exe` | gpu-process | 29.4 | 29.1 | 29.8 | 29.4 |
| `msedgewebview2.exe` | renderer | 28.8 | 29.2 | 31.4 | 29.2 |
| `msedgewebview2.exe` | browser | 26.9 | 27.1 | 27.6 | 27.1 |
| `lumen.exe` | app | 8.4 | 8.3 | 8.4 | 8.4 |
| `msedgewebview2.exe` | utility, network service | 6.2 | 6.2 | 6.2 | 6.2 |
| `msedgewebview2.exe` | utility, storage service | 2.7 | 2.7 | 2.7 | 2.7 |
| `msedgewebview2.exe` | crashpad-handler | 2.0 | 2.0 | 2.0 | 2.0 |
| **Total (7 processes, 30 s settle)** | | **104.5** | **104.6** | **108.0** | **104.6** |
| **Total (7 processes, immediate)** | | **108.7** | **108.5** | **111.4** | **108.7** |

This explains the **116.9 and 120.0 MB** readings noted during #46 (PR #51) while switching pages in an open window: visiting all views warms renderer allocations (23.1 MB idle -> 32.3 to 34.8 MB immediately after navigation, 29.2 MB median settled) and GPU compositor surfaces (24.3 MB -> 29.4 MB median settled). The 100 MB limit applies to **idle on Home** (93.1 MB median, pass), and active browsing + 5,000-row playlist sits well below the 150 MB active limit.

### Milestone 1 method notes

- Clean shutdown between every run posted `WM_CLOSE` (`0x0010`) directly to the `Tauri Window` class HWND (avoiding `Process.CloseMainWindow()`, which can hit `Tao Thread Event Target` as found in spike #4) and waited until both `lumen.exe` and all 6 tracked `msedgewebview2.exe` child PIDs exited before starting the next launch.
- Window: `1200 x 800` client area (`1216 x 839` outer), visible, untouched during settle, 100 % display scaling. 6 unrelated `msedgewebview2.exe` processes were on the machine and excluded by `ParentProcessId` walking.
- To repeat (PowerShell, from the repo root):

```powershell
pnpm install --frozen-lockfile
pnpm tauri build --no-bundle
$app = Start-Process .\src-tauri\target\release\lumen.exe -PassThru   # add -ArgumentList --list for the 5,000-track playlist
pwsh -File scripts/measure-ram.ps1 -ProcessId $app.Id               # waits 30 s, then reports
```

### Milestone 1 machine, toolchain and build

| Item | Value |
|---|---|
| OS | Microsoft Windows 11 Home, build 26300 |
| CPU / RAM | Intel Core i7-8700, 15.9 GB |
| GPU / display | NVIDIA GeForce RTX 5060 (driver 32.0.16.2140), 1920 x 1080 at 100 % |
| WebView2 runtime | 158.0.4370.45 |
| Rust | 1.100.0 (MSVC) |
| Tauri | tauri 2.12.7, wry 0.57.4, tao 0.37.3, CLI 2.12.6 |
| Frontend | Svelte 5.59.2, Vite 8.3.8, Tailwind CSS 4.3.5, TypeScript 6.0.3 |
| Node / pnpm | 24.14.1 / 12.23.0 |
| Build | `pnpm tauri build --no-bundle`; release profile `codegen-units = 1`, `lto = true`, `opt-level = "s"`, `strip = true` |
| Executable size | 6.1 MB (6,362,624 bytes) |

---

## Milestone 0 (spike #3)

Measured 2026-11-26 on the release build of [`spikes/3-tauri-ram`](../spikes/3-tauri-ram) with
[`scripts/measure-ram.ps1`](../scripts/measure-ram.ps1). MB means 1,048,576 bytes.

### Decision

**Keep the budget: at most 100 MB idle, at most 150 MB while playing a large playlist.** It is not relaxed
to 150 / 200 MB.

| Scenario | Result | Budget | Headroom | Verdict |
|---|---|---|---|---|
| Idle, home screen (measured, 3-run median) | 72.2 MB | 100 MB | 27.8 MB | pass |
| 5,000-row virtualized list open (measured, 3-run median) | 74.2 MB | 150 MB | 75.8 MB | pass |
| Playing + 5,000-row list (**estimate**, list + 13.1 MB) | about 87.3 MB | 150 MB | about 62.7 MB | pass |

The spike is minimal. It has no artwork, SQLite, HTTP client, media keys or libmpv, so the headroom is what
those have to fit into. Idle is the tighter limit: watch it when the home screen gets thumbnails.

### Idle, home screen

Each run is a fresh launch, read about 30 s later (31.2 to 33.5 s). Private working set in MB.

| Process | Role | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| `msedgewebview2.exe` | browser | 22.3 | 25.1 | 25.4 | 25.1 |
| `msedgewebview2.exe` | gpu-process | 18.8 | 19.0 | 18.8 | 18.8 |
| `msedgewebview2.exe` | renderer | 15.7 | 12.9 | 12.9 | 12.9 |
| `msedgewebview2.exe` | utility, network service | 6.0 | 6.3 | 6.5 | 6.3 |
| `tauri-ram-spike.exe` | app (holds the 5,000 tracks) | 4.2 | 4.2 | 4.3 | 4.2 |
| `msedgewebview2.exe` | utility, storage service | 2.7 | 2.7 | 2.7 | 2.7 |
| `msedgewebview2.exe` | crashpad-handler | 2.0 | 2.0 | 2.0 | 2.0 |
| **Total (7 processes)** | | **71.7** | **72.2** | **72.7** | **72.2** |

### 5,000-row virtualized list (`--list`)

| Process | Role | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| `msedgewebview2.exe` | browser | 24.6 | 24.9 | 25.2 | 24.9 |
| `msedgewebview2.exe` | gpu-process | 19.2 | 19.6 | 19.2 | 19.2 |
| `msedgewebview2.exe` | renderer | 14.8 | 14.6 | 14.5 | 14.6 |
| `msedgewebview2.exe` | utility, network service | 6.2 | 6.5 | 6.2 | 6.2 |
| `tauri-ram-spike.exe` | app (holds the 5,000 tracks) | 4.5 | 4.2 | 4.2 | 4.2 |
| `msedgewebview2.exe` | utility, storage service | 2.7 | 2.7 | 2.7 | 2.7 |
| `msedgewebview2.exe` | crashpad-handler | 2.0 | 2.0 | 2.0 | 2.0 |
| **Total (7 processes)** | | **74.1** | **74.5** | **74.2** | **74.2** |

Row medians are taken per row, so they need not add up exactly to the median total.

An earlier list batch (73.6, 76.2, 74.5 MB; median 74.5) was thrown out and repeated because the window was
resized during run 2. It leads to the same decision.

### Playing estimate

The spike #2 numbers in this section come from `spikes/2-audio/FINDINGS.md` on PR #25 (head `d96c532`, not
yet merged to `main`).

Spike #2 measured its release binary plus `libmpv-2.dll` at 13.1 MB while playing (3-run median), up from
6.2 MB before playback.

74.2 MB (list) + 13.1 MB = **about 87.3 MB**. This was not measured in one app. It leans high, because the
13.1 MB includes a host process that Lumen already counts; adding only the 6.9 MB growth would give about
81 MB.

#### Open: resolve peaks

The estimate is steady-state playback. While yt-dlp resolves a stream it adds memory for a moment, and
sidecars count towards the budget.

Spike #2's peaks are whole-tree totals, so each one includes its own host process, `audio-spike.exe`, at
about 6.2 to 6.3 MB. The 87.3 MB estimate already contains that host (inside the 13.1 MB), so adding a whole
peak would count it twice. Only the sidecar memory is added:

- sidecars = spike #2 peak - 6.3 MB host
- estimated total = 87.3 MB + sidecars

| While resolving | Spike #2 peak (whole tree) | Sidecars only | Estimated total | Against 150 MB |
|---|---|---|---|---|
| `default,-tv_samsung` clients, median | 57.2 MB | `yt-dlp` 50.9 MB | about 138 MB | under |
| `default,-tv_samsung` clients, highest run | 65.8 MB | `yt-dlp` 59.5 MB | about 147 MB | just under |
| default clients with a Deno JS challenge, 1 to 2 s | up to 216.8 MB | `yt-dlp` + `deno` up to 210.5 MB | up to about 298 MB | far over |

Relaxing to 200 MB would not cover the Deno case either, so these peaks do not change the decision above.
The budget rule itself is unchanged. Milestone 2 has to measure a resolve inside the app, keep resolves
serialized, and either keep the total within budget or bring the question to the owner.

### Method

- `scripts/measure-ram.ps1` starts at the app's process id and follows `Win32_Process.ParentProcessId` to
  every descendant. It sums `WorkingSetPrivate` from `Win32_PerfFormattedData_PerfProc_Process`, matched by
  `IDProcess`. It does not use `Get-Process`.
- A listed child that is older than its parent is skipped, since Windows reuses process ids.
- Other apps' WebView2 processes are not counted: 6 unrelated `msedgewebview2.exe` processes were running on
  the machine after the runs.
- Window: 1200 x 800 client area (1216 x 839 outer), kept visible and on top, not touched during a run.
- The WebView2 profile already existed from earlier launches. The very first launch was not measured.
- Not covered: scrolling during the reading, a minimized window, display scaling other than 100 %, a second
  machine, and a manual Task Manager cross-check.

Before measuring, the release app was driven once over a local DevTools port (not open during the measured
runs): the list kept at most 25 rows in the DOM at the top, middle and end, reached track 5000, and logged
no console errors.

To repeat (PowerShell, from the repo root):

```powershell
cd spikes/3-tauri-ram
pnpm install
pnpm tauri build --no-bundle
$app = Start-Process .\src-tauri\target\release\tauri-ram-spike.exe -PassThru   # add -ArgumentList --list for the list
pwsh -File ..\..\scripts\measure-ram.ps1 -ProcessId $app.Id                     # waits 30 s, then reports
```

Close the app between runs. The script's own tests: `pwsh -File scripts/measure-ram.test.ps1`.

### Machine, toolchain and build

| Item | Value |
|---|---|
| OS | Windows 11 Home, build 26300 |
| CPU / RAM | Intel Core i7-8700, 15.9 GB |
| GPU / display | NVIDIA GeForce RTX 5060 (driver 32.0.16.2140), 1920 x 1080 at 100 % |
| WebView2 runtime | 158.0.4370.33 |
| Rust | 1.100.0 (MSVC) |
| Tauri | tauri 2.12.7, wry 0.57.4, tao 0.37.3, CLI 2.12.6 |
| Frontend | Svelte 5.59.1, Vite 8.3.8, Tailwind CSS 4.3.5, TypeScript 6.0.3 |
| Node / pnpm | 24.14.1 / 12.23.0 |
| Build | `pnpm tauri build --no-bundle`; release profile `codegen-units = 1`, `lto = true`, `opt-level = "s"`, `panic = "abort"`, `strip = true` |
| Executable size | 3.0 MB |
