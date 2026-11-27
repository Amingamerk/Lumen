---
name: lumen-ram-check
description: Measure Lumen release RAM usage with scripts/measure-ram.ps1 against the 100 MB idle / 150 MB playing budget at the end of a milestone or after app changes.
---

# Lumen RAM Check

Procedure for measuring real RAM against the budget in `docs/PROJECT.md` section 2 (`<= 100 MB` idle, `<= 150 MB` playing + 5,000-row playlist). `.claude/agents/ram-budget-auditor.md` is the read-only reviewer checklist; this skill is the measurement procedure.

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

Launch `src-tauri/target/release/lumen.exe` fresh for each run, reach the target state, and run (PowerShell):

```powershell
pwsh -File scripts/measure-ram.ps1 -ProcessName lumen
# Add -AsJson for structured output, or -ProcessId <pid> -WaitSeconds 30
```

Scenarios to cover (as applicable to the milestone):
1. **Idle, home screen** (30 s after launch) -> budget `<= 100 MB`.
2. **5,000-row virtualized list open** (and after scrolling end-to-end) -> budget `<= 150 MB`.
3. **Playing + 5,000-row list open** (once libmpv playback exists) -> budget `<= 150 MB`.
4. **After closing a login window** (when login flows changed) -> confirm total returns to `<= 100 MB`.

Note transient `yt-dlp` + Deno resolve peaks separately (see `docs/ram-baseline.md`); never hide them.

## Step 3: Clean up and report

Kill any remaining `lumen.exe` (`taskkill //im lumen.exe //f`). Format results to match `docs/ram-baseline.md`:

```markdown
| Scenario | Run 1 | Run 2 | Run 3 | Median | Budget | Headroom | Verdict |
|---|---|---|---|---|---|---|---|
| Idle, home screen | X.X MB | X.X MB | X.X MB | X.X MB | 100 MB | X.X MB | pass/fail |
```

Include a per-process breakdown for the median run. If over budget, stop and diagnose the cause (unvirtualized list, full images instead of `lumen://art/...`, large mpv cache, webview state) before moving on. Never relax the budget in `docs/PROJECT.md` without owner approval.
