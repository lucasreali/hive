# Spike 27.2: More diff bases (the agent's last turn, a chosen commit)

Design task 27.2 (`TODO.md`, D27.4: no production code before the human approves). The experiments ran outside the repository, on temporary repositories under a temporary `HOME` (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM`, `XDG_*` all pointing into a scratch folder), and were deleted when the spike ended. Written 2026-11-26 against `main` at `ca0944ef` (v0.30.0), git 2.54.0 (Apple Git-157) on the Apple Silicon host. Decisions read: #21 (hooks), #52 (Hive only observes), #56 (nothing inside `.git`), 9.11 (diff bases today), 22.1 (Hive's folder).

## 1. Recommendation in one paragraph

At each `UserPromptSubmit` of an agent, the service takes a **snapshot** of the agent's worktree as a git tree: a **temporary index in Hive's folder** (`GIT_INDEX_FILE`), seeded from a copy of the user's index so only changed files are hashed, then `git add -A` and `git write-tree`. Untracked files go in, ignored ones stay out. The new objects go to **Hive's own object store** (`~/.hive/snapshots/<repo>/store.git`, a bare repository whose `objects/info/alternates` names the repository's object folder), never into `.git/objects`. Refs in that store (`refs/turns/<session>/<n>`) keep the kept snapshots alive across `git prune` there and across a service restart. The user's index, branches, refs and object store get no new or changed file; the one trace is that git **refreshes the modification time** of an object already in the repository when the snapshot meets the same content again (section 4.3). "Since the agent's last turn" then lists the worktree against that tree through the same temporary index (`git add -A` + `git diff-index --cached <tree>`). On the measured repositories a snapshot costs about what one `git status` costs (0.2 s on TypeScript's 70 000 files), plus hashing the new content. "A commit…" needs no snapshot: it is 9.11's branch base with a commit the user picks from the branch's log. **#56:** with Hive's own store, no file is written inside `.git`; the human decides whether the timestamp refresh counts (question Q1).

## 2. Today (9.11)

`changes::against` resolves `DiffBase::Head` (`HEAD`, or the empty tree before the first commit) or `DiffBase::Branch` (`git merge-base HEAD refs/heads/<main branch>`). Against `HEAD` the files are `git status`; against a commit they are `git diff --raw <commit> --` (tracked files, committed since, staged or not) plus the untracked entries of `git status`, with `git diff --numstat <base> --` for the lines. `open_file` reads the base side with `ls-tree` + `cat-file blob`. Every git call goes through `git::command`: `-C <dir>`, `-c core.fsmonitor=false`, `GIT_INDEX_FILE` removed, `GIT_OPTIONAL_LOCKS=0`. The pick is UI state per worktree (`diffBases`), not remembered.

A tree base cannot reuse the commit path as is: `git diff <tree> --` compares the tree with the worktree **through the user's index**, so a file that was untracked at the snapshot and changed since is missed, and one untracked in the snapshot but in neither index nor worktree shows wrongly (checked in section 5).

## 3. The snapshot

### 3.1 Commands

Per worktree, Hive keeps a **snapshot index** in its folder (`~/.hive/snapshots/<repo>/<worktree>.index`). For one snapshot, in the worktree, with separate arguments as always:

```
env GIT_INDEX_FILE=<hive>/<worktree>.index  GIT_OBJECT_DIRECTORY=<hive>/store.git/objects
git -C <worktree> -c core.fsmonitor=false -c core.hooksPath=<hive>/no-hooks -c core.splitIndex=false \
    [-c filter.<name>.clean= -c filter.<name>.process= -c filter.<name>.required=false]…  add -A
git … write-tree                                    # prints the tree id
git --git-dir=<hive>/store.git update-ref refs/turns/<session>/<n> <tree>
```

- **Seeding:** the first snapshot of a worktree copies the user's index (`git rev-parse --git-path index`, which for a linked worktree is `.git/worktrees/<name>/index`) into Hive's file: a **read**, never a write. With its stat data, `add -A` hashes only the files that changed. Measured: on 100 000 files, seeded 1.1 s against 18.2 s from an empty index (section 7). Later snapshots reuse Hive's index (warm); it is re-seeded when it is missing or unreadable.
- **The store:** `git init --bare` once per repository (keyed by its common dir, `git rev-parse --git-common-dir`, so linked worktrees share it) and one line in its `objects/info/alternates`: the absolute path of the repository's `objects`. A file, not `GIT_ALTERNATE_OBJECT_DIRECTORIES`, so it also works with Windows paths and needs no separator rules; git follows the repository's own alternates from there.
- **Turned off for Hive's calls:** `core.fsmonitor` (already, 18.x), **hooks** (`core.hooksPath` to an empty folder of Hive's: git runs `post-index-change` whenever an index is written, Hive's temporary one included; verified, the hook ran once, and not with the override), the **split index** (it would write a `sharedindex.*` file into `.git`), and **clean filters**. Filters: `git::command`'s comment says "no `-c` turns them off, their names are arbitrary", but a filter only runs when its driver is configured, so the names are exactly those of `git config --name-only --get-regexp '^filter\.'`; emptying their `clean` and `process` (and `required=false`) stops them. Verified with a spy filter: two runs with the repository's config, none with the overrides. This keeps `git-lfs` from writing into `.git/lfs` and stops a repository's own program from running; the price is that a changed LFS file is snapshotted as its raw content, which only shows as a binary change.
- **No locks shared with the user:** Hive's index has its own `index.lock` next to it, so a `git commit` in a terminal and a snapshot never block each other (verified: the user's `index.lock` present, the snapshot ran). Two snapshots of one worktree are serialised by the service (one mutex per worktree), since git refuses a second writer of Hive's index.
- **Time limit:** a snapshot runs on a blocking thread, never on the hook's path (`hive hook` still answers in 200 ms, #21), with `git::run_within` and a longer limit than `TIME_LIMIT` (proposed 60 s: a first snapshot after a big checkout hashes much). A snapshot that fails or times out leaves that turn without a base, and the picker says why.

### 3.2 What goes in

- Tracked files as they are on disk (staged or not), **untracked files that are not ignored**, deletions. Ignored files, `.git` and Hive's own files (outside the worktree) never. Verified on a repository with `ignored/` and `*.log` ignored.
- A nested repository is a gitlink (`160000 commit <id>`), as `git add -A` records it (it prints a warning, which Hive discards); its contents are not snapshotted. A submodule likewise.
- Sparse checkouts keep their skip-worktree entries; files outside the cone are not read.
- No size cap of git's own: a 100 MB untracked file adds 105 MB (random bytes) or 4 MB (log text) to the store. Section 6 caps the store, and Q4 asks whether large untracked files should be left out.

### 3.3 The user's index and branches are untouched

On the small repository: `cmp` of `.git/index` before and after (byte-identical, with a staged file and an untracked one in place), `find .git -newer <marker>` after two snapshots (nothing), and `git cat-file -t <tree>` in the repository alone (the tree is not there). Refs, `HEAD`, the reflog and the index are never written; only `objects/info/alternates` in **Hive's** store names the repository.

### 3.4 The race with the agent's first edit

`UserPromptSubmit` arrives before Claude Code calls the model, so the first edit comes at least one model round trip later (seconds). The snapshot needs 0.2–1.4 s on the repositories measured (up to 5 s on the 100 000-file worst case), so it is usually taken before any edit. It cannot be made safe by waiting: the hook never delays the agent (#21, #52). The service notes when a `PostToolUse` of `Edit`, `MultiEdit`, `Write` or `NotebookEdit` of that session arrives before the snapshot is done, and the base then carries a warning ("taken after the agent's first edit: that edit may be missing").

## 4. Where the objects go, and #56

### 4.1 Options

| | Objects | What keeps them | `.git` written | Risk |
|---|---|---|---|---|
| **A** | the repository's `.git/objects`, loose | nothing (only Hive's list) | new files in `.git/objects` | `git gc` prunes them after `gc.pruneExpire` (2 weeks), `git gc --prune=now` at once; many turns add loose objects that count towards `gc.auto` and can bring "too many unreachable loose objects" warnings to the user |
| **C** | the repository's `.git/objects` | refs `refs/hive/turns/…` in the repository | new files and refs in `.git` | the refs show in `git for-each-ref`, in GUIs, and go out with `git push --mirror`; `git log --all` tolerates tree refs (checked) but tools differ |
| **B** | **Hive's store** in `~/.hive/snapshots/<repo>/store.git`, with the repository's objects as its alternate | refs in Hive's store | **no file**; git refreshes the mtime of objects it finds again (4.3) | a blob only the user's store had, unreachable there, may be pruned by the user's `git gc` (4.4) |
| D | no objects: a copy of the changed files in Hive's folder | Hive | none | re-implements git's diff, renames and binary detection; rejected |

Option A is what the task's example (`git add -A` + `git write-tree` with only `GIT_INDEX_FILE`) does, since `write-tree` and `add` write into the repository's object store. B differs by one environment variable and one alternates file, and costs the same (section 7: 1.4 s A against 1.4 s B on TypeScript after a turn).

### 4.2 Recommendation: B

B keeps every new file in Hive's folder (22.1), its gc is Hive's own (`git --git-dir=<store> prune --expire=now` after dropping refs: verified, the dropped snapshot's objects go, the kept ones stay complete, the repository's objects are not touched; `git gc` there packs only Hive's objects, 20 kB after a `gc` with three snapshots), and removing a repository from Hive or uninstalling (23.5) is deleting one folder.

### 4.3 What B still does inside `.git`

When `git add` meets content that already exists as an object in the repository (a file touched but not changed, a file put back to its committed text, a racily clean file), git does not write it again but **refreshes that object's modification time** (`freshen_loose_object` / `freshen_packed_object`, which look in alternates too). Verified: re-hashing 50 known files changed the mtime of their 50 loose objects in `.git/objects`; nothing else changed. Contents never change, nothing is created, and the effect for the user is only that those objects survive `git gc` a little longer (as after their own `git add`). Git offers no switch to turn it off. This is the point the human decides (Q1): #56 was written for the files the service creates and moves ("a file moved into `.git` could become a hook"); a refreshed timestamp creates nothing that git could run.

For scale, Hive already has git write inside `.git` in other places it was decided for: `git worktree add`/`remove` (#15), branches it creates (4.x, 27.4), and 27.6's commits.

### 4.4 The user's gc and Hive's trees

Hive's trees point at blobs the user's store already holds (most files: they did not change). Those reachable from the user's commits or index are safe. A blob that is only a loose, unreachable object there (staged, then unstaged) can be pruned by the user's `git gc` (after two weeks by default, at once with `--prune=now`). Verified: after unstaging a file and `git gc --prune=now`, the snapshot's tree still lists the file and `diff-tree` still works on ids, but its content is gone. The service treats a missing object while listing or opening a file as "this snapshot is no longer complete": the base falls back to `HEAD` with that `base_error`, as 9.11 does for a missing branch. Copying every such blob into Hive's store would avoid it but costs a full walk of the user's store; not worth it for a base kept a few days.

## 5. Listing changes since a turn

Two ways were tried to answer "what changed since tree T" at each refresh of the Changes panel:

1. **Recommended: the warm snapshot index.** With the same environment and overrides as the snapshot: `git add -A`, then `git diff-index --cached -z --raw --find-renames <T>` and `git diff-index --cached -z --numstat --find-renames --no-ext-diff --no-textconv <T>`. It refreshes the worktree's snapshot index (it is only a cache; the snapshot is the tree id) and writes a loose object into **Hive's** store for each new version of a changed file; those objects belong to no ref, so the next prune (section 6) removes them. Cost: 142 ms on TypeScript, about one `git status` (section 7).
2. **A view index, nothing written:** copy the user's index to `<worktree>.view`, `git add --intent-to-add --all` (untracked files recorded without hashing or storing), then `git diff --raw <T> --` and `git diff --numstat <T> --`, which hash in memory. Verified correct, and five listings wrote one object in total (the empty blob, once). But it re-hashes every file untracked since T at each refresh, twice: 1.4 s on TypeScript, 2–5× today's listing. Kept as the fallback if the human prefers no writes at all per refresh.

Both were checked on a turn that edits, deletes, renames, changes a file untracked before the turn and adds one: the result is exactly the `diff-tree` of T against a fresh snapshot (M, D, R100, M, A). Comparing with the user's index instead (`git diff <T> --` with no `GIT_INDEX_FILE`) misses the new untracked file, as section 2 says. The entries map to `FileStatus` as `parse_raw` already does; the line counts come from numstat for every file (an untracked file's too), so 9.14's untracked budget does not apply. `open_file` reads the base side with `ls-tree <T> -- <path>` and `cat-file blob` under the store's `GIT_OBJECT_DIRECTORY`, as for a commit today.

## 6. How many are kept, and pruning

- **Per session:** the last **10** turns (proposed; the picker uses only the last, a later task could list older turns). The list is the store's refs, `refs/turns/<session>/<n>`, with the turn's start time in a small JSON next to the store (`turns.json`, size-capped, 0600): the service finds them again after a restart, and a resumed session (`claude --resume`, same id) keeps its turns.
- **Dropped:** a session's refs at its `SessionEnd`; a worktree's index files when it is removed (`WorktreeRemove`, 4.4); the whole store when its repository is no longer a project of any space, or when its alternates path no longer exists (the repository moved); anything older than 7 days.
- **Size cap:** 1 GiB per store (proposed). After a snapshot pushes the store past it, the oldest snapshots of that repository are dropped until it fits; a single snapshot bigger than the cap is dropped at once ("this turn's snapshot was too large to keep").
- **Prune:** `git --git-dir=<store> prune --expire=now` on a blocking thread after refs are dropped, at most once a minute per store; `git gc --auto` there now and then to pack. With 10 kept snapshots a prune takes 55–151 ms (section 7). It also removes the objects the Changes panel's listings left (section 5).
- Session ids come from hook payloads (untrusted): only `[A-Za-z0-9_-]{1,64}` is used in a ref or file name; any other id gets no snapshot.

## 7. Measurements

Apple Silicon host, APFS, warm caches, median of 5 runs (first snapshots: one run). Repositories: **r10k** and **r100k**, synthetic (10 000 / 100 000 tracked text files of ~4 kB in 10 000 / 37 000 folders, plus an ignored `node_modules` of 5 000 / 50 000 files); **TypeScript**, a shallow clone of `microsoft/TypeScript` (69 657 files, 282 MB with `.git`). A "turn": 20 files edited, 3 deleted, 200 new text files, and 2 000 small binary files in `target/` (ignored in the synthetic ones, not in TypeScript, which therefore hashes 4 MB more).

| | r10k | r100k | TypeScript |
|---|---:|---:|---:|
| `git status --porcelain=v2 --untracked-files=all` | 348 ms | 1 187 ms | 165 ms |
| Today's listing against `HEAD` (`status` + `diff --numstat HEAD`) | 400 ms | 1 206 ms | 276 ms |
| First snapshot, empty temporary index (every file hashed) | 1 799 ms | 18 177 ms | 6 137 ms |
| First snapshot, seeded from a copy of the user's index | 420 ms | 1 087 ms | 166 ms |
| Snapshot, nothing changed (warm) | 354 ms | 2 355 ms | 163 ms |
| Snapshot after the turn | 520 ms | 4 773 ms | 1 376 ms |
| Same, option A (objects into `.git/objects`) | 412 ms | 2 989 ms | 1 410 ms |
| `diff-tree --raw` between two snapshots | 18 ms | 41 ms | 147 ms (2 223 entries) |
| Read one old file (`cat-file -p <tree>:<path>`) | 9 ms | 24 ms | 13 ms |
| Store growth for the turn | 77 kB | 78 kB | 4.2 MB |
| Snapshot with a new 100 MB untracked file (random bytes) | 2.0 s, +105 MB | 8.3 s, +105 MB | 3.0 s, +105 MB |
| Listing since the turn, warm snapshot index (section 5, way 1) | 316 ms¹ | 3 379 ms | 142 ms |
| Listing since the turn, view index (section 5, way 2) | 742 ms | 2 958 ms | 1 416 ms |
| `git prune` of Hive's store with 10 snapshots kept | 61 ms | 151 ms | 55 ms |
| Hive's store with 10 snapshots | 360 kB | 357 kB | 1.5 MB |

¹ `add -A` + `write-tree` + `diff-tree`; the `diff-index` row was added after that run.

The r100k numbers are noisy (one run of the same command ranged 0.9–2.2 s): that layout, about 2.7 files a folder, is a worst case for git's untracked-file scan, which costs per folder (`status --untracked-files=no` takes 148 ms there). What it says:

- A snapshot costs one `git status`-sized walk of the worktree plus hashing what changed: about 0.2 s on TypeScript, 1–5 s on 100 000 files spread over 37 000 folders. It runs off the hook's path, once per prompt.
- Seeding from the user's index is what makes the first one cheap (15–37× faster than an empty index).
- New content is hashed at roughly 25–50 MB/s for incompressible bytes (zlib); large untracked binaries are the only thing that makes a snapshot slow or the store big. Changing `core.looseCompression` did not help consistently.
- Option A and B cost the same; the store stays small (kilobytes per ordinary turn).
- Listing since a turn through the warm snapshot index costs about what today's listing does (142 ms against 276 ms on TypeScript); the view index costs 2–5× more.

## 8. Agents, subagents, hosts

- **Several agents in one worktree:** each session gets its own snapshots (the same snapshot index of the worktree, serialised; two prompts with no change between give the same tree, so no extra objects). The picker has one item per agent of the worktree ("Since <agent>'s last turn", the agent's title or its short id). The diff is the worktree's, not the agent's: what another agent or the user changed since that moment shows too, and the item's tooltip says so. Who wrote which line is 27.5's work.
- **An agent that moves:** the snapshot is of the worktree the agent is in at its prompt (`Agent::worktree`, from the payload's `cwd`). The picker of worktree W lists only agents whose last snapshot is of W. An agent outside any followed project gets none.
- **Subagents** (Task tool, 5.x) send no `UserPromptSubmit`: they run inside their parent's turn, so the parent's snapshot covers their edits in the same worktree. A subagent in **its own worktree** (5.4) starts from a fresh `git worktree add`, so 9.11's branch base (its fork point) already shows all its work; no snapshot is proposed for it (Q5). **Teammates** (26.10) are listed as subagents of the lead and get the same rule.
- **Remote hosts (Stage 26):** the whole service runs on the host, so the snapshot, its store (the host's `~/.hive/snapshots`) and the listing run there with the host's git; nothing crosses the link but the messages below. The new service → app messages (`commits`) join `REMOTE_TO_UI` and get a list cap in `src-tauri/src/limits.rs` (26.19), and the remote's text is clipped there as for `changes`.
- **Folder projects** (23.1, no repository): their Changes have no base today; no turn base for them.
- **Windows** (D17.0): nothing Windows-only; the alternates file and the paths work there as they are, and the code keeps building.

## 9. "A commit…"

- **The list:** `list_commits {path}` → `commits {path, commits, truncated}`, from `git log -z --format=%H%x00%h%x00%an%x00%ct%x00%s --max-count=500 HEAD --` (separate arguments, `log.showSignature=false` as today), each `{id, short, author, time_ms, subject, on_branch}`; `on_branch` marks the commits since the fork point (`merge-base..HEAD`, 9.11's), which come first under a "This branch" heading, then the older ones. The subject is clipped to 200 characters, the author to 100; the list to 500 (`truncated`). In the main worktree, or without a main branch, it is `HEAD`'s last 500.
- **The base:** `DiffBase::Commit {id}`. The service accepts only 40 or 64 hex digits and checks `git rev-parse --verify --quiet --end-of-options <id>^{commit}` (never read as an option), then lists exactly as for the branch base (`changes::list(dir, Some(id))`): no new git work. A commit that is gone (after a rebase) falls back to `HEAD` with `base_error`.
- **The picker:** "A commit…" opens a searchable dialog in the manner of `WorktreePicker` (filter on subject, short id and author; arrows, Enter, Esc), fed by the `commits` answer. Filtering the received list is presentation (#37).

## 10. Protocol and UI

- `DiffBase` gains two variants carrying data: `Turn { session: String }` and `Commit { id: String }` (serde: an internally tagged form, `{"kind":"turn","session":"…"}`, with `head` and `branch` kept readable; the exact shape is the implementer's, the catalog in `docs/architecture.md` updated with it).
- `changes` gains `turns: Vec<TurnBase>` for the worktree, each `{session, title, started_ms, late}` (the agents of that worktree with a usable snapshot), so the picker knows what to offer; `base_error` keeps its meaning.
- The segmented HEAD | `<branch>` toggle becomes a small menu (the toggle cannot hold N agents): HEAD, `<branch>`, one "Since <agent>'s last turn" per entry of `turns`, "A commit…". The trigger shows the current base ("Since claude-3's turn", "a1b2c3d"). The pick stays UI state per worktree (`diffBases`), not remembered (9.11). The default stays 9.11's.
- The mock transport answers `turns` and `commits` with fixed data.

## 11. Security

- Session ids from payloads are validated before any use in a path or ref (section 6); commit ids from the app are hex only and verified; tree ids come from git's own output.
- No user program runs from a snapshot: hooks, filters and fsmonitor off (3.1). Diffs keep `--no-ext-diff` and `--no-textconv`.
- Hive's store, indexes and `turns.json` are created 0700/0600 under Hive's folder; nothing of a snapshot is logged but its tree id, worktree and duration.
- Output limits as today (`git::run`, 16 MiB); a snapshot's stderr is clipped.
- `security-reviewer` on the new git calls.

## 12. Questions for the human

| # | Question | Recommendation |
|---|---|---|
| Q1 | **#56.** Does option B fit "nothing inside `.git`"? It creates and changes no file there, but git refreshes the mtime of objects it meets again (4.3). The alternatives are A (new loose objects in `.git/objects`, at the mercy of gc) or C (objects and `refs/hive/*` in the repository). | **B**, and read #56 as "no file created, moved or changed inside `.git`"; the timestamp refresh is what any `git add` of the user does. |
| Q2 | How many turns per session are kept, and for how long? | The last 10, at most 7 days, dropped at `SessionEnd`; only the last one in the picker for now. |
| Q3 | Size cap of Hive's store per repository? | 1 GiB, oldest snapshots dropped first. |
| Q4 | Untracked files over a size (dumps, videos) in snapshots? Leaving them out needs one more walk (`ls-files -o`, about +0.15 s on TypeScript) and they would show as "added" against the turn. | Keep them (no extra walk); the store cap bounds the cost. |
| Q5 | A snapshot for a subagent working in its own worktree (at `SubagentStart`)? | No: its branch base already shows all its work. |
| Q6 | A snapshot that finished after the agent's first edit: show the base with a warning, or hide it? | Show it with the warning. |
| Q7 | The commit list: how many, and only the branch's commits or older ones too? | 500 from `HEAD`, the branch's first under their own heading. |
| Q8 | Listing since a turn: through the warm snapshot index (fast, leaves loose objects in Hive's store until the next prune) or the view index (writes nothing, 2–5× slower)? | The warm snapshot index. |
| Q9 | Filter drivers off for snapshots (an LFS file then snapshots as its raw content, shown as a binary change)? | Off: no repository program runs from Hive, nothing goes into `.git/lfs`. |

## 13. Proposed tasks and tests

After the human's answers, 27.2 is built as three tasks (or one branch with three commits, the orchestrator's call):

1. **27.2a Snapshots (Rust).** `hive::snapshot`: the store (create, alternates, refs, `turns.json`, caps, prune), `take(worktree, session)` with the overrides of 3.1, the per-worktree mutex, the late flag; called from `daemon::agents::saw` on `PromptSubmitted` on a blocking thread; drops on `SessionEnd`, `WorktreeRemove`, project removal and age. Tests (temp repos, temp `HIVE_HOME`): a snapshot per turn; untracked in, ignored out; the user's index byte-identical and nothing under `.git` created (a `find -newer` over `.git` ignoring object mtimes); a spy `post-index-change` hook and a spy filter never run; the cap (11th turn drops the 1st; size cap drops oldest; prune removes dropped objects, kept ones complete); a missing blob reported as incomplete; session ids rejected; a restart finds the refs; the late flag. Mutants on the diff.
2. **27.2b The bases (Rust + protocol).** `DiffBase::Turn`/`Commit`, `changes.turns`, `list_commits`/`commits`, `changes::list` and `file::open` for a tree base through the warm snapshot index (section 5), the fallbacks to `HEAD` with `base_error`; `REMOTE_TO_UI` and `limits.rs` for `commits`; catalog and sequences in `docs/architecture.md`. Tests: the diff since a turn (edit, delete, rename, changed-untracked, new; equal to `diff-tree` of a fresh snapshot), nothing created in `.git` by a listing and its objects gone after the next prune, a commit base equal to the branch path, bad ids refused, a vanished commit; integration with the fake `claude` (`common::Env::fake_claude`): its `UserPromptSubmit` and `PostToolUse` hooks fired through the wrapper, then `list_changes` with the turn base shows only the turn's edits.
3. **27.2c The picker (frontend).** The base menu with HEAD, branch, one item per `turns` entry, "A commit…" and its searchable dialog; the mock's `turns` and `commits`. Bun tests: the menu's items and disabled states, the trigger's label, the dialog's filter and keys, `setDiffBase` with each kind; e2e on the mock transport: pick a turn, pick a commit, the list and the open file follow.

`docs/hive.md` stays as it is until the human updates it (a line for 27.2 and, depending on Q1, a note under #56).
