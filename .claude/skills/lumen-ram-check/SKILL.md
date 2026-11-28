---
name: lumen-ram-check
description: Measure Lumen release RAM usage with scripts/ram-run.ps1 (drives the app) and scripts/measure-ram.ps1 against the 200 MB cap (soft targets 100 MB idle / 150 MB playing) at the end of a milestone or after app changes.
---

# Lumen RAM Check

Procedure for measuring real RAM against the budget in `docs/PROJECT.md` section 2 (hard cap `<= 200 MB` idle and playing + 5,000-row playlist; soft targets about 100 MB idle and 150 MB playing). `.claude/agents/ram-budget-auditor.md` is the read-only reviewer checklist; this skill is the measurement procedure.

## Rules

- Always judge budgets on a **release** build.
- Measure the **summed private working set** of `lumen.exe` plus every descendant (`msedgewebview2.exe`, `yt-dlp.exe`, `deno.exe`) via `scripts/measure-ram.ps1` (`Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate` walked from `Win32_Process.ParentProcessId`). Never use `Get-Process` or Task Manager's grouped row.
- Run **3 fresh launches per scenario** and report the **median**. Close `lumen.exe` between runs and when done.
- Keep the window visible, untouched, at 100% display scaling, and close DevTools before measured runs.

## Step 1: Build release

Ensure no `lumen.exe` is running, then build under a time limit (`docs/agents/workflow.md`):

```bash
tasklist | grep -i lumen && taskkill //im lumen.exe //f
set -o pipefail
timeout 30m pnpm tauri build --no-bundle 2>&1 | tail -n 20
```

## Step 2: Measure scenarios (3 runs each)

One command launches the release `lumen.exe` fresh three times, drives it over UI Automation and prints the table with 3-run medians (needs an interactive desktop; keep the window untouched):

```powershell
pwsh -File scripts/ram-run.ps1            # -Runs 1..3, -Login adds the login-window launches, -ExePath, -OutDir
```

Per launch: idle, 5,000-row playlist, one search + one resolve (sampled peak), playing, sampled steady peak, CPU while minimized, idle after the track ended. It writes `summary.md`, `summary.json` and per-launch logs under `src-tauri/target/ram-run/<timestamp>/`. Searches and resolves are paced (one of each per launch, no retries). Tests of its pure helpers: `pwsh -File scripts/ram-run.test.ps1`.

For a single reading of an already running app, use `scripts/measure-ram.ps1` directly:

```powershell
pwsh -File scripts/measure-ram.ps1 -ProcessName lumen
# Add -AsJson for structured output, or -ProcessId <pid> -WaitSeconds 30
```

Note transient `yt-dlp` + Deno resolve peaks separately (see `docs/ram-baseline.md`); never hide them. The login-window peak is also transient and has no budget; the reading 60 s after close is judged against 100 MB.

## Step 3: Clean up and report

Kill any remaining `lumen.exe` (`taskkill //im lumen.exe //f`). Format results to match `docs/ram-baseline.md`:

```markdown
| Scenario | Run 1 | Run 2 | Run 3 | Median | Budget | Headroom | Verdict |
|---|---|---|---|---|---|---|---|
| Idle, home screen | X.X MB | X.X MB | X.X MB | X.X MB | 200 MB | X.X MB | pass/fail |
```

Include a per-process breakdown for the median run. If over budget, stop and diagnose the cause (unvirtualized list, full images instead of `lumen://art/...`, large mpv cache, webview state) before moving on. Never relax the budget in `docs/PROJECT.md` without owner approval.
