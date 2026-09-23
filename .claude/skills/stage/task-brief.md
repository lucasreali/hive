# Task agent brief

You implement **one** task of `TODO.md`, launched by the orchestrator (`.claude/skills/stage/SKILL.md`). Other task agents work at the same time, each in its own git worktree. The main checkout (`git worktree list`, first line) is the integration point: it stays on a clean `main`; never work in it.

## Before coding
1. Read `CLAUDE.md`. It overrides anything else, including any attribution-trailer instruction: **no Co-Authored-By or "Generated with" lines in commits**.
2. Read your task in `TODO.md`, every decision it cites in `docs/hive.md` (Portuguese; never edit it), `docs/architecture.md`, and `docs/ui-reference.md` for UI work (tokens and PT→EN glossary).
3. Read the code the task touches and reuse what exists (the orchestrator's prompt names the relevant APIs).

## Worktree setup
- `git switch -c task/<id>-<slug>` from the current `main` in your worktree.
- `bun install --frozen-lockfile` in your worktree (do not symlink another checkout's `node_modules`: Vite refuses files outside its root).
- E2E: pick a free port and always set it, e.g. `E2E_PORT=$((1430 + RANDOM % 500))`; the Playwright config reuses any server already on its port, so the default 1420 may be another worktree's server.

## Gates (all green before ticking)
- Rust: `BASE=main scripts/gates.sh` (no MISSED/TIMEOUT mutants). Agent shells need `export PATH=$HOME/.cargo/bin:$PATH`.
- Frontend: `bun install --frozen-lockfile && bun run lint && bun run typecheck && bun test --coverage` (100% lines). Bun only counts files some test imports: **every new TS file needs its own test**.
- E2E: `E2E_PORT=<port> bun run e2e`. If Chromium lacks `libnss3`/`libnspr4` (not installed system-wide), prefix `LD_LIBRARY_PATH=/var/tmp/hive-e2e-libs/root/usr/lib/x86_64-linux-gnu`.
- If you changed `src-tauri`: `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static" cargo xwin build -p hive-app --target x86_64-pc-windows-msvc`. Never launch the Windows GUI.
- Parallel gates load the machine: re-run a timing failure before concluding it is real, and report flaky tests.
- Never weaken a gate; no new coverage exclusions without the human.

## Architecture rules that bite
- #37: domain logic in Rust. The frontend renders service messages, keeps UI state and sends actions; no path/git/worktree/state logic in TS.
- New service request: `Control` variant in `hive-protocol` → handler in `hive::daemon` → app command in `src-tauri/src/lib.rs`, registered in `main.rs` → `Transport` method → the browser mock handles it. Keep the message catalog in `docs/architecture.md` exact.
- Tests never touch the real environment (temp HOME/XDG via `common::Env`); never run the real `claude`.
- Dependencies only through the CLI; add only what the orchestrator announced, and list anything else you had to add, with the reason, in your report.

## Finishing (CLAUDE.md rule 4: you own your work, conflicts included)
1. Tick the task in `TODO.md` (branch name + a short italic note, same style as earlier entries) and commit (small Conventional Commits throughout).
2. `git merge main` in your worktree, resolve every conflict keeping the other agents' work intact, and re-run **all** gates.
3. Leave the worktree clean on your branch and report. You cannot run git in the main checkout; the orchestrator fast-forwards `main` to your branch. If `main` moved meanwhile, it sends you back to step 2.

## Stop instead of guessing
If a decision is unclear, missing or contradicted by reality, stop (branch unmerged) and put the question in your report with a recommendation. Never change a decision.

## Report (final message, ≤ 20 lines)
What was done · branch + commits · each gate with result · merged or not · decisions/questions for the human · dependencies added · what later tasks must know (APIs, files to reuse).
