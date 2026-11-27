# Spike 28.14: several people on one remote service

Design task 28.14 (`TODO.md`). No product code. Written 2026-11-27 against `main` at `89ec3828`, after Stage 26 (remote services over ssh) and Stage 27 (seen marks, anchored comments). D27.1 is the starting point: teammates reach **the same remote service** of Stage 26 with their own app, over their own `ssh`. There is no Hive server, no Hive account and no network of Hive's own (#14). This document holds the design, one recommendation per point, what was tried and what was not, the questions for the human, and the proposed tasks. The tasks go into Stage 29 (29.4 onwards) only after the human approves.

## 1. Recommendation in one paragraph

**The owner shares their remote Hive with named teammates.** Each person logs in to the host as their **own Unix user**. The owner's service (Stage 26, unchanged in what it runs) opens a **second Unix socket** that a Unix group can reach. A teammate's own `hive bridge --join <owner>` connects to it. The service learns **who is connecting from the kernel** (the socket peer's uid, mapped to a login name) and checks that name against the owner's **Sharing** list. That list gives each name a role: **Viewer** or **Collaborator**. Everything still runs **as the owner**: terminals, agents, git, the `claude` login. Teammates see only the spaces the owner shares.

Each terminal has one **driver**, at first the person who opened it. Only the driver types. "Hand over" passes the driver role to someone else, and the owner can always take a terminal back. The PTY takes the **driver's size**, and watchers see it scaled down when their pane is smaller.

**Presence** (who is connected, and which worktree and tab each one has open) comes from the service. "Follow <name>" is UI state built on top of it. Seen marks are **per person**. Comments are **shared**, and the service fills in their author. An `@name` in a comment puts a **mention** in that person's inbox and sends them a notification.

Every teammate's app is **as untrusted as a hostile remote** (26.19), in both directions. Every request is authorized in the service, every connection gets its own caps, and nothing new listens on the network.

## 2. What exists today (the constraints)

Read from the code at `89ec3828`:

| Fact | Where | What it means for several apps |
|---|---|---|
| **One app per service.** `State.app` is a single `Option<sender>`. A second app with another `app_id` gets "another app is already connected". | `daemon/app.rs` `app_connection` | The core change: the service needs one session per connection. |
| The socket is `0600` in a runtime folder only the user can reach. Both ends check that the peer has **the same uid** (`check_peer`), and `hive hook` and the app share that socket. | `paths.rs`, `daemon/claim.rs` | Another Unix user cannot reach the service today. Sharing needs a second socket with its own rules. |
| **Per-connection state is global.** There is one watched worktree (`watching`, from `watch_worktree`), one `listed`, one `scripted`, one `view`/`visible`. | `daemon/mod.rs` `State` | All of these move into the per-connection session. |
| **The app picks the terminal channel ids** (`open_terminal` on the channel it chose). Ids are shared across the app's links (`Link::ids`). Agent messages travel on the terminal's channel, and `View.terminal` names a channel. | protocol, `src-tauri/src/remote.rs` | With several apps, ids clash. Each connection needs its own channel map. |
| Output pacing: each terminal stops reading its PTY at 512 KiB the app has not acknowledged (9.19). A terminal of a kept connection writes into a 1 MiB **ring** with `u64` offsets (26.15). `attach_terminal {from}` and `terminal_reset` replay that ring (26.16). | `terminal.rs`, `ring.rs` | The ring is the fan-out mechanism. A watcher is just one more reader at its own offset. |
| `pending` and `notify` are computed against the one app's `View` (#5, 5.6). | `daemon/agents.rs` | They become per person. |
| Seen marks are keyed by worktree and diff base, with no person in the key (`<data>/seen.json`). | `hive::seen` | The key gains the person. |
| `ReviewComment.author` exists, filled with the service's user name (`comments::author()`). The doc comment says "Stage 28 shares comments". | protocol, `hive::comments` | The author becomes the connected person, set by the service. |
| The app already treats a remote service as hostile: caps on rates, lists, text and queues, opens refused, only `https` URLs opened (26.19, `src-tauri/src/limits.rs`). | `limits.rs` | Holds unchanged for a shared service. What is new is the **service** side. |
| xterm.js has no OSC 52 handler (no `ClipboardAddon`). | `src/` | A program run by a teammate cannot write the owner's clipboard. This must stay true. |

## 3. Who is who

### Options

| | **A. One Unix user each; the owner's service shared to a group (recommended)** | B. One shared Unix user (`team@host`), a name per app | C. One Unix user each, each person's own service, services federated on the host |
|---|---|---|---|
| Identity | **The kernel**: `SO_PEERCRED` gives the bridge's uid, and `getpwuid` maps it to a login name. No one can claim to be someone else. | Self-declared in `hello` (a setting "Your name"), or the ssh key, if the admin sets `ExposeAuthInfo yes` (lab, section 10). Either way it can be forged: everyone has a shell as `team` and can edit `authorized_keys` or `SSH_USER_AUTH`. | The kernel, as A. |
| What a teammate can do outside Hive | Only what their own Unix user can do. They have no shell as the owner unless the owner gives them one. | **Everything the account can do**: read every file, every credential, kill every agent. Hive's roles would be a courtesy only. | Only what their own user can do. |
| Credentials (#45) | The owner's `~/.claude`, `gh` and keys stay in the owner's `0700` home. Hive never sends them, and the kernel stops a teammate from reading them. | One home for everyone: per-person `CLAUDE_CONFIG_DIR`s are all readable by all. "Never shown" would hold in Hive's UI only. | Each person's own, isolated. |
| Whose `claude` runs | The owner's: every agent in the service is the owner's. | The account's. | Each person's own. |
| Host setup | The admin adds teammates to a Unix group the owner is in (one `usermod -aG`). | One account, keys in its `authorized_keys`. Nothing else, or `ExposeAuthInfo` for key identity. | Group-writable project folders (setgid folders, `core.sharedRepository=group`, a umask), since two services write the same repository. |
| Code | One more listener with its own checks. Per-connection sessions (needed by every option). | No socket change, but identity has to be done anyway. | A service-to-service protocol. Two services watching and writing one repository (`index.lock` races, two `comments.json` owners). Relays for presence, review and terminals. By far the largest. |
| Fits D27.1 ("the same remote service") | Yes. | Yes. | **No**: several services. |

**Recommendation: A.** It is the only one where identity and the "never shown" rule are enforced by Unix, not by Hive's good manners, and it needs no sshd configuration. **B is rejected**: it is cheaper only because it gives up on identity. **C is rejected**: it contradicts D27.1 and costs several times more.

A variant of A covers a team that wants agents on a **team** login instead of one person's: the owner is a dedicated account (`hive-team`), which someone logs in to once to start it. Nothing in the design changes.

### How A works

- **Sharing is off by default.** With no Sharing list, nothing changes: the service has one socket, one app, and the refusal of a second app stays. A local service (the Mac) is **never** shared. Sharing is a remote Linux service only (Stage 26 Q5), because sharing needs other people's logins on the same host.
- **Settings → Sharing**, owner only:
  - a Unix group, chosen from the owner's own groups (`getgroups`, no root needed);
  - the shared spaces;
  - the people, each with a login name (checked against `^[a-z_][a-z0-9_-]{0,31}$` and `getpwnam`), a display name, a git name and e-mail (used for their terminals, section 5.5) and a role.
- **The shared socket.** The owner's service creates `/tmp/hive-shared-<owner uid>/` with mode `0710` and group = the sharing group (a user may `chown` to their own group without root). Inside it, `hive.sock` has mode `0660`. On Linux, connecting to a Unix socket needs write permission on the socket file, so the group is the first gate. The service removes the folder and socket when sharing is turned off and when the service ends.
  - `/tmp` is shared, so both sides check it. The joining bridge refuses a folder that is not owned by the owner's uid or that others can write to. It also refuses a socket whose peer uid (`peer_cred`) is not the owner's. This is the same idea as today's `check_peer`, with a named uid instead of "mine". A folder another user planted first is refused, and sharing then fails with that reason.
- **The second gate is the service.** It accepts a shared-socket connection only when the peer's uid maps to a name in the Sharing list. Group membership alone is not enough. The shared socket accepts **only `role: app`**: a teammate never sends hook events, which would forge agent states.
- **Joining.** Settings → Hosts gains "Join someone's Hive": the ssh alias the teammate uses (their own login on that host) and the owner's login name. The remote command stays the 26.17 constant, with one more positional argument, the validated owner name, given as a separate `sh -c` argument (`'<version>' '<owner>'`), never spliced in. It runs the **teammate's own** `~/.hive/bin/hive bridge --join <owner>`, installed and version-checked as 26.17 does. If the owner's Hive is not running, the teammate sees "alice's Hive is not running on gpu-box" and the link retries on the 26.17 ladder. A guest never starts the owner's service: it cannot, since it runs as the owner.
- **Versions.** #29's strict check stays: the service and every app must be the same version. One teammate updating first cannot join until the owner updates. The 26.17 "Restart the remote service" offer is **owner only**. A guest sees "alice's Hive is v0.33.0, yours is v0.34.0; ask alice to update". (Q5)
- **What a teammate can see.** Only the shared spaces: their projects, worktrees, agents, terminals, sessions, comments and pull requests.
  - Never shown to a teammate: Settings (accounts, `gh` accounts, Claude logins, scripts, the settings file), usage windows, diagnostics, the shared folder (`open_shared`), Uninstall, and the owner's other spaces.
  - An account appears only as its name ("alice's account"), never its folder.

### Roles

The capability table, enforced by the service for every request:

| Action | Owner | Collaborator | Viewer |
|---|---|---|---|
| See shared spaces: tree, states, terminals (watching), files, diffs, sessions list, pull requests and runs | ✓ | ✓ | ✓ |
| Presence, follow, seen marks (their own), comments (add, resolve, reopen; delete their own) | ✓ | ✓ | ✓ |
| Type in a terminal they drive; receive a hand over | ✓ | ✓ | — |
| Open terminals and chats (they run **as the owner**, on the owner's `claude` login) | ✓ | ✓ | — |
| Save, create, rename, move and delete files; create, rename and remove worktrees; run project scripts | ✓ | ✓ | — |
| Commit | ✓ | ✓ (their git name and e-mail as author, section 5.5) | — |
| Push, create a pull request, act on a pull request or run, run a workflow (with the owner's `gh` login) | ✓ | — (Q4) | — |
| Delete anyone's comment; take any terminal back; end the service; change any setting; add or remove projects and spaces | ✓ | — | — |

## 4. Several apps on one service

### What each app receives

The service keeps one **session** per connection: its person, role, `app_id`, channel map, watched worktree, view, `listed` and `scripted` caches, and its bounded output queue. Every message falls into one of three kinds:

| Kind | Examples | Goes to |
|---|---|---|
| **Shared state** | `projects`, `spaces` (the shared ones only), `agent_*` (except per-person fields), `worktree_status`, `comments`, the terminal list (section 5), `presence` | Every session that may see it: the owner's, plus guests for shared spaces only. |
| **Per person** | `agent_state.pending` and `notify`, `changes.files[].seen`, mentions | Each session gets its own computed copy. |
| **Reply** | `file`, `files`, `changes`, `branches`, `sessions`, `search_results`, the `*_failed` replies | The session that asked, only. |

`to_app` splits into `to_session(id, …)` and `to_shared(space, …)`. Messages that carry a terminal channel (frames on a terminal's channel, `View.terminal`, `terminals_kept`) are translated through each session's channel map, in both directions.

### Protocol changes (still protocol 1: app and service are always the same version, #29)

| Message | Change |
|---|---|
| `welcome` | Gains `you {name, role}` and `shared: bool`. |
| `view` | Gains `worktree`, `tab` (`terminal <id>`, `file <path>`, `diff <path>` or none) and `line` (the first line in view). This is the source of presence. |
| `presence {people: [{name, display, app, worktree, tab, line, focused, away}]}` | New. Service → app, sent when it changes, at most 4 per second per session, shared spaces only. |
| `terminals {terminals: [{id, opener, driver, cwd, worktree, cols, rows, account}]}` | New. The service-wide terminal list, with **service-assigned ids**. Sent at `welcome` and when it changes. |
| `watch_terminal {terminal, from}` / `unwatch_terminal` | New. App → service, on a channel the app picks: start reading a terminal it did not open, from offset `from` (0 for the whole ring). This reuses 26.16's `attach_terminal` path. |
| `hand_over {to}`, `take_back` | New. App → service, on the terminal's channel. Answered by `terminals`. |
| `open_terminal` | Unchanged for the app, which still picks its own channel. The service maps it to a new service-wide id. |
| `mark_seen`, `changes` | Unchanged on the wire: the person comes from the session. |
| `add_comment` | Unchanged; `author` is set by the service. `ReviewComment` gains `mentions: [name]` and `author_name` (the display name). |
| `mentions {items: [{id, worktree, comment, by, ms, read}]}`, `read_mention {id}` | New (section 7). |
| `not_allowed {request}` | New. The reply to a request the role does not allow. The app shows it; it is never silent. |
| `bye {end}` | From a guest, `end` is ignored: a guest's goodbye ends only their session. Only the owner's `bye {end: true}` ends terminals. |

### Limits per connection (the service side of 26.19)

Today's control queue to the app is an **unbounded** `mpsc`, which is safe with one trusted app. With teammates, a slow or stuck guest app must not grow the owner's service without bound. Each session gets:

- **An output queue** capped at 1024 frames and 64 MiB, the same numbers as the app's `QUEUE_FRAMES` and `QUEUE_BYTES`. A session past its cap is **dropped**, with `disconnected {kind: limit}` shown on that app. The app reconnects and resyncs. The others never wait on it.
- **A request rate**: 1000 control frames per second and 16 MiB per second, as the app's `RATE`. Past that, the session is dropped.
- **Terminal input**: frames from a session that is not the terminal's driver are **dropped silently**, and a resize from a watcher is ignored.
- **At most 8 sessions in all, and 2 per person** (two laptops). A ninth connection is refused with a message.
- **Comments**: today's per-worktree cap, plus at most 30 per minute per person. **Mentions**: 100 kept per person (the inbox's size).
- **Presence**: 4 messages per second out; `view` in at most 20 per second per session.
- Everything else (file sizes, `READ_LIMIT`, list caps) already holds per request.

### When one app drops

- **Its session ends and its presence goes**: "bob (reconnecting)" for 30 s, then gone. Followers of bob stop following.
- **Terminals it drove keep running**, as a kept terminal does (26.15). The driver role stays with bob, so he gets it back when he reconnects. Until then the owner can take it back, and so can its opener.
- **Reconnect**: the same person with the same `app_id` takes over their stale session (26.15's takeover, now keyed by person and `app_id`). A different `app_id` is a second app of that person, within the cap of 2.
- **The service's life.** The service ends when **no session is left** and `keep_secs` has passed (26.15). The keep time is the owner's. A guest's connection keeps the service alive while it is connected. If the owner quits with "Keep running" while guests are connected, they keep watching.

## 5. Terminals

### 5.1 Who types

| Option | For | Against |
|---|---|---|
| Only the opener, ever (Delta) | Simple, and nobody types into your agent. | You cannot let a teammate answer your agent's permission prompt while you are away. |
| **The driver: the opener at first, "Hand over" to one person, "Take back" (recommended)** | Delta's rule by default, plus explicit pairing. One typist at a time, so no interleaved keystrokes. | A little more UI. |
| Anyone with a role may type | Like a shared tmux. | Two people's keys interleave, and the agent gets garbage. |

**Recommendation: the driver.**

- Hand over goes only to a Collaborator who is connected.
- The driver, the opener and the owner can take back at any time, at once, with no asking. The owner's power is that it is the owner's account.
- The tab shows the driver ("bob is typing").
- A watcher who presses a key sees a quiet one-line note: "alice is driving · Ask to drive". "Ask" sends a request the driver accepts or ignores (Q3).
- All of this is enforced in the service. The app only renders it.

### 5.2 PTY size

| Option | For | Against |
|---|---|---|
| Smallest of every viewer (tmux's default) | Everyone sees the whole screen. | One teammate with a small window shrinks the owner's agent to 40 columns: Claude Code reflows, and it is a nuisance a guest can cause. |
| **The driver's size (recommended)** | The person typing sees a normal terminal. A watcher cannot affect it. | A smaller watcher sees it scaled down or cropped. |
| Largest | — | Every smaller viewer crops. |

**Recommendation: the driver's size.**

- A watcher's xterm gets the driver's `cols × rows`. When the pane is smaller, it is **scaled down to fit** (a CSS transform on the read-only view, never above 100 %), with "120×40 · alice" in the corner.
- On a hand over, the PTY takes the new driver's size, and the resize makes Claude Code redraw (SIGWINCH), as 26.16's reattach does.

### 5.3 Output to several apps

- On a shared service, **every terminal has a ring** (26.15's 1 MiB), as a kept terminal already does.
- The **driver's** acknowledgements pace the PTY (9.19's 512 KiB window), so a slow watcher never slows the agent.
- Each watcher reads the ring from its own offset with its own window.
- A watcher that falls behind the ring's start gets `terminal_reset` and the whole ring (26.16's path), and its xterm redraws.
- While the driver is gone, the terminal reads freely into the ring, as a detached terminal does today.
- No terminal emulator goes into the service (Stage 26 Q9 still holds).

### 5.4 Watching from the sidebar

Every terminal of a shared space shows under its worktree on every app, tagged with its opener's display name. Opening it sends `watch_terminal`. A watched tab is read-only (no cursor, input blocked) until it is handed over.

### 5.5 Terminals a collaborator opens

They run as the owner, so they get the owner's `claude` login and the owner's files. That is the meaning of the Collaborator role, and the Sharing list says so. The service sets `GIT_AUTHOR_NAME` and `GIT_AUTHOR_EMAIL` from the person's Sharing entry, so commits name who made them; the committer stays the owner. `HIVE_TERMINAL_ID`, hooks and the wrapper are unchanged: hooks come from the owner's own processes, through the owner's private socket.

## 6. Presence and follow

- **Presence comes from `view`** (section 4) and is sent only within shared spaces.
- **What the app shows:**
  - an avatar chip per person in the title bar: their initials and colour (picked by the service from the name);
  - the same chips on the sidebar rows of the worktree and agent each person is on, and on the tab they have open;
  - "away" when their window has had no focus for 5 minutes.
- **Follow `<name>`**: click a chip, or use the palette's "Follow…". Your app then shows their worktree, their tab, and in a file or diff the line they have at the top. It stops when you click, type or scroll; a bar says "Following bob · Stop" (Zed's behaviour).
  - Follow is **UI state** built on `presence` (#37: the service sends facts, the app chooses what to show).
  - Following moves you only to things your role can see.
  - Following a terminal tab watches it.
- **Not sent**: the cursor, the selection, keystrokes, scroll inside a terminal, anything in a space that is not shared, and the owner's Settings.

## 7. Shared review

- **Seen marks (27.1) are per person.** `seen.json` keys by person, worktree and base, and each `changes` carries the asker's own marks. The diff header gains small chips: "bob saw 4 of 9" in its tooltip, from the other people's counts (`seen_by: [{name, count}]`, at most 8).
- **Comments (27.3) are shared.**
  - The author is the connected person, set by the service, never by the app.
  - Anyone with a role may add a comment, resolve it or reopen it.
  - Delete: the author or the owner. Edit: the author only. Editing is a new small request, `edit_comment {id, body}`; today there is none.
  - Re-anchoring is unchanged.
  - "Send to agent" (28.21's picker) sends only to a terminal you drive, since others' input is dropped.
- **`@name` mentions:**
  - The service parses `@<login>` and `@<display name with no spaces>` in a comment's body against the Sharing list. Unknown names stay plain text. At most 8 per comment.
  - For each named person, the service keeps a mention: comment, worktree, author and time, at most 100 per person, in `<data>/mentions.json` (`0600`, atomic, 256 KiB cap).
  - A connected person gets it at once. A person who is not connected gets it at their next `welcome`.
  - In the app, it is an inbox item (6.5, "bob mentioned you in src/x.rs:12"; clicking it opens the comment), the bell's badge, the tone, and an OS notification under #5's rules, so not while that comment is in view.
  - "Read" is per person, and opening the inbox marks it read, as with agents.
  - A comment that mentions you and is later deleted removes its mention.
- **Agent alerts** (pending, sound, notification, #5): with several people, alerting everyone for every agent is noise. **Recommendation:** an agent alerts its terminal's **driver**, plus the people following the driver. Everyone else sees the state dot and the sidebar's propagated state, but gets no sound, no notification and no bell count. A per-app setting "Alert me for: my agents (default) / every shared agent" changes that (Q6). `pending` and `notify` are computed per session, and 5.6's "already seen" exception uses that session's own `view`.

## 8. Security (for `security-reviewer`)

**A teammate's app is untrusted, in both directions.**

### Service side (new)

1. **Who.** Identity comes only from `peer_cred` on the shared socket, mapped through `getpwuid`, and must be in the Sharing list. No name, role or author from the app is ever believed. `hello.app_id` is only a reconnect key within one person.
2. **What.** Every request is checked against the role table (section 3) and against shared spaces before any work is done.
   - Every path-bearing request (`path`, `cwd`, a worktree, a file) must resolve inside a project of a shared space. That uses the existing `projects::place` check, resolved, so `..` and links cannot escape.
   - A refusal is `not_allowed`, and it is logged at debug level without the payload.
3. **Hooks.** Hook events come only through the owner's `0600` socket. The shared socket refuses `role: hook`.
4. **Input.** A watcher's terminal frames are dropped. A Viewer can open nothing that runs code: no terminal, no chat, no script, no worktree with a setup script (#49).
5. **Resources.** Per-session caps (section 4). A guest cannot make the owner's service exhaust memory or wait on it.
6. **Environment.** A collaborator's terminal gets only the validated `GIT_AUTHOR_*` from the owner's own Sharing entry, never values sent by the app.
7. **The shared folder in `/tmp`.** Checked on both ends (owner uid, mode, peer uid). It is removed when sharing turns off. The socket is `0660`, the folder `0710`.
8. **Leaving the team.** Removing a person from the Sharing list ends their sessions at once. Removing them from the Unix group stops new connections at the file permission.

### App side (26.19 holds)

1. Everything a teammate writes reaches the app through the shared service: names, comment bodies, mention text, presence paths, terminal output from a terminal they drive. 26.19's caps, clipping, refusals and "only `https` URLs" apply unchanged, because a shared service is a remote.
2. Display names are validated by the service (no control characters, at most 40 characters) and clipped again by the app.
3. **Terminal output driven by a teammate.** It is the same xterm as today, with **no OSC 52 handler** (clipboard writes) and OSC 8 links only through `open_url`. This must stay true: adding a `ClipboardAddon` later would let a teammate write the owner's clipboard. Say so in a test.
4. **Follow** never opens or reveals anything outside the app (opens are already refused for a remote).

### Not prevented (to say plainly in the docs)

- A Collaborator runs code as the owner. That is what the role is.
- The host's root, and anyone who can become the owner, sees everything.
- A teammate with their own shell on the host sees what Unix lets them see: files the owner made group- or world-readable.

### No new port (#14)

Every byte travels inside each person's own `ssh` session to their own `hive bridge`, then over a Unix socket on the host. Nothing listens on TCP, no Hive server exists, and Hive stores no secret. Turning sharing on creates one Unix socket file and nothing else.

## 9. Out for now

- Sharing a **local** (Mac) service, and any sharing without ssh;
- B's shared Unix account, as a supported mode;
- C's federation of several services;
- cursors and selections of others in files;
- co-editing one file live: saves keep #31's version check, so a teammate's save over yours is refused as a conflict, as with an agent;
- voice, chat and screen sharing;
- web and phone clients (already out by D27.1);
- teammates on another host (Delta's cloud);
- Hive-managed Unix users and groups (the admin does that);
- protocol compatibility across versions (Q5).

## 10. What was tried, and what was not

The lab ran under the session's scratch folder, with a temporary `HOME` and `XDG_*`, and was deleted afterwards:

- `/usr/sbin/sshd` (OpenSSH, macOS 26), run as the agent's own user on `127.0.0.1:42291`, with a temp host key, a temp `authorized_keys` holding two temp ed25519 keys (`alice`, `bob`), `UsePAM no`, passwords off and `ExposeAuthInfo yes`;
- the client used `-F /dev/null`, `IdentityAgent none`, a temp `known_hosts`, and `BatchMode=yes` with `StrictHostKeyChecking=yes`.

| Experiment | Result |
|---|---|
| `ExposeAuthInfo yes`, logging in with each key | Each session got `SSH_USER_AUTH=/tmp/sshauth.<random>` (mode `0600`, owned by the login user) holding `publickey ssh-ed25519 <the key used>`. So with that sshd option, the key, and thus the person, **can** be known on one shared account. |
| `environment="HIVE_NAME=bob"` on bob's `authorized_keys` line | Ignored: `HIVE_NAME` was empty, since `PermitUserEnvironment` is off by default. |
| What that means for B | Both mechanisms need the admin to change sshd. Even then a teammate with a shell on the shared account can point `SSH_USER_AUTH` at a file they wrote, or add a key to `authorized_keys`. Key identity on a shared account is attribution, not security. That is why A is recommended. |

**Not tried (needs root, which was not used):**

- a second Unix user;
- a group-shared `0660` socket reached by another uid;
- `chown` of the shared folder to a group;
- `SO_PEERCRED` across users.

These rest on documented Linux behaviour: connecting to a Unix socket needs write permission on it, `SO_PEERCRED` reports the connecting process's uid, and a user may `chgrp` to a group they belong to. They also rest on today's `peer_cred` use in `paths.rs`.

**Also not tried:**

- a Linux sshd (the remote is Linux only; this was macOS);
- several real apps;
- PTY scaling in xterm.js;
- the cost of presence traffic.

Task 29.11 (section 12) tests all of this in CI with two real users in a container, where root is available.

## 11. Questions for the human

Each is phrased for the questionnaire, with the recommended answer first.

| # | Question | Recommended | Alternatives |
|---|---|---|---|
| Q1 | How are teammates identified on the host? | A Unix user each. The owner's service is shared to a Unix group, and the identity comes from the kernel (A). | One shared Unix account, with a name per app or per ssh key (B); each person's own service, federated (C). |
| Q2 | Who runs the agents a collaborator starts? | The owner (the service's user): the owner's `claude` login, with the collaborator as git author. A team that wants a team login makes the owner a team account. | Collaborators cannot start terminals at all (watch, comment and drive by hand over only). |
| Q3 | Who types in a terminal? | Its driver: the opener, until a "Hand over". The driver, opener or owner take it back at once; "Ask to drive" is a request. | Only the opener, ever (Delta); anyone with a role. |
| Q4 | May a Collaborator push, create pull requests or run workflows with the owner's `gh`? | No, owner only. | Yes, for Collaborators. |
| Q5 | A teammate's app of another version? | Refused with both versions; the owner updates (#29 strict). | A compatible protocol range for guests (relaxes #29). |
| Q6 | Who gets an agent's alerts (sound, notification, bell)? | Its driver and whoever follows the driver; a per-app setting "every shared agent". | Everyone connected; only the owner. |
| Q7 | The PTY size with several viewers? | The driver's; smaller watchers see it scaled down. | The smallest viewer's (tmux); the largest. |
| Q8 | Where is the shared socket? | `/tmp/hive-shared-<owner uid>/hive.sock` (folder `0710`, socket `0660`, the sharing group), checked on both ends. | A folder the admin creates (e.g. `/srv/hive/<owner>`), set in Settings → Sharing. |
| Q9 | Session history of a shared space? | Visible to every role. Resume and Open as Chat for Collaborators. Delete and Move for the owner only. | Owner only. |
| Q10 | The caps per connection? | 8 sessions, 2 per person; output queue 1024 frames / 64 MiB; requests 1000/s; comments 30/min per person; 100 mentions per person. | Other numbers. |

**Also to check, not a Hive decision:** whether the human's Claude plan allows teammates to use the owner's login through a shared host. Consumer plans are for one person. A team may need a Team or Enterprise seat, or an API key, on the owner (team) account. Hive never chooses or stores this (#45).

## 12. The plan (Stage 29, written there only after approval)

Points are a first guess for planning poker; together they fill 29.4's 89 reserved points. Each task touching `crates/hive` or `src-tauri` gets `security-reviewer`.

| Task | What | Pts | Needs |
|---|---|---|---|
| 29.4 Sharing and identity | Settings → Sharing (group, spaces, people, roles), with validation in the service. The shared folder and socket, with checks on both ends. `peer_cred` → name → Sharing list. Hook role refused on it. `hive bridge --join <owner>` and the constant command's owner argument. Settings → Hosts "Join someone's Hive". Owner-only version restart. | 13 | — |
| 29.5 Several apps on one service | Per-connection sessions (`watching`, `view`, `listed`, `scripted`, queue). `to_session` / `to_shared` routing. Channel maps. The role table and shared-space filter on every request, with `not_allowed`. Per-session caps. Guest `bye`. Takeover by person + `app_id`. The service's life with guests. | 21 | 29.4 |
| 29.6 Shared terminals | Service-wide ids and `terminals`. `watch_terminal`. Ring fan-out paced by the driver, with `terminal_reset` for a watcher left behind. Driver, `hand_over`, `take_back`, "Ask to drive". The driver's PTY size and watcher scaling. `GIT_AUTHOR_*` for a collaborator's terminal. The OSC 52 test. | 13 | 29.5 |
| 29.7 Presence and follow | `view`'s new fields, `presence` (throttled), chips in the title bar, sidebar and tabs, away, Follow and its stop. | 8 | 29.5 |
| 29.8 Alerts per person | `pending` and `notify` per session. Driver and followers. The "every shared agent" setting. | 5 | 29.6, 29.7 |
| 29.9 Shared review | Seen per person and `seen_by`. Comment author from the session, `edit_comment`, delete rights. Mentions (parse, store, deliver at `welcome`, read). Inbox items and notifications. | 13 | 29.5 |
| 29.10 A teammate cannot harm the owner | Fake guest apps against a real service: forged names, paths outside shared spaces, every owner-only request, input to undriven terminals, floods, a guest that stops reading, a planted `/tmp` folder, a removed person. App side: a hostile teammate's text through a shared service. `security-reviewer` on the whole path. | 8 | 29.4–29.9 |
| 29.11 Two real users in CI, docs, hand check ⏸ | A container with sshd and two Unix users in a group. The owner shares, the guest joins, and the test runs watch, hand over, comment, mention, drop and rejoin. README and `docs/architecture.md` (message catalog). A hand check by the human with a second account on a real host. | 8 | 29.4–29.10 |

## 13. Changes `docs/hive.md` would need (the human's; never edited here)

| Item | Today | Proposed |
|---|---|---|
| #14 | Service on a Unix socket, the app's lifetime, no network | Add: a remote service may also open a **shared** Unix socket for a Unix group (sharing on). Still no network; each teammate comes through their own `ssh`. |
| #18 / Stage 26 | One app per service; kept terminals | Several apps on a shared remote service. Only the owner's `bye {end: true}` ends terminals. The service lives while any session is connected, then for the keep time. |
| #24 | Protocol messages | `welcome.you`, `view` fields, `presence`, `terminals`, `watch_terminal`, `hand_over`, `take_back`, `mentions`, `read_mention`, `edit_comment`, `not_allowed`. |
| #45 | Hive never reads credentials | Also: a teammate never sees the owner's accounts, `gh` logins, usage or settings; agents a collaborator starts use the owner's login (Q2). |
| #48 | Spaces | A space may be **shared**; teammates see only shared spaces. |
| #5 / 6.5 | Alerts and the inbox, in memory | Alerts per person (Q6); `@name` mentions in the inbox, kept by the service (100 per person). |
| 27.1 / 27.3 | Seen marks; comments with an author | Seen per person; comment authors from the connection; edit and delete rights. |
| New | — | Sharing: the owner, a Unix group, named people with Viewer or Collaborator roles (Q1), the driver of each terminal (Q3), the driver's PTY size (Q7). Out for now: section 9. |
