---
name: gates
description: Run every quality gate from CLAUDE.md safely (narrow builds, one mutant at a time) and summarize the results in the task-report format. Use before ticking a task, before merging, or when the human asks to "run the gates".
---

# Run the quality gates

CI (`.github/workflows/ci.yml`, `macos.yml`) is authoritative; this is the local run. Heavy gates have taken the whole WSL VM down before, with the human's terminals: never widen the build or run gates in parallel with another checkout's gates.

1. From the checkout being checked (main checkout or your worktree), with `export PATH=$HOME/.cargo/bin:$PATH`:
   - Rust: `BASE=<base> scripts/gates.sh`. `<base>` is `main` for a task branch, or the commit before your work when working on `main`; pass `MUTANTS=0` only when the human asked for a quick run or on `main` itself (stage finish). Never raise `CARGO_BUILD_JOBS` above 3, never set a `TMPDIR` inside the repository.
   - Frontend: `bun install --frozen-lockfile && bun run lint && bun run typecheck && bun test --coverage`.
   - E2E, when the frontend or protocol changed: `E2E_PORT=$((1430 + RANDOM % 500)) bun run e2e` (prefix `LD_LIBRARY_PATH=/var/tmp/hive-e2e-libs/root/usr/lib/x86_64-linux-gnu` if Chromium lacks `libnss3`).
   - If `src-tauri` changed: the `cargo xwin` build from `.claude/skills/stage/task-brief.md`.
2. On a FAIL, read the log `gates.sh` names, fix the cause, and re-run that gate. Re-run a timing failure once before calling it real; report it as flaky. Never weaken a gate (CLAUDE.md rule 6).
3. If the VM restarted or memory ran low, stop and tell the human instead of retrying.

Report one line per gate: `ok`/`FAIL` + the reason, plus uncovered lines and any MISSED/TIMEOUT mutant.
