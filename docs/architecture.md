# Hive architecture

The WSL side is the `hive` binary and the `hive-protocol` crate (Stage 0); the Windows side is the Tauri app (`src-tauri`) and its React UI (`src`).
Decisions live in `docs/hive.md` (Portuguese). This file describes how the code implements them.

## Pieces

```
Windows                         WSL
┌────────────┐  wsl.exe   ┌──────────────┐  Unix socket  ┌─────────────────────────────┐
│ Tauri app  │◄──stdio───►│ hive bridge  │◄─────────────►│ hive daemon                  │
└────────────┘  (frames)  └──────────────┘               │  terminals (PTY + fish)      │
                                                          │  hook events → app           │
             Claude Code in a Hive terminal               │  unhooked-claude watcher     │
             └─ hooks ─► hive hook <event> ──────────────►└─────────────────────────────┘
```

| Command | Role |
|---|---|
| `hive daemon` | The service. It owns the PTYs, receives hook events and lives exactly as long as the app connection. |
| `hive bridge` | stdio ↔ socket relay run by the app through `wsl.exe`. It starts the daemon when needed. |
| `hive hook <event> [--record FILE]` | Called by Claude Code hooks. Forwards one event and always exits 0. |
| `hive worktree create/list/remove/hook-create/hook-remove` | Worktrees following Claude's convention. |

## Module map

| Crate / module | Responsibility |
|---|---|
| `hive-protocol` | Frame codec, `Control` messages, handshake constants, internal event model (`AgentEvent`, `EventKind`). Shared with the app. |
| `hive::cli` | clap subcommands. Runs each async command on a runtime that is dropped without waiting for a pending stdin read. |
| `hive::paths` | Runtime dir (`$XDG_RUNTIME_DIR/hive`, or `/tmp/hive-<uid>`, checked to be ours and mode 0700), socket, lockfile, `daemon.log`, data dir, bin dir, hooks settings. |
| `hive::daemon` | Lockfile, socket (0600), handshake, app/hook connections, prioritized writer, terminal registry, shutdown. |
| `hive::terminal` | Spawns `fish -C 'set -gx PATH <bin> $PATH'` on a PTY with `HIVE_TERMINAL_ID`. Handles input and resize, and ends process groups. |
| `hive::procs` | Minimal `/proc` reader (pid, ppid, pgrp, session, comm; skips zombies). |
| `hive::watch` | Pure state machine for the unhooked-`claude` warning. |
| `hive::adapter` | `Adapter` trait and `ClaudeCode` adapter: raw hook payload → `AgentEvent` (raw payload kept). |
| `hive::hook` | `hive hook`: reads stdin (512 KiB limit), optionally records JSONL, sends with a 200 ms timeout. |
| `hive::bridge` | Relay plus detached daemon start (`setsid --fork`, stderr to `daemon.log`). |
| `hive::wrapper` | Installs `<data>/hive/bin/claude` (sh wrapper) and `<data>/hive/hive-hooks.json` when the daemon starts. |
| `hive::projects` | The projects the app follows: validation, `<data>/hive/projects.json`, worktrees per project for the sidebar. |
| `hive::worktree` | Worktrees in `.claude/worktrees/<name>` on branch `worktree-<name>`; git through the executable with separate arguments; `.worktreeinclude` copy. |

## Wire protocol

Every message is a frame, big-endian: `[type: u8][channel: u32][length: u32][payload]`.

- **type** `0` = control: the payload is one JSON `Control` message, tagged by `"type"` in snake_case. **type** `1` = terminal: the payload is raw PTY bytes.
- **channel** `0` is the connection itself. Channels from `1` up are terminals, and the channel number is also the terminal's `HIVE_TERMINAL_ID`.
- **Size limit.** The maximum payload is 4 MiB (`MAX_PAYLOAD`) in both directions. The decoder never panics: it fails with `Oversized`, `UnknownType` or `Json` errors.
- **Priority.** The service's writer always drains queued control frames before terminal frames (`biased` select). Terminal output goes through a bounded queue of 256 frames, so a slow app slows the PTYs instead of growing memory.

### Handshake

The first frame from every client is `Hello { protocol, version, role }`, where `role` is `app` or `hook`.
- If both `protocol` (`PROTOCOL_VERSION`) and `version` (the `hive` binary version) match, the service answers `Welcome { version, distro }`. `distro` is the service's `WSL_DISTRO_NAME` (null when unset), shown in the app's status bar: the service knows its distribution, so Windows never has to guess it from `wsl.exe`. It was added as an optional field (`#[serde(default)]`), which old decoders ignore and new decoders default, so `PROTOCOL_VERSION` stayed 1.
- Otherwise it answers `VersionMismatch { protocol, version }` with its own values and closes the connection. This is a hard error: the app must block and tell the user.
- Any other first message gets `Error` and the connection is closed.

### Message catalog

| Message | Direction | Channel | Meaning |
|---|---|---|---|
| `hello` | client → service | 0 | Starts the handshake. |
| `welcome {version, distro}` | service → client | 0 | Handshake accepted. |
| `version_mismatch {protocol, version}` | service → client | 0 | Handshake refused (the service's own values); the connection is closed. |
| `open_terminal {cwd, cols, rows}` | app → service | n ≥ 1 | Start a terminal on channel n. |
| `terminal_opened` | service → app | n | The terminal is running. |
| terminal frame | both | n | Keystrokes (app → service) or output (service → app). |
| `resize {cols, rows}` | app → service | n | Resize the PTY. |
| `close_terminal` | app → service | n | End the terminal's processes. |
| `terminal_exited {code}` | service → app | n | The shell exited; `code` is null when it was killed by a signal. The channel is free again. |
| `hook {event, terminal_id, payload}` | `hive hook` → service | 0 | One raw hook call. The service closes the connection after it. |
| `agent {…AgentEvent}` | service → app | 0 | A translated hook event. |
| `unhooked_agent` | service → app | n | A `claude` runs in terminal n without sending hook events. |
| `list_projects` | app → service | 0 | Asks for every project; answered by `projects`. |
| `projects {projects}` | service → app | 0 | Every project with its worktrees, in the order they were added. |
| `add_project {path}` | app → service | 0 | Follow the git repository containing `path`. |
| `project_added {project}` | service → app | 0 | The project, with its worktrees. Also the answer when it was already followed. |
| `add_project_failed {path, error, message}` | service → app | 0 | `path` was refused. `error` is `not_absolute`, `not_found`, `not_a_directory`, `not_a_git_repository` or `storage`; `message` is shown as is. |
| `list_branches {project}` | app → service | 0 | The local and remote branches of a followed project; answered by `branches`. |
| `branches {project, local, remote, current, error}` | service → app | 0 | Short names from `git for-each-ref` (remote `HEAD` symrefs skipped, at most 1 MiB of names). `current` is the branch checked out in the main worktree, shown as "default"; `error` says why they could not be listed. |
| `validate_worktree_name {project, name}` | app → service | 0 | Sent as the user types a new worktree's name. |
| `worktree_name_validated {project, name, folder, branch, error}` | service → app | 0 | The CLI's verdict (`error`, or null) and where the worktree would go (`.claude/worktrees/<name>/`, `worktree-<name>`; `<name>` when empty). Answers may arrive out of order, so the app matches them by name. |
| `create_worktree {project, name, base}` | app → service | 0 | `hive worktree create` in a followed project, from `base` (null: the main worktree's HEAD). |
| `worktree_created {project, path, notes}` | service → app | 0 | The project with its updated worktrees, the new path, and what the CLI prints on stderr (a competing `WorktreeCreate` hook, the files copied from `.worktreeinclude`). |
| `create_worktree_failed {project, name, message}` | service → app | 0 | The CLI's error, shown as is. |
| `error {message}` | service → client | 0 or n | A refused request, e.g. channel 0, a channel already open, a bad cwd, an unexpected message, or a second app. |

Between the app's Rust side and the WebView (#24), control messages travel on one Tauri `Channel` (given by the `connect` command) as the service's JSON plus a `channel` field, e.g. `{"type":"terminal_opened","channel":1}`. Terminal output travels as raw bytes on a separate `Channel` per terminal (given by `open_terminal`). No Tauri events are used. The Rust side adds `app_version` and `app_protocol` (its own values) to `version_mismatch`, so the UI can show both sides, and one message of its own:

| Message | Direction | Meaning |
|---|---|---|
| `disconnected {reason}` | app (Rust) → UI | The bridge exited or its stdout closed. `reason` is the bridge's stderr (at most 16 KiB), a protocol error, or "the hive bridge exited". Every open terminal gets `terminal_exited {code: null}` first. Not sent after `version_mismatch`. |

A project is `{id, name, path, worktrees, error}`: `id` and `path` are the main worktree's path, `name` its folder name, and `error` (or null) says why `git worktree list` failed, e.g. for a moved folder. A worktree is `{id, name, path, branch, main, claude}`: `id` is its path, `branch` is null when detached, `main` marks the main worktree and `claude` one in `<repo>/.claude/worktrees/`. `name` is the folder name for a Claude worktree and the branch otherwise (the folder name when detached). The app never derives any of these.

`AgentEvent` has these fields: `provider`, `terminal_id`, `session_id`, `subagent {id, agent_type}`, `cwd`, `kind`, and `raw` (the unchanged payload).
`kind` is one of: `session_started`, `prompt_submitted`, `tool_started`/`tool_finished`/`tool_failed {tool}`, `permission_requested {tool}`, `notification {notification}`, `turn_finished`, `turn_failed {error}`, `subagent_started`, `subagent_stopped`, `session_ended {reason}`, or `other {event}`.
`notification` is one of `permission_prompt`, `elicitation_dialog`, `idle_prompt`, `agent_needs_input`, or `other`.

## Sequences

### Bridge start
1. The app runs `wsl.exe [-d $HIVE_WSL_DISTRO] --exec /bin/sh -c 'exec "${1:-$HOME/.cargo/bin/hive}" bridge' sh [$HIVE_BRIDGE]`: the `hive` installed by `cargo install` (#29), or the absolute Linux path in `HIVE_BRIDGE` (development, `scripts/win-dev.sh`). `--exec` skips the user's login shell, so no fish config runs. The script is a constant; the override is a separate argument. On Windows the process gets `CREATE_NO_WINDOW`, and it is killed when the app drops the connection.
2. The bridge connects to `<runtime>/hive.sock`.
3. If the connection fails, the bridge prepares the runtime dir, truncates `daemon.log` (0600) and runs `setsid --fork hive daemon`. The daemon's stdin and stdout are null and its stderr goes to the log.
4. The bridge retries the connection for up to 5 s. If the daemon never listens, the bridge fails with "the hive service did not start; see <log>".
5. Two bridges racing is harmless: the second daemon cannot take the lockfile and exits.
6. The bridge then copies bytes in both directions without looking at them. When either side closes, the bridge exits.

### App connect (app side)
1. The UI calls `connect(onMessage)` once at startup (`src/main.tsx`), handing a `Channel` to Rust.
2. Rust starts the bridge and queues `hello {role: app, version}`; `version` is the app's `CARGO_PKG_VERSION`, so `hive-app` and `hive` share one version number.
3. `welcome` or `version_mismatch` goes to the UI. After `version_mismatch`, Rust drops the bridge and sends nothing more.
   The UI (`src/shell/ConnectionBlock.tsx`) then blocks the workspace (`inert`, with a modal `alertdialog`; the title bar stays usable) and shows both versions and the fix: `cargo install --path crates/hive` and `pkill -f 'hive daemon'`, because a refused handshake leaves the old service running. `disconnected` blocks the same way and shows the reason. The dialog's "Reconnect" calls `connect` again, which starts a new bridge.
4. A UI that reloads calls `connect` again: if the connection is up, Rust closes that UI's old terminals, drops their later messages, and replays `welcome`; otherwise it starts a new bridge.
5. When the bridge's stdout closes, Rust waits up to 2 s for its stderr, sends `terminal_exited` for every open terminal and then `disconnected {reason}`. Commands then fail with "not connected to the hive service".

### Terminal from the UI
1. `openTerminal(cwd, cols, rows, onData)`: Rust picks the next channel (never reused while the app runs), registers `onData` and sends `open_terminal` on it. The call resolves with the channel id.
2. Output bytes for that channel go only to its `onData`. Control messages for channels the UI did not open are dropped.
3. `writeTerminal` sends terminal frames, split at `MAX_PAYLOAD`; `resizeTerminal` and `closeTerminal` send `resize` and `close_terminal`.
4. `terminal_exited` releases the channel's `onData`.

### Projects
1. After `welcome` (also a replayed one), the app's Rust side sends `list_projects`; the UI's "Refresh worktrees" button sends it again. Worktrees are not watched yet (Stage 3).
2. The service answers `projects`. Each project's worktrees come from `git worktree list --porcelain -z`, bare entries skipped.
3. `add_project {path}` (the add-project dialog): the path must be absolute, an existing directory and inside a git repository with a working tree. It is normalised to the main worktree (the first entry of `git worktree list`), so a subfolder or a linked worktree adds its repository. A new project is appended to `<data>/hive/projects.json`; if that write fails the list is unchanged and the answer is `add_project_failed {error: storage}`.
4. Project requests run on a blocking thread, off the app's frame loop, because git can be slow.
5. Worktree requests name a project by id and are refused ("<id> is not a followed project") for any other.

### New worktree (screens 1c/1d)
1. The dialog opens for a project (the row's "New worktree", or `openModal("new-worktree", id)`), sends `list_branches` and `validate_worktree_name` for the empty name.
2. Every keystroke in the name sends `validate_worktree_name`: the service runs `worktree::check_name` (the rule and the existing-folder check of `hive worktree create`, no git), so the dialog never has its own rule (#33, #37). Create stays disabled until the current name has a clean verdict.
3. The branch list is filtered in the UI (case-insensitive substring) and virtualized (TanStack Virtual); ↑/↓ in the filter move the pick. A pick hidden by the filter gives way to the first branch shown.
4. `create_worktree` runs `worktree::create`, the CLI's code. On `worktree_created` the UI selects the new worktree, opens a terminal in it if asked (`open_terminal` with its path), and closes the dialog; with notes, the dialog stays open to show them.

The list is a JSON array of paths, written through a temporary file (mode 0600) renamed over it. A missing file is an empty list. An unreadable or corrupt one is moved to `projects.json.corrupt` with a warning on stderr (`daemon.log`), and the service starts with an empty list.

### Daemon start
1. Prepare the runtime dir (0700, owned by the user).
2. Take the lockfile with an exclusive `flock`. If it is already held, exit with an error.
3. Install the wrapper and the hooks settings.
4. Remove any stale socket, bind it and set it to 0600.
5. Serve.

### Handshake
See [Handshake](#handshake). A refused client is not the app, so the daemon keeps waiting for one.

### Terminal open and close
1. `open_terminal` on channel n.
2. The service spawns `fish -C 'set -gx PATH <bin> $PATH'` on a new PTY. fish is the session leader, and the environment has `HIVE_TERMINAL_ID=n` and `TERM=xterm-256color`.
3. The service replies `terminal_opened`.
4. A pump task copies PTY output into terminal frames. There is no scrollback on the service side.
5. When the PTY closes, the pump reaps fish and sends `terminal_exited {code}`.
6. `close_terminal` ends the terminal's session (see below). The exit is reported by the pump.

### Hook event
1. Claude Code, started through the wrapper with `--settings <data>/hive/hive-hooks.json`, runs `<abs path>/hive hook <Event>` in exec form with a 1 s timeout.
2. `hive hook` reads stdin: at most 512 KiB, invalid JSON kept as a string.
3. If `--record` is given, it appends a JSONL line.
4. It connects and sends `hello` and `hook` within 200 ms. It prints nothing and exits 0.
5. The service translates the call with the `ClaudeCode` adapter. A `SessionStart` marks the terminal's `claude` as hooked.
6. The service forwards `agent` to the app. With no app connected, the event is dropped.

### App disconnect (or SIGTERM)
1. The accept loop stops and the watcher stops.
2. For every terminal, every process group in its PTY session gets SIGHUP.
3. The service waits until no process in those sessions is alive, for at most 2 s, then sends SIGKILL to what is left.
4. The socket is removed and the daemon exits 0.

fish job control puts each job in its own process group, which is why the service ends the groups of the whole session and not only fish's group. Processes that left the session with `setsid` or `nohup` survive (risk 9).

### Unhooked claude
1. Every 1 s the watcher lists `/proc`.
2. A terminal whose session contains a process named `claude`, with no `SessionStart` from that terminal for 5 s, gets one `unhooked_agent`.
3. The state resets when that `claude` exits.

### Worktree create (CLI and `WorktreeCreate` hook mode)
1. Find the main repository root, even from inside a linked worktree: the first entry of `git worktree list --porcelain -z`. A bare repository is refused.
2. Validate the name with `^[a-z0-9][a-z0-9._-]*$`, the same rule as the UI. Refuse an existing directory. An existing `worktree-<name>` branch is refused by git itself, and a `--base` starting with `-` is refused.
3. Collect the `.worktreeinclude` files before creating anything. `git ls-files --others --ignored --exclude-from=.worktreeinclude` finds the matches and `git check-ignore` keeps only the gitignored ones.
4. Run `git worktree add -b worktree-<name> .claude/worktrees/<name> <base>`. The base is a local or remote branch, or HEAD.
5. Copy the included regular files. Existing paths are never overwritten and symlinks are never followed.
6. Print only the path. In hook mode, any failure exits non-zero with an empty stdout.

`create` returns the path and its notes, which the CLI prints on stderr as `hive: <note>` and the service sends to the app: a warning when the project has its own `WorktreeCreate` hook in `.claude/settings{,.local}.json`, and how many `.worktreeinclude` files were copied. `remove` and `hook-remove` run `git worktree remove` without `--force` and keep the branch. `hook-remove` only removes paths directly under `<repo>/.claude/worktrees/`. Neither hook is registered in `hive-hooks.json` yet (Stage 4).

## Files written

| Path | Mode | Content |
|---|---|---|
| `<runtime>/hive.sock` | 0600 | Service socket. |
| `<runtime>/hive.lock` | 0600 | Single-instance lock. |
| `<runtime>/daemon.log` | 0600 | stderr of a daemon started by the bridge. |
| `<data>/hive/bin/claude` | 0755 | Wrapper: finds the real `claude` on `PATH` (skipping the bin dir and itself). If `HIVE_WRAPPED` is unset it exports it and adds `--settings`; otherwise it runs the real claude unchanged. |
| `<data>/hive/projects.json` | 0600 | JSON array of the followed projects' paths. |
| `<data>/hive/hive-hooks.json` | 0644 | One exec-form hook per observed event (12 events, no worktree events yet). |

## Run and test

```
export PATH=$HOME/.cargo/bin:$PATH
cargo build -p hive                         # target/debug/hive
cargo test --workspace                      # unit + integration (tests/integration)
cargo llvm-cov --workspace --fail-under-lines 100
cargo mutants --gitignore true --in-diff <(git diff main -- '*.rs')   # TMPDIR on disk: /tmp is a small tmpfs
cargo bench -p hive --bench hook_latency    # p99 target < 20 ms
cargo bench -p hive-protocol                # codec throughput (criterion)
cd crates/hive-protocol && cargo +nightly fuzz run --target x86_64-unknown-linux-gnu decode
```

Integration tests run the real `hive` binary with a temporary `HOME` and `XDG_*` (`tests/integration/common.rs`).
- fish runs with that temporary config.
- A stand-in `claude` is a renamed copy of `dash`, so the real Claude Code never runs.
- When a test ends, every process still carrying the test's `XDG_RUNTIME_DIR` is killed, so failing tests do not leak processes.
- `llvm-cov` records a spawned `hive` only if it exits normally, so tests stop daemons with SIGTERM or by closing the app connection.

### Frontend without Tauri

Outside Tauri (a plain browser, `bun run dev`, Playwright) or with `?mock` in the URL, `src/transport/mock.ts` stands in for the service: it answers `welcome` (distribution "Ubuntu") and `projects` (two of three fake repositories under `/home/user`; `?mock=empty` starts with none, and `add_project` accepts only the fake paths; branches, name checks and new worktrees follow the CLI's wording, with a long remote branch list in `shop`), or with `?mock=mismatch` / `?mock=disconnected` a `version_mismatch` / `disconnected` instead, and each terminal prints `mock$ `, echoes input, repeats the line on Enter and exits on `exit`. `bun run e2e` needs `libnss3` and `libnspr4`; without root, extract them with `apt-get download` + `dpkg -x` and point `LD_LIBRARY_PATH` at them.

### Windows app during development

The code and every build stay in WSL (TODO 1.0, human decision 2026-09-23). `scripts/win-dev.sh`:
1. cross-compiles `hive-app` for `x86_64-pc-windows-msvc` with `cargo xwin`, with a static CRT because a stock Windows has no VC++ redistributable;
2. copies the `.exe` to `%LOCALAPPDATA%\hive-dev`;
3. builds `target/debug/hive` and exports `HIVE_BRIDGE` (that path) and `HIVE_WSL_DISTRO` through `WSLENV`, so the app's bridge runs the development build;
4. starts Vite in WSL on port 1420. The debug build loads `devUrl`, which Windows reaches through WSL localhost forwarding, so hot reload works;
5. opens the app through PowerShell, because launching a Windows `.exe` straight from WSL interop fails.

One-time setup: `sudo apt install clang lld llvm`, `rustup target add x86_64-pc-windows-msvc`, `cargo install cargo-xwin --locked`, `bun install`. The first build downloads the MSVC CRT and Windows SDK into `~/.cache/cargo-xwin`. Release bundles (MSI/NSIS) are not covered yet.
