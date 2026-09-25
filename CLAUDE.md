# Hive — Agent Rules

Hive is a desktop companion for Claude Code agents: a Tauri app on Windows with embedded terminals, and a Rust service inside WSL that owns the PTYs, manages git worktrees and receives Claude Code hook events. Hive only **observes** agents; it never starts, controls or talks to them.

**Where logic lives (#37):** all domain logic is Rust — protocol, PTYs, worktrees, git, hooks, agent states, file watching, saving files. The frontend (React + TypeScript) only presents: it renders what the service sends, keeps UI state (selection, open panels, dialogs, tabs) and sends user actions back. Bun is the frontend's package manager, script runner and test runner only; the shipped app has no Bun or Node runtime.

## Where things are

| File | Role |
|---|---|
| `docs/hive.md` | Single source of truth for every decision (Portuguese). **Never edit it.** |
| `TODO.md` | The work plan. Do the next unchecked task only; stop at every ⏸ checkpoint |
| `docs/prototype/` | The human's prototype: look, layout, states, glossary. Read-only. Shortcuts follow `docs/hive.md` #35, not the prototype |
| `COVERAGE_EXCLUSIONS.md` | Approved coverage exclusions (created when the first one is approved) |
| `.claude/skills/stage/` | How to develop a whole stage: the orchestrator procedure (`SKILL.md`, `/stage <N>`) and the brief every task agent follows (`task-brief.md`) |
| `.claude/skills/gates/`, `release/` | `/gates` runs every gate safely; `/release <version>` tags a release (the human pushes) |
| `.claude/agents/` | `gate-integrity-reviewer` (run before merging a task) and `security-reviewer` (trust boundaries in the service/app) |
| `.claude/settings.json` + `hooks/` | Hooks: `guard.py` blocks edits to read-only files, npm/npx/pnpm/yarn, push, rebase, force and global git config; `format.sh` runs `cargo fmt` / biome after each edit |

## Hard rules

1. **Decisions are the human's.** Never change a decision from `docs/hive.md`. If one is unclear, missing, or proven wrong, stop and ask.
2. **English everywhere**: code, identifiers, comments, commits, repo docs, CLI output, UI text.
3. **Packages only through the CLI.** Rust: `cargo add -p <crate> <dep>` / `cargo remove`. Frontend: **bun only** (`bun add`, `bun add -d`, `bun remove`, `bunx`, `bun create`); never npm, pnpm, yarn or npx. Never hand-edit dependency sections or lockfiles: a new package goes in with `bun add`, never by writing it into `package.json`. After merging or pulling a change to `package.json` / `bun.lock`, run `bun install --frozen-lockfile` so `node_modules` matches (a stale one breaks the running dev server). Before adding any dependency, tell the human what it is and why.
4. **Git:** work directly on `main` with small commits. Create a branch (`task/<id>-<slug>`, e.g. `task/0.2-protocol`, from `main`) only for a very complex task that may conflict with other agents working in parallel (e.g. a `/stage` run with worktrees). Never push, never force, never rewrite history.
   - **When a branched task is finished** (every gate green, task ticked in `TODO.md`, report written), the branch goes into `main` and is deleted (`git branch -d`). Everything must end up on `main`, with no task branch left behind.
   - **Each agent owns its work, conflicts included.** Agents may work in parallel, each in its own git worktree; never touch another agent's worktree or branch. Before finishing, merge `main` into your task branch (`git merge main`), resolve every conflict yourself while keeping the other work intact, and run every gate again. Never rebase, never force.
   - **Integration:** an agent working alone in the main checkout merges its own branch into `main`. Agents in worktrees cannot run git in the main checkout, so the orchestrator (`.claude/skills/stage/`) integrates their finished branches with `git merge --ff-only <branch>`; if `main` moved in the meantime, the task goes back to its agent to merge `main` again.
   - At a ⏸ checkpoint, merge the finished task first, then stop.
   - Commit messages in English, following [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/): `type(scope): description` (`feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `build`, `ci`, `perf`, `style`), `!` / `BREAKING CHANGE:` footer for breaking changes.
   - **No attribution trailers**: never add `Co-Authored-By: Claude`, "Generated with Claude Code" or any similar line to commits or PRs.
   - Commits use the human's **personal** identity, set only in this repo's `.git/config` (`git config user.name` / `user.email`, no `--global`). Never change the global git config; other repos on this machine use a work identity.
5. **Never touch the real environment in tests or experiments:** temp dirs only, temporary `HOME`/`XDG_*`, never `~/.claude`, never shell config files.
6. **Never weaken a gate to pass it:** no `#[ignore]`, no loosened or deleted assertions, no `#[allow(...)]` to silence clippy, no `#[coverage(off)]`, no `#[mutants::skip]`, no tests that execute code without asserting on behaviour.

## Quality gates (every task, before ticking it)

Rust:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --locked
cargo deny check
cargo machete
cargo llvm-cov --workspace --fail-under-lines 100 --ignore-filename-regex 'src-tauri/src/main\.rs'   # exclusions: COVERAGE_EXCLUSIONS.md
cargo mutants --in-diff <(git diff main -- '*.rs')     # no "missed" mutants
```

Frontend (from Stage 1):

```
bun install --frozen-lockfile
bun run lint && bun run typecheck
bun test --coverage        # 100% line threshold set in bunfig.toml
```

Frontend tests use **`bun test`** with a DOM from `happy-dom`. Parts that need a real browser (xterm.js WebGL, CodeMirror layout, end-to-end flows) are tested with Playwright against the mock transport.

**Coverage exclusions:** only whole files that are pure wiring with no logic (e.g. `main.rs` calling a tested `run()`). Ask the human; once approved, record the file and the reason in `COVERAGE_EXCLUSIONS.md`. "Hard to test" is never a reason: inject the dependency (filesystem, process, socket, clock) behind a trait so the error paths can be tested.

## Code baseline

- No `unwrap()` / `expect()` outside tests. A panic in the service kills every terminal.
- External processes: separate arguments only, never a shell string built from input.
- Treat hook payloads, socket messages, file names and environment as untrusted; enforce size limits.
- Socket and lockfile mode `0600`. No secrets or full hook payloads in logs at default level.

## After each task

Report: what was done, branch and commits, every gate with its result, anything the human should decide. Then tick the task in `TODO.md`.

## Working notes (learned in Stage 0)

- **State:** Stage 0 is on `main` (merged on 2026-09-23). Every new task branch starts from `main`.
- **Toolchain:** agent shells need `export PATH=$HOME/.cargo/bin:$PATH` (the human's shell is fish). `cargo fuzz` needs `+nightly --target x86_64-unknown-linux-gnu`.
- **Gates:** `scripts/gates.sh` runs every Rust gate. Use `BASE=<previous task branch>` to limit mutants to your diff, and `MUTANTS=0` to skip them.
  - It builds with `CARGO_BUILD_JOBS=3` and runs one mutant at a time with a memory watchdog: wider builds or a runaway mutant have taken the whole WSL VM down, with the human's terminals. Write loops that a mutated helper cannot make endless (bound them by the input).
  - `/tmp` is a small tmpfs, so mutant trees go to `/var/tmp/hive-mutants`. Never put a `TMPDIR` inside this repository: git in the tests would find this repo by walking up.
- **Integration tests:** they live in one binary, `crates/hive/tests/integration/`. `common::Env` gives a temporary `HOME`/`XDG_*`; on drop it kills any process still carrying that environment.
  - Coverage of a spawned `hive` is recorded only when it exits normally: stop daemons with `common::stop` (SIGTERM) or by dropping the app connection, never SIGKILL.
- **Never run the real `claude`**, not even to test a launcher: `script -c` and `fish -C` load the human's shell config, which puts the real `claude` first on `PATH`. For a process named `claude`, copy `/usr/bin/dash` (coreutils here is multicall and refuses to run under another name).
- **Push denied (403, `Permission to lucasreali/hive.git denied to lucasreali-visusai`):** `gh` is the git credential helper and has two accounts; the active one is the work account. The human pushes with the personal account and switches back right after (other repos use the work one): `gh auth switch -u lucasreali; git push origin main <tag>; gh auth switch -u lucasreali-visusai`. Agents never push (the `guard.py` hook blocks it), so hand the human this command.
- **Background waits:** `pgrep -f <pattern>` matches the waiting shell's own command line; do not use it to wait for a process to finish.
