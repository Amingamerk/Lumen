---
name: lumen-verify
description: Run Lumen verification checks (cargo fmt, clippy, cargo test, pnpm lint/check/test) before reporting a step done or opening a PR.
---

# Lumen Verify

Run required backend and frontend checks cleanly without flooding context or stalling on file locks. Full workflow rules live in `docs/agents/workflow.md`.

## Before running

1. Make sure no `lumen.exe` is running (Windows locks the binary and linking fails or stalls):
   ```bash
   tasklist | grep -i lumen
   taskkill //im lumen.exe //f
   ```
2. Run at most one cold Rust build at a time across worktrees.
3. If `Blocking waiting for file lock` appears, another `cargo` process is active; wait or ask the coordinator. Never delete lock files.

## While iterating

Run only the targeted test for the slice you are editing:
```bash
set -o pipefail; timeout 10m cargo test -q <test_name> 2>&1 | tail -n 40
set -o pipefail; timeout 5m pnpm exec vitest run <test_file> 2>&1 | tail -n 40
```

## Full check suite (run once before reporting done or opening a PR)

Always use `set -o pipefail` so a failing or timed-out command piped to `tail` still exits non-zero (Git Bash on Windows provides GNU `timeout`):

```bash
set -o pipefail
timeout 20m cargo fmt --check 2>&1 | tail -n 20
timeout 20m cargo clippy --all-targets -- -D warnings 2>&1 | tail -n 40
timeout 20m cargo test -q 2>&1 | tail -n 40
timeout 10m pnpm run lint 2>&1 | tail -n 30
timeout 10m pnpm run check 2>&1 | tail -n 30
timeout 10m pnpm run test 2>&1 | tail -n 40
```

Run `node --test .claude/hooks/block-ai-credit.test.js` if hooks, skills, or git-attribution rules changed. Run `set -o pipefail; timeout 15m pnpm run build 2>&1 | tail -n 20` when verifying a production frontend bundle.

## Hang rule

A cold Rust build in a fresh worktree takes several minutes and is not a hang. Count a command as hung only after 10+ minutes of real time with no output (check with `date`). If hung, stop it and report the command, elapsed time, and last output lines.

## Report format

Report the exact commands run and one pass/fail line per check (including any failures), plus any RAM numbers if app code changed:

```text
Checks:
- cargo fmt --check: pass
- cargo clippy --all-targets -- -D warnings: pass
- cargo test -q: pass (N passed)
- pnpm run lint: pass
- pnpm run check: pass
- pnpm run test: pass (N passed)
```
