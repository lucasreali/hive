# Hive — Development TODO

**Source of truth:** `docs/hive.md` (Portuguese). Every task cites the decisions (`#N`, `D#`) it implements; read them before starting the task.
**Visual reference:** `docs/prototype/` (read `docs/prototype/README.md` first).
**Rules:** `CLAUDE.md`.

## How to use this file

1. Work **top to bottom**, one task at a time. Do not start a task whose dependencies are not checked.
2. Before a task: read the cited decisions in `docs/hive.md`. After a task: all quality gates green (CLAUDE.md), then tick the box and add the branch name next to it.
3. **⏸ Checkpoint** = stop and wait for the human. Do not continue past a checkpoint on your own.
4. If a decision is unclear, missing, or contradicted by reality: stop and ask. Never change a decision yourself.

---

## Stage 0 — Foundation (Rust, WSL side, no UI)

- [x] **0.1 Workspace layout.** `task/0.1-workspace` — *human decisions (2026-09-23, applied in `task/0.1-gates`): `src-tauri/src/main.rs` excluded from coverage and mutants (COVERAGE_EXCLUSIONS.md); the 6 unmaintained-crate advisories pulled in only by Tauri are ignored by ID in `deny.toml`; MPL-2.0 stays allowed.* Cargo workspace at the repo root with members: the existing Tauri crate (`src-tauri`), `crates/hive-protocol` (lib) and `crates/hive` (bin, produces the `hive` binary). Workspace lints: `clippy::unwrap_used` and `clippy::expect_used` denied outside tests. Crates created with `cargo new`, dependencies only with `cargo add -p`. *(#9, #26, #36)*
- [x] **0.2 Protocol crate `hive-protocol`.** `task/0.2-protocol` — *control-frame priority lives in the service writer (0.4); fuzz: `cargo +nightly fuzz run --target x86_64-unknown-linux-gnu decode` from `crates/hive-protocol`.* Frames `[type: u8][channel: u32][length: u32][payload]`, big-endian, explicit max frame size. Control frames carry JSON (serde, versioned enum); terminal frames carry raw bytes. Codec on `tokio-util` + `bytes`. Version handshake (protocol + binary version); mismatch is a hard, explicit error. Control frames have priority over terminal frames. Decoder never panics: typed errors for malformed/oversized frames. Tests: round-trip, partial frames, oversized, unknown type, zero length. `cargo fuzz` target for the decoder. *(#24, #29)*
- [x] **0.3 Internal event model + adapters.** `task/0.3-events` Provider-independent events; adapter trait (raw provider payload in, internal events out, raw payload preserved); Claude Code adapter for hook payloads. No state machine yet (Stage 2). *(#6, #17)*
- [x] **0.4 `hive daemon`.** `task/0.4-daemon` — *fallback runtime dir: `/tmp/hive-<uid>` (checked: owned by the user, mode 0700); killing PTY process groups lands with the PTYs in 0.5.* Unix socket and lockfile with mode `0600` in `$XDG_RUNTIME_DIR` (documented fallback); single instance. Accepts the app connection (through the bridge) and CLI connections. Lifetime tied to the app connection: when it drops, kill every PTY **process group** (graceful signal, then forced after a short grace) and exit. *(#14, #18)*
- [x] **0.5 PTY management.** `task/0.5-pty` — *fish job control gives each job its own process group, so the service ends every process group in the PTY's session (SIGHUP, 2 s grace, SIGKILL); `setsid`/`nohup` escapes survive (risk 9).* Spawn terminals running `fish -C 'set -gx PATH <hive-bin-dir> $PATH'` (never `fish_add_path` without flags) with `HIVE_TERMINAL_ID` in the environment. Input, output, resize, close. One protocol channel per terminal. Pass-through only: no server-side scrollback. No limit on the number of terminals. *(#18, #19, #25, #28)*
- [x] **0.6 `hive bridge`.** `task/0.6-bridge` — *transparent byte relay (the handshake result reaches the app unchanged); a service that fails to start is reported after 5 s with its stderr in `<runtime>/daemon.log`.* stdio ↔ socket. If the socket does not exist, start the daemon detached (`setsid`), guarded by the lockfile. Forward the handshake result. *(#14)*
- [x] **0.7 `hive hook <event>`.** `task/0.7-hook` — *input limit 512 KiB (larger input is replaced by an error marker so the frame stays under 4 MiB); nothing is ever printed on stdout.* Read the hook JSON from stdin with a size limit, attach `HIVE_TERMINAL_ID`, send to the daemon with a ~200 ms internal timeout, **always exit 0**. `--record <file>` appends raw payload + timestamp to a JSONL file. *(#10, #27)*
- [x] **0.8 `hive worktree create | list | remove`.** `task/0.8-worktree` (subagent; merged into `task/0.13-architecture`) — *human decisions (2026-09-23, applied in `task/0.8-followups`): `remove` keeps the `worktree-<name>` branch; hook mode reuses an existing Hive worktree (`claude -w <existing>`); names outside the rule are refused, not normalised; a failed `.worktreeinclude` copy rolls the new worktree back.* Claude convention: `.claude/worktrees/<name>/`, branch `worktree-<name>`. Base branch local **or remote**. Name validation `^[a-z0-9][a-z0-9._-]*$` + reject existing names (identical to the UI dialog). Copy files listed in `.worktreeinclude`. List via `git worktree list --porcelain`. Git always through the `git` executable with separate arguments. Hook mode for `WorktreeCreate` (stdout = **only** the worktree path; any failure = non-zero exit) and `WorktreeRemove`, tested with synthetic payloads but **not registered** in the hooks settings yet (Stage 4). Warn if the project has its own `WorktreeCreate` hook. *(#7, #8, #13, #15, #33, hook details)*
- [x] **0.9 `claude` wrapper.** `task/0.9-claude-wrapper` (subagent; merged into `task/0.11-bench`) — *human-approved (2026-09-23): hook timeout 1 s; a claude started inside a hooked claude (`HIVE_WRAPPED=1`) runs without Hive's settings; empty `PATH` entries are skipped.* The daemon installs a POSIX `sh` script at `<data-dir>/hive/bin/claude`: finds the real `claude` by searching `PATH` without the hive bin dir, recursion guard `HIVE_WRAPPED=1`, then `exec` the real claude with `--settings <hive-hooks.json>`. The daemon also generates `hive-hooks.json` pointing every observation event at `hive hook <event>` by absolute path, with a short hook `timeout`. *(#21, #25, #26, #27)*
- [x] **0.10 Unhooked-claude detection.** `task/0.10-unhooked` — *checks every 1 s for a process named `claude` in the terminal's session; warns (`unhooked_agent` on the terminal channel) after 5 s without `SessionStart`, once per run. An npm-installed claude shows up as `node` and is not detected.* If a `claude` process appears in a terminal's process tree without a `SessionStart` event from that `HIVE_TERMINAL_ID` within a few seconds, emit a warning event. *(#25)*
- [x] **0.11 Benchmarks.** `task/0.11-bench` — *hook latency (300 runs, release): p50 3.8 ms, p99 8.3 ms, max 11.3 ms (`cargo bench -p hive --bench hook_latency`, fails above 20 ms); codec: ~14 GiB/s for 64 KiB terminal frames, ~4.7 M control frames/s (`cargo bench -p hive-protocol`).* Hook latency (process start → event received by the daemon) p99 < 20 ms; codec throughput baseline with `criterion`. *(D3)*
- [ ] ~~**0.12 Hooks spike.**~~ **Moved to 1.12** (human decision, 2026-09-23): passive recording while developing Stage 1, plus one short provoked session. Tooling ready in `task/0.12-spike` (`scripts/spike/`). Original scope, kept for reference: Using `hive hook --record`, run real Claude Code sessions in a scratch repo under `/tmp` (never in this repo, never touching `~/.claude/settings.json`; use `--strict-mcp-config` and the cheapest model) and answer, with evidence:
  1. Does a `WorktreeCreate` fired by a subagent carry `agent_id` / `agent_type`?
  2. Does Claude Code keep writing to the terminal (spinner) during a long tool call? Measure output gaps.
  3. Does the "subagent + WorktreeCreate hook → isolation error" bug reproduce?
  4. Does Claude Code refuse to edit a file changed externally since its last read?
  5. Confirm `Stop` does not fire on Esc; what does?
  6. What happens to a subagent's worktree and branch when it finishes with changes?

  Write `docs/spike/stage-0.md`: answers, evidence, catalog of hook events and fields, and **proposed changes to `docs/hive.md`** (as table rows, in Portuguese) — do not apply them. *(Pontos em aberto #7)*
- [x] **0.13 `docs/architecture.md`.** `task/0.13-architecture` — *to be updated with the spike findings (0.12).* Contracts, module map, message catalog, sequences (bridge start, handshake, terminal open/close, hook event, app disconnect), how to run and test.

**⏸ Checkpoint 0** — the human reviews Stage 0 and updates `docs/hive.md` (the spike report comes at 1.12, before checkpoint 1; "Pontos em aberto" #7 in `docs/hive.md` still says "spike da Etapa 0" — the human updates it).

---

## Stage 1 — Terminal + one agent (Tauri app)

- [x] **1.0 ⚠ Ask the human first: how the Windows app is built and run during development.** `task/1.0-windows-dev` — *human decision (2026-09-23): build in WSL with `cargo xwin` (msvc target, static CRT), Vite in WSL, run via `scripts/win-dev.sh` (see `docs/architecture.md`).* The code lives in WSL, but the app is a Windows app (WebView2) that talks to WSL through `wsl.exe hive bridge`. Do not guess; wait for the decision. *(Pendente para a Etapa 1)*
- [x] **1.1 App shell per the prototype.** `task/1.1-app-shell` — *`docs/prototype/README.md` is missing, so tokens and glossary are a proposal in `docs/ui-reference.md` (pending human approval). Regions in `src/shell/`, tokens in `src/styles.css`, fonts via `@fontsource`. Window has no native decorations; the title bar drags it and holds the window buttons. Sidebar resize, breadcrumb and "N active agents" deferred to the tasks that feed them. `bun run e2e` (Playwright, `e2e/`) needs `libnss3`/`libnspr4` on the system.* Title bar, sidebar, terminal area, right panel placeholder, status bar. Design tokens from `docs/prototype/README.md`. IBM Plex Sans/Mono **bundled locally** (no Google Fonts). Dark theme. All UI text in English using the glossary in `docs/prototype/README.md`. *(#23, #32, #34)*
- [x] **1.2 Frontend foundations.** `task/1.2-frontend-foundations` — *done before 1.1. Zustand store in `src/store.ts` (service data only through `apply(message)`, `useAgent(id)` selector); React Compiler via Babel (`@rolldown/plugin-babel`); Biome lint; `bun test` collects only `src/` (Playwright goes in `e2e/`); template `greet` removed. TanStack Virtual not added yet: add it with the first long list. Bun coverage only counts files some test imports.* React + React Compiler. Agent state in an external store with per-agent subscriptions (`useSyncExternalStore` or Zustand selectors), fed only by service events: no domain logic in TypeScript. TanStack Virtual for long lists. **No router library**: which view, panel or dialog is open is UI state in the store (#38). `bun test` with `happy-dom` and a 100% line coverage threshold in `bunfig.toml`. *(#30, #37, #38, D2)*
- [x] **1.3 Transport.** `task/1.3-transport` — *`src-tauri/src/lib.rs`: `wsl.exe [-d $HIVE_WSL_DISTRO] --exec /bin/sh -c 'exec "${1:-$HOME/.cargo/bin/hive}" bridge' sh [$HIVE_BRIDGE]` (no login shell, so no fish config; `win-dev.sh` points `HIVE_BRIDGE` at `target/debug/hive` through `WSLENV`). Control messages go to the UI on one `Channel`, terminal bytes on one `Channel` per terminal; Rust allocates terminal channels. `disconnected {reason}` (bridge stderr) added to the store. A reloaded UI re-attaches: old terminals closed, `welcome` replayed. TS `Transport` in `src/transport/` (Tauri + mock; mock outside Tauri or with `?mock`). App and `hive` must share one version number (handshake).* Rust side of the app spawns `wsl.exe hive bridge` and speaks the frame protocol. One Tauri `Channel` per terminal (no Tauri events for terminal bytes). A mock transport so the UI runs in a browser and can be tested with Playwright. *(#14, #24)*
- [x] **1.4 Connection status.** `task/1.4-connection-status` — *`welcome` gained an optional `distro` (the service's `WSL_DISTRO_NAME`; `PROTOCOL_VERSION` stays 1); the app side adds `app_version`/`app_protocol` to `version_mismatch`. `src/shell/ConnectionBlock.tsx` makes the workspace `inert` under a modal on mismatch or disconnect (title bar stays usable) with a "Reconnect" that calls `connect` again. The fix text includes `pkill -f 'hive daemon'` because a refused handshake leaves the old service running. Mock: `?mock=mismatch`, `?mock=disconnected`.* Status bar shows the WSL distribution and connection state; a version mismatch blocks with a clear message and how to fix it. *(#29)*
- [x] **1.5 Projects.** `task/1.5-projects` — *orchestrator defaults, pending human review at checkpoint 1: the service owns the list in `<data>/hive/projects.json` (atomic, 0600; a corrupt file is moved to `projects.json.corrupt` and the list starts empty); `add_project` takes a typed WSL path (no Windows picker), refuses with a typed `error` + `message`, normalises to the main worktree (a subfolder or linked worktree adds its repository) and treats a duplicate as a no-op that answers the same project; ids and names come from the service (Claude worktree → folder name, else branch). The app asks `list_projects` after every `welcome`; a "Refresh worktrees" button in the sidebar header (not in the prototype) asks again; no file watching until Stage 3. The add-project dialog is new (the prototype has none) and uses a native `<dialog>`. No remove-project (not in the prototype), no tree arrow keys yet (1.9), no TanStack Virtual yet. `PROTOCOL_VERSION` stays 1 (new messages only; binary versions must match anyway). Mock: `?mock=empty`.* Add project (a folder inside WSL), list its worktrees, empty state (screen 1e).
- [ ] **1.6 New worktree dialog** (screens 1c/1d). Same name validation as the CLI; base branch from local and remote branches with a filter; "Open a terminal in the new worktree". Calls `hive worktree create`. *(#33)*
- [ ] **1.7 Embedded terminals.** xterm.js managed **outside** React; tabs; WebGL renderer only on visible terminals; bounded, configurable scrollback; Ctrl+Shift+C / Ctrl+Shift+V copy and paste. *(#18, #28)*
- [ ] **1.8 Agent detection.** A `SessionStart` event from a terminal shows the agent under its worktree (position by the payload's `cwd`, link to the tab by `HIVE_TERMINAL_ID`). *(#19)*
- [ ] **1.9 Shortcuts.** Ctrl+Shift+T (worktree picker → new terminal), Ctrl+Shift+N (new worktree), Ctrl+Shift+B (files panel), Ctrl+Shift+O (add project), F8 (next pending). Every other key goes to the terminal untouched. *(#35)*
- [ ] **1.10 App lifetime.** Closing the app ends every terminal and agent; confirmation dialog if any agent is working, waiting for permission or waiting for you. *(#18)*
- [ ] **1.11 Load test.** 20 terminals replaying recorded Claude Code output at the same time; the focused terminal must stay fluid. If it fails, stop and report: the planned fallback (headless terminal emulator in the service) is a human decision. *(#28)*

- [ ] **1.12 Hooks spike report (moved from 0.12).** Throughout Stage 1 the human runs Claude Code in this repo through `scripts/spike/dogfood.sh` (records in `target/spike/`), and once runs the provoked session in `scripts/spike/README.md` (Part 2, `/tmp/hive-spike`). Answer the six questions of 0.12 with evidence and write `docs/spike/stage-0.md` (answers, evidence, catalog of hook events and fields, proposed changes to `docs/hive.md` as table rows in Portuguese — do not apply them). Ask the human when the recordings are ready; never run the real `claude` yourself. *(Pontos em aberto #7)*

**⏸ Checkpoint 1**

---

## Stage 2 — Agent states

- [ ] **2.1 State machine in the Rust service** (the UI only renders the resulting state) from the mapping table in `docs/hive.md` ("Mapeamento de estados"), including `Notification` by `notification_type`, "the most urgent state wins", and the PTY-silence reconciliation for interrupts.
- [ ] **2.2 Sidebar tree** Project → Worktree → Agent → Subagents, with the state icon (color + shape, from `StateIcon.dc.html`).
- [ ] **2.3 Propagation and pending counter.** Collapsed nodes show the most urgent state inside; "N pending" counter; F8 jumps to the next pending agent.
- [ ] **2.4 Notifications.** Sound only when entering waiting-for-permission, waiting-for-you or error; OS notification when an agent finishes.

**⏸ Checkpoint 2**

---

## Stage 3 — See what the agent did

- [ ] **3.1 File watching** in the service (inotify) → real-time file tree.
- [ ] **3.2 Git diff** per worktree; right panel (screen 1g), toggled by Ctrl+Shift+B.
- [ ] **3.3 Viewer and diff** with CodeMirror 6 and `@codemirror/merge` (diff read-only). *(#31)*
- [ ] **3.4 Selection → terminal.** Selecting code writes only the reference into the active terminal, e.g. `@src/checkout/validators.ts (lines 44–46)`.
- [ ] **3.5 Editing (3b).** Save through the service (temp file + rename) with a version check; auto-reload when the buffer is clean; conflict banner when the buffer is dirty (reload / keep mine / view diff); "agent working here" badge; "open in external editor". No LSP. *(#31)*

**⏸ Checkpoint 3**

---

## Stage 4 — Subagents and worktrees

- [ ] **4.1** Register `WorktreeCreate` / `WorktreeRemove` in the generated hooks settings. *(#15)*
- [ ] **4.2** Link subagents to their parent (using what the spike found); a subagent's own worktree appears nested under it, not at project level. *(#22)*
- [ ] **4.3** Worktrees removed by `WorktreeRemove` disappear from the sidebar.

**⏸ Checkpoint 4 — v1 complete.**

---

## Not now (Fase 2)

Kanban screen, rich interactions (permissions and choices answered in the app), live edit view, session history, GitHub CLI integration. Do not build any of it during v1.

## Open items for the human

- Export PNG screenshots of prototype screens 1a–1g into `docs/prototype/stage-1/` before Stage 1 (Ponto em aberto 13).
