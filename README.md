# Hive

Hive is a desktop companion for [Claude Code](https://code.claude.com) agents. You run the interactive `claude` in Hive's embedded terminals, and Hive shows what every agent is doing: its state, its subagents, its worktree and the files it changed. It is built for running many agents in parallel, each in its own git worktree.

Hive only **observes** agents, through Claude Code hooks. It never controls them or talks to them: every interaction with an agent happens in its terminal. For convenience it can type a `claude` command into a new terminal (start, resume or fork a session).

Why another one: tools like [Orca](https://github.com/stablyai/orca) (Electron + TypeScript) felt heavy with many terminals open. Hive is a Tauri app (the system WebView, no bundled Chromium) with all the work done by a Rust service.

## Features

**Agent states in the sidebar.** The tree is Project → Worktree → Agent → Subagents. A subagent with its own worktree is nested under that worktree. States come from Claude Code hooks:

| State | Color | Meaning |
|---|---|---|
| waiting for permission | yellow | a permission prompt or a question is waiting for you |
| error | red | the turn failed (`StopFailure`) |
| waiting for you | orange | the turn ended, or the terminal went silent for 5 s (an interrupted turn fires no hook) |
| working | blue | a prompt or a tool call is running |
| running subagents | purple | the agent has subagents running |
| idle | green | the session started and nothing has happened yet |
| ended | grey | the session ended |

The most urgent state wins: a subagent waiting for permission puts its agent in that state. A collapsed project or worktree shows the most urgent state inside it.

**Pending bell.** Waiting for permission, error and waiting for you count as pending. The bell in the title bar shows how many agents are pending; clicking it or pressing **F8** jumps to the next one.

**Notifications.** A short tone when an agent enters waiting for permission, waiting for you or error, and an OS notification when an agent finishes its turn. No notification (and no pending count) when Hive has focus and that agent's terminal is the one you are looking at.

**Embedded terminals.** xterm.js tabs, grouped per worktree: the tab bar shows the selected worktree's terminals. A tab running Claude shows the agent's state and the session name Claude gave it. Any number of terminals; only visible ones use WebGL.

**Worktrees.** Create one from any local or remote branch. Hive follows Claude Code's convention (`.claude/worktrees/<name>/`, branch `worktree-<name>`, the same as `claude -w <name>`) and copies the files listed in `.worktreeinclude`. The new worktree can open a terminal with `claude` already started. Worktrees created by `claude -w` or by subagents in Hive's terminals go through Hive too, and disappear from the sidebar when Claude removes them. Right-click a worktree to rename it, delete it, open a terminal there, copy its path or open its folder.

**Files, diff and editor.** The side panel (Files | Diff | Sessions) shows the worktree's files and git changes, updated live while the agent works. Files open in CodeMirror 6 with a read-only diff against `HEAD`. You can edit and save: the save is refused if the file changed on disk since you opened it, and a clean buffer reloads by itself. File search by name or content.

**Code references.** Select lines in the viewer or the diff and press **Ctrl+Shift+L**: Hive types `@src/checkout/validators.ts (lines 44–46)` into the active terminal (no Enter).

**Session history.** The Sessions tab lists the Claude sessions of the shown worktree, including sessions run outside Hive, with their state. Resume one, fork it into a new session, copy its resume command or ID, open its log, or delete it.

**Sessions restored.** Terminals and agents end when Hive closes (it asks first if an agent is busy). The Claude sessions that were open come back with `claude --resume` the next time Hive starts.

**Add project with a folder browser.** On Windows you can pick a WSL folder or a Windows one (under `/mnt/c`; slower, and without live file updates).

**Auto-update.** On start Hive checks GitHub for a newer release and shows **Update to vX** in the title bar.

## Platforms and requirements

| | Windows | macOS |
|---|---|---|
| OS | Windows with WSL2 | macOS 12 or later, Apple Silicon only |
| Where the service runs | inside the **default** WSL distribution | natively |
| Terminal shell | **fish**, installed in WSL | your login shell (`$SHELL`, zsh by default) |
| Also needed | `git` and Claude Code installed in WSL | `git` and Claude Code |

Your projects should live in the WSL file system for the best performance (Windows folders work, but are slower and not watched live).

## Installation

Download the latest release from [GitHub Releases](https://github.com/lucasreali/hive/releases).

### Windows

1. Run `Hive_<version>_x64-setup.exe` (releases before 0.1.2 name it `hive_<version>_x64-setup.exe`).
2. The installer is not signed, so SmartScreen may block it: click **More info → Run anyway**.
3. On first launch Hive copies its service into WSL at `~/.local/share/hive/bin/hive` and starts it. There is nothing to install by hand.

If Hive shows **"The app and the hive service versions differ"**, an old service is still running. Stop it in WSL, then click **Reconnect**:

```sh
pkill -f 'hive daemon'
```

### macOS

1. Download the `.dmg` and drag Hive to Applications.
2. The app is ad-hoc signed (no Apple Developer account), so macOS blocks the first open. Go to **System Settings → Privacy & Security** and click **Open Anyway**.

### Updates

When a newer release exists, click **Update to vX** in the title bar. Hive asks first if agents are running (they end with the restart), installs the update and restarts.

## Quick start

1. **Add a project**: **Ctrl+Shift+O**, then pick the repository folder (on Windows, choose `WSL` or `Windows` in the selector first).
2. **Create a worktree**: **Ctrl+Shift+N**, give it a name (`^[a-z0-9][a-z0-9._-]*$`), pick the base branch and keep "Start claude in the terminal" ticked.
3. **Work in the terminal** as usual. The agent appears in the sidebar under its worktree, with its state.
4. **Press F8** whenever the bell shows something pending.

| Action | Windows | macOS |
|---|---|---|
| Next pending agent | F8 | F8 |
| Worktree picker → new terminal | Ctrl+Shift+T | ⇧⌘T |
| New worktree | Ctrl+Shift+N | ⇧⌘N |
| Side panel (files, diff, sessions) | Ctrl+Shift+B | ⇧⌘B |
| Add project | Ctrl+Shift+O | ⇧⌘O |
| Send the selected lines to the terminal | Ctrl+Shift+L | ⇧⌘L |
| Copy / paste in the terminal | Ctrl+Shift+C / Ctrl+Shift+V | ⌘C / ⌘V |
| Save (in the editor) | Ctrl+S | ⌘S |
| Move in the tree | ↑ ↓ ← → Enter | ↑ ↓ ← → Enter |

Every other key goes to the terminal untouched.

## How it works

```
Windows                         WSL (or natively on macOS)
┌────────────┐  wsl.exe   ┌──────────────┐  Unix socket  ┌──────────────────────────────┐
│ Tauri app  │◄──stdio───►│ hive bridge  │◄─────────────►│ hive daemon                  │
│ React UI   │  (frames)  └──────────────┘               │  terminals (PTYs)            │
└────────────┘                                           │  worktrees, git, files       │
                                                         │  agent states                │
             claude in a Hive terminal                   │                              │
             └─ hooks ─► hive hook <event> ─────────────►└──────────────────────────────┘
```

- One Rust binary, `hive`, is the service (`hive daemon`), the stdio relay the app runs (`hive bridge`), the hook receiver (`hive hook <event>`) and a worktree CLI (`hive worktree create | list | remove`).
- The app starts `hive bridge` (through `wsl.exe` on Windows, `/bin/sh` on macOS). The bridge starts the daemon if needed; a version handshake refuses a mismatched pair. The daemon lives as long as the app connection: when the app closes, every terminal ends.
- Hive's terminals put a small `claude` wrapper first on `PATH`. It runs the real `claude` with `--settings <hive-hooks.json>`, which merges Hive's hooks with yours. Your global Claude Code settings are never touched, and `claude` outside Hive is unaffected.
- All domain logic is in Rust; the React frontend only renders what the service sends.

Details: [`docs/architecture.md`](docs/architecture.md). Decisions: [`docs/hive.md`](docs/hive.md) (Portuguese).

## Development

### Layout

| Path | What |
|---|---|
| `crates/hive` | The `hive` binary: daemon, bridge, hooks, terminals, worktrees, git, files, sessions |
| `crates/hive-protocol` | Frame codec, control messages, handshake, event model; shared with the app |
| `src-tauri` | The Tauri app (`hive-app`): spawns the bridge, relays frames to the UI, updater |
| `src` | React + TypeScript UI; `src/transport/mock.ts` is an in-browser fake service |
| `e2e` | Playwright specs against the mock transport |
| `scripts` | `win-dev.sh`, `gates.sh`, `release.sh` |
| `docs` | Decisions, architecture, UI reference, prototype |

### Prerequisites

- Rust stable (`rustup`), [bun](https://bun.sh), fish and git.
- To build the Windows app from WSL: `sudo apt install clang lld llvm`, `rustup target add x86_64-pc-windows-msvc`, `cargo install cargo-xwin --locked`.
- For the gates: `cargo-llvm-cov`, `cargo-mutants`, `cargo-deny`, `cargo-machete`.

```sh
bun install
```

### Running

- **Windows app from WSL:** `scripts/win-dev.sh` cross-compiles the app with `cargo xwin`, builds `target/debug/hive`, points the app's bridge at it (`HIVE_BRIDGE`, `HIVE_WSL_DISTRO`), starts Vite on port 1420 (hot reload) and launches the app through PowerShell.
- **UI in a browser:** `bun run dev`, then open `http://localhost:1420`. Outside Tauri the mock transport stands in for the service. Scenarios: `?mock=empty`, `?mock=states` (every agent state), `?mock=mismatch`, `?mock=disconnected`, `?mock=update`, `?mock=load` (20-terminal load test).
- **Service only:** `cargo build -p hive` gives `target/debug/hive`.

### Tests and quality gates

Every change must pass these (see `CLAUDE.md`):

```sh
# Rust (scripts/gates.sh runs them all; MUTANTS=0 skips mutation testing, BASE=<rev> sets its diff base)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --locked
cargo deny check
cargo machete
cargo llvm-cov --workspace --fail-under-lines 100 --ignore-filename-regex 'src-tauri/src/main\.rs|crates/hive/src/macos\.rs'
cargo mutants --in-diff <(git diff main -- '*.rs')    # no missed mutants

# Frontend
bun install --frozen-lockfile
bun run lint && bun run typecheck
bun test --coverage     # 100% line threshold in bunfig.toml
bun run e2e             # Playwright; needs libnss3 and libnspr4
```

- 100% line coverage in Rust and in the frontend, plus mutation testing on the changed code. Excluded files are listed with their reasons in [`COVERAGE_EXCLUSIONS.md`](COVERAGE_EXCLUSIONS.md).
- Integration tests run the real `hive` with a temporary `HOME`/`XDG_*` and a stand-in `claude`; they never touch your environment.
- CI: [`macos.yml`](.github/workflows/macos.yml) runs on every push to `main` (clippy, coverage and mutants of the macOS-only code, `bun test`, a release bundle). [`release.yml`](.github/workflows/release.yml) runs on `v*` tags.

### Releasing

```sh
scripts/release.sh 0.1.3          # bumps every manifest, commits and tags v0.1.3
git push origin main v0.1.3       # starts release.yml
```

The release workflow builds the Linux `hive` and bundles it into the Windows NSIS installer, then builds the macOS `.dmg` with the macOS `hive` inside, signs the updater artifacts and publishes them with `latest.json`. It needs the `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) repository secrets; the matching public key is in `src-tauri/tauri.conf.json`.

### Dependencies

Only through the package managers' CLIs, never by editing manifests or lockfiles: `cargo add -p <crate> <dep>` / `cargo remove` for Rust, and **bun only** (`bun add`, `bun add -d`, `bun remove`, `bunx`) for the frontend. No npm, pnpm, yarn or npx.

## Status

A personal project, at v0.1.x: expect rough edges.

## License

No license has been chosen yet.
