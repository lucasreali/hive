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
| `hive-protocol` | Frame codec, `Control` messages, handshake constants, internal event model (`AgentEvent`, `EventKind`), `AgentState`. Shared with the app. |
| `hive::cli` | clap subcommands. Runs each async command on a runtime that is dropped without waiting for a pending stdin read. |
| `hive::paths` | Runtime dir (`$XDG_RUNTIME_DIR/hive`, or `/tmp/hive-<uid>`, checked to be ours and mode 0700), socket, lockfile, `daemon.log`, data dir, bin dir, hooks settings. |
| `hive::daemon` | Lockfile, socket (0600), handshake, app/hook connections, prioritized writer, terminal and agent registries, shutdown. |
| `hive::terminal` | Spawns `fish -C 'set -gx PATH <bin> $PATH'` on a PTY with `HIVE_TERMINAL_ID`. Handles input and resize, and ends process groups. |
| `hive::procs` | Minimal `/proc` reader (pid, pgrp, session, comm; skips zombies). |
| `hive::watch` | Pure state machine for the unhooked-`claude` warning. |
| `hive::states` | Pure agent state machine: hook events → state per agent and subagent, "the most urgent wins", PTY-silence reconciliation (the clock is passed in). |
| `hive::adapter` | `Adapter` trait and `ClaudeCode` adapter: raw hook payload → `AgentEvent` (raw payload kept). |
| `hive::hook` | `hive hook`: reads stdin (512 KiB limit), optionally records JSONL, sends with a 200 ms timeout. |
| `hive::bridge` | Relay plus detached daemon start (`setsid --fork`, stderr to `daemon.log`). |
| `hive::wrapper` | Installs `<data>/hive/bin/claude` (sh wrapper) and `<data>/hive/hive-hooks.json` when the daemon starts. |
| `hive::projects` | The projects the app follows: validation, `<data>/hive/projects.json`, worktrees per project for the sidebar, placing an agent's `cwd` in a worktree. |
| `hive::files` | The files panel's worktree: `git ls-files` listing (sorted, capped), inotify watches on the listed directories and the git dir, debounce (the clock is passed in). |
| `hive::git` | Every git call: `git -C <dir>` with separate arguments and `GIT_OPTIONAL_LOCKS=0`, stdout size-limited, stderr in the error; `read_limited` for any size-limited read. |
| `hive::worktree` | Worktrees in `.claude/worktrees/<name>` on branch `worktree-<name>`; `.worktreeinclude` copy. |
| `hive::changes` | A worktree's changes against `HEAD` for the files panel: `git status` and `git diff --numstat` parsers, untracked line counts, the `changes` message. |
| `hive::file` | One file of a worktree for the viewer and diff: path checks, the text on disk and at `HEAD` (`git ls-tree`, `git cat-file`), the version token, the `file` message; saving it (temporary file + rename, version check) and its Windows path for an external editor. |

## Wire protocol

Every message is a frame, big-endian: `[type: u8][channel: u32][length: u32][payload]`.

- **type** `0` = control: the payload is one JSON `Control` message, tagged by `"type"` in snake_case. **type** `1` = terminal: the payload is raw PTY bytes.
- **channel** `0` is the connection itself. Channels from `1` up are terminals, and the channel number is also the terminal's `HIVE_TERMINAL_ID`.
- **Size limit.** The maximum payload is 4 MiB (`MAX_PAYLOAD`) in both directions. The decoder never panics: it fails with `Oversized`, `UnknownType` or `Json` errors. A message too big to encode never ends a connection: the app's Rust side refuses the command with the `Oversized` error (e.g. saving a huge paste), and the service's writer drops the frame with a warning in `daemon.log`.
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
| `agent_detected {id, project, worktree, cwd}` | service → app | n | An agent (`id` = its session id) started in terminal n. `project`/`worktree` are the ids of the followed worktree containing `cwd`, both null outside every followed project. |
| `agent_state {id, state, urgency, pending, subagents}` | service → app | n | The agent's displayed state, its urgency (0 ended … 6 waiting for permission; higher wins), whether it is pending (needs the user), and its live subagents `[{id, agent_type, state, worktree}]`, `worktree` being the id of the subagent's own worktree or null (see [Agent states](#agent-states)). Sent when it changes, after `agent_detected`, and for every live agent right after `welcome`. |
| `agent_removed {id}` | service → app | n | The agent's session ended, or terminal n exited (sent before `terminal_exited`). |
| `list_projects` | app → service | 0 | Asks for every project; answered by `projects`. |
| `projects {projects}` | service → app | 0 | Every project with its worktrees, in the order they were added. Also sent unasked after a `WorktreeCreate` or `WorktreeRemove` hook. |
| `add_project {path}` | app → service | 0 | Follow the git repository containing `path`. |
| `project_added {project}` | service → app | 0 | The project, with its worktrees. Also the answer when it was already followed. |
| `add_project_failed {path, error, message}` | service → app | 0 | `path` was refused. `error` is `empty_path`, `not_absolute`, `not_found`, `not_a_directory`, `not_a_git_repository` or `storage`; `message` is shown as is. |
| `list_branches {project}` | app → service | 0 | The local and remote branches of a followed project; answered by `branches`. |
| `branches {project, local, remote, current, error}` | service → app | 0 | Short names from `git for-each-ref` (remote `HEAD` symrefs skipped, at most 1 MiB of names). `current` is the branch checked out in the main worktree, shown as "default"; `error` says why they could not be listed. |
| `validate_worktree_name {project, name}` | app → service | 0 | Sent as the user types a new worktree's name. |
| `worktree_name_validated {project, name, folder, branch, error}` | service → app | 0 | The CLI's verdict (`error`, or null) and where the worktree would go (`.claude/worktrees/<name>/`, `worktree-<name>`; `<name>` when empty). Answers may arrive out of order, so the app matches them by name. |
| `create_worktree {project, name, base}` | app → service | 0 | `hive worktree create` in a followed project, from `base` (null: the main worktree's HEAD). |
| `worktree_created {project, path, notes}` | service → app | 0 | The project with its updated worktrees, the new path, and what the CLI prints on stderr (a competing `WorktreeCreate` hook, the files copied from `.worktreeinclude`). |
| `create_worktree_failed {project, name, message}` | service → app | 0 | The CLI's error, shown as is. |
| `watch_worktree {path}` | app → service | 0 | Watch this worktree of a followed project for the files panel, instead of any other (one at a time). Refused with `error` ("<path> is not a worktree of a followed project"). |
| `unwatch_worktree` | app → service | 0 | Stop watching (the files panel closed). |
| `files {path, files, truncated}` | service → app | 0 | Every file git lists in the watched worktree `path` (tracked, and untracked but not ignored), sorted `/`-separated relative paths. Sent right after `watch_worktree` and after every change that alters the list (debounced). `truncated` when the list hit the cap (50 000 files or 3 MiB of names). A listing that fails (e.g. the worktree was removed) is sent as `error`. |
| `list_changes {path}` | app → service | 0 | What changed in `path`, a worktree of a followed project; answered by `changes`. |
| `changes {path, files, added, removed, error}` | service → app | 0 | Every file that differs from `HEAD` (see [Changes](#changes)), sorted by path: `files` = `[{path, status, old_path, added, removed}]`, `status` one of `added`, `modified`, `deleted`, `renamed`, `untracked`; `old_path` is a rename's source; `added`/`removed` are line counts, null for a binary file. `added`/`removed` at the top are the totals of every file. `error` says why nothing could be listed (e.g. "<path> is not a worktree of a followed project"), or that the list was cut short. Also sent, unasked, after every refresh of the watched worktree (see [Files panel watch](#files-panel-watch-31-31)). |
| `open_file {worktree, path}` | app → service | 0 | One file of `worktree` (a worktree of a followed project); `path` is relative to it. Answered by `file`. |
| `file {worktree, path, content, base, version, binary, too_large, error}` | service → app | 0 | The file as asked (see [Viewer and diff](#viewer-and-diff-33-31)): `content` is its text on disk (null when gone), `base` its text at `HEAD` (a staged rename's old path; null when new or before the first commit), `version` an opaque token for the bytes on disk (null when they were not read), `binary` / `too_large` leave both texts null, `error` says why it could not be read (e.g. "<path> is not a worktree of a followed project", "a.txt does not exist", "not a relative path inside the worktree"). |
| `save_file {worktree, path, content, version}` | app → service | 0 | Write `content` over the file (see [Editing](#editing-35-31)), only if its bytes on disk still have `version`; `version` null means the file must not exist (it is created). Answered by `file_saved` or `save_failed`. |
| `file_saved {worktree, path, version}` | service → app | 0 | The file now holds the saved text; `version` is its new token. |
| `save_failed {worktree, path, error, message}` | service → app | 0 | Nothing was written. `error` is `conflict` (the bytes on disk are not `version`, or are over 1 MiB), `too_large` (`content` over 1 MiB), `invalid_path` (not a followed worktree, or the path rules of `open_file`) or `io`; `message` is shown as is. |
| `open_in_editor {worktree, path}` | app → service | 0 | Where Windows sees this file, to open it in an external editor. Answered by `editor_target`. |
| `editor_target {worktree, path, windows_path, error}` | service → app | 0 | The file's Windows path (`wslpath -w`), or why not (`error`, e.g. a file Windows would run). |
| `error {message}` | service → client | 0 or n | A refused request, e.g. channel 0, a channel already open, a bad cwd, an unexpected message, or a second app. |

Between the app's Rust side and the WebView (#24), control messages travel on one Tauri `Channel` (given by the `connect` command) as the service's JSON plus a `channel` field, e.g. `{"type":"terminal_opened","channel":1}`. Terminal output travels as raw bytes on a separate `Channel` per terminal (given by `open_terminal`). No Tauri events are used. The Rust side adds `app_version` and `app_protocol` (its own values) to `version_mismatch`, so the UI can show both sides, and one message of its own:

| Message | Direction | Meaning |
|---|---|---|
| `disconnected {reason}` | app (Rust) → UI | The bridge exited or its stdout closed. `reason` is the bridge's stderr (at most 16 KiB), a protocol error, or "the hive bridge exited". Every open terminal gets `terminal_exited {code: null}` first. Not sent after `version_mismatch`. |

A project is `{id, name, path, worktrees, error}`: `id` and `path` are the main worktree's path, `name` its folder name, and `error` (or null) says why `git worktree list` failed, e.g. for a moved folder. A worktree is `{id, name, path, branch, main, claude}`: `id` is its path, `branch` is null when detached, `main` marks the main worktree and `claude` one in `<repo>/.claude/worktrees/`. `name` is the folder name for a Claude worktree and the branch otherwise (the folder name when detached). The app never derives any of these.

`AgentEvent` has these fields: `provider`, `terminal_id`, `session_id`, `subagent {id, agent_type}`, `cwd`, `kind`, and `raw` (the unchanged payload).
`kind` is one of: `session_started`, `prompt_submitted`, `tool_started`/`tool_finished`/`tool_failed {tool}`, `permission_requested {tool}`, `notification {notification}`, `turn_finished`, `turn_failed {error}`, `subagent_started`, `subagent_stopped`, `session_ended {reason}`, `worktree_created {name, path}`, `worktree_removed {path}`, or `other {event}`.
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
1. The UI calls `connect(onMessage)` at startup (`connect` in `src/connect.ts`, called by `src/main.tsx`), handing a `Channel` to Rust. "Reconnect" goes through the same function, so every connection has the same handler.
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

### Terminals in the UI (xterm.js)
`src/terminals.ts` owns every xterm.js `Terminal`, outside React (#30); output goes straight from the transport's `onData` into `term.write`, never through React state.
1. `openTerminal(cwd)` creates the `Terminal` first, then calls the transport at 80×24, so output that arrives before the tab renders is kept. It adds a tab to the store (`tabs`, `activeTab`, `selection` = the worktree path); the tab's title is the worktree's name from `projects` (the project's name is added when two tabs share a name; an unknown path shows as is).
2. `TerminalArea` renders one host element (`mountTerminals`) and calls `showTerminal(activeTab)`. Only the shown terminal is opened in the DOM, fitted (`FitAddon`) and rendered with the WebGL addon; a hidden one loses its WebGL addon and keeps parsing output into its buffer. A lost WebGL context disposes the addon (xterm falls back to its DOM renderer); WebGL is tried again the next time the terminal is shown.
3. Host size changes are debounced (50 ms) into one fit of the shown terminal; a new size sends `resizeTerminal`.
4. Keys: `interceptKeys(handler)` sees every key event first and keeps the app shortcuts from the terminal (see Shortcuts); then Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes through the clipboard API (#35); everything else goes to xterm and, as `onData`, to `writeTerminal`. Input stops once the terminal exited.
5. `closeTerminal(id)` sends `close_terminal` unless the shell already exited, disposes the `Terminal` and removes the tab (the right neighbour, or the new last tab, is shown). An exited terminal keeps its tab, marked "exited", until closed; `unhooked_agent` adds a "no hooks" badge.
6. `scrollback` (default 5000 lines) in the store is applied to each new terminal; it is not persisted yet.

### Shortcuts (#35)
`src/shortcuts.ts` has one `keydown` listener on the window. A focused terminal gives each key to `interceptKeys` first: a shortcut is kept from xterm, which leaves it unhandled, so it bubbles up to the window listener and runs once; every other key (Ctrl+Shift+C/V included) is the terminal's.
1. Ctrl+Shift+T opens the worktree picker (every worktree of every project, filtered by worktree or project name; ↑/↓, Enter or a click opens a terminal in its path; Esc or a click outside closes). Ctrl+Shift+N opens the new worktree dialog for the selected project, the project of the selected worktree (a selected agent counts as its worktree, `selectedPlace` in `src/store.ts`; the tab bar's "+" uses it too), or the first project (add project when there is none). Ctrl+Shift+B toggles the files panel. Ctrl+Shift+O opens add project. F8 (no modifiers, or a click on the "N pending" counter) selects the pending agent after the selected one (else after the one whose terminal is shown) in tree order, wrapping; it expands the agent's project and worktree, shows its terminal when it has a tab and scrolls it into view. Agents outside every project come last. With nothing pending it does nothing.
2. Nothing runs under the connection block (`version_mismatch`, `disconnected`) or while a dialog is open; the key then goes on as usual.
3. The tree (sidebar): ↑/↓ move between rows, ←/→ collapse and expand a project or a worktree with agents, Enter selects (rows are buttons).

### Closing the app (#18)
1. The title bar Close, Alt+F4 and the taskbar all become one Tauri close request. `guardClose` (`src/window.ts`) listens to it with `onCloseRequested` and asks `confirmClose` (`src/shell/CloseAppDialog.tsx`).
2. When `agentsAtRisk` is empty the window is destroyed at once. Otherwise the request is cancelled and the "Close Hive?" dialog opens: Cancel/Esc keeps the app, "Close Hive" (focused, Enter) calls `closeWindow`, which destroys the window without asking again. Stage 1 counts every detected agent; Stage 2 narrows `agentsAtRisk` to working, waiting for permission and waiting for you.
3. The last window gone, Tauri emits `RunEvent::Exit`; `on_run_event` runs `Hive::shutdown`: the frame queue closes, so the bridge's stdin closes, the bridge exits and the daemon runs "App disconnect" below. Shutdown waits up to 2 s for the bridge to end, then kills it (no `wsl.exe` left behind). A crashed app closes the same pipe through the OS.
4. Outside Tauri (browser, mock transport) only the title bar Close requests a close, and a close that goes through sets `data-closed` on `<html>` for the browser checks.

### Notifications (hive.md item 5)
1. `src/connect.ts` passes every service message to `notify` (`src/notify.ts`) before `apply`, so the store still holds the agent's previous state. Only a change the service sent counts: an agent's first state is silent, and so is the snapshot after `welcome` (it always lands in an empty store: a fresh page, or after `disconnected` cleared it).
2. Entering waiting for permission, waiting for you or error plays a short tone synthesized with Web Audio (no audio file); changes within 500 ms share one tone.
3. Working or with subagents → waiting for you is "agent finished": `showNotification` (`src/window.ts`) sends an OS notification through `tauri-plugin-notification` ("Agent finished", "project · worktree: waiting for you"). The capability allows only `is_permission_granted`, `request_permission` and `notify`. Outside Tauri nothing is shown.

### Projects
1. After `welcome` (also a replayed one), the app's Rust side sends `list_projects`; the UI's "Refresh worktrees" button sends it again. The service also sends `projects` after each worktree hook (see [Worktree hooks](#worktree-hooks)); other worktree changes are not watched (only the files panel's worktree is, see [Files panel watch](#files-panel-watch-31-31)).
2. The service answers `projects`. Each project's worktrees come from `git worktree list --porcelain -z`, bare and prunable entries (the directory is gone) skipped.
3. `add_project {path}` (the add-project dialog): the path must be absolute, an existing directory and inside a git repository with a working tree. It is normalised to the main worktree (the first entry of `git worktree list`), so a subfolder or a linked worktree adds its repository. A new project is appended to `<data>/hive/projects.json`; if that write fails the list is unchanged and the answer is `add_project_failed {error: storage}`.
4. Project requests run on a blocking thread, off the app's frame loop, because git can be slow.
5. Worktree requests name a project by id and are refused ("<id> is not a followed project") for any other.

### New worktree (screens 1c/1d)
1. The dialog opens for a project (the row's "New worktree", or `openModal("new-worktree", id)`), sends `list_branches` and `validate_worktree_name` for the empty name.
2. Every keystroke in the name sends `validate_worktree_name`: the service runs `worktree::check_name` (the rule and the existing-folder check of `hive worktree create`, no git), so the dialog never has its own rule (#33, #37). Create stays disabled until the current name has a clean verdict.
3. The branch list is filtered in the UI (case-insensitive substring) and virtualized (TanStack Virtual); ↑/↓ in the filter move the pick. A pick hidden by the filter gives way to the first branch shown.
4. `create_worktree` runs `worktree::create`, the CLI's code. On `worktree_created` the UI selects the new worktree, opens a terminal tab in it if asked (`openTerminal` of `src/terminals.ts` with its path), and closes the dialog; with notes, the dialog stays open to show them.

The list is a JSON array of paths, written through a temporary file (mode 0600) renamed over it. A missing file is an empty list. An unreadable or corrupt one is moved to `projects.json.corrupt` with a warning on stderr (`daemon.log`), and the service starts with an empty list.

### Files panel watch (3.1, #31)
1. `followPanel` (`src/follow.ts`) keeps the service watching `panelWorktree`: while the files panel is open, the selected worktree (a selected project is its main worktree, which has the same id) or the selected agent's worktree. It sends `watch_worktree` when that changes, `unwatch_worktree` when the panel closes, and `watch_worktree` again after a new `welcome` (a new service has no watch). The store keeps the last `files` as `worktreeFiles` (`{path, files, truncated}`). The panel's "All" mode shows it when `path` matches, each file with its status from `changes` and the changed files it no longer lists (deleted ones) merged in (`allFiles` in `src/shell/RightPanel.tsx`, presentation only); a truncated list says so. "Changed", or "All" before the list arrives, shows only `changes`.
2. The service checks the path against the followed projects' worktrees, stops the previous watch task, and starts one (`hive::files::Watcher`, git on a blocking thread). The worktree's git dir comes from `git rev-parse --absolute-git-dir` (a linked worktree's is under the main repository's `.git/worktrees/<name>`).
3. Listing: `git ls-files --cached --others --exclude-standard -z` (at most 32 MiB read; names that are not UTF-8 and nested repositories `dir/` skipped; duplicates from conflicts merged; sorted; capped). Every git command Hive runs has `GIT_OPTIONAL_LOCKS=0`, so reading never rewrites the index the watcher sees.
4. Watches (inotify, `nix`): the directory of every listed file, every untracked directory (`git ls-files --others --directory`, so a new empty folder is watched), at most 8192; never inside ignored trees such as `node_modules` or `target`. In the git dir only `HEAD` and `index` count. Each re-list removes the watches of directories no longer listed before adding new ones (a renamed directory keeps its inotify watch, which must not be reused).
5. Any other event, an inotify queue overflow included, starts a burst; 200 ms after its last event (at most 1 s after its first) the worktree is listed again from scratch. `State::worktree_changed` (`hive::daemon`) is the single point that reports the change: it sends `files` when the list differs from the last one sent, then the worktree's `changes` every time (an edit changes the diff, not the list).
6. Known limit: in a chain of new empty untracked directories (`mkdir -p a/b/c`) only the top one is watched until a file appears; a file created deep inside it later shows up on the next re-list.

### Changes
The files panel (screen 1g, "Árvore de arquivos com diff do git").
1. The panel shows the selected worktree (a selected project is its main worktree), the selected agent's worktree, or else the shown terminal's (`panelWorktree` in `src/store.ts`). When it opens and whenever that worktree changes it sends `list_changes {path}`; the answer is stored by path in `changes`.
2. The service refuses a path that is not a worktree of a followed project, then runs, on a blocking thread: `git status --porcelain=v2 -z --untracked-files=all --find-renames` and `git diff --numstat -z --find-renames --no-ext-diff --no-textconv <base> --`. The base is `HEAD`, or the empty tree (`git hash-object -t tree /dev/null`) before the first commit, so staged, unstaged and untracked changes all count, as `git status` shows them.
3. Status per file against `HEAD`: `?` untracked; `2` (rename) renamed with its source; `1` added when the index says `A`, deleted when either side says `D`, else modified; `u` (unmerged) modified; ignored entries skipped. Line counts come from numstat (`-` = binary = null). An untracked file is counted here: a regular file of at most 8 MiB with no NUL in its first 8000 bytes (git's binary test) has as many lines as newlines, plus one for an unterminated last line; anything else is null.
4. Limits: each git command's output is read up to 16 MiB (more is an error, and git stops on the broken pipe); file names are kept as bytes to read the file and sent as lossy UTF-8. The files go into the message until their JSON reaches 3 MiB, well under `MAX_PAYLOAD`; the rest are left out with `error` "too many changes: N files not shown", while the totals still count every file.
5. The UI groups the paths into folders (presentation), folders first; a folder shows the strongest status inside (deleted > added > renamed > modified), and a collapsed one a dot of that color (`collapsed["folder:<worktree>/<path>"]`). Letters: M, A (added and untracked), D (struck through), R. The tree is virtualized and is one focusable element: ↑/↓ move, ←/→ collapse and expand, Enter opens a file or toggles a folder. Opening a file sets `openFile {worktree, path}`; `FileView` (in `src/shell/RightPanel.tsx`) shows its header under the tree and, below it, the file (see [Viewer and diff](#viewer-and-diff-33-31)). "All" / "Changed" is `changedOnly`; until the full file list exists (3.1) both show the changed files.

### Viewer and diff (3.3, #31)
1. `followOpenFile` (`src/follow.ts`) sends `open_file {worktree, path}` when `openFile` is set, again whenever a new `changes` for its worktree arrives (the file may have changed with it) and after a new `welcome`. The store keeps the last answer as `file`; `FileView` shows it only while it is the open file.
2. The service checks the worktree against the followed projects and the path on a blocking thread: not empty, at most 4096 bytes, only normal components (no `/` start, `.` or `..`); on disk it is resolved (`canonicalize`, symlinks followed) and must stay inside the worktree's resolved path and be a regular file (never a FIFO or device, which could block). A missing file has no `content`.
3. `base`: nothing before the first commit (`git rev-parse --verify --quiet HEAD`); else the blob of `git --literal-pathspecs ls-tree -l -z HEAD -- <path>` (a tree or submodule entry counts as none); a path absent at `HEAD` that is a staged rename (`git status --porcelain=v2 -z --untracked-files=no --find-renames`) takes its old path's blob. The blob is read with `git cat-file blob <id>`.
4. Limits: each side is read up to 1 MiB (`TEXT_LIMIT`; the blob size from `ls-tree` is checked first); over it, `too_large`. A side with a NUL in its first 8000 bytes (the rule of [Changes](#changes)) or that is not UTF-8 makes the file `binary`. Both leave both texts null. If the JSON of the answer would still exceed `MAX_PAYLOAD` (escaping can double text), it is sent as `too_large` without texts. Both sides missing is the error "<path> does not exist".
5. `version` = `<byte length>-<64-bit SipHash of the bytes, hex>` (`hive::file::version`, Rust's `DefaultHasher`, stable within one build of the service). A save recomputes it from the file on disk and compares for equality (see [Editing](#editing-35-31)).
6. UI (`src/viewer/`): `CodeView` creates one CodeMirror `EditorView` per open file (keyed by worktree and path), outside React state (#30), through `createViewer` (`editor.ts`), and calls `show({content, original})` with each answer: a new read-only state that keeps the scroll position. A file among the worktree's `changes` shows as `unifiedMergeView` (`@codemirror/merge`, no merge controls, unchanged stretches over 6 lines collapsed to a margin of 3) with `original` = `base` (a deleted file: empty content, all removed; a new one: empty original, all added); any other file shows its text under "No changes in this file.". `binary`, `too_large` and `error` show a message instead ("Binary file not shown.", "File too large to show.", the error). The language comes from the file name through `@codemirror/language-data`, loaded on demand; the look is One Dark with the prototype's surfaces and diff colors.

### Editing (3.5, #31)
1. Which views are editable: a file without changes opens (from the tree) as editable text; a changed file opens as its read-only diff (#31: the diff stays read-only) and the header's "Edit" switches it to editable text, "Diff" back (disabled while there are unsaved edits). No LSP.
2. The buffer (`src/viewer/buffer.ts`, pure; kept in the store as `edit`, so it survives the panel closing) holds the text being edited, the text on disk it is based on and its `version`. The CodeMirror view (`createEditor` in `editor.ts`) is created once per open file outside React state (#30); it splits lines on "\n" only, so a "\r" is text and saves unchanged. Undo, the standard keys and Tab indenting come from `@codemirror/commands` (Esc, then Tab, leaves the editor).
3. Save: Ctrl+S while the editor has the focus (a CodeMirror key, so the terminal never loses it and no app-wide shortcut is taken, #35) or the header's Save sends `save_file` with the buffer's base `version`; one save at a time. A save the app cannot send (not connected, over the frame limit) fails at once with the reason. `file_saved` makes the sent text the new base; the header's dot marks unsaved edits.
4. The service (blocking thread) checks the worktree and the path as `open_file` does. An existing file is written through its resolved path (a symlink inside the worktree stays a symlink); a new one only in a folder that resolves inside the worktree. `content` over 1 MiB is `too_large`. The text goes to `.<name>.hive-<pid>-<n>.tmp` in the same folder (created 0600, then given the file's permission bits, 0644 for a new file), is synced, the version on disk is checked, and the temporary file is renamed over the file; the folder is then synced. Any failure removes the temporary file. Known limit: a write between the version check and the rename is lost (agents take no lock, so none would help).
5. Every new `file` answer for the open file (see [Viewer and diff](#viewer-and-diff-33-31)) updates the buffer: text equal to the buffer's makes it clean at that version; text equal to its base changes nothing (e.g. a new service build with new version tokens); a clean buffer reloads (keeping scroll and selection); a dirty one keeps the edits and shows the conflict banner "Changed on disk." (or "Deleted on disk."): Reload takes the disk's text; Keep mine takes the disk's version as the base, so the next save overwrites it (a deleted file is created again); View diff shows the edits against the disk as a read-only unified diff. A `save_failed` conflict asks for the file again, which brings the banner.
6. Opening another file, or closing it, with unsaved edits asks "Discard your unsaved changes to <path>?" (`window.confirm`). Known limit: closing the app does not ask.
7. "Agent working here" in the header: an agent placed in the file's worktree, or a subagent in its own worktree there, is working, with subagents or waiting for permission (the states in which it may write files).
8. "Open in external editor": `open_in_editor`; the service resolves the file as `open_file` does and runs `wslpath -w <path>` (separate arguments). A file Windows would run instead of opening (`.bat`, `.cmd`, `.exe`, `.js`, `.lnk`, `.ps1`, `.vbs` and similar: a fixed list in `hive::file`) is refused. `openExternal` (`src/viewer/external.ts`) opens the answer's path with the Windows default app for the file through `@tauri-apps/plugin-opener` (`openPath`; the capability allows `opener:allow-open-path` for any path); outside Tauri nothing opens and the file view says so.

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
6. Agents (see [Agent detection](#agent-detection)) are updated.
7. The service forwards `agent` to the app (used by tests and the latency bench; the UI ignores it). With no app connected, the event is dropped.

### Agent detection
1. A `SessionStart` with a `session_id`, no `agent_id` (a subagent belongs to its agent, see [Agent states](#agent-states)) and a `HIVE_TERMINAL_ID` naming an open terminal registers the agent: session id → terminal.
2. The service places it by the payload's `cwd`, never the terminal's (#19): the followed worktree whose path contains `cwd`, by whole path components, the deepest one winning (Claude worktrees live inside the main one). Placement runs git (`projects.list()`) on a blocking thread, and is done once; a project followed later does not move an agent already detected.
3. It sends `agent_detected` on the terminal's channel. The agents lock is held while placing, so a `SessionEnd` or the terminal's exit arriving meanwhile is handled after the announcement (other terminals are not blocked). An agent outside every followed project is sent with a null `project` and `worktree`; the UI does not show it.
4. A `SessionEnd` for that session (after its `agent_state` `ended`), or the terminal's exit, removes it with `agent_removed`. A `/clear` is a `SessionEnd` plus a `SessionStart` with a new session id, in either order.
5. Channel n messages go through the app's Rust side like other terminal messages, so a reloaded UI never sees agents of terminals it did not open. On `disconnected` the UI drops every agent.
6. The sidebar shows each agent as a "Claude" row (idle icon) under its worktree; clicking it shows its terminal's tab (`activateTab`).

### Agent states
`hive::states`, from `docs/hive.md` "Mapeamento de estados". The UI only renders `agent_state`.
1. A detected agent starts idle. Each later hook event with its `session_id` sets the state of the agent, or of the subagent named by `agent_id`: `SessionStart` idle; `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure` (never error) and `SubagentStart` working; `PermissionRequest` and `Notification` `permission_prompt`/`elicitation_dialog` waiting for permission; `Stop` and `Notification` `idle_prompt`/`agent_needs_input` waiting for you; `StopFailure` error; `SessionEnd` ended. Anything else leaves the state as it is.
2. A subagent joins the list on its first event with a state and leaves it on `SubagentStop` (or a `SessionEnd` carrying its `agent_id`). At most 32 are kept per agent, and ids or types over 256 bytes are ignored, so a message always fits in a frame.
3. Rule 1: `state` is the most urgent of the agent's own state, its subagents' states and "with subagents" when any subagent is live. Urgency, highest first: `waiting_permission`, `error`, `waiting_you`, `with_subagents`, `working`, `idle`, `ended`.
4. Rule 2: every second (the unhooked-claude tick) the service checks each agent: when its terminal has printed nothing for 5 s (`states::SILENCE`), counted from the later of the last output and the last hook event, the agent and each subagent in working or waiting for permission go to waiting for you. Output alone never moves a state back; only hook events do.
5. Pending (the "N pending" counter and F8): waiting for permission, error and waiting for you. `urgency` is the state's rank in the order above; the app shows a collapsed project or worktree with the state of highest `urgency` inside and counts agents with `pending`, so it keeps no table of its own.
6. `agent_state` is sent on the agent's terminal channel only when the message changes.
7. A subagent's own worktree (#22), sent as its `worktree` (the worktree's id, i.e. its path). Linked by a `worktree_created` carrying the subagent's `agent_id` (the path the hook created), or else by the subagent's own events: the first time each new `cwd` of a subagent without a worktree is seen, the service places it like an agent (`projects::place`, git on a blocking thread) and links the worktree found when it is not the agent's own. Only when the agent itself was placed; paths over 4096 bytes are ignored. A `worktree_removed` of that path unlinks it; the subagent leaving drops it. The sidebar shows the worktree under the subagent and not at project level. To confirm by spike 1.12: whether `WorktreeCreate` carries `agent_id`, and that a subagent's `cwd` is its worktree.

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

`create` returns the path and its notes, which the CLI prints on stderr as `hive: <note>` and the service sends to the app: a warning when the project has its own `WorktreeCreate` hook in `.claude/settings{,.local}.json`, and how many `.worktreeinclude` files were copied. `remove` and `hook-remove` run `git worktree remove` without `--force` and keep the branch. `hook-remove` only removes paths directly under `<repo>/.claude/worktrees/`. Both hooks are registered in `hive-hooks.json` (see below).

### Worktree hooks
1. Claude Code, through the wrapper's settings, runs `<abs path>/hive worktree hook-create` for `WorktreeCreate` (60 s timeout: `git worktree add` plus the `.worktreeinclude` copy) and `hive worktree hook-remove` for `WorktreeRemove` (10 s timeout), in exec form. The hook input is at most 64 KiB.
2. Each does its work as above. Only on success, it then forwards the call to the service like `hive hook` (same `hello` + `hook`, 200 ms, `HIVE_TERMINAL_ID`), as event `WorktreeCreate` with the created path added as `worktree_path`, or `WorktreeRemove` unchanged. Forwarding never changes stdout nor the exit code.
3. The `ClaudeCode` adapter maps them to `worktree_created {name, path}` and `worktree_removed {path}`, keeping `session_id`, `cwd` and `agent_id`/`agent_type` (a subagent's worktree). They do not change agent states.
4. On either one the service lists the projects again off the frame loop and sends `projects`, so a `claude -w` or subagent worktree appears, and a removed one disappears, without "Refresh worktrees". It also forwards `agent` as for every hook.
5. The store replaces its list on every `projects`. A selected worktree that is no longer listed leaves its project selected (nothing, if the project went too), and `worktree:<id>` collapsed keys of unlisted worktrees are dropped. Its terminal tabs stay open. An agent placed in it keeps its `worktree` id, so, like an agent outside every project, it is no longer shown in the tree but still counts as pending and F8 still reaches its tab.

## Files written

| Path | Mode | Content |
|---|---|---|
| `<runtime>/hive.sock` | 0600 | Service socket. |
| `<runtime>/hive.lock` | 0600 | Single-instance lock. |
| `<runtime>/daemon.log` | 0600 | stderr of a daemon started by the bridge. |
| `<data>/hive/bin/claude` | 0755 | Wrapper: finds the real `claude` on `PATH` (skipping the bin dir and itself). If `HIVE_WRAPPED` is unset it exports it and adds `--settings`; otherwise it runs the real claude unchanged. |
| `<data>/hive/projects.json` | 0600 | JSON array of the followed projects' paths. |
| `<data>/hive/hive-hooks.json` | 0644 | One exec-form hook per observed event (12 events, `hive hook <Event>`, 1 s) plus `WorktreeCreate` → `hive worktree hook-create` (60 s) and `WorktreeRemove` → `hive worktree hook-remove` (10 s). |

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

Outside Tauri (a plain browser, `bun run dev`, Playwright) or with `?mock` in the URL, `src/transport/mock.ts` stands in for the service: it answers `welcome` (distribution "Ubuntu") and `projects` (two of three fake repositories under `/home/user`; `?mock=empty` starts with none, and `add_project` accepts only the fake paths; branches, name checks and new worktrees follow the CLI's wording, with a long remote branch list in `shop`; `list_changes` answers `MOCK_CHANGES`, sample changes after screen 1g, and `open_file` a sample text shaped by the file's status there, `MOCK_TEXTS` for `src/auth/session.ts`, a `.png` as binary), or with `?mock=mismatch` / `?mock=disconnected` a `version_mismatch` / `disconnected` instead, and each terminal prints `mock$ `, echoes input, repeats the line on Enter and exits on `exit`; `cd <dir>` moves it and `claude` sends `agent_detected` placed at the worktree whose path is exactly that directory, then `agent_state` idle; every later line sets that agent working (`agent_removed` when the terminal exits); `worktree-remove <name>` stands in for a `WorktreeRemove` hook, dropping that Claude worktree and sending `projects`. `?mock=states` adds, without terminals, agents in every state, two of them with subagents, one subagent owning a worktree (`tests-login` in `shop`, shown under it). `?mock=load` replays a recording into every terminal (see Load test below). `bun run e2e` needs `libnss3` and `libnspr4`; without root, extract them with `apt-get download` + `dpkg -x` and point `LD_LIBRARY_PATH` at them.

### Load test (1.11, #28)

`e2e/load.e2e.ts` (Playwright project `load`, run after the other specs, alone) opens 20 terminals with `?mock=load`: the mock replays a recording into each one through the real path (transport `onData` → terminal manager → xterm.js), at recorded timing, starting 0.5 s after the terminal opens plus 100 ms per terminal id. After a 3 s warm-up it types into the focused terminal for 12 s (10 keys/s) and measures:
- **input latency**: key `timeStamp` → the echo parsed (the mock appends `OSC 7777` to every echo in this mode) → the next animation frame;
- **frames**: every `requestAnimationFrame` interval; **long tasks**: `PerformanceObserver("longtask")`;
- **dropped output**: after the replays end, every hidden terminal's buffer must equal a fresh xterm of the same size fed the whole recording at once.

Pass: input p95 < 50 ms (typing starts to feel laggy past ~50 ms; native terminals sit at 10–40 ms) and p99 < 100 ms (RAIL response budget), frame p95 < 33.3 ms (30 fps), no long task ≥ 100 ms, nothing dropped. Results go to `target/e2e/load.json`.

**Recording.** `src/transport/replay.ts` generates a deterministic ~19 s Claude Code-like session: three turns of prompt, spinner with shimmer (one truecolor per character) + todo list + input box redrawn at 12.5 Hz the way Ink does it (DEC 2026 synchronized output, erase-line + cursor-up over the whole dynamic region), streamed answer redrawn every 33 ms, and an 80-line diff with truecolor backgrounds flushed in 4 KiB chunks. 449 KB, ~23 KB/s per terminal, ~460 KB/s for 20. To use a real capture instead, record one with asciinema (v2 or v3 cast, `asciinema rec --command claude target/claude.cast`), put it under the repo so Vite serves it, and run with `HIVE_LOAD_CAST=/target/claude.cast` (or open `/?mock=load&cast=/target/claude.cast`); only output events are replayed, and the end-of-recording check is skipped.

**GPU.** The spec launches Chromium with ANGLE on the system GL (`--use-gl=angle --use-angle=gl`), and inside WSL sets `GALLIUM_DRIVER=d3d12` with `/usr/lib/wsl/lib`, so WebGL runs on the Windows GPU. Chromium's default SwiftShader saturates the main thread with a single replaying terminal (~11 fps) and says nothing about a real GPU; Mesa's llvmpipe (no GPU) competes with the page for CPU and fails the thresholds.

**Result (2026-09-23, Chromium headless on Linux/WSL2, Intel Iris Xe through D3D12, machine otherwise idle, three runs):** input p50 28.5–30.3 ms, p95 36.4–38.0 ms, p99 38.1–43.9 ms, max 39–64 ms; frame p50 and p95 16.7 ms (60 fps), max 33–117 ms; no long task; nothing dropped. Inside the full `bun run e2e`: p95 38.7 ms, p99 62.5 ms, max 175 ms. **Pass.** While another agent was compiling Rust (load average 8–16 on 12 cores) the same test failed (input p95 70–1400 ms, with stalls of 0.4–2 s that had no long task, so outside the page), so timings are only meaningful on a quiet machine.

**Waste found (not fixed):** a hidden terminal loses its WebGL addon and xterm.js falls back to its DOM renderer. On every scroll xterm refreshes the selection, and `RenderService.handleSelectionChanged` makes the DOM renderer rebuild every row even though rendering is paused; in a `display: none` pane each glyph width measures 0, is never cached, and is measured again (`WidthCache._measure`, a forced layout). With 20 terminals this was ~35% of the main thread in a CPU profile. It still passes, but it is margin lost on slower machines; disabling that call for hidden terminals (xterm internals) removed it entirely in an experiment.

**This is not WebView2 on Windows.** To re-run in the real app: start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and point the spec at it with Playwright's `chromium.connectOverCDP("http://localhost:9222")` after navigating the window to `?mock=load` (the transport honours `?mock` inside Tauri); or, by hand, open `?mock=load` in the debug build's DevTools and run the same probes.

### Windows app during development

The code and every build stay in WSL (TODO 1.0, human decision 2026-09-23). `scripts/win-dev.sh`:
1. cross-compiles `hive-app` for `x86_64-pc-windows-msvc` with `cargo xwin`, with a static CRT because a stock Windows has no VC++ redistributable;
2. copies the `.exe` to `%LOCALAPPDATA%\hive-dev`;
3. builds `target/debug/hive` and exports `HIVE_BRIDGE` (that path) and `HIVE_WSL_DISTRO` through `WSLENV`, so the app's bridge runs the development build;
4. starts Vite in WSL on port 1420. The debug build loads `devUrl`, which Windows reaches through WSL localhost forwarding, so hot reload works;
5. opens the app through PowerShell, because launching a Windows `.exe` straight from WSL interop fails.

One-time setup: `sudo apt install clang lld llvm`, `rustup target add x86_64-pc-windows-msvc`, `cargo install cargo-xwin --locked`, `bun install`. The first build downloads the MSVC CRT and Windows SDK into `~/.cache/cargo-xwin`. Release bundles (MSI/NSIS) are not covered yet.
