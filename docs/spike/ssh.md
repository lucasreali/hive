# Spike 26.1: Hive over SSH

Design task 26.1 (`TODO.md`). No product code. The experiments ran outside the repository and were deleted when the spike ended. This document holds the design, the measurements, the decisions the human must make, the proposed tasks (26.13 onwards) and the proposed changes to `docs/hive.md`. Written 2026-11-09 against `main` at `d961a5ff` (v0.28.0). It assumes 26.11 has landed, so the agent's browser and `hive-browser` are gone.

## 1. Recommendation in one paragraph

Run the **whole service on the remote host**, reached through the **same stdio bridge** Hive already uses for WSL: the app starts `ssh -T <options> -- <alias> <fixed command>`, and that command runs `~/.hive/bin/hive bridge` on the remote. Terminals, worktrees, git, hooks, the `claude` wrapper, the files and the session history all stay on the remote, so #37 holds unchanged there. Three things are new:

1. The app keeps **one link per host** (the local service plus each remote).
2. A **space belongs to one host**. Each remote service keeps its own spaces in its own `~/.hive`, and the app merges them into one space switcher.
3. A **remote service outlives a dropped link**: its terminals keep running, their output goes into a ring buffer, and the app reattaches when the link comes back. This one change to #18 is what makes SSH worth having on a laptop.

Hive stores no secrets. It uses the user's `ssh` and `~/.ssh/config` with `BatchMode=yes` and `StrictHostKeyChecking=yes`, so an unknown or changed host key is refused and never accepted silently.

## 2. Which Orca

"Orca" here means **stablyai/orca** (`github.com/stablyai/orca`, MIT, Electron + TypeScript). `docs/hive.md` already names it as Hive's main inspiration ("Referências para estudar"; "igual ao Orca"; requirement 3 about Orca being heavy), so the reference is not ambiguous. Sources read on 2026-11-09:

- the source tarball of `main` (`package.json` says 1.4.214, but the code mentions 1.4.225), mainly `src/main/ssh/**`, `src/main/managed-server/**` and `src/main/daemon/emulation`;
- its docs in `docs/site/content/docs/`: `ssh.mdx`, `ways-to-run.mdx` and `recipes/remote-worktrees.mdx`.

## 3. What Orca does

| Topic | Orca (from its source and docs) |
|---|---|
| Adding a remote | Settings → SSH → Add Target: host, user, port and key, or a pick from `~/.ssh/config` (with `Include`). It runs `ssh -G` to find duplicates. A worktree then picks **Run on: <host>**. |
| Transport | Two transports. The built-in one is the `ssh2` JS library with Orca's own `known_hosts` reader, key loading and passphrase prompts. The other is the **system OpenSSH**, used for `ProxyJump`, `ProxyCommand`, FIDO2 and GSSAPI (`ssh-transport-selection.ts`). System `ssh` gets `-T`, `BatchMode`, `ControlMaster=auto`, `ControlPersist=300` and `ServerAliveInterval=15` / `CountMax=3` (`system-ssh-args.ts`). |
| What runs on the remote | **orcad**, a whole Orca server on Node, deployed by Orca over the user's SSH connection (a Node runtime is downloaded when the host lacks one). It owns the projects, tabs and PTYs. The older design was a `relay.js` with `node-pty`, which needed a C/C++ toolchain on Linux hosts. That design is kept only in `legacy-relay/` for terminals still alive from it. The laptop reaches orcad through an SSH port forward, or through a stdio bridge "when sshd blocks forwarding" (`ssh-host-server-on-connect.ts`). |
| What the local app keeps | The target list, cached project metadata so the sidebar is not empty while connecting, and passphrases held in memory for the session. |
| Reconnect | A ladder of retries, 1, 2, 5, 5, 10, 10, 10, 30 and 30 s, then it gives up; a link stable for 60 s resets the ladder (`ssh-reconnect-ladder.ts`). A chip shows connected, reconnecting or disconnected, with a Connect button on the workspace card. |
| Dropped link, app closed | **Terminals live on.** "Keep terminals alive until reset" is on by default, or a timeout from 60 s to 7 days. On reattach the scrollback is replayed and full-screen apps get their **rendered frame** back from a headless terminal emulator on the server (`daemon/emulation`). |
| Host keys | On first contact Orca accepts and **remembers the key in its own store** (trust on first use) and shows the fingerprint. A changed key is rejected with an `ssh-keygen -R` hint. |
| Extras | Port forwarding with detection from `/proc/net/tcp`, file downloads over SFTP, "Open in VS Code / Cursor / Zed" over Remote-SSH, Windows hosts (PowerShell), and a mobile companion app. |
| Not supported | Linux hosts without build tools had no terminals under the old relay. Other editors and compound shell commands cannot open a remote path. |

### Copy

- **The service on the remote, the app as a viewer.** orcad is the same idea as Hive's service. Hive's version is easier: one static Rust binary, no Node, no `node-pty`, no toolchain on the host.
- **Terminals survive the link.** That is the reason to use SSH at all; without it a laptop lid kills the agents.
- **A reconnect ladder with a cap**, a connected/reconnecting/disconnected state on the host, and an explicit Reconnect action.
- **The user's `~/.ssh/config` as the source of hosts**, including `ProxyJump` and agents.
- **Screen restore on reattach** for full-screen programs (Claude Code is one). Hive should do it more simply first (section 6.4).

### Do not copy

- **An in-process SSH library and Orca's own known_hosts store.** It re-implements OpenSSH (keys, agents, FIDO, Kerberos, `Match`, `Include`) and needs a fallback to system `ssh` anyway. Hive only ever uses system `ssh`.
- **Trust on first use.** The task says an unknown key asks or refuses. Orca's silent accept is the behaviour we exclude.
- **Passphrases kept by the app.** Hive never sees one: keys come from the user's agent (macOS keychain).
- **Port forwarding, SFTP downloads, remote editors, Windows hosts and mobile** for now (section 9).
- **The relay-to-orcad migration machinery.** Orca has a large compatibility layer because it changed designs mid-life. Hive should ship one remote design only.

## 4. The shape that fits Hive: alternatives compared

| | **A. Whole service remote, bridge over `ssh` (recommended)** | B. Service local, PTYs or git over SSH | C. Service remote, its socket forwarded over SSH | D. Service local, files over SFTP or `sshfs` |
|---|---|---|---|---|
| Echo latency | One RTT plus about 0.3 ms (measured, section 7). | One RTT per keystroke too, but git, file and hook traffic each pay RTTs: every `git status` of the watcher crosses the link. | Same as A. | Every file stat and git call crosses the link. `sshfs` git on a big repo is seconds per status. |
| When the link drops | With 6.4 the remote service and its agents keep running, and the app reattaches. | The PTYs die with the SSH session unless a remote multiplexer (`tmux`) is added. Hooks have nowhere to go (`hive hook` needs a service on the same host). | Same as A, but a forwarded socket needs `AllowStreamLocalForwarding` (often off) and a second process to keep the forward up. | The mount hangs. Agents must run somewhere: a local `claude` on a network mount is not "remote". |
| Hooks and `claude` | Local to the remote service, unchanged (#19, #25, #27). | `hive hook` on the remote has no service; a callback channel is needed. | Unchanged. | Not applicable. |
| Watching files | `notify` on the remote, unchanged. | Remote polling, or a second remote agent. | Unchanged. | No inotify over `sshfs`. |
| Code change | App: one link per host and ssh/install. Service: survive a drop and reattach. Frontend: host-aware state. | A remote PTY and git layer, remote hooks, remote file watching: a second service in all but name. | Like A, plus forward management and sshd config requirements. | A whole VFS layer, and it still does not run agents remotely. |
| Fits #37 | Yes: all logic stays in the (remote) Rust service. | Splits the domain across hosts. | Yes. | No. |

**B and D are rejected**: each re-creates the remote service piece by piece and is slower. **C is rejected**: it is A with an extra moving part (a forwarded Unix socket, which sshd often forbids) and no gain, because Hive's bridge already turns stdio into the socket. Orca itself falls back to a stdio bridge for the same reason.

## 5. One app, several services

**Options.** (i) A separate window per remote. (ii) A space (#48) bound to a host. (iii) Projects of any space on any host.

**Recommendation: (ii), a space lives on one host.** Each remote service keeps its own `~/.hive`, with its spaces, projects, settings, accounts and session history. The app's space switcher lists the local spaces and then, per host, that host's spaces (`Work · gpu-box`). Reasons:

- **One source of truth per host.** In (iii) the local service would have to tell a remote one which projects to follow, and a space's identity (`CLAUDE_CONFIG_DIR`, `GH_CONFIG_DIR`, git name and e-mail, #48) consists of local paths that mean nothing on the remote.
- **The remote's identity is the remote's own** (its `claude` login, its git config, its `gh`), which #45 already requires: Hive never copies credentials.
- **#48 already does most of the UI.** Agents of every space stay alive and alert, and the bell, F8, the sound and notifications name the space. "Go to agent" switches spaces. With (ii), "space" grows a host and those paths carry over.
- **(i) splits the bell and the notifications** across windows, against "local and remote at the same time is the expected case", and adds multi-window state to the app.

What each part shows for a remote space:

| Part | Remote space |
|---|---|
| Sidebar | The same tree (Project → Worktree → Agent → Subagents), with the host in the space header. While the link is down: the last tree, dimmed, with "Reconnecting to gpu-box…" and a Reconnect button. |
| Status bar | A host chip per remote with a link (connected / reconnecting since 12 s / disconnected: <reason>), plus the remote's version warning or 22.1 warning. |
| Terminals | Unchanged while up. While down: an overlay "Reconnecting…", input blocked, the buffer kept. On reattach the missing output is replayed (6.4). |
| Notifications and bell (#5) | The same alerts, from the remote service's `agent_state`. Title "Claude · gpu-box" (space and host). After a reattach, one summary ("2 agents waiting on gpu-box") instead of a burst for every state change missed. |
| Session history | The remote's `~/.claude/projects` (the remote service reads it). "Copy Resume Command" works as is. Open log / reveal: see below. |
| Files panel, viewer, diff, editing | Unchanged: they are service requests. |
| Open in default app, Reveal in Finder, open the settings file, Open shared folder, files dropped from Finder | **Off for a remote space.** The path is not on this machine (`open_path` checks that the service sent it, but it would open a local path of the same name). The menu items are hidden; a drop types nothing and shows a toast. |
| Dev server preview (24.4, `http://localhost:<port>`) | **Off** until port forwarding exists (out for now, section 9). |
| Chat (#40) | Works: `claude -p` runs on the remote under the remote's login. |

The local service is not special in this model: it is the host "This Mac".

## 6. Design details

### 6.1 The link

- **Command:** `ssh -T -o BatchMode=yes -o StrictHostKeyChecking=yes -o ClearAllForwardings=yes -o ServerAliveInterval=10 -o ServerAliveCountMax=3 -o ConnectTimeout=15 -- <alias> <fixed remote command>`.
  - Separate arguments. The alias is validated: no leading `-`, no whitespace or control characters, at most 255 bytes. `--` comes before it.
  - Options given on the command line win over `~/.ssh/config`, so a user's `StrictHostKeyChecking no` cannot weaken Hive's link (verified, section 7.5).
  - `ClearAllForwardings` keeps a `LocalForward` in the user's config from failing the second connection.
  - Everything else (`HostName`, `User`, `Port`, `ProxyJump`, `IdentityFile`, the agent, `ControlMaster`) comes from the user's config.
- **The remote command.** sshd always joins the command into a string for the user's **login shell**, which may be fish, zsh, bash or csh. True "separate arguments" therefore stop at the local `ssh`. The command is a **constant**: `exec /bin/sh -c '<script>' '<version>'`, built from compile-time text only, single-quoted, one line, no `!`. Nothing from input is spliced in. The script runs `~/.hive/bin/hive bridge` when `hive --version` matches the app's version, and otherwise prints `hive-needs <uname -s> <uname -m>` and exits 3. One SSH round trip covers both the check and the bridge. Tested through a fish login shell (section 7.4).
- **Host keys.** An unknown key ("No ED25519 host key is known…") or a changed key ("REMOTE HOST IDENTIFICATION HAS CHANGED") makes `ssh` exit 255. The app recognizes the stderr text and shows: "gpu-box's host key is not known (or has changed). Run `ssh gpu-box` once in a terminal, check the fingerprint and accept it." The fingerprint comes from that `ssh` prompt, not from Hive. Hive never writes `known_hosts`.
- **Authentication.** Keys from the agent, or from files without a passphrase. `BatchMode=yes` turns a passphrase or password prompt into a failure: "gpu-box asked for a password or passphrase; add the key to your ssh agent (`ssh-add --apple-use-keychain`)". FIDO2 keys that need a touch are an open question (section 10).
- **`ControlMaster`.** Hive does not force it. A user who configured it gets it (it is the user's config). Measured at 100 ms RTT, reuse cuts connect time from about 1.0 s to about 0.32 s (section 7.3). Since Hive opens one long-lived `ssh` per host, reuse only helps reconnects and installs. A stale master after sleep can delay a reconnect until its own `ServerAlive` fires.
- **Detecting a dead link.** Without keepalives, a silent link was not noticed in 40 s. With `ServerAliveInterval=5`, `ServerAliveCountMax=3` it was noticed after 19.2 s (section 7.6). The proposal is 10 × 3, so about 30 s, plus an immediate reconnect when macOS reports wake (`NSWorkspaceDidWakeNotification`) or a network change. That is human question Q7.
- **Reconnect ladder.** 1, 2, 5, 10, 30 s, then every 30 s while the host has a live space, with no give-up while terminals are kept there. Errors that will not fix themselves (host key, authentication, version refused, unsupported platform) stop retrying and say why.

### 6.2 Installing and updating the remote service

- **Platforms:** Linux x86_64 and aarch64 first. macOS arm64 hosts work almost for free (the app already bundles that `hive`), but macOS hosts have their own catches: Orca documents the Local Network privacy loss for processes started by an SSH session, and the measured first run of a new binary was slow (section 7.4). Q5 asks about them.
- **Builds.** Today's release builds a glibc `hive` on ubuntu-22.04 (for WSL), which needs glibc ≥ 2.35. Remote hosts are older and more varied (RHEL 8 has 2.28, Debian 11 has 2.31), so the proposal is **static musl builds** `hive-linux-x86_64` and `hive-linux-aarch64`. Whether the WSL path switches to the musl build too is a separate choice; keeping it unchanged is the default.
- **Where the binary comes from: bundled in the app (recommended, Q4).** That is the same trust path as #29 (the signed installer carries the service), works offline and always matches the app's version, at about 10 MB more per app. The alternative is that the app downloads the matching release asset and checks a minisign signature: a smaller app but a new trust path and a network dependency. The remote downloading it itself is rejected: it needs internet access on the remote and runs `curl | sh`-shaped code.
- **Upload.** When the check says `hive-needs Linux x86_64`, the app runs a second constant command over `ssh` with the binary on stdin:
  1. `umask 077`, `mkdir -p ~/.hive/bin`, `cat > hive.new`;
  2. check the byte count, `chmod 755`;
  3. `hive.new --version` must print the expected version;
  4. `mv -f hive.new hive`, so a running service keeps its file.
  
  Then it reconnects. Measured: 4.7 MB in 2.9 s on loopback and in 7.6 s on a 1 MiB/s, 100 ms link (section 7.4). An unknown platform ("FreeBSD amd64", "Linux armv7l") is refused with a message.
- **Versions.** The handshake is unchanged: the remote `hive` must equal the app's version (#29). An app update means the next connect finds a different `--version`, uploads, and starts the new binary. A remote service of the old version that is still running (terminals kept) refuses the new app: see Q8.
- **What the remote needs:** `/bin/sh`, `uname`, `git`, and `claude` installed and logged in on the remote (#45: Hive never copies credentials). `gh` is optional. The "no `claude`" and "not logged in" cases surface as they do locally.
- **Shell on Linux.** Today Linux terminals run `fish -C` (#25, built for the human's WSL). A remote Linux host rarely has fish. The remote service must start the user's `$SHELL` as a login shell, as `terminal::login` already does on macOS (bash, zsh and fish are already handled there), with the bin dir first. The service's own `PATH` (for `git`, `claude -p`, `gh`) must come from that login shell too, because sshd's non-interactive `PATH` is minimal (measured: `/usr/bin:/bin:/usr/sbin:/sbin`). This is a real gap and task 26.14.

### 6.3 A remote service outlives a dropped link

Today the service ends with the app connection (#14, #18). Verified over `ssh`: when the link closed, the remote bridge, service and the terminal's running loop were gone within 6 s. That is right for a local app and wrong for a laptop on Wi-Fi. Proposal:

- **`hello` gains two optional fields** (protocol 1 stays compatible, as `welcome`'s `distro` did):
  - `keep_secs`: how long the service keeps its terminals after the app connection drops without saying goodbye. The app sends it only for a remote link; the local link sends nothing and keeps #18.
  - `app_id`: a random id per app launch.
- **A goodbye.** On quit the app sends `bye {end}`. With `end: true` the service ends everything, as #18 does. With `end: false` it keeps the terminals for `keep_secs`. The quit dialog for a host with running agents asks which one (Q2).
- **A stale connection is replaced.** After a silent drop, the remote sshd keeps the old session and its bridge until TCP gives up; sshd's `ClientAliveInterval` is 0 by default, so that can take hours. Measured: the remote bridge and service were still there after 40 s of a silent link, and today a new connection would get "another app is already connected". With the same `app_id`, or with the old connection idle past its own keepalive, the new connection **takes over** and the old one is closed. With a different `app_id`, today's refusal stays: two laptops on one host is out for now.
- **While detached:**
  - **No backpressure.** Today a terminal stops reading its PTY at 512 KiB unacknowledged (9.19, `terminal::Output`). Detached, that would **freeze every agent** once its output passed 512 KiB, so while no app is attached each terminal writes into a **ring buffer** instead (proposed 1 MiB per terminal, oldest bytes dropped).
  - Hooks, states, transcripts and worktrees keep being processed, as now.
  - After `keep_secs` with no app, the service ends as today (terminals ended, `open-sessions.json` written, so the next app offers to resume them).
- **The service's `app_gone` exit** (`daemon/mod.rs`) becomes "no app, and nothing kept, or `keep_secs` passed".

### 6.4 Reattach

- Every terminal counts the bytes it has produced, as a 64-bit offset. The app already counts what it acknowledged (9.19), so it knows the offset its xterm holds.
- After `welcome`, a service with kept terminals sends `terminals_kept [{channel, cwd, worktree, account, offset}]`, the service-side twin of the app's 18.21 `terminal_kept`. The app answers `attach_terminal {channel, from}`.
- If `from` is still inside the ring, only the missing bytes are sent. Otherwise the service sends `terminal_reset`; the app clears that xterm and gets the whole ring.
- Either way the app then sends a resize to the same size plus one column and back, so a full-screen program (Claude Code) redraws itself (SIGWINCH).
- This is simpler than Orca's headless emulator. A terminal emulator in Rust (`vt100`, `alacritty_terminal`) giving a true snapshot is a later option and a new dependency (Q9).
- Terminals the app no longer knows (the app restarted) reopen as tabs from `terminals_kept`, under their worktree.

## 7. Measurements

**Setup.** No Docker daemon was running (`/opt/homebrew/bin/docker` could not reach its socket), so nothing was started or installed. The lab ran entirely under `/var/tmp/hive-26.1`, which was deleted after the spike:

- **sshd:** `/usr/sbin/sshd` (OpenSSH 10.2p1) ran as the agent's own user on `127.0.0.1:42231`, with a temp config, temp host and client ed25519 keys, `UsePAM no`, passwords off, and `SetEnv` giving the session a temp `HOME`, `XDG_*`, `XDG_RUNTIME_DIR` and `HIVE_HOME`.
- **ssh client:** `ssh -F <temp config>` with `IdentityAgent none`, `IdentitiesOnly`, a temp `UserKnownHostsFile` and `HostKeyAlias`. The real `~/.ssh` was never read.
- **Remote host:** this Mac, not Linux. The remote login shell was the user's fish (from the passwd entry) with an empty temp config.
- **Slow link:** a Python asyncio TCP proxy delaying each direction by 50 ms (100 ms RTT) or 100 ms (200 ms RTT). A third proxy also capped the rate at 1 MiB/s with 100 ms RTT. A control file could make a proxy silent ("blackhole") or close every connection. This is not `tc netem`: no loss, no jitter.
- **Hive:** a throwaway `cargo build --release -p hive` of `d961a5ff` (v0.28.0), built outside the repo and copied to the temp `~/.hive/bin/hive`.
- **App stand-in:** a Python client speaking Hive's real frames (`hello`, `open_terminal`, terminal frames, `ack` every 64 KiB received) over the stdio of `ssh … hive bridge`. It answers fish's DA1 query as xterm.js would; fish 4 waits about 10 s for it otherwise.
- **What latency means here:** from writing a key frame to stdin until the **first output frame of that terminal** arrives. That includes ssh, the bridge, the service, the PTY and fish's echo, but **not** the Tauri app, the WebView or xterm's paint. Keys alternated `x` and backspace, 60 ms apart.

### 7.1 Echo latency (200 keys each)

| Link | p50 | p95 | min | max |
|---|---|---|---|---|
| Local `hive bridge`, no ssh | 0.2 ms | 0.6 ms | 0.1 ms | 42.8 ms |
| ssh, loopback | 0.4 ms | 0.9 ms | 0.2 ms | 31.5 ms |
| ssh, 100 ms RTT | 102.7 ms | 104.7 ms | 100.8 ms | 121.7 ms |
| ssh, 200 ms RTT | 202.7 ms | 203.5 ms | 200.9 ms | 213.6 ms |
| Baseline, no Hive: `ssh -tt host cat`, loopback | 3.7 ms | 23.3 ms | 0.1 ms | 41.5 ms |
| Baseline, no Hive: `ssh -tt host cat`, 100 ms RTT | 116.6 ms | 126.5 ms | 100.8 ms | 139.1 ms |
| Baseline, no Hive: `ssh -tt host cat`, 200 ms RTT | 205.5 ms | 224.5 ms | 200.9 ms | 243.5 ms |

Hive's path adds about 0.2 to 2.7 ms to the RTT. That is no worse than a plain interactive `ssh`; the baseline's worse tail is the client's tty, not the link.

### 7.2 Echo while another terminal floods (`yes`), and throughput

| Link | Echo p50 | Echo p95 | Echo max | Flood rate meanwhile |
|---|---|---|---|---|
| ssh, loopback | 0.9 ms | 29.9 ms | 63.8 ms | 30.3 MiB/s |
| ssh, 100 ms RTT | 112.7 ms | 178.9 ms | 219.6 ms | 3.0 MiB/s |
| ssh, 1 MiB/s and 100 ms RTT | 286.3 ms | 447.8 ms | 483.8 ms | 1.0 MiB/s |

60 keys each, 100 ms apart. On a slow link a flooding terminal **delays the others' echo**: all frames share one ssh stream and the flood fills its buffers, which is head-of-line blocking. The control-frame priority (#24) acts only inside the service's writer, not in ssh's or TCP's buffers.

Time for 50 MiB of output (`head -c 52428800 /dev/zero | tr '\0' x`) in one terminal:

| Link | Rate |
|---|---|
| Local, no ssh | 9.2 MiB/s |
| ssh, loopback | 8.7 MiB/s |
| ssh, 100 ms RTT | 3.3 MiB/s |
| ssh, 200 ms RTT | 2.2 MiB/s |
| ssh, 1 MiB/s and 100 ms RTT | 0.9 MiB/s |

With a delay the rate is **window-bound**: 512 KiB unacknowledged per terminal (9.19) divided by the RTT is about 5 MiB/s at 100 ms and 2.5 MiB/s at 200 ms. The real app acknowledges after xterm parses, so it will be a little lower. Q10 asks whether to raise the window for remote links.

### 7.3 Connecting (spawn of `ssh` until `welcome`; the service started by each bridge)

| Link | Three runs |
|---|---|
| ssh, loopback | 76, 73, 85 ms |
| ssh, 100 ms RTT | 974, 948, 950 ms |
| ssh, 200 ms RTT | 1870, 1882, 1846 ms |
| ssh, 100 ms RTT, `ControlMaster=auto` + `ControlPersist` | 1037 (opens the master), 319, 317 ms |

That is about 9 RTTs: TCP, key exchange, authentication, session, exec, the bridge starting the service, then hello and welcome.

### 7.4 Check and install over ssh (the proposed constant commands, through a fish login shell)

| Link | Check (missing `hive`) | Upload of 4,747,616 bytes | Check again (bridge runs) |
|---|---|---|---|
| Loopback | exit 3 in 84 ms, `hive-needs Darwin arm64` | 2.9 s | exit 0 in 106 ms |
| 1 MiB/s and 100 ms RTT | exit 3 in 879 ms | 7.6 s | exit 0 in 1014 ms |

The uploaded file was byte-identical (`cmp`). The loopback upload time includes the first run of a new unsigned binary (`--version`) on macOS, which macOS checks before running; it was not measured separately and should not be taken as a Linux number.

### 7.5 Host keys (`BatchMode=yes`, `StrictHostKeyChecking=yes`)

| Case | Result |
|---|---|
| Unknown key | Refused, exit 255: "No ED25519 host key is known for hive-lab and you have requested strict checking." |
| Changed key | Refused, exit 255, with OpenSSH's "REMOTE HOST IDENTIFICATION HAS CHANGED" banner. |
| Changed key, with `StrictHostKeyChecking=no` given after Hive's option (as a user's config would) | Still refused: the first value wins. |

### 7.6 Dropped links, today's code

| Case | App side | Remote side |
|---|---|---|
| Link closed (both ends reset) | `ssh` exits 255 at once: "Connection … closed by remote host". | Bridge, service and the terminal's running loop gone within 6 s (#18 as designed). |
| Link silent 40 s, no keepalive | Not noticed in 40 s. | Bridge, service and the loop still running, so a reconnect would be refused as "another app is already connected". |
| Link silent 40 s, `ServerAliveInterval=5`, `CountMax=3` | Noticed after 19.2 s: "Timeout, server not responding". | Still running after 40 s. Gone only once the proxy closed the connection. |

### 7.7 Not measured

These are open questions in section 10:

- a real Linux remote;
- packet loss and jitter (no `tc netem` without Docker);
- latency to paint in the real app;
- reattach, which does not exist yet;
- laptop sleep;
- FIDO2 keys;
- a real ProxyJump chain.

## 8. Risks and security (for `security-reviewer`)

A **remote is a less trusted peer than the local service.** It is someone else's machine, a shared box or a compromised VM. Whoever controls it can send the app any frame.

**Limits that already apply:**

- frames of at most 4 MiB (`MAX_PAYLOAD`), with a codec that never panics;
- unknown frame types refused;
- JSON control messages parsed into typed enums;
- `READ_LIMIT` 1 MiB file ranges;
- the app's `STDERR_LIMIT` of 16 KiB for the bridge's error text;
- `open_path` allowing only paths the service just sent;
- the WebView never opening a URL by itself.

**New exposure, and what to do about it:**

1. **`editor_target`, `session_located`, `open_settings_file`, `open_shared`.** A remote could name a local path or an executable for the app to open. For a remote link the app **refuses every open or reveal**, whatever the service sends (section 5).
2. **URLs** (pull requests, Actions, the update link). A remote could send `file://`, `javascript:` or a custom scheme. Only `https://` URLs are opened, for every link, with a test for a remote.
3. **Notifications and text.** Agent titles, badges and errors from a remote are shown as text only (already true in React). Lengths are clipped by the app too, not only by the remote service.
4. **Memory.** A remote can flood control messages (unbounded `mpsc` in the app's link). Per-link caps on queued control frames and on `projects` and `sessions` list sizes are needed.
5. **Terminal escape sequences.** OSC 52 clipboard writes, OSC 8 links and title changes from a remote program reach xterm as they would from a local one. The same xterm settings apply. Clipboard reads stay off.
6. **The `ssh` command line.** Only the validated alias is variable; the remote command is constant. `ssh` is never run through a shell.
7. **Credentials.** Hive never reads keys, passphrases or `known_hosts`, and never forwards the agent by itself. `ForwardAgent` in the user's config is the user's choice, and a compromised remote could use a forwarded agent; the docs should say so.
8. **The uploaded binary.** It comes from the signed app bundle (if Q3 is accepted) and is checked by size and `--version` before the rename. Its folder is `0700`. A remote that already holds a different `hive` with the right `--version` text is running code the user does not control. That is inherent: the user already trusts the remote's shell. It should be said in the docs.
9. **Reattach takeover by `app_id`.** The id travels only inside the user's own SSH session to the user's own socket (`Paths::connect` checks the peer uid), so it is not a credential.

## 9. Out for now

Say these plainly:

- Windows hosts and WSL behind ssh;
- password, keyboard-interactive and OTP prompts;
- passphrase prompts by Hive;
- a remote with no POSIX `/bin/sh`;
- FreeBSD and 32-bit ARM;
- port forwarding and the dev-server preview of a remote;
- open, reveal or drop of remote files from Finder;
- SFTP downloads;
- "Open in VS Code" over Remote-SSH;
- two apps on one remote;
- Hive on Windows connecting to a remote (D17.0 freezes Windows-only work; the code should not prevent it);
- a remote host list synced between machines.

## 10. Open questions (not settled by this spike)

1. FIDO2 or security-key `ssh` with `BatchMode=yes`: does the touch prompt work with no tty?
2. Packet loss: does a lossy link (2–5 %) make echo unusable before the keepalive gives up? This needs `tc netem` (Docker or a Linux runner).
3. Latency to paint in the real app (Tauri, WebView, xterm) over 100 ms: expected to be RTT + a few ms, not measured.
4. Laptop sleep on macOS: does the `ssh` child survive App Nap or sleep, or always die? How fast is wake to reconnect?
5. Linux musl `hive` on old glibc hosts and in containers: smoke test in CI (task 26.13).
6. Head-of-line blocking (section 7.2): if it matters in practice, the app could ask the service for less terminal output per flush on remote links, or a terminal could get a smaller window while others are interactive. Measure in the real app first.

## 11. Decisions for the human

Each is phrased for the questionnaire, with the recommended answer first.

| # | Question | Recommended answer | Alternatives |
|---|---|---|---|
| Q1 | How does the app show a remote host? | A space belongs to one host. The remote keeps its own spaces in its `~/.hive`, and the switcher lists them as "Space · host". | A separate window per host; or projects of any space on any host. |
| Q2 | When the link drops or Hive quits, what happens to the remote terminals and agents? | They keep running. Quit asks "Keep N agents running on gpu-box?" (default keep), and a dropped link always keeps them. | Remote follows #18 (they end); or keep only on a dropped link, never on quit. |
| Q3 | How long does a remote keep terminals with no app? | 24 hours by default, a setting from 1 minute to 7 days. After that they end and are offered for resume. | Until ended by hand (Orca's default); or 1 hour. |
| Q4 | Where does the remote `hive` come from? | Bundled in the macOS app (Linux x86_64 + aarch64 musl, about 10 MB more) and uploaded over ssh. | Downloaded by the app from the release and checked with minisign; or downloaded by the remote. |
| Q5 | Which remote platforms first? | Linux x86_64 and aarch64; macOS arm64 hosts as best effort, since the binary is already there. | Linux only; or Linux and macOS both fully supported. |
| Q6 | Unknown or changed host key? | Refuse, and tell the user to run `ssh <alias>` once in a terminal to check and accept it. Hive never writes `known_hosts`. | Show the fingerprint in Hive and accept it there (Hive would write `known_hosts`). |
| Q7 | How is a dead link noticed? | `ServerAliveInterval=10`, `ServerAliveCountMax=3` (about 30 s), plus an immediate reconnect on wake or network change. | 5 × 3 (about 15 s); or 15 × 3 (Orca's, about 45 s). |
| Q8 | After an app update, a remote service of the old version still holds kept terminals. What then? | Show both versions and offer "Restart the remote service (ends N terminals)" or "Keep them and stay disconnected". | Always restart it; or keep a protocol-compatible old service working (relaxes #29). |
| Q9 | Screen restore after a reattach? | Replay the ring buffer and force a redraw by resize, no new dependency. | A terminal emulator in the service (`alacritty_terminal` or `vt100`) for a true snapshot. |
| Q10 | Raise the 512 KiB unacknowledged window for remote links (throughput 3.3 MiB/s at 100 ms)? | No, keep 9.19's window; measure in the real app first. | 2 MiB for remote links (about 4× the rate, more memory per terminal). |
| Q11 | Linux terminals on a remote: which shell? | The user's `$SHELL` as a login shell, as on macOS. fish stays for WSL. | fish required on the remote. |
| Q12 | Does Hive set `ControlMaster`? | No: the user's config decides. | Hive adds `ControlMaster=auto` with its own `ControlPath`. |

## 12. Proposed tasks (written into `TODO.md` only after the human approves)

Each is sized like the other stages' tasks. All are Rust plus frontend unless noted; every `crates/hive` or `src-tauri` change gets `security-reviewer`.

- **26.13 Remote Linux builds.** Static musl `hive` for x86_64 and aarch64 in `ci.yml` and `release.yml` (`cross` or `cargo zigbuild`, a dependency to approve). Bundled in the macOS app per Q4, with `macos-bundle.sh check` verifying them. A smoke test runs each in an old-glibc container (`hive --version`, `hive daemon` start and stop). README: supported remote platforms.
- **26.14 Login shell on Linux.** The service started over ssh (`hive bridge --remote` tells the daemon) runs terminals as the user's `$SHELL` login shell with the bin dir first (`terminal::login` made portable), and takes its own `PATH` from that shell as macOS does. WSL keeps `fish -C`. Tests: bash, zsh, fish and dash login shells in integration tests (`common::Env`).
- **26.15 The service outlives a dropped link.** `hello {keep_secs, app_id}` and `bye {end}` (protocol 1, optional fields). Takeover of a stale connection by the same `app_id`. A ring buffer per terminal while detached (no backpressure). The end after `keep_secs`. `docs/architecture.md` (handshake, App disconnect). Integration tests: drop the app socket mid-flood, and the agent keeps writing; takeover; the end after the grace time; the ring overflow.
- **26.16 Reattach.** Output offsets, `terminals_kept`, `attach_terminal {from}`, `terminal_reset`, and the redraw resize in the app. Tests in Rust (offsets inside and outside the ring) and bun (xterm cleared and replayed, tabs reopened).
- **26.17 The app's links.** `src-tauri`:
  - one `Link` per host;
  - the `ssh` command builder (alias validation, fixed options), the constant check and install commands, the upload;
  - stderr classified into host key, authentication, network, platform and version;
  - the reconnect ladder, wake and network-change reconnect;
  - UI messages tagged with `host`, and channels mapped per link.
  
  Tests with a fake `ssh` program (a C fixture like `fake_claude.c`) that replays each stderr case and pipes to a test service.
- **26.18 Hosts in the UI.** Settings → Hosts (add by `~/.ssh/config` alias, test, remove; kept in the local service's settings), the space switcher with remote spaces, the host chip, the reconnect overlay on terminals, notifications and bell naming the host, the summary after a reattach, and the remote-only refusals (open, reveal, drop, preview). Mock transport with a second host. e2e for the switcher and the overlay.
- **26.19 Hostile remote.** The app-side limits of section 8 (refuse opens for remote links, `https` only, caps on queued control frames and list sizes, clipped text), with tests that feed malformed and oversized frames through a fake remote. `security-reviewer` on the whole remote path.
- **26.20 A real remote in CI.** A Linux job running sshd in a container with temp keys, `tc netem` 100 ms delay and 2 % loss. Checks: install, echo bound (p95 below RTT + 50 ms), drop and reattach, host key refusal. It answers open questions 2 and 5.
- **26.21 Docs and hand check.** README "Remote hosts" (requirements, `ssh-add`, host keys, what is off for remotes, `ForwardAgent` warning), `docs/architecture.md`, and a manual checklist on a real Linux host (sleep, ProxyJump, FIDO2, update with kept terminals). ⏸ Checkpoint.

## 13. Changes `docs/hive.md` would need (the human's; never edited here)

| Item | Today | Proposed |
|---|---|---|
| #3 | App on Windows + service in WSL | Add: the service can also run on a remote host over SSH; the app talks to several services (local and one per remote). |
| #14 | Service with the app's lifetime, bridge via `wsl.exe` | Add: a remote service is started by `ssh -T … -- <alias> <fixed command>` running `~/.hive/bin/hive bridge`; it outlives a dropped link for the grace time (Q3). |
| #18 | Terminals and agents end when the app closes, even in a crash; "no reconnection or snapshot" | Exception for remote hosts: they keep running after a dropped link (and after quit, per Q2) for the grace time; the app reattaches and replays the missing output (Q9). |
| #24 | Handshake compares versions | `hello` gains optional `keep_secs` and `app_id`; new `bye`, `terminals_kept`, `attach_terminal {from}`, `terminal_reset`. |
| #25 | `claude` wrapper put first on `PATH` through `fish -C` | On a remote Linux host, through the user's login shell, as on macOS (Q11). |
| #26 | Single binary in Hive's bin folder | The same `~/.hive/bin/hive` on each remote. |
| #29 | Distribution and versions | The macOS app also carries the Linux musl `hive` for x86_64 and aarch64 (Q4), uploaded over ssh when the remote's `--version` differs; an old remote service with kept terminals: Q8. |
| #45 | Hive never reads credentials | Also never stores ssh passwords, passphrases or keys, never writes `known_hosts`; the remote's own `claude` login is used. |
| #48 | A space groups projects and an identity | A space belongs to one host; a remote's spaces live in its `~/.hive` and show as "Space · host" (Q1). |
| #5 | Notifications | They name the host for a remote agent; one summary after a reattach. |
| New | — | SSH link: the user's `ssh` and `~/.ssh/config`, `BatchMode=yes`, `StrictHostKeyChecking=yes` (unknown or changed key refused, Q6), `ServerAlive` 10 × 3 (Q7), no forced `ControlMaster` (Q12); out for now: section 9. |
