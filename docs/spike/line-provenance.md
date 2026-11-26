# Line provenance: from a line to the session that wrote it (TODO 27.5, design)

Status: **proposal** (D27.4: no production code before the human approves it). Built from the code on `main` (after v0.30.0), Claude Code's hooks and Agent SDK reference (`code.claude.com/docs/en/hooks`, `…/agent-sdk/typescript`, read through context7 on 2026-11-26) and `docs/spike/agent-states.md`. No real `claude` was run and `~/.claude` was not read. The only experiments were a throwaway Rust benchmark in a scratch folder and `git blame` timings on this repository (§9).

Decisions this rests on: #21 (hooks only in Hive's terminals, merged through `--settings`), #27 (synchronous hooks, short timeout, `hive hook` always exits 0), #37 (logic in Rust), #52 (Hive only observes), #56 (nothing written inside `.git`), 4.11 (Sessions view), 22.1 (the Hive folder `~/.hive`, `$HIVE_HOME`), and the code baseline: payloads are untrusted, everything has a size limit, and nothing is logged in full.

## 1. Summary of the proposal

- **What gets recorded.** For every `PostToolUse` of a file-editing tool (`Edit`, `Write`, plus `MultiEdit` if it ever shows up), Hive keeps one *edit record*: the session, the subagent if any, the tool, `tool_use_id`, the time, the file, and the new lines (their line numbers and a 64-bit hash of each line). The file's text itself is never stored.
- **Who each line is from.** For each file an agent has touched, Hive keeps a *line map*: one hash per current line, plus an *origin* per line. The origin is one of: an edit record (a session), "you, in Hive's editor" (the service saves those files, #31), "unknown", or "before Hive's record".
- **How lines are followed.** When the file changes, Hive diffs the stored line hashes against the file's current hashes. Lines that match keep their origin, and new lines get the origin of whoever is known to have written them. There is no watcher on every worktree: Hive re-syncs only at the moments listed in §4.
- **One shared module, `anchor`.** It holds line hashing, a bounded diff of hash vectors, mapping a range through that diff, and relocating an anchor (text plus context, nearest to the old line). Task 27.5 uses it, and so does 27.3's anchored comments (§5).
- **After a commit.** A commit does not change the working file, so the line map still answers. When it cannot (the base side of a diff, another worktree, a merged branch, a record that expired), Hive runs `git blame` to get commit C and the line as it was in C. It then searches the repository's edit records made before C for a matching run of lines, and answers "probably session X (committed in C)" (§6).
- **Hashing happens in `hive hook`.** An `Edit`'s `tool_response` carries the whole original file (`originalFile`). Today any payload over 512 KiB is replaced by a marker, so a large file's edit, and even its session id, would be lost. `hive hook` therefore reads up to 32 MiB for `PostToolUse`, turns the edit into hashes and line numbers there, drops the bulky fields, and forwards a payload of a few KiB (§3). The file text never crosses the socket and never stays in the service's memory.
- **Storage.** One JSON file per repository in `~/.hive/provenance/` (0600, atomic writes, size-capped), never inside `.git`. The worktree's line maps go when the worktree is removed. The edit records stay until the age or size cap, so lookups through a commit keep working after a merge (§7).
- **UI.** A right-click on a line number in the editor or the diff, or the palette, offers "Who wrote this line?". A known origin opens the Sessions view (4.11) at that session, with its "Files changed" list open and the file highlighted. An unknown or probable origin shows a small popover first. Each session's row gets a "Files changed" list (§10).

## 2. What the hooks give

Only `PostToolUse` matters here; `PreToolUse` is not needed (§4 explains why). Every event below already runs `hive hook <event>` in Hive's terminals (`wrapper::EVENTS`), so nothing new is registered.

| Field | Where | Used for | Untrusted-input rule |
|---|---|---|---|
| `session_id` | every event | the edit's session | `sessions::valid_id` (≤ 64 chars, UUID charset) |
| `agent_id`, `agent_type` | inside a subagent | the subagent shown with the session | ≤ 256 bytes as today (`Subagent`); `agent_type` clipped to 64 chars |
| `tool_name` | `PostToolUse` | `Edit`, `Write`, `MultiEdit` only. `NotebookEdit` gets a file-level record only (no lines, since cells are JSON) | exact match |
| `tool_use_id` | `PostToolUse` | ties the record to the transcript entry | `[A-Za-z0-9_-]{1,64}`, else dropped |
| `cwd` | every event | not used for placing. The file is placed by its own path | — |
| `tool_input.file_path` | `Edit`/`Write`/`MultiEdit` | the file | absolute, ≤ 4096 bytes, canonicalized, and inside a followed worktree (`projects::place`). Never inside `.git` (#56). Must be a regular file (no FIFO; same check as `transcript`). Otherwise ignored |
| `tool_response.filePath` | `Edit`/`Write` | must equal `file_path`, else ignored | — |
| `tool_response.structuredPatch[]` | `Edit`, `Write` (update) | hunks: `newStart`, `newLines`, and `lines` prefixed with `' '`, `'-'`, `'+'` (and `\ No newline…`). Gives the new side's line numbers of each `+` line | at most 4096 hunks and 200 000 lines in all. A hunk that disagrees with its counts is dropped |
| `tool_response.originalFile` | `Edit`, `Write` (update; `null` for a new file or a previous file over ~10 MB) | its digest, compared with Hive's stored map (§4) | read only in `hive hook`, never forwarded |
| `tool_response.type` | `Write` | `create` (every line is the session's) or `update` | — |
| `tool_input.content` / `tool_response.content` | `Write` | on `create`, or `update` with an empty patch (Claude Code skipped the diff), the line hashes of the whole file | read only in `hive hook` |
| `tool_response.userModified` | `Edit`, `Write` | "you changed it before accepting" is shown with the edit | boolean |
| `tool_input.old_string`/`new_string`/`replace_all`, `tool_response.oldString`/`newString`/`gitDiff` | `Edit` | not needed: the patch already holds the result | dropped in `hive hook` |

What the docs say about `Write` (the TypeScript SDK reference): `structuredPatch` is empty for a new file, when the write changed nothing, when the diff timed out, or when the previous content was over ~10 MB. `originalFile` is `null` for a new file and for that over-10-MB case.

**`MultiEdit`.** It is not in the current SDK's list of tool outputs (`ToolOutputSchemas` has `FileEditOutput`, `FileWriteOutput` and `NotebookEditOutput`, but no MultiEdit). The adapter treats any of the three names the same way: `file_path` plus a `structuredPatch`. A `MultiEdit` with a different shape gets only a file-level record.

**Failures.** `PostToolUseFailure`, or a `tool_response` that is a string (the error text), records nothing.

**To confirm with a recording** (as in agent-states §6). The field names above come from the docs, not from a payload Hive saw. A plain shell with `hive hook --record` (the existing record mode, in `/tmp`, nothing committed) should capture one `Edit`, one `Write` create, one `Write` update, one `replace_all` and one edit the user changed before accepting. The implementation's fixtures then copy that shape, with the text replaced.

### Edits by other means, and how they show

| Who changed the line | What Hive sees | Shown as |
|---|---|---|
| An agent's `Edit`/`Write` in a Hive terminal | `PostToolUse` with the patch | **the session** (and subagent), the tool, and the time |
| You, in Hive's editor | the service's own `save_file` (#31) | **"You, in Hive's editor"**, and the time |
| An agent's Bash (`sed -i`, `cat >`, `git checkout`, a code generator, a formatter run by a PostToolUse hook) | only the new content at the next re-sync (§4) | **unknown**: "changed outside an agent's Edit/Write (a shell command, another editor, or git)" |
| You, in another editor, or `claude` outside Hive (#21: no hooks there) | the same | **unknown** |
| Lines untouched since Hive started following the file | — | **before Hive's record**; the lookup goes on to `git blame` (§6) |

A Bash call could be guessed ("changed while session X ran `sed …`"), but only from timing. 27.7 already rules out acting on a timer alone, so v1 says "unknown" (Q3).

## 3. `hive hook` turns the edit into hashes before sending

Why: `Edit`'s `tool_response.originalFile` is the whole file before the edit, and `Write` carries the content twice. Today `hook::read_payload` replaces any input over `MAX_INPUT` (512 KiB) with `{"hive_error": …}`, which also loses `session_id` and `tool_name`. A large Bash output already hits the same limit. The frame limit is 4 MiB (`MAX_PAYLOAD`).

Proposal, inside `hook::run`, for `PostToolUse` only:

1. Read up to **32 MiB** instead of 512 KiB. That covers Claude Code's own ~10 MB `originalFile` limit, plus `content` and the patch. Past 32 MiB, the payload still becomes the marker, but `hive hook` first pulls `session_id` and `tool_name` out of the first 4 KiB with a bounded scan, so the agent's state still follows the event.
2. Parse it, then **slim** it: keep the fields `adapter` and `states` read today (`session_id`, `agent_id`, `agent_type`, `teammate_name`, `cwd`, `tool_name`, `tool_use_id`, and `tool_response.backgroundTaskId`/`taskId`/`agentId`), and drop the rest of `tool_input` and `tool_response`, for every tool. A slim Bash or Read event stays far under the limit.
3. For `Edit`/`Write`/`MultiEdit`, add `hive_edit`:

```json
{ "file": "/abs/path",
  "kind": "edit" | "create" | "update",
  "before": "<64-bit digest of originalFile's line hashes>" | null,
  "lines": 812,
  "added": [[120, 3, ["9f…", "41…", "c0…"]], [400, 1, ["77…"]]],
  "user_modified": false }
```

- `added` holds one entry per run of `+` lines: the new-side start line (1-based), the count, and the line hashes.
- `lines` is the expected line count after the edit.
- A run longer than 5000 lines keeps its range but not its hashes. Such a run is followed only through the diff, never searched for later (§6).
- On `create`, or `update` with an empty patch, the run is the whole content (Q5).

The service never sees the text. At 8 to 16 bytes per added line, a large rewrite stays under 1 MiB.

**Cost (measured, §9).** Parsing a 10 MB payload and hashing its lines takes about 16 ms on this Mac; 30 MB takes about 48 ms. That is well inside the hook's 1 s timeout, and the 200 ms send budget is unchanged because it starts after the read. Memory peaks at about 3× the payload, freed when `hive hook` exits.

`--record` keeps the raw payload, as today, for recordings.

## 4. The line map, and how it follows the file

Per (worktree, file) that an agent's edit or Hive's editor touched, Hive keeps:

- `hashes`: the file's line hashes when Hive last looked, as packed u64.
- `origins`: runs of `[start, len, origin]`, where the origin is an edit id, `editor@time`, `unknown`, or `before`.
- `size`, `mtime`: to skip re-reading a file that did not change.

**Line hash.** FNV-1a 64 over the line's bytes, without its `\n` and without a trailing `\r`. It is written in the `anchor` module, about ten lines of code: stable across Rust versions (std's `DefaultHasher` is not) and needing no new dependency. A collision only mislabels one line, which is acceptable for an advisory answer.

**Re-sync** (read the file, hash its lines, diff against `hashes`):

- On each `PostToolUse` edit record for the file, the edit's lines are applied as described below.
- On each `PostToolUse` (any tool) and `Stop` of an agent in that worktree, only for the tracked files whose `size`/`mtime` changed. This is a `stat` per tracked file, at most once a second per worktree.
- When Hive's editor saves the file. Changed lines become `editor`.
- When the files panel's watcher (`files.rs`, the shown worktree only) reports a tracked file.
- Before every lookup (§6).

Applying an edit record E to file F:

1. Read F (only if ≤ 2 MiB and ≤ 50 000 lines; otherwise F gets a file-level record only, "edited by session X at T") and hash it: N.
2. If a map exists: `diff(map.hashes, N)` gives each old line's new place. Mapped lines keep their origin.
3. Each run of E's `added` is checked at its claimed line in N. Where the hashes match, those lines become E. Where they do not (a formatter hook ran, or another change landed between the tool's write and this read), the run is relocated with `anchor::relocate`: the same hashes, nearest to the claimed line. If it is not found, the lines it would have covered stay unknown.
4. Any other new line becomes `unknown`.
5. If there is no map yet, the lines E does not claim become `before`. They were there before Hive followed the file, and `git blame` answers for them.
6. Unknown lines that changed back to an earlier text (a `git stash pop`, a checkout back to the branch) are matched against the file's own edit records with `anchor::relocate`. Only runs of at least 3 lines, or single lines with at least 8 non-blank characters, are matched, so a lone `}` is never claimed.

Ordering: hook events are applied in `sent_ns` order (as `states` already does), so two agents editing the same file are labelled in the order their tools ran.

**Why `PreToolUse` is not needed.** Re-syncing before applying each edit makes every change that happened since the last look "unknown", and the edit's own lines are then claimed by hash. There is nothing to hold open between Pre and Post, and no race with the watcher: if the watcher sees the write first, those lines become unknown, and the `PostToolUse` that follows claims them by hash (step 3).

## 5. One shared module for 27.3 and 27.5: `crates/hive/src/anchor.rs`

27.3 (anchored review comments, not built yet) needs exactly this: follow a line range as the file changes, matching "the anchored text and context, nearest to the old line", or mark it outdated. Pure functions, no I/O, so every path is unit-tested and covered by mutants:

```rust
pub fn line_hash(line: &[u8]) -> u64;                         // FNV-1a 64, no "\n"/"\r"
pub fn hashes(text: &[u8], most: usize) -> Option<Vec<u64>>;  // None past `most` lines
pub fn digest(hashes: &[u64]) -> u64;

/// Old line → new line, from a diff of two hash vectors (Myers O(ND), D capped;
/// past the cap every line counts as replaced, so the cost stays bounded by the input).
pub struct LineMap { /* … */ }
pub fn diff(old: &[u64], new: &[u64]) -> LineMap;
impl LineMap {
    pub fn line(&self, old: usize) -> Option<usize>;
    pub fn range(&self, old: Range<usize>) -> Mapped;   // Kept(range) | Partly(range) | Gone
}

/// A range's own line hashes plus up to 3 lines of context on each side.
pub struct Anchor { pub start: usize, pub lines: Vec<u64>, pub before: Vec<u64>, pub after: Vec<u64> }
impl Anchor { pub fn at(file: &[u64], range: Range<usize>, context: usize) -> Anchor; }
/// Where the anchor is in `file`: an exact run of `lines`, scored by matching context,
/// ties broken by distance from `hint`; None = outdated.
pub fn relocate(anchor: &Anchor, file: &[u64], hint: usize) -> Option<Range<usize>>;
```

- **27.3 uses it** like this: a comment stores an `Anchor` and the file's `hashes` when it was placed. On a change: `diff`, then `range()`; on `Partly`/`Gone`, `relocate`; on `None`, the comment is outdated. Comments and provenance keep their own stores (comments have an author, a state and text; provenance has edit records). Only the code is shared, so neither feature's cap or lifetime constrains the other.
- **27.5 uses it** for following the line map (`diff`), re-claiming a moved run (`relocate`, §4 step 3), and the search after a commit (§6).
- **The diff is written by hand**, a bounded Myers over `u64` slices of about 100 lines, rather than adding `imara-diff` or `similar` (Q2). Hive has 11 service dependencies and no diff crate yet. The input is hashes, not text, so no text-diff features are needed.
- **Order of work.** Whichever of 27.3 and 27.5 is built first builds `anchor.rs` to this interface. 27.3 has no design checkpoint, so this section also serves as its spec (Q1).

## 6. Lookup, and after a commit

`who_wrote {worktree, path, line, side}`, where `side` is `worktree` (the editor, or the diff's new side) or `base` (the diff's old side at its base commit, 9.11):

1. **Live.** For the worktree side, re-sync (§4), then read the line's origin. A session or editor origin is the answer, exact. A commit in between changes nothing, because the working file is the same.
2. **Through `git blame`** (an origin of `before` or `unknown`, the base side, a file with no map, a file in another worktree or on `main` after a merge):
   - Run `git blame --porcelain -L <n>,<n> [<base>] -- <path>`, with separate arguments as `git.rs` does. It gives the commit C, the path and line number in C (it follows renames), C's committer time, and the line's text. Hive hashes that text and drops it.
   - Then `git show C:<path>` (≤ 2 MiB), hashed, gives an `Anchor` of that line with 3 lines of context.
   - Search the repository's edit records for that path (and C's paths before a rename), keeping only those made before C's committer time plus 5 minutes, newest first. A candidate must contain the line's hash in an `added` run, and `relocate`'s context score ranks the candidates.
   - The best candidate is the answer, as **probable**: "probably session X, committed in `abc1234`". The trivial-line rule of §4 step 6 applies, so a blank line or `}` never matches through this path.
   - With no candidate: **unknown**, with C's short id, author, time and summary.
3. A diff's **deleted** line (old side only) goes through step 2 at the base.

Rebases and squashes change the sha, but the search keys on line hashes and time, not on the sha. A squash of the session's own commits is still found. A rebase onto a much later base can push the edit before C's time window out of range, which only misses an answer and never gives a wrong one.

**Commits are not watched.** Nothing is recorded when a commit happens. The commit path is computed when asked, so there is no hook on HEAD and nothing is written inside `.git` (#56).

**Cost of a lookup**, measured on this repository (2471 commits): `git blame -L n,n` of one line took about 20 ms; a whole-file blame of `TODO.md` (1159 commits touching it) about 270 ms. The search is over the repository's records, at most 20 000, held in memory once loaded: under a millisecond. A lookup is a click, so even a slow blame on a huge repository is fine. It runs off the async runtime (`block_in_place`, as `changes` does) with a 10 s limit.

## 7. Storage and caps

`~/.hive/provenance/<repo>.json` (`$HIVE_HOME` honoured, 22.1):

- `<repo>` is the hex digest of the canonical `git rev-parse --git-common-dir`, so all worktrees of a repository share one file and an edit record survives its worktree. The folder is 0700 and the file 0600.
- Writes are atomic (temp file plus rename, as `file::save`), at most every 2 s, plus on shutdown.
- A file is loaded the first time it is needed. Malformed JSON is set aside (`<repo>.json.bad`, as other Hive files do) and Hive starts empty.

```json
{ "version": 1,
  "edits": [ { "id": 41, "session": "uuid", "agent": "a1b2…", "agent_type": "general-purpose",
               "tool": "Edit", "tool_use_id": "toolu_…", "ms": 1795000000000,
               "path": "src/x.rs", "user_modified": false,
               "added": [[120, 3, "base64 of 3 u64"]] } ],
  "worktrees": { "/abs/worktree": { "src/x.rs": {
      "size": 18211, "mtime_ms": 1795000000000,
      "hashes": "base64 of packed u64",
      "origins": [[1, 119, "before"], [120, 3, 41], [123, 690, "before"]] } } } }
```

Caps (defaults pending the human's review, Q6):

| What | Cap | Past it |
|---|---|---|
| A followed file | 2 MiB and 50 000 lines | a file-level record only ("edited by session X") |
| Hashes kept per edit record | 5000 added lines | the range only (followed through the diff, not searchable after a commit) |
| Edit records per repository | 20 000 | the oldest go |
| Age of an edit record | 90 days | it goes |
| One repository's file | 8 MiB | first, the line maps of the files untouched longest go (those lines fall back to §6); then the oldest edit records |
| Repository files in all | 64 MiB | the least recently used repository's file goes |

Lifetimes:

- A worktree's line maps go when Hive removes the worktree or sees it gone (as 27.1's marks), but its edit records stay for §6.
- A project removed from Hive takes its repository's file with it.
- A session deleted from the Sessions view (4.11) keeps its edit records. The lookup then says "session deleted" (Q7).

Size in practice: a 10 000-line file's map is about 110 KB in base64, and an edit of 30 lines is about 0.5 KB. 20 000 such records come to about 10 MB, which is why the repository cap evicts.

## 8. Security and logging

- A hook frame can come from any process of the same user (socket 0600). Provenance is therefore **advisory**: it tells the user what Hive saw, and it is never proof and never input to anything that acts. The UI wording says "recorded" and "probably", never "verified".
- Every path is checked as in §2: inside a followed worktree, not in `.git`, a regular file, and read with the size cap. Numbers from the payload are bounded (hunk counts, line numbers ≤ `lines` + 1, run lengths). Ids are checked against their character sets.
- A claim whose hashes do not match the file labels nothing: the hash check is the gate between payload and record.
- The service never sees file text from a payload (§3). `git blame` and `git show` text is hashed and dropped at once.
- Logs: at the default level, nothing from a payload. Only failures, as "cannot record an edit of <relative path>: <reason>", with the path clipped like activities and never any content. A `security-reviewer` run is due on the hook slimming and the new git calls.

## 9. Measurements (Apple Silicon Mac, release build, 2026-11-26)

Throwaway benchmark in a scratch folder: `serde_json::from_slice` into `Value`, then FNV-1a over the lines of `originalFile`.

| Payload | Lines | Parse | Hash lines |
|---|---|---|---|
| 1 MB | 12 346 | 0.9 ms | 0.7 ms |
| 10 MB | 123 457 | 9.0 ms | 6.8 ms |
| 30 MB | 370 371 | 26.7 ms | 20.8 ms |

`git blame --porcelain` on this repository: one line of `crates/hive/src/states.rs` took 22 ms; all of `TODO.md` took 272 ms.

The diff is not measured yet: Myers on hashes is O((N+M)·D). An agent's edit has a small D, so a 50 000-line file with a few hunks costs well under a millisecond. The D cap bounds a file rewritten end to end. Task 27.5-b measures it on a 50 000-line file with 1 and 500 hunks.

## 10. UI (presentation only, #37)

- **"Who wrote this line?"**
  - On a right-click on a line number in the editor (CodeView/EditView) and on either side of the diff. This needs a small Hive menu on the gutter, like `FileMenu`. The text area keeps the native menu #53 gives it.
  - Also as a palette command for the cursor's line.
  - No new shortcut, since #35 is the human's (Q8).
- **The answer:**
  - **Session (live):** opens the right panel's Sessions tab, scrolls to and selects that session, opens its "Files changed" list with the file highlighted, and shows the edit's time and tool, e.g. "Edit · 3h ago · lines 120–122 · you changed it before accepting". A subagent shows as "general-purpose subagent of <session title>".
    - The session may belong to another worktree (an agent edited a file outside its cwd, or the lookup went through a commit after a merge), while 4.11 lists only the shown worktree's sessions. In that case the Sessions view shows that one session at the top under "From <worktree>", without switching worktree (Q4).
  - **Probable (through a commit):** a popover by the line: "Probably <session title>, committed in abc1234 · 2 days ago", with "Show session". The word "probably" stays.
  - **You (Hive's editor):** a popover: "You, in Hive's editor · 10m ago".
  - **Unknown:** a popover: "No session recorded for this line. It was changed outside an agent's Edit or Write (a shell command, another editor, git), or before Hive recorded the file." It adds the blame's commit, author and time when there is one.
- **"At that edit".** Hive has had no conversation view since 9.35 (the human did not want one), so "at that edit" means the session row with the edit highlighted. "Open Log" (4.11) still opens the `.jsonl`. `tool_use_id` is kept so a later view could jump to the entry (Q4).
- **The session row's "Files changed".**
  - A count on the row ("7 files").
  - Expanding the row lists each file (relative path, worktree when not the shown one), its number of edits, how many of its lines are still the session's now (from the origin runs), and the last time.
  - A click on a file opens it in the editor at the first of those lines.
  - It is served by a new request (below), not added to the 5-second session listing, so the listing's cost does not change.

Protocol (proposal; `docs/architecture.md`'s catalog is updated with it):

- `who_wrote {worktree, path, line, side: "worktree" | "base", base?: DiffBase}` returns `who_wrote {path, line, origin}`. `origin` is one of:
  - `{session: {id, agent?, agent_type?, tool, tool_use_id, ms, user_modified, lines: [start, len]}, via: "live" | {commit, probable: true}}`
  - `{editor: {ms}}`
  - `{unknown: {commit?: {id, author, ms, summary}}}`
- `session_files {id}` returns `session_files {id, files: [{worktree, path, edits, lines_now, last_ms}], truncated}`, at most 500 files, sorted by `last_ms`.
- `Session` gains `files_changed: number` (0 when unknown), read from the store's in-memory index.

## 11. Remote hosts (Stage 26)

Nothing new is needed. On a remote host the wrapper, the hooks, the files and git are that host's, so `hive hook` there forwards to that host's service. The store sits in that host's `~/.hive/provenance/`, and `who_wrote` / `session_files` go to the service that owns the worktree, like every other request since 26.x.

A Mac and a host never mix records. A repository cloned on both has two independent stores, and each answers for its own worktrees.

With Stage 28 (two people on one host), the records are already on the host and shared. Seen marks and comments get an author field (27.3), and provenance needs none: its "author" is the session.

## 12. Questions for the human

- **Q1. One shared `anchor` module for 27.3 and 27.5, built by whichever lands first, with §5 as the interface 27.3 follows.** *Recommendation: yes.* Only the code is shared; each feature keeps its own store, caps and lifetime.
- **Q2. A hand-written bounded Myers diff over line hashes, or a crate (`imara-diff` or `similar`)?** *Recommendation: hand-written* (about 100 lines, no new dependency, fully covered by tests and mutants). A crate would need your approval (rule 3) and brings text-diff features Hive does not need.
- **Q3. Lines changed by an agent's Bash (`sed -i`, generators) or by a formatter hook: "unknown", or a guess "probably session X (shell command)" from timing?** *Recommendation: "unknown" in v1.* A guess from timing alone goes against what 27.7 settled, and the wording already tells the user why.
- **Q4. "Opens the session in the Sessions view at that edit".** *Recommendation:* the session's row is selected, with its "Files changed" list open and the file and edit highlighted, and no transcript view (removed by 9.35 at your request). A session from another worktree is shown at the top of the Sessions view under "From <worktree>", without switching the shown worktree.
- **Q5. `Write` over an existing file with no patch** (Claude Code skipped the diff: the previous content was over ~10 MB, or the diff timed out). Should every line be the session's, or only the lines whose hashes differ from Hive's stored map? *Recommendation: diff against the stored map when there is one; otherwise every line is the session's* (it did write the whole file).
- **Q6. Caps and lifetime** (§7): 2 MiB / 50 000 lines per file, 20 000 records and 8 MiB per repository, 90 days, 64 MiB in all, line maps dropped with the worktree and records kept for lookups through a commit. *Recommendation: as listed*, recorded as `pending` in the defaults ledger.
- **Q7. When a session is deleted in the Sessions view, keep its edit records** (the lookup says "session deleted"), **or delete them with it?** *Recommendation: keep them.* The lines are still in the code, and "a deleted session wrote this" is still an answer.
- **Q8. A keyboard shortcut for "Who wrote this line?"** *Recommendation: none.* Use the gutter menu and the palette, since #35's shortcut list is yours to extend.
- **Q9. Raise `hive hook`'s read limit for `PostToolUse` from 512 KiB to 32 MiB, and slim every `PostToolUse` payload before sending** (it also fixes a large Bash output losing its event today). *Recommendation: yes.* Parsing and hashing 30 MB took about 48 ms (§9), well inside the hook's 1 s timeout.

## 13. Proposed implementation tasks (after approval)

Each one has the usual gates (100% lines, mutants of the diff) and is small enough for one agent.

1. **27.5-a `anchor` module** (if 27.3 has not built it): `line_hash`, `hashes`, `digest`, bounded `diff`/`LineMap`, `Anchor`, `relocate`.
   - Tests: hashes ignore `\r\n` vs `\n`; diff on insert, delete, move and replace; the D cap falls back to "all replaced" and stops; `range` gives Kept, Partly and Gone; `relocate` finds a moved run nearest the hint, breaks ties by context, refuses a trivial line, and returns `None` once the text is gone.
   - A property test: diff then map, against applying the edit script.
2. **27.5-b slim hook payloads** (`hook.rs`): a 32 MiB read for `PostToolUse`, the bounded id scan past it, `slim()` for every tool, and `hive_edit` for Edit/Write/MultiEdit.
   - Tests from fake payloads (recorded shape, text replaced): Edit with one hunk and with `replace_all`; Write create and update; update with an empty patch; `userModified`; a string `tool_response` (failure); oversized input (id kept, marker); malformed hunks dropped; `--record` still writes the raw payload; the forwarded frame stays under 512 KiB.
   - `security-reviewer`.
3. **27.5-c the store and recording** (`provenance.rs`, used from `daemon::agents::saw`): place the file, check the path, apply the edit record (§4 steps 1–6), re-sync triggers, editor saves as `editor`, atomic and debounced writes, the caps and eviction, drop on worktree or project removal.
   - Rust tests: record from a fake payload; lines followed through a later Edit, a Bash-style rewrite (unknown), a formatter change after the edit (relocated), and a revert (re-claimed, but `}` not); two sessions on one file in `sent_ns` order; a path outside the worktree, in `.git`, or a FIFO is refused; each cap; a malformed store is set aside; nothing of the payload in the logs.
4. **27.5-d lookup and the commit path:** `who_wrote` (live, then `git blame` plus record search at C's time), the base side, deleted lines, and the `session_files` index.
   - Rust tests on a temp repository: live after a commit; another worktree after a merge (probable); a squash still found; a trivial line not matched; no record gives unknown with the commit; rename followed.
   - Integration in `crates/hive/tests/integration/` with `common::Env` and the fake `claude`: `hive hook PostToolUse` fed fake payloads from a Hive terminal, then `who_wrote` over the socket.
5. **27.5-e protocol and app:**
   - `Control` variants, handlers, Tauri commands, `Transport` methods and the mock.
   - The gutter menu in the editor and both diff sides, and the palette command.
   - The answer popover, the Sessions view selection/"From <worktree>", the "Files changed" list on the session row, and `files_changed` on `Session`.
   - `docs/architecture.md`'s catalog.
   - Bun tests for each new file (menu, popover wording for each origin, the Sessions row and list); e2e on the mock transport: right-click a line, then the Sessions tab opens at the session with the file highlighted; an unknown line shows the popover.

The recording of §2 comes before 27.5-b, so the fixtures match what Claude Code sends.
