# RAM baseline (Milestone 0, spike #3)

Measured 2026-11-26 on the release build of [`spikes/3-tauri-ram`](../spikes/3-tauri-ram) with
[`scripts/measure-ram.ps1`](../scripts/measure-ram.ps1). MB means 1,048,576 bytes.

## Decision

**Keep the budget: at most 100 MB idle, at most 150 MB while playing a large playlist.** It is not relaxed
to 150 / 200 MB.

| Scenario | Result | Budget | Headroom | Verdict |
|---|---|---|---|---|
| Idle, home screen (measured, 3-run median) | 72.2 MB | 100 MB | 27.8 MB | pass |
| 5,000-row virtualized list open (measured, 3-run median) | 74.2 MB | 150 MB | 75.8 MB | pass |
| Playing + 5,000-row list (**estimate**, list + 13.1 MB) | about 87.3 MB | 150 MB | about 62.7 MB | pass |

The spike is minimal. It has no artwork, SQLite, HTTP client, media keys or libmpv, so the headroom is what
those have to fit into. Idle is the tighter limit: watch it when the home screen gets thumbnails.

## Idle, home screen

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

## 5,000-row virtualized list (`--list`)

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

## Playing estimate

The spike #2 numbers in this section come from `spikes/2-audio/FINDINGS.md` on PR #25 (head `d96c532`, not
yet merged to `main`).

Spike #2 measured its release binary plus `libmpv-2.dll` at 13.1 MB while playing (3-run median), up from
6.2 MB before playback.

74.2 MB (list) + 13.1 MB = **about 87.3 MB**. This was not measured in one app. It leans high, because the
13.1 MB includes a host process that Lumen already counts; adding only the 6.9 MB growth would give about
81 MB.

### Open: resolve peaks

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

## Method

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

## Machine, toolchain and build

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
