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
| `hive badge <text…>` / `hive badge --clear` | Run inside a Hive terminal: sets or clears that terminal's label on its tab and agent row. Exits 2 without a valid `HIVE_TERMINAL_ID`, 1 when the service cannot be reached. |
| `hive worktree create/list/remove/hook-create/hook-remove` | Worktrees following Claude's convention. |

## Module map

| Crate / module | Responsibility |
|---|---|
| `hive-protocol` | Frame codec, `Control` messages, handshake constants, internal event model (`AgentEvent`, `EventKind`), `AgentState`. Shared with the app. |
| `hive::cli` | clap subcommands. Runs each async command on a runtime that is dropped without waiting for a pending stdin read. |
| `hive::paths` | Runtime dir (`$XDG_RUNTIME_DIR/hive`, or `/tmp/hive-<uid>`, checked to be ours and mode 0700), socket, lockfile, `daemon.log`, data dir, bin dir, hooks settings, `spaces.json`, `projects.json`, `ports.json`, config dir (`$XDG_CONFIG_HOME/hive`, or `~/.config/hive`) and its `settings.json`. |
| `hive::daemon` | Lockfile, socket (0600), handshake, app/hook connections, prioritized writer, terminal and agent registries, shutdown. |
| `hive::terminal` | Spawns `fish -C 'set -gx PATH <bin> $PATH'` on a PTY with `HIVE_TERMINAL_ID`, its space's environment entries and its worktree's `HIVE_*` entries. Handles input and resize, and ends process groups. |
| `hive::scripts` | Project scripts (6.8): each worktree's block of 10 ports (`<data>/hive/ports.json`), the `HIVE_*` environment, and running the archive script (`sh -c`, time limit, process group killed). |
| `hive::procs` | Minimal `/proc` reader (pid, pgrp, session, comm; skips zombies). |
| `hive::watch` | Pure state machine for the unhooked-`claude` warning. |
| `hive::states` | Pure agent state machine: hook events → state per agent and subagent, "the most urgent wins", PTY-silence reconciliation (the clock is passed in). |
| `hive::adapter` | `Adapter` trait and `ClaudeCode` adapter: raw hook payload → `AgentEvent` (raw payload kept). |
| `hive::hook` | `hive hook`: reads stdin (512 KiB limit), optionally records JSONL, sends with a 200 ms timeout. `hive badge` sends through the same hook-role connection. |
| `hive::bridge` | Relay plus detached daemon start (`setsid --fork`, stderr to `daemon.log`). |
| `hive::wrapper` | Installs `<data>/hive/bin/claude` (sh wrapper) and `<data>/hive/hive-hooks.json` when the daemon starts. |
| `hive::settings` | The user's settings (`<config>/hive/settings.json`): read at start (256 KiB limit; missing: the defaults; invalid: the defaults plus a warning, the file left alone), range checks, saved whole (temporary file + rename, 0600). |
| `hive::projects` | The projects the app follows, grouped in spaces: validation, `<data>/hive/spaces.json` (migrated from `projects.json`), worktrees per project for the sidebar, placing an agent's `cwd` in a worktree, a terminal's space environment. |
| `hive::spaces` | Spaces (6.14): the rules (names, environment checks, one space per project, only an empty space deleted, never the last) and the environment entries a space's terminals get. |
| `hive::files` | The files panel's worktree: `git ls-files` listing (sorted, capped), inotify watches on the listed directories and the git dir, debounce (the clock is passed in). |
| `hive::git` | Every git call: `git -C <dir>` with separate arguments and `GIT_OPTIONAL_LOCKS=0`, stdout size-limited, stderr in the error; an optional time limit (`output_within`) that kills git and its children; `read_limited` for any size-limited read. |
| `hive::health` | A worktree's status for the sidebar (changed files, ahead/behind the main worktree's branch, merged, last commit) and the last status sent per worktree. |
| `hive::worktree` | Worktrees in `.claude/worktrees/<name>` on branch `worktree-<name>`; `.worktreeinclude` copy. |
| `hive::changes` | A worktree's changes against `HEAD` for the files panel: `git status` and `git diff --numstat` parsers, untracked line counts, the `changes` message. |
| `hive::transcript` | A subagent's conversation (6.10): its transcript's path beside its agent's, checks (agent id, inside Claude's projects folder), record parsing into entries and the tail read as it grows. Token usage (6.9): `Tokens` counts a conversation's usage records (also for the Sessions panel), `Usage` reads an agent's transcript as it grows. |
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
| `badge {text}` | `hive badge` → service → app | n | Terminal n's label (`hive badge`); empty clears it. The service drops control and invisible (zero-width, bidi) characters, trims, cuts it at 40 characters (the last one becomes "…") and forwards it only while terminal n is open; `hive badge` sends it on a hook-role connection, which closes after it. The app drops the label when the terminal exits. |
| `agent {…AgentEvent}` | service → app | 0 | A translated hook event. |
| `unhooked_agent` | service → app | n | A `claude` runs in terminal n without sending hook events. |
| `agent_detected {id, project, worktree, cwd}` | service → app | n | An agent (`id` = its session id) started in terminal n. `project`/`worktree` are the ids of the followed worktree containing `cwd`, both null outside every followed project. |
| `agent_state {id, state, urgency, pending, subagents, activity, since_ms}` | service → app | n | The agent's displayed state, its urgency (0 ended … 6 waiting for permission; higher wins), whether it is pending (needs the user), and its live subagents `[{id, agent_type, state, worktree, activity, since_ms}]`, `worktree` being the id of the subagent's own worktree or null; `activity` is what the agent (or subagent) itself is doing, or null, and `since_ms` the wall clock time (ms since the epoch) its displayed state (a subagent's: its state) began (see [Agent states](#agent-states)). Sent when it changes, after `agent_detected`, and for every live agent right after `welcome`. |
| `agent_usage {id, context_tokens, context_limit, output_tokens}` | service → app | n | The agent's tokens from its transcript (see [Tokens and context](#tokens-and-context-69)): the last turn's context (input + cache writes + cache reads), the assumed window (200000, or 1000000 once the context passed 200k) and the session's output so far. Sent when they change and for every agent with a usage right after `welcome`. |
| `agent_removed {id}` | service → app | n | The agent's session ended, or terminal n exited (sent before `terminal_exited`). |
| `list_projects` | app → service | 0 | Asks for every project; answered by `spaces`, then `projects`. |
| `projects {projects}` | service → app | 0 | Every project with its worktrees, in the order they were added. Also sent unasked after a `WorktreeCreate` or `WorktreeRemove` hook. |
| `add_project {path}` | app → service | 0 | Follow the git repository containing `path` in the current space. |
| `project_added {project}` | service → app | 0 | The project, with its worktrees, after `spaces`. Also the answer when it was already followed in the current space. |
| `add_project_failed {path, error, message}` | service → app | 0 | `path` was refused. `error` is `empty_path`, `not_absolute`, `not_found`, `not_a_directory`, `not_a_git_repository`, `in_other_space` or `storage`; `message` is shown as is. |
| `spaces {spaces[{id, name, projects, env}], current}` | service → app | 0 | Every space (see [Spaces](#spaces-614)) and the current one's id. Sent before `projects` in answer to `list_projects`, before `project_added`, and in answer to every space request. |
| `create_space {name, env}` | app → service | 0 | A new, empty space, made current. `env` is `{claude_config_dir, git_name, git_email, gh_config_dir}`, each null or a string. Answered by `spaces` or `space_failed`. |
| `update_space {id, name, env}` | app → service | 0 | Renames a space and replaces its environment. Answered by `spaces` or `space_failed`. |
| `delete_space {id}` | app → service | 0 | Removes a space without projects, never the last one. Answered by `spaces` or `space_failed`. |
| `select_space {id}` | app → service | 0 | Makes `id` the current space. Answered by `spaces` or `space_failed`. |
| `space_failed {message}` | service → app | 0 | A space request refused, nothing changed; shown as is in the space dialog. |
| `list_dirs {path, windows}` | app → service | 0 | The subfolders of the folder typed in "Add project" (`hive::dirs`): `path` is Linux, or Windows (`C:\...`) when `windows`; empty is the home folder (`$HOME`, or `cmd.exe /c echo %USERPROFILE%`). Without a trailing separator, the folder holding the last name is listed. Answered by `dirs`. |
| `dirs {path, windows, linux_path, parent, dirs[{name, git}], error}` | service → app | 0 | `path` echoes the request (the home folder, ending with a separator, for an empty one); `linux_path` is `path` for `add_project`, converted with `wslpath -u` when `windows`; `parent` is the folder above the listed one in the same form (`null` at the top); `dirs` are its subfolders without hidden ones, links to folders included, sorted ignoring case, at most 1000; `git` when one holds a `.git` entry. |
| `list_branches {project}` | app → service | 0 | The local and remote branches of a followed project; answered by `branches`. |
| `branches {project, local, remote, current, error}` | service → app | 0 | Short names from `git for-each-ref` (remote `HEAD` symrefs skipped, at most 1 MiB of names). `current` is the branch checked out in the main worktree, shown as "default"; `error` says why they could not be listed. |
| `validate_worktree_name {project, name}` | app → service | 0 | Sent as the user types a new worktree's name. |
| `worktree_name_validated {project, name, folder, branch, error}` | service → app | 0 | The CLI's verdict (`error`, or null) and where the worktree would go (`.claude/worktrees/<name>/`, `worktree-<name>`; `<name>` when empty). Answers may arrive out of order, so the app matches them by name. |
| `create_worktree {project, name, base}` | app → service | 0 | `hive worktree create` in a followed project, from `base` (null: the main worktree's HEAD). |
| `worktree_created {project, path, notes}` | service → app | 0 | The project with its updated worktrees, the new path, and what the CLI prints on stderr (a competing `WorktreeCreate` hook, the files copied from `.worktreeinclude`). |
| `create_worktree_failed {project, name, message}` | service → app | 0 | The CLI's error, shown as is. |
| `watch_worktree {path}` | app → service | 0 | Watch this worktree of a followed project for the files panel, instead of any other (one at a time). Refused with `error` ("<path> is not a worktree of a followed project"). |
| `unwatch_worktree` | app → service | 0 | Stop watching (the files panel closed). |
| `worktree_status {path, status}` | service → app | 0 | A worktree's new status, sent only when it differs from the last one sent (see [Worktree health](#worktree-health)); `status` is null when git could not tell. |
| `watch_transcript {agent, subagent}` | app → service | 0 | Follow the conversation of subagent `subagent` (its `agent_id`) of agent `agent` (a session id), instead of any other (one at a time); see [Subagent conversation](#subagent-conversation-610). Answered by `transcript`, or `error` ("no transcript is known for this subagent", or why it cannot be read). |
| `unwatch_transcript {agent, subagent}` | app → service | 0 | Stop following it (the view closed); ignored when another one replaced it. |
| `transcript {agent, subagent, entries, truncated}` | service → app | 0 | The conversation so far (empty while the transcript is not written yet): `entries` = `[{role, text, tool}]`, `role` one of `user`, `assistant`, `tool` (a tool call: `tool` is its name and `text` its input as compact JSON; null otherwise); text cut at 2000 characters, at most the last 200 entries of the transcript's last 8 MiB. `truncated` when earlier entries were left out. |
| `transcript_appended {agent, subagent, entries}` | service → app | 0 | Entries written since the last message, checked every second while followed (at most 8 MiB read and 200 entries per check). |
| `view {terminal, focused}` | app → service | 0 | The terminal shown (null when none is, or a file or a subagent's conversation is) and whether the app window has the focus. Sent when either changes and after every `welcome` (`followView` in `src/follow.ts`; focus from `watchFocus` in `src/window.ts`). See [Agent states](#agent-states) item 5. |
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
| `get_settings` | app → service | 0 | Answered by `settings`. |
| `settings {settings}` | service → app | 0 | The service's settings (see [Settings](#settings)). Sent right after `welcome`, and in answer to `get_settings` and to a saved `set_settings`. |
| `set_settings {settings}` | app → service | 0 | Check and save the whole settings. Answered by `settings`, or by `settings_failed` with nothing saved. |
| `open_settings_file` | app → service | 0 | Open the settings file in an editor; the service first writes the settings in use when there is no file. Answered by `editor_target` with an empty `worktree` and `path` (`windows_path` from `wslpath -w`; the path itself on macOS). |
| `get_diagnostics` | app → service | 0 | Answered by `diagnostics`. |
| `diagnostics {settings_file, wrapper, claude}` | service → app | 0 | For the settings' About section: the settings file, the `claude` wrapper and the `claude` it runs, as found on the service's `PATH` (null when none is). |
| `settings_failed {message}` | service → app | 0 | Settings not saved (a value out of range, e.g. "agents.silence_secs must be between 2 and 60 (got 61)", or the write failed), or, right after `settings`, why the settings file was ignored ("Ignoring <file>: <why>. Using the defaults until the settings are saved."). Shown as is. |
| `error {message}` | service → client | 0 or n | A refused request, e.g. channel 0, a channel already open, a bad cwd, an unexpected message, or a second app. |

Between the app's Rust side and the WebView (#24), control messages travel on one Tauri `Channel` (given by the `connect` command) as the service's JSON plus a `channel` field, e.g. `{"type":"terminal_opened","channel":1}`. Terminal output travels as raw bytes on a separate `Channel` per terminal (given by `open_terminal`). No Tauri events are used. The Rust side adds `app_version` and `app_protocol` (its own values) to `version_mismatch`, so the UI can show both sides, and one message of its own:

| Message | Direction | Meaning |
|---|---|---|
| `disconnected {reason}` | app (Rust) → UI | The bridge exited or its stdout closed. `reason` is the bridge's stderr (at most 16 KiB), a protocol error, or "the hive bridge exited". Every open terminal gets `terminal_exited {code: null}` first. Not sent after `version_mismatch`. |

A project is `{id, name, path, worktrees, error}`: `id` and `path` are the main worktree's path, `name` its folder name, and `error` (or null) says why `git worktree list` failed, e.g. for a moved folder. A worktree is `{id, name, path, branch, main, claude, status}`: `id` is its path, `branch` is null when detached, `main` marks the main worktree and `claude` one in `<repo>/.claude/worktrees/`. `name` is the folder name for a Claude worktree and the branch otherwise (the folder name when detached). `status` is `{changes, ahead, behind, merged, last_commit_ms}` or null (see [Worktree health](#worktree-health)). The app never derives any of these.

`AgentEvent` has these fields: `provider`, `terminal_id`, `session_id`, `subagent {id, agent_type}`, `cwd`, `kind`, `activity` (below), and `raw` (the unchanged payload).
`kind` is one of: `session_started`, `prompt_submitted`, `tool_started`/`tool_finished`/`tool_failed {tool}`, `permission_requested {tool}`, `notification {notification}`, `turn_finished`, `turn_failed {error}`, `subagent_started`, `subagent_stopped`, `session_ended {reason}`, `worktree_created {name, path}`, `worktree_removed {path}`, or `other {event}`.
`notification` is one of `permission_prompt`, `elicitation_dialog`, `idle_prompt`, `agent_needs_input`, or `other`.
`activity` is set on `tool_started` and `permission_requested` only: a short text from the payload's untrusted `tool_input` (`hive::adapter`): Edit/Write/MultiEdit/NotebookEdit "Editing <file>" and Read "Reading <file>", the file relative to the payload's `cwd` when inside it; Bash its `description`, else the first line of its `command`; Grep/Glob "Searching <pattern>"; Agent/Task its `description`; any other tool, or a missing field, the tool's name. Control characters are dropped and it is cut at 120 characters.

## Sequences

### Bridge start
1. The app runs `wsl.exe [-d $HIVE_WSL_DISTRO] --exec /bin/sh -c <BRIDGE_SCRIPT> sh "$HIVE_BRIDGE" "<bundled>"` (`src-tauri/src/lib.rs`). `--exec` skips the user's login shell, so no fish config runs. On macOS (5.2) the app runs the same script natively: `/bin/sh -c <BRIDGE_SCRIPT> sh "$HIVE_BRIDGE" "<bundled>"` (`HIVE_WSL_DISTRO` is ignored). The script is a constant; both values are separate arguments, empty when absent. The script runs, in order:
   - `$HIVE_BRIDGE`, an absolute Linux path (development, `scripts/win-dev.sh`);
   - the `hive` the installer bundles (4.18): `<bundled>` is its Windows path in the app's resources, only when that file exists. a POSIX path (macOS) is used as is, else `wslpath -u` (after dropping a verbatim `\\?\` prefix) finds it in WSL; when `cmp` finds it differs from `<data>/hive/bin/hive`, it is copied to `hive.new` there, loses its `com.apple.quarantine` attribute when `xattr` exists (macOS; a missing or failing `xattr` is ignored) and is renamed over it, so a running `hive` keeps its file. On macOS the copy also gives hooks a stable path, since an app opened from Downloads runs from a random translocated one. The copy runs, so hooks (which call the daemon's `current_exe`) use it too;
   - otherwise `~/.cargo/bin/hive` (`cargo install`, development).

   On Windows the process gets `CREATE_NO_WINDOW`, and it is killed when the app drops the connection.
2. The bridge connects to `<runtime>/hive.sock`.
3. If the connection fails, the bridge prepares the runtime dir, truncates `daemon.log` (0600) and runs `setsid --fork hive daemon`. The daemon's stdin and stdout are null and its stderr goes to the log.
4. The bridge retries the connection for up to 5 s. If the daemon never listens, the bridge fails with "the hive service did not start; see <log>".
5. Two bridges racing is harmless: the second daemon cannot take the lockfile and exits.
6. The bridge then copies bytes in both directions without looking at them. When either side closes, the bridge exits.

### App connect (app side)
1. The UI calls `connect(onMessage)` at startup (`connect` in `src/connect.ts`, called by `src/main.tsx`), handing a `Channel` to Rust. "Reconnect" goes through the same function, so every connection has the same handler.
2. Rust starts the bridge and queues `hello {role: app, version}`; `version` is the app's `CARGO_PKG_VERSION`, so `hive-app` and `hive` share one version number.
3. `welcome` or `version_mismatch` goes to the UI. After `version_mismatch`, Rust drops the bridge and sends nothing more.
   The UI (`src/shell/ConnectionBlock.tsx`) then blocks the workspace (`inert`, with a modal `alertdialog`; the title bar stays usable) and shows both versions and the fix: `pkill -f 'hive daemon'`, because a refused handshake leaves the old service running (the installed app brings its own `hive`), and, for development builds, `cargo install --path crates/hive`. `disconnected` blocks the same way and shows the reason. The dialog's "Reconnect" calls `connect` again, which starts a new bridge.
4. A UI that reloads calls `connect` again: if the connection is up, Rust closes that UI's old terminals, drops their later messages, replays `welcome` and sends `get_settings` and `list_projects` for it; otherwise it starts a new bridge.
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
4. Keys: `interceptKeys(handler)` sees every key event first and keeps the app shortcuts from the terminal (see Shortcuts); then Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes through the clipboard API (#35; on macOS Cmd+C and Cmd+V, and Ctrl+C/Ctrl+Shift+V reach the shell); everything else goes to xterm and, as `onData`, to `writeTerminal`. Input stops once the terminal exited.
5. `closeTerminal(id)` sends `close_terminal` unless the shell already exited, disposes the `Terminal` and removes the tab (the right neighbour, or the new last tab, is shown). An exited terminal keeps its tab, marked "exited", until closed; `unhooked_agent` adds a "no hooks" badge.
6. The `terminal` settings (see [Settings](#settings)) and the theme's colors are each terminal's xterm options (`termOptions`), set on new terminals and on every open one when `settings` change (the shown one refits). With `copy_on_select`, a new selection is copied to the clipboard.

### Shortcuts (#35)
`src/shortcuts.ts` has one `keydown` listener on the window and one command table, `COMMANDS` (id, label, keys, action): `shortcut()` runs the command whose keys were pressed, and the settings' Shortcuts section lists them. A focused terminal gives each key to `interceptKeys` first: a shortcut is kept from xterm, which leaves it unhandled, so it bubbles up to the window listener and runs once; every other key (Ctrl+Shift+C/V included) is the terminal's.
1. Ctrl+Shift+T opens the worktree picker (every worktree of every project, filtered by worktree or project name; ↑/↓, Enter or a click opens a terminal in its path; Esc or a click outside closes). Ctrl+Shift+N opens the new worktree dialog for the selected project, the project of the selected worktree (a selected agent counts as its worktree, `selectedPlace` in `src/store.ts`; the tab bar's "+" uses it too), or the first project (add project when there is none). Ctrl+Shift+B toggles the files panel. Ctrl+Shift+O opens add project. Ctrl+, opens the settings. Ctrl+Shift+P opens the command palette (`src/shell/Palette.tsx`): one list, filtered by a subsequence match ranked by runs and word starts, grouped as Commands (every `COMMANDS` entry with its keys, plus `paletteCommands` extras such as "Remove merged worktrees…" and "Send review"), Agents and worktrees (Enter goes to the agent as F8 does, `goToAgent`, or selects the worktree and expands its project) and Files (the lines of the files panel's worktree holding the text, `search_files` once typing pauses; a row opens the file at that line as the Files panel does). F8 (no modifiers, or a click on the "N pending" counter) selects the pending agent after the selected one (else after the one whose terminal is shown) in tree order, wrapping; it expands the agent's project and worktree, shows its terminal when it has a tab and scrolls it into view. Agents outside every project come last. With nothing pending it does nothing.
2. On macOS every Ctrl+Shift+letter shortcut is Cmd+Shift+letter instead (and Ctrl+, is Cmd+,) (`commandKey` in `src/window.ts`, from the user agent); Ctrl+Shift+letter then goes to the terminal. Tooltips and kbd chips show `⇧⌘B` there (`keyText`).
3. Nothing runs under the connection block (`version_mismatch`, `disconnected`) or while a dialog is open; the key then goes on as usual.
4. The tree (sidebar): ↑/↓ move between rows, ←/→ collapse and expand a project or a worktree with agents, Enter selects (rows are buttons).

### Closing the app (#18)
1. The title bar Close, Alt+F4 and the taskbar all become one Tauri close request. `guardClose` (`src/window.ts`) listens to it with `onCloseRequested` and asks `confirmClose` (`src/shell/CloseAppDialog.tsx`).
2. When `agentsAtRisk` is empty, or the `agents.confirm_close` setting is off, the window is destroyed at once. Otherwise the request is cancelled and the "Close Hive?" dialog opens: Cancel/Esc keeps the app, "Close Hive" (focused, Enter) calls `closeWindow`, which destroys the window without asking again. Stage 1 counts every detected agent; Stage 2 narrows `agentsAtRisk` to working, waiting for permission and waiting for you.
3. The last window gone, Tauri emits `RunEvent::Exit`; `on_run_event` runs `Hive::shutdown`: the frame queue closes, so the bridge's stdin closes, the bridge exits and the daemon runs "App disconnect" below. Shutdown waits up to 2 s for the bridge to end, then kills it (no `wsl.exe` left behind). A crashed app closes the same pipe through the OS.
4. Outside Tauri (browser, mock transport) only the title bar Close requests a close, and a close that goes through sets `data-closed` on `<html>` for the browser checks.

### Notifications (hive.md item 5)
1. `src/connect.ts` passes every service message to `notify` (`src/notify.ts`) before `apply`, so the store still holds the agent's previous state. Only a change the service sent counts: an agent's first state is silent, and so is the snapshot after `welcome` (it always lands in an empty store: a fresh page, or after `disconnected` cleared it).
2. Entering waiting for permission, waiting for you or error plays a short tone synthesized with Web Audio (no audio file) at the `notifications.volume` setting (0 plays none); changes within 500 ms share one tone.
3. Working or with subagents → waiting for you is "agent finished": `showNotification` (`src/window.ts`) sends an OS notification through `tauri-plugin-notification` ("Agent finished", "project · worktree: waiting for you"), only when the message is `pending`: the service marks an agent that finished in view of the focused window as not pending (see [Agent states](#agent-states) item 5), and then there is no notification. The tone still plays. The capability allows only `is_permission_granted`, `request_permission` and `notify`. Outside Tauri nothing is shown.
4. Inbox (6.5): every entry into an alerting state (item 2, muted or not) is also kept in the store's `inbox` (`addToInbox`, at most 100, newest first: `{id, agent, state, at, text}`, text "<name> is waiting for permission" / "finished" / "is waiting for you" / "failed"; an optional `space` is left for 6.14). The title bar bell opens it as a menu (`ContextMenu`): the pending agents in tree order, then the alerts with relative times; an item goes to its agent through `goToAgent` (`src/shortcuts.ts`, shared with F8), an ended agent's item is disabled. A dot shows alerts newer than `inboxSeen`; opening marks them read. It lives in memory only.

### Settings
1. The service owns the settings (#37) in `<config>/hive/settings.json` (`$XDG_CONFIG_HOME/hive`, or `~/.config/hive`): `terminal {font_family, font_size 8–32, scrollback 1000–100000, cursor_style block|bar|underline, cursor_blink, copy_on_select}`, `appearance {theme one-dark|one-light}`, `notifications {volume 0–100}`, `agents {silence_secs 2–60, confirm_close}`, `worktrees {default_base}` (null: the project's current branch) and `projects {<project id>: {scripts {setup, run [{name, command}], archive}}}` (see [Project scripts and ports](#project-scripts-and-ports-68)). Defaults: `"IBM Plex Mono", monospace`, 13, 5000, block, no blink, no copy on select, one-dark, 100, 5 s, confirm, null. Every key is optional (a missing one takes its default) and unknown keys are ignored. Texts (run script names too) must be non-blank, at most 256 bytes, without control characters; scripts non-blank, at most 16 KiB, without control characters but newlines and tabs; run script names unique per project.
2. At start the service reads the file (at most 256 KiB). A missing file is the defaults. An unreadable or invalid one (bad JSON, a value out of range) is the defaults too: the app gets `settings` with them and then `settings_failed` with the reason, on every connect and `get_settings`, until settings are saved; the file is not touched until then. The warning also goes to stderr (`daemon.log`).
3. `set_settings` replaces the whole settings: checked, written (at most 256 KiB, temporary file renamed over it, mode 0600) and then in use at once (the silence rule reads it on every tick). A refusal changes nothing.
4. The store keeps the last `settings` as `settings` (`DEFAULT_SETTINGS` in `src/store.ts` until they arrive) and a `settings_failed` message as `settingsError` (cleared by the next `settings`), also shown in the status bar. The app applies the `terminal` settings to its terminals, `appearance.theme` as `data-theme` on the root element (One Light's CSS variables in `src/styles.css`, from Zed's One theme), `notifications.volume` to the tone and `agents.confirm_close` to the close guard.
5. The settings dialog (`src/shell/SettingsDialog.tsx`, Ctrl+,) shows them by section (Terminal, Appearance, Notifications, Agents, Worktrees, Projects, Shortcuts, About), or every field whose label matches the search. A change sends `set_settings` with the whole settings (text and number fields once typing pauses for 400 ms); `settingsError` shows at the top. "Open settings file" sends `open_settings_file`. About shows the app's version (`package.json`), the service's (`welcome`), `diagnostics` (asked when the section opens) and the terminals with an `unhooked_agent` badge.

### Projects
1. After `welcome` (also a replayed one), the app's Rust side sends `list_projects`; the UI's "Refresh worktrees" button sends it again. The service also sends `projects` after each worktree hook (see [Worktree hooks](#worktree-hooks)); other worktree changes are not watched (only the files panel's worktree is, see [Files panel watch](#files-panel-watch-31-31)).
2. The service answers `projects`. Each project's worktrees come from `git worktree list --porcelain -z`, bare and prunable entries (the directory is gone) skipped.
3. `add_project {path}` (the add-project dialog): the path must be absolute, an existing directory and inside a git repository with a working tree. It is normalised to the main worktree (the first entry of `git worktree list`), so a subfolder or a linked worktree adds its repository. A new project is appended to the current space in `<data>/hive/spaces.json`; if that write fails the list is unchanged and the answer is `add_project_failed {error: storage}`. A project already in another space is refused (`in_other_space`, "<path> is already in the space <name>").
4. Project requests run on a blocking thread, off the app's frame loop, because git can be slow.
5. Worktree requests name a project by id and are refused ("<id> is not a followed project") for any other.

### New worktree (screens 1c/1d)
1. The dialog opens for a project (the row's "New worktree", or `openModal("new-worktree", id)`), sends `list_branches` and `validate_worktree_name` for the empty name.
2. Every keystroke in the name sends `validate_worktree_name`: the service runs `worktree::check_name` (the rule and the existing-folder check of `hive worktree create`, no git), so the dialog never has its own rule (#33, #37). Create stays disabled until the current name has a clean verdict.
3. The branch list is filtered in the UI (case-insensitive substring) and virtualized (TanStack Virtual); ↑/↓ in the filter move the pick. A pick hidden by the filter gives way to the first branch shown.
4. `create_worktree` runs `worktree::create`, the CLI's code. On `worktree_created` the UI selects the new worktree, runs the project's setup script, if any, in a terminal tab of its own (`openWith`), opens a terminal tab in it if asked (`openTerminal` of `src/terminals.ts` with its path), and closes the dialog; with notes, the dialog stays open to show them.

The list is `spaces.json`, `{current, spaces: [{id, name, projects, env}]}`, written through a temporary file (mode 0600) renamed over it. Without it, the flat list of earlier versions (`projects.json`, a JSON array of paths) becomes the "Default" space (id `default`), and is left as it is; `spaces.json` is written on the first change. A missing file is an empty list. An unreadable or invalid one (bad JSON, over 1 MiB, or breaking a space rule: no space, an unknown current one, a repeated id, a project in two spaces, a bad name or environment) is moved to `<name>.corrupt` with a warning on stderr (`daemon.log`), and the service starts with an empty list.

### Spaces (6.14)
A space groups projects (work, personal) with an optional identity for the terminals opened in them. Agents of every space stay alive and keep alerting.
1. Rules (`hive::spaces`): a name is trimmed, not blank, at most 64 characters, without control characters. Each `env` value is trimmed and blank means unset; git name and email are at most 256 bytes, the two folders at most 4096 bytes and absolute, all without control characters. On `create_space`/`update_space` the folders must exist as directories (a file read at start only needs the shape, so a folder gone later does not lose the list). New spaces get the ids `space-<n>`, the first free `n`. A project is in one space only. Only a space without projects is deleted, never the last one; deleting the current one makes the first one current. Every change is saved at once; a refused or unsaved one changes nothing (`space_failed`).
2. A terminal opened in a worktree of a project gets its space's `env` as environment entries (never a shell string): `CLAUDE_CONFIG_DIR`, `GIT_AUTHOR_NAME` + `GIT_COMMITTER_NAME`, `GIT_AUTHOR_EMAIL` + `GIT_COMMITTER_EMAIL`, `GH_CONFIG_DIR`, only those set. The placement uses `projects::place` on the terminal's `cwd`; outside every project nothing is added. Changes apply to terminals opened afterwards.
3. The terminal keeps its space's Claude config folder, and an agent detected in it takes it: its session name, and its subagents' transcripts (checked inside `<folder>/projects`), are read from there, else from the service's own root (`$CLAUDE_CONFIG_DIR/projects`, else `~/.claude/projects`). The Sessions panel (`list_sessions`, `locate_session`, `delete_session`) covers the current space's projects in its folder the same way.
4. UI: a select at the left of the sidebar header lists the spaces plus "New space…" and "Edit space…" (the dialog `SpaceDialog`: name, Claude config folder, git name, git email, GitHub CLI config folder; "Delete space" only enabled for an empty one). The sidebar shows the current space's projects (`spaceProjects`); the Sessions panel lists again when the space changes. The pending bell, F8, tones and OS notifications cover every space; a notification and each inbox item (pending or kept) name the agent's space when there are several (`agentPlace`, `spaceName` in `src/notify.ts`; `InboxItem.space`), and going to an agent of another space (`goToAgent` in `src/shortcuts.ts`, used by F8 and the inbox) sends `select_space` for it.

### Worktree health
1. `hive::health` reads, per worktree, on a blocking thread and with a 10 s limit per git command (git and its children, in their own process group, are killed after it): `changes` = the entries of `git status --porcelain=v2 -z --untracked-files=all --find-renames` (as `changes` counts them, so linked worktrees inside the main one count as untracked folders unless ignored); `last_commit_ms` from `git log -1 --format=%ct HEAD`; for a linked worktree, when the main worktree is on a branch, `ahead`/`behind` from `git rev-list --left-right --count HEAD...refs/heads/<branch> --`, and `merged` = `ahead` is 0 (every commit of its `HEAD` is on that branch, what `git merge-base --is-ancestor` answers, without another git run). The main worktree has `ahead`/`behind` null and `merged` false. Any git failure (no commit yet, the branch is gone, the time limit) makes the status null; `projects` never fails because of it.
2. Every message carrying projects (`projects`, `project_added`, `worktree_created`, `worktree_removed`, `worktree_renamed`) carries fresh statuses. The service remembers the last status sent per worktree path (`health::Sent`) and sends `worktree_status` only when one differs: every 30 s for every followed worktree (first right when the service starts), and for the watched worktree after each of its changes (after its `changes`).
3. The sidebar shows the status as muted badges after the worktree's row: `↑ahead`, `↓behind`, `●changes` (each only when not 0) and "merged", each with a tooltip. The project row's context menu has "Remove merged worktrees…" (`RemoveMergedDialog` in `src/shell/WorktreeMenu.tsx`): it lists the project's linked worktrees that are merged with no changes, as they were when it opened, all checked, and sends a plain `remove_worktree` (never `force`) for each checked one; each shows "Removing…", "Removed" or its `remove_worktree_failed` message (kept by path in `worktreeDialog.removeFailures`). Nothing is ever merged (#12).

### Project scripts and ports (6.8)
1. A project's scripts live only in the settings (`projects.<id>.scripts`), edited in the settings dialog's Projects section (a project Select; setup and archive text areas; run scripts as name + command rows with an Add form); they are never read from a repository, so a cloned repository cannot run code on its own.
2. Setup: after `worktree_created`, the new worktree dialog opens a terminal in the worktree and types the setup script + Enter into it (`openWith` in `src/terminals.ts`, the mechanism "Start claude" uses), before the dialog's own terminal. Run: the worktree's context menu has "Run: <name>" per run script of its project; each opens a terminal there and types the command + Enter. From then on Hive only observes.
3. Ports: the first time a process needs them, a worktree gets a block of 10 ports, the lowest free one of 20000–29999 (below Linux's ephemeral range), kept by worktree path in `<data>/hive/ports.json` (0600, temporary file + rename) so it stays the same across restarts. Blocks of worktrees whose folder is gone are taken back when a new block is given. A file that cannot be read, or a block outside the range, counts as none.
4. Every terminal whose `cwd` lies in a followed worktree gets `HIVE_PORT` (the first port of its block; left out when no block can be saved, with a warning on stderr), `HIVE_WORKTREE_PATH` and `HIVE_ROOT_PATH` (the project's main worktree), as environment entries.
5. Archive: `remove_worktree` first checks the worktree (linked, of a followed project; unless `force`, not in use), then the service runs the project's archive script, if any, as `sh -c <script>` in the worktree with the `HIVE_*` entries, stdin from `/dev/null`, stdout and stderr together, in its own process group. After 60 s the group is killed. A failure (exit code not 0, a signal, the time limit) cancels the removal with `remove_worktree_failed` whose message ends with the last 4 KiB of the output ("the archive script failed (exit status: 4):\n…"); with `force` the removal goes on anyway. Once the script ends, whatever it left in its process group is killed (its worktree is about to go); the removal waits at most 0.5 s more for output held by a process that left the group.

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
2. The service spawns `fish -C 'set -gx PATH <bin> $PATH'` on a new PTY. fish is the session leader, and the environment has `HIVE_TERMINAL_ID=n` and `TERM=xterm-256color`, plus, in a followed worktree, `HIVE_PORT`, `HIVE_WORKTREE_PATH` and `HIVE_ROOT_PATH` (see [Project scripts and ports](#project-scripts-and-ports-68)).
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

### Badge (6.12)
1. `hive badge <text…>` (words joined with spaces) or `hive badge --clear` reads `HIVE_TERMINAL_ID` (a number from 1; otherwise an error and exit 2).
2. It connects, sends `hello` (role `hook`) and `badge {text}` on channel `HIVE_TERMINAL_ID` within 200 ms; on failure it prints `hive: cannot reach the Hive service: …` and exits 1.
3. The service cleans the text (control and invisible characters dropped, trimmed, at most 40 characters) and forwards `badge` to the app on that channel if the terminal is open.
4. The app shows a non-empty label as a muted pill on the terminal's tab and on its agent's row; `terminal_exited` clears it.

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
2. A subagent joins the list on its first event with a state and leaves it on `SubagentStop` (or a `SessionEnd` carrying its `agent_id`), unless it ended its turn to wait on a background task it launched: the task ids come from its `PostToolUse` `tool_response` (`backgroundTaskId` for Bash `run_in_background`, `taskId` for `Monitor`, `agentId` for `Agent`), and Claude Code lists the session's running ones as `background_tasks` on every `Stop`/`SubagentStop`. While one of its launches is in that list it stays listed as working (silence does not change it) until it is woken (`SubagentStart` again); it leaves when a later list has none of them, when its own worktree is removed, or with its agent. At most 32 are kept per agent, and ids or types over 256 bytes are ignored, so a message always fits in a frame.
3. Rule 1: `state` is the most urgent of the agent's own state, its subagents' states and "with subagents" when any subagent is live. Urgency, highest first: `waiting_permission`, `error`, `waiting_you`, `with_subagents`, `working`, `idle`, `ended`.
4. Rule 2: every second (the unhooked-claude tick) the service checks each agent: when its terminal has printed nothing for `agents.silence_secs` (5 s by default, see [Settings](#settings)), counted from the later of the last output and the last hook event, the agent and each subagent in working or waiting for permission go to waiting for you. Output alone never moves a state back; only hook events do.
5. Pending (the "N pending" counter and F8): waiting for permission, error and waiting for you. `urgency` is the state's rank in the order above; the app shows a collapsed project or worktree with the state of highest `urgency` inside and counts agents with `pending`, so it keeps no table of its own. Except (hive.md item 5): an agent whose displayed state goes from working or with subagents to waiting for you while its terminal is the one in `view` and the window has the focus is seen, so not pending, until its displayed state changes again (looking away later keeps it seen).
6. `agent_state` is sent on the agent's terminal channel only when the message changes.
7. A subagent's own worktree (#22), sent as its `worktree` (the worktree's id, i.e. its path). Linked by a `worktree_created` carrying the subagent's `agent_id` (the path the hook created), or else by the subagent's own events: the first time each new `cwd` of a subagent without a worktree is seen, the service places it like an agent (`projects::place`, git on a blocking thread) and links the worktree found when it is not the agent's own. Only when the agent itself was placed; paths over 4096 bytes are ignored. A `worktree_removed` of that path unlinks it; the subagent leaving drops it. The sidebar shows the worktree under the subagent and not at project level. To confirm by spike 1.12: whether `WorktreeCreate` carries `agent_id`, and that a subagent's `cwd` is its worktree.
8. Activity and time: an event with an `activity` sets it for the agent, or for the subagent that fired it; `Stop`, `StopFailure`, `SessionEnd` and (for a subagent) `SubagentStop` clear it, and so does rule 2 for the agent or subagent it interrupts. `since_ms` is when the displayed state (a subagent's: its own state) last changed, on the wall clock: the daemon gives each agent the wall time it was detected and later times are counted from it with the monotonic clock. The sidebar shows the time in the state ("12s", "3m", "1h", from `since_ms`, one shared 1 s timer) and the activity, muted, after the state's name.

### Subagent conversation (6.10)
Clicking a subagent in the sidebar shows its conversation, read-only, in place of the terminals (`TranscriptView`); clicking its agent, a tab or "Back to terminal" shows the terminal again. Hive never types into Claude for it.
1. The agent's `SessionStart` `transcript_path` is kept on its `states::Agent` (`transcript`) when it is an absolute `.jsonl` path of at most 4096 bytes; later tasks (usage, 6.9) read it from there.
2. `watch_transcript` derives the subagent's transcript: `<transcript_path without .jsonl>/subagents/agent-<agent_id>.jsonl`, only for an `agent_id` of 1–64 characters in `[A-Za-z0-9_-]`. Each read canonicalizes it (links resolved) and reads it only when it is a regular file inside the agent's Claude projects folder (its space's, see [Spaces](#spaces-614); else `$CLAUDE_CONFIG_DIR/projects`, else `~/.claude/projects`).
3. Records are parsed like the Sessions panel's logs: `user` and `assistant` records, meta messages skipped; each text block is an entry, each `tool_use` a `tool` entry; thinking and tool results are left out. Lines that are not JSON are skipped.
4. The first read takes the last 8 MiB (a line cut at the start does not parse and is skipped), then every tick of the unhooked-claude timer reads the whole lines added since (a line still being written waits; a transcript that shrank is read again from its start). The store keeps at most the last 1000 entries.

### Tokens and context (6.9)
From the transcripts only: no 5 h or weekly limits, and the user's statusline is never touched.
1. An agent's `PostToolUse`, `Stop` or `SubagentStop` (its subagents' too) marks its usage due; the next tick of the unhooked-claude timer (every second) reads the whole lines its transcript (`transcript_path`, same checks and 8 MiB bounds as the subagent conversation) gained since the last read. So a transcript is read at most once a second per agent, and only the last 8 MiB of a long one count at first.
2. Each `assistant` record outside a sidechain with a `message.usage` counts: the context is its `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` (records with no context, such as API errors, are skipped); `output_tokens` add up, once per `message.id` (Claude repeats a message's usage on each of its content blocks; the last one wins). A field that is missing, not a number or over 100 million counts as 0.
3. The context window is taken as 200k, or 1M once the context passed 200k (a heuristic: the real window only reaches the statusline).
4. `agent_usage` is sent on the agent's channel when context, window or output changed. The sidebar shows "ctx 42%" after the agent's state line.
5. Sessions panel: `sessions` entries carry `context_tokens` and `output_tokens`, counted the same way while summarizing the log (cached with the summary).

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
| `<data>/hive/spaces.json` | 0600 | The spaces, their projects and environments, and the current space (see [Spaces](#spaces-614)). |
| `<data>/hive/projects.json` | 0600 | The followed projects' paths before spaces (read only while `spaces.json` is missing). |
| `<data>/hive/ports.json` | 0600 | JSON object: worktree path → first port of its block of 10. |
| `<config>/hive/settings.json` | 0600 | The user's settings (see [Settings](#settings)); written only when the app saves them. |
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

Outside Tauri (a plain browser, `bun run dev`, Playwright) or with `?mock` in the URL, `src/transport/mock.ts` stands in for the service: it answers `welcome` (distribution "Ubuntu"), `settings` (the defaults; `set_settings` keeps new ones in memory, unchecked), `spaces` (one "Default" space holding the initial projects; space requests follow the service's rules in memory, without the folder checks, and `list_sessions` lists the current space's sessions) and `projects` (two of three fake repositories under `/home/user`; `?mock=empty` starts with none, and `add_project` accepts only the fake paths; branches, name checks and new worktrees follow the CLI's wording, with a long remote branch list in `shop`; `list_changes` answers `MOCK_CHANGES`, sample changes after screen 1g, and `open_file` a sample text shaped by the file's status there, `MOCK_TEXTS` for `src/auth/session.ts`, a `.png` as binary), or with `?mock=mismatch` / `?mock=disconnected` a `version_mismatch` / `disconnected` instead, and each terminal prints `mock$ `, echoes input, repeats the line on Enter and exits on `exit`; `cd <dir>` moves it and `claude` sends `agent_detected` placed at the worktree whose path is exactly that directory, then `agent_state` idle; every later line sets that agent working (`agent_removed` when the terminal exits); `worktree-remove <name>` stands in for a `WorktreeRemove` hook, dropping that Claude worktree and sending `projects`; `hive badge <text>` / `hive badge --clear` sends `badge`. `?mock=states` adds, without terminals, agents in every state, two of them with subagents, one subagent owning a worktree (`tests-login` in `shop`, shown under it), with fake activities, times and context (`agent_usage`); the fake sessions carry made-up token totals. `?mock=load` replays a recording into every terminal (see Load test below). `bun run e2e` needs `libnss3` and `libnspr4`; without root, extract them with `apt-get download` + `dpkg -x` and point `LD_LIBRARY_PATH` at them.

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
