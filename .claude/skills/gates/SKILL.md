---
name: gates
description: Run every quality gate from CLAUDE.md locally and summarize the results in the task-report format. Use before ticking a task, before merging, or when the human asks to "run the gates".
---

# Run the quality gates

CI (`.github/workflows/ci.yml` on Linux, `macos.yml`, `windows.yml`; mutants sharded by `mutants.yml`) is authoritative: a task merges only when CI is green on its branch tip. This is the local run on the Mac host (D17.6), with the same mutants setup (`.cargo/mutants.toml`, needs `cargo-nextest`) in one shard. Code behind a Linux-only `cfg` is checked only by `ci.yml`, Windows only by `windows.yml`.

1. From the checkout being checked (main checkout or your worktree), with `export PATH=$HOME/.cargo/bin:$PATH`:
   - Rust: `BASE=<base> scripts/gates.sh`. `<base>` is `main` for a task branch, or the commit before your work when working on `main`; pass `MUTANTS=0` only when the human asked for a quick run or on `main` itself (stage finish). The script sizes the build from the machine's cores, keeps a memory watchdog on the mutants, and on macOS checks coverage and mutants of the macOS code as `macos.yml` does. Never set a `TMPDIR` inside the repository.
   - Frontend: `bun install --frozen-lockfile && bun run lint && bun run typecheck && bun test --coverage`.
   - E2E, when the frontend or protocol changed: `E2E_PORT=$((1430 + RANDOM % 500)) bun run e2e` (Chromium and WebKit; browsers from `bunx playwright install webkit chromium`). It runs natively on the Mac (the specs press the host's keys, `e2e/host.ts`); Playwright's Linux container (`mcr.microsoft.com/playwright:v1.63.0-noble`, the command in the README's "Frontend" gates) reproduces CI's Linux run.
   - If `src-tauri` changed: `cargo clippy -p hive-app --all-targets -- -D warnings` (the native macOS build); Windows is verified by `windows.yml`.
2. On a FAIL, read the log `gates.sh` names, fix the cause, and re-run that gate. Re-run a timing failure once before calling it real; report it as flaky. Never weaken a gate (CLAUDE.md rule 6).
3. If memory ran low (the watchdog stopped a mutant, or the Mac swapped hard), stop and tell the human instead of retrying.

Report one line per gate: `ok`/`FAIL` + the reason, plus uncovered lines and any MISSED/TIMEOUT mutant.
