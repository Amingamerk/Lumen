# Agent workflow

How agents set up, run and tear down ticket work in Lumen. CLAUDE.md links here; the agent files in `.claude/agents/` follow it.

## Approvals

- Feature-branch commits, pushes and ticket PRs into `dev` need no approval. Report the diff in the PR and to the coordinator.
- Every merge (ticket PR into `dev`, promotion PR into `main`) and every release needs the owner's explicit OK for that PR. The `pr-merger` agent runs only after that OK has been relayed to it, naming the PR and its head SHA.
- Nothing is ever pushed to `main` directly.

## Worktree setup

Worktrees created by tools often start from `main`. Before any change:

```bash
git fetch origin && git checkout -B <type>/<issue#>-short-name origin/dev
```

Read-only reviews check out the PR head, not the base, so tests run against the code under review:

```bash
git fetch origin pull/<N>/head && git checkout --detach FETCH_HEAD
git rev-parse --short HEAD   # must equal the head SHA named in the review
```

## Long commands: time limits and trimmed output

Nothing may run without a limit or flood the context. Always enable `set -o pipefail` when piping to `tail` so a failed or timed-out check still returns a non-zero exit status (Git Bash on Windows has GNU `timeout`):

```bash
set -o pipefail
timeout 20m cargo fmt --check 2>&1 | tail -n 20
timeout 20m cargo clippy --all-targets -- -D warnings 2>&1 | tail -n 40
timeout 20m cargo test -q 2>&1 | tail -n 40
timeout 10m pnpm run lint 2>&1 | tail -n 30
timeout 10m pnpm run check 2>&1 | tail -n 30
timeout 10m pnpm run test 2>&1 | tail -n 40
timeout 15m pnpm run build 2>&1 | tail -n 20
```

- A first build in a fresh worktree compiles the whole Tauri dependency tree and takes several minutes. That is not a hang. Run at most one cold Rust build at a time across worktrees.
- Before building, make sure no `lumen.exe` from this worktree is still running (Windows locks the exe and linking fails or stalls): `tasklist | grep -i lumen`, then `taskkill //im lumen.exe //f` if needed. Close the app again after every real-app check.
- `Blocking waiting for file lock` means another cargo process uses the same directory. Wait for it or ask the coordinator; do not delete lock files.
- A command counts as hung only if it printed nothing for 10+ minutes of real time. Check with `date` before deciding, then stop it and report the command, how long it ran and its last output lines.
- While iterating, run the targeted test (`cargo test -q <name>`, `pnpm exec vitest run <file>`). Run the full set once before reporting done.

## Saving context

Every turn re-reads the whole conversation, so wasted reads are paid again and again.

- Search first (`rg`, Grep, Glob), then read only the relevant ranges. Read `docs/PROJECT.md` by section, not whole.
- Review from `git diff --stat` and `gh pr diff <N>` before opening whole files.
- Never print lockfiles, `dist/`, generated schemas, full CI logs or `target/` output.
- Screenshots: crop or downscale (about 1000 px wide) before reading, and read at most two per session. Prefer DOM or row-count assertions.
- Briefs to other agents point to files and sections instead of pasting them, and ask for a short report.
- If a session's context grows large before the work is done, stop at a clean point: commit WIP on the ticket branch, write next steps in the PR description or a note for the coordinator, and continue in a fresh session.

## Teardown after a merge

The squash-merged ticket branch is deleted on GitHub automatically. Remove the local worktree and branch:

```bash
cmd //c "rmdir /s /q <worktree>\\node_modules"
cmd //c "rmdir /s /q <worktree>\\src-tauri\\target"
cmd //c "rmdir /s /q <worktree>\\dist"
git worktree remove --force <worktree>
git branch -D <type>/<issue#>-short-name
git worktree prune
git fetch origin dev:dev
```

`rmdir` first, because `git worktree remove` is slow on (or fails on) deep `node_modules` and `target` trees on Windows, and some hooks hold plain `rm -rf`.
