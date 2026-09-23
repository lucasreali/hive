# Hive architecture (Stage 0)

What exists after Stage 0: the WSL side, meaning the `hive` binary and the `hive-protocol` crate. There is no UI yet.
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
| `hive::worktree` | Worktrees in `.claude/worktrees/<name>` on branch `worktree-<name>`; git through the executable with separate arguments; `.worktreeinclude` copy. |

## Wire protocol

Every message is a frame, big-endian: `[type: u8][channel: u32][length: u32][payload]`.

- **type** `0` = control: the payload is one JSON `Control` message, tagged by `"type"` in snake_case. **type** `1` = terminal: the payload is raw PTY bytes.
- **channel** `0` is the connection itself. Channels from `1` up are terminals, and the channel number is also the terminal's `HIVE_TERMINAL_ID`.
- **Size limit.** The maximum payload is 4 MiB (`MAX_PAYLOAD`) in both directions. The decoder never panics: it fails with `Oversized`, `UnknownType` or `Json` errors.
- **Priority.** The service's writer always drains queued control frames before terminal frames (`biased` select). Terminal output goes through a bounded queue of 256 frames, so a slow app slows the PTYs instead of growing memory.

### Handshake

The first frame from every client is `Hello { protocol, version, role }`, where `role` is `app` or `hook`.
- If both `protocol` (`PROTOCOL_VERSION`) and `version` (the `hive` binary version) match, the service answers `Welcome { version }`.
- Otherwise it answers `VersionMismatch { protocol, version }` with its own values and closes the connection. This is a hard error: the app must block and tell the user.
- Any other first message gets `Error` and the connection is closed.

### Message catalog

| Message | Direction | Channel | Meaning |
|---|---|---|---|
| `hello` | client → service | 0 | Starts the handshake. |
| `welcome` | service → client | 0 | Handshake accepted. |
| `version_mismatch` | service → client | 0 | Handshake refused; the connection is closed. |
| `open_terminal {cwd, cols, rows}` | app → service | n ≥ 1 | Start a terminal on channel n. |
| `terminal_opened` | service → app | n | The terminal is running. |
| terminal frame | both | n | Keystrokes (app → service) or output (service → app). |
| `resize {cols, rows}` | app → service | n | Resize the PTY. |
| `close_terminal` | app → service | n | End the terminal's processes. |
| `terminal_exited {code}` | service → app | n | The shell exited; `code` is null when it was killed by a signal. The channel is free again. |
| `hook {event, terminal_id, payload}` | `hive hook` → service | 0 | One raw hook call. The service closes the connection after it. |
| `agent {…AgentEvent}` | service → app | 0 | A translated hook event. |
| `unhooked_agent` | service → app | n | A `claude` runs in terminal n without sending hook events. |
| `error {message}` | service → client | 0 or n | A refused request, e.g. channel 0, a channel already open, a bad cwd, an unexpected message, or a second app. |

`AgentEvent` has these fields: `provider`, `terminal_id`, `session_id`, `subagent {id, agent_type}`, `cwd`, `kind`, and `raw` (the unchanged payload).
`kind` is one of: `session_started`, `prompt_submitted`, `tool_started`/`tool_finished`/`tool_failed {tool}`, `permission_requested {tool}`, `notification {notification}`, `turn_finished`, `turn_failed {error}`, `subagent_started`, `subagent_stopped`, `session_ended {reason}`, or `other {event}`.
`notification` is one of `permission_prompt`, `elicitation_dialog`, `idle_prompt`, `agent_needs_input`, or `other`.

## Sequences

### Bridge start
1. The app runs `wsl.exe hive bridge`.
2. The bridge connects to `<runtime>/hive.sock`.
3. If the connection fails, the bridge prepares the runtime dir, truncates `daemon.log` (0600) and runs `setsid --fork hive daemon`. The daemon's stdin and stdout are null and its stderr goes to the log.
4. The bridge retries the connection for up to 5 s. If the daemon never listens, the bridge fails with "the hive service did not start; see <log>".
5. Two bridges racing is harmless: the second daemon cannot take the lockfile and exits.
6. The bridge then copies bytes in both directions without looking at them. When either side closes, the bridge exits.

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

A project that has its own `WorktreeCreate` hook in `.claude/settings{,.local}.json` gets a warning on stderr. `remove` and `hook-remove` run `git worktree remove` without `--force` and keep the branch. `hook-remove` only removes paths directly under `<repo>/.claude/worktrees/`. Neither hook is registered in `hive-hooks.json` yet (Stage 4).

## Files written

| Path | Mode | Content |
|---|---|---|
| `<runtime>/hive.sock` | 0600 | Service socket. |
| `<runtime>/hive.lock` | 0600 | Single-instance lock. |
| `<runtime>/daemon.log` | 0600 | stderr of a daemon started by the bridge. |
| `<data>/hive/bin/claude` | 0755 | Wrapper: finds the real `claude` on `PATH` (skipping the bin dir and itself). If `HIVE_WRAPPED` is unset it exports it and adds `--settings`; otherwise it runs the real claude unchanged. |
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

### Windows app during development

The code and every build stay in WSL (TODO 1.0, human decision 2026-09-23). `scripts/win-dev.sh`:
1. cross-compiles `hive-app` for `x86_64-pc-windows-msvc` with `cargo xwin`, with a static CRT because a stock Windows has no VC++ redistributable;
2. copies the `.exe` to `%LOCALAPPDATA%\hive-dev`;
3. starts Vite in WSL on port 1420. The debug build loads `devUrl`, which Windows reaches through WSL localhost forwarding, so hot reload works;
4. opens the app through PowerShell, because launching a Windows `.exe` straight from WSL interop fails.

One-time setup: `sudo apt install clang lld llvm`, `rustup target add x86_64-pc-windows-msvc`, `cargo install cargo-xwin --locked`, `bun install`. The first build downloads the MSVC CRT and Windows SDK into `~/.cache/cargo-xwin`. Release bundles (MSI/NSIS) are not covered yet.
