# Spike 28.11: Jujutsu repositories

Spike task 28.11 (`TODO.md`, no production code). The question: what happens when a Hive project is a **colocated** Jujutsu (`jj`) repository, one with both `.jj/` and `.git/` at its root, as Delta's early support assumes. Covered: worktrees, the Changes panel, the diff, the sidebar's health, 9.36's registry watch, and what the test suites assume. Written 2026-11-27 against `main` (v0.32.0), git 2.54.0 (Apple Git-157) on the Apple Silicon host.

**`jj` is not installed on the host**, and was not installed for the spike (no global install without the human's approval). The findings marked **tried** come from plain git in a temporary folder (temporary `HOME`, `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM`), reproducing what a colocated jj repository leaves behind: a detached `HEAD` at the working copy's parent, a `.jj/` folder holding a `.gitignore` of `/*`, refs under `refs/jj/keep/`, a local bookmark exported as `refs/heads/main` that does not follow new commits. The findings marked **reasoned** come from jj's documentation on colocated repositories and workspaces and from reading Hive's code; they need one check with a real `jj` before any fix is built (section 6).

## 1. Recommendation in one paragraph

Hive mostly works in a colocated jj repository today, because **every worktree Hive or Claude Code makes is a plain git worktree on a branch**: agents, their Changes panel, their diff, commit and push behave exactly as in a git repository. What breaks comes from one fact: jj keeps git's `HEAD` **detached** in the main worktree, always. Hive takes "the branch the other worktrees are compared with" from the main worktree's branch (9.11, health), so in a jj repository **no linked worktree has a branch base**: no ahead/behind, never "merged", "Compare with: branch" is an error, and the remove-merged dialog finds nothing. The fix worth doing is small (about **2 pts**): when the main worktree is detached and a `.jj/` folder sits next to its `.git`, take the base branch from the refs jj exports (`refs/heads/main`, then `master`, then `trunk`, the names jj's own `trunk()` looks for), without ever running `jj`. Everything else is either already fine or a warning in the Changes panel of the main worktree only (staging and committing there with git surprises a jj user; about **1 pt**). Running the `jj` binary, showing jj changes or supporting jj workspaces is **not recommended** now: it costs a new external program, its own snapshots writing into `.jj/` and `.git/` (#52 observe only, #56), and a CI install, for a niche of Hive's users. Decision for the human: Q1 and Q2 (section 7).

## 2. What jj leaves in a colocated repository

| What | Effect on git | How known |
|---|---|---|
| `.git/HEAD` detached at `@-`, the parent of jj's working-copy commit | `git worktree list` shows the main worktree as `detached`; `git symbolic-ref HEAD` fails | tried (state), reasoned (jj always does it) |
| The git index reset to `@-`'s tree after a jj command that moves `@-` | `git status` shows jj's working-copy changes as unstaged/untracked, as Hive expects | tried (state), reasoned (jj writes it) |
| `.jj/.gitignore` with `/*` | git lists `.jj/` as one ignored folder | tried |
| `refs/jj/keep/<id>` per kept commit (older jj; newer versions keep fewer) | invisible to `for-each-ref refs/heads refs/remotes`, which is all Hive reads | tried |
| Bookmarks exported as `refs/heads/<name>`; they move only by `jj bookmark set` or `jj git fetch` | `refs/heads/main` exists but usually lags the work | tried (state), reasoned |
| `jj workspace add <dir>`: a second workspace with only `.jj/repo` pointing back, **no `.git`** | git says "not a git repository", or, inside another repository, finds that one | tried |
| A non-colocated repository (`jj git init` without colocation; the default before jj made colocation the default) | no `.git` at the root: not a repository for Hive | reasoned |

## 3. Area by area

### 3.1 Worktrees (sidebar, create, remove)

- **Listing (tried):** `git worktree list --porcelain -z` works; the main worktree comes as `HEAD <id>` + `detached`, so `worktree::Worktree.branch` is `None`. The sidebar still names it by its folder, as `projects::worktrees` does for any detached worktree; it just shows no branch.
- **Create (tried):** `git worktree add -b worktree-<name> .claude/worktrees/<name>` with no start point starts from `HEAD`, which is `@-`: the user's last described change, not the uncommitted working copy (`@`). That is what git does with uncommitted work too, so the behaviour is consistent. From a base branch (13.2) works as before.
- **Remove (reasoned):** `git worktree remove` and the branch delete are pure git; jj imports the deleted branch on its next command as a deleted bookmark. Nothing breaks.
- **jj sees Hive's worktrees (reasoned, to check):** a linked worktree under `.claude/worktrees/<name>/` sits inside jj's working copy. jj skips any folder that holds a `.git` (as it does submodules), so the worktree's files should not be snapshotted into `@`; and when `.claude/worktrees/` is gitignored (Claude Code's own advice) jj skips it anyway. If the check in section 6 shows jj does snapshot them, the human's jj history would fill with every agent's files: then Hive should say so when a worktree is created in a jj repository (the cheap answer) rather than move worktrees elsewhere (#7, a decision).
- **jj workspaces (tried, partly):** a second jj workspace has no `.git`. Added to Hive, it is "not a git repository" (or, nested in a group folder, a folder project, 23.1); nested inside the repository, git finds the parent repository instead. Hive does not see jj workspaces as worktrees of the project. Not a regression: Hive is built on git worktrees (#4).

### 3.2 Changes panel and diff

- **Main worktree against `HEAD` (tried):** `git status --porcelain=v2` lists jj's working-copy changes (`a` modified, `wc` untracked) against `@-`, which is what `jj diff` shows. The diff, line counts and the turn snapshots of 27.2 (a temporary index, `git add -A`) work unchanged; `.jj/` is ignored so it never shows as a change.
- **Linked worktrees against `HEAD` (tried):** plain git worktrees on a branch, unchanged.
- **"Compare with: branch" (tried): broken.** `changes::merge_base` needs the main worktree's branch; with it detached every linked worktree gets `NO_BRANCH` ("this is the main worktree, or it is detached"). The commit exists: `git merge-base HEAD refs/heads/main` in the linked worktree answers.
- **"A commit…" (tried):** `git log … HEAD` lists the commits of `@-` and its ancestors; fine.
- **Stage, commit, push from the main worktree (reasoned):** git has an index and jj has none. jj resets the index to `@-` on its next command that moves `@-`, so what the user staged in Hive can vanish. A `git commit` on the detached `HEAD` (tried: it works, `HEAD` moves to the new commit) is imported by jj on its next command, which puts a new working-copy commit on top of it; jj documents mixing mutating git and jj commands as workable but confusing. Push is refused already ("on no branch (a detached HEAD)", `commit.rs:265`), which is right: jj users push with `jj git push`. In linked worktrees all three work as in git.
- **Review worktrees (27.4, reasoned):** a review of the main worktree is refused as detached (`review.rs:202`); a review of a linked worktree works.

### 3.3 Health (sidebar)

- `git log -1 --format=%ct HEAD --` and the change counts work (tried).
- **Broken (tried):** `health::branch` takes the main worktree's branch; `None` leaves every linked worktree with `ahead`/`behind` `None` and `merged: false`. The sidebar shows no ahead/behind, and "remove merged worktrees" never offers one. With `refs/heads/main` as the base, `git rev-list --left-right --count HEAD...refs/heads/main --` answers (`1 0` in the reproduction).
- The tick's cost is unchanged.

### 3.4 9.36's registry watch

- The common git dir is `.git` (tried: `rev-parse --git-common-dir`), so the watch is the same as in a git repository; `.jj/` is never watched.
- **More events, no harm (reasoned):** jj rewrites `.git/HEAD` whenever `@-` changes (`jj new`, `jj commit`, `jj squash`, `jj describe @-`, a rebase), and `Registry::counts` counts the common dir's `HEAD` (19.5). Each such jj command costs one coalesced re-list (`worktree list` per project); `State::worktrees_changed` then sends nothing, since the detached main worktree's `branch` stays `None` and the list is the same. A busy jj session costs a few `git worktree list` runs a minute.
- jj's writes of `.git/index` and `.git/packed-refs` do not count (`counts` takes only `worktrees/`, its entries and `HEAD`). The files panel does re-list on the `index` write (it counts `HEAD` and `index` in the git dir), as it does for a `git add`.

### 3.5 The test suites' assumptions

- 32 test files make repositories with `git init` and assume a git repository whose main worktree is on a branch (health's and the Changes panel's branch base, `projects.rs`' naming, review). None is wrong for jj; none covers it.
- A fix can be tested **without jj**: the reproduction in section 2 (detached `HEAD`, `.jj/.gitignore`, `refs/heads/main` lagging) is what Hive reads, built with plain git in a temporary repository, as the existing tests build theirs. No CI change.
- A test with the real `jj` would need it in CI (`cargo install jj-cli` is several minutes per runner, or a pinned release download on three platforms) and a temporary `HOME` for its config: not worth it for this scope.

## 4. What breaks, ranked

| # | What | Who sees it | Severity |
|---|---|---|---|
| B1 | No branch base for any linked worktree: no ahead/behind, never merged, "Compare with: branch" errors, remove-merged finds none | every jj user with agents in worktrees | high: the sidebar's main review signal |
| B2 | Staging/committing with git in the main worktree surprises jj (index reset, `HEAD` moved under jj) | a jj user who uses Hive's Stage/Commit on the main worktree | medium |
| B3 | jj might snapshot Hive's worktrees into `@` (to check, section 6) | jj users without `.claude/worktrees/` ignored | high if true, likely false |
| B4 | jj workspaces are not Hive worktrees; non-colocated jj repositories are not projects | users of those | low: not regressions |
| B5 | More registry re-lists per jj command | nobody (no message sent) | none |

## 5. What a fix costs

- **F1, the base branch in a jj repository (B1): about 2 pts.** One function, used by `health::branch` and the Changes panel's branch base (and so by remove-merged): the main worktree's branch as today; when it is detached **and** `<main root>/.jj` is a folder, the first of `main`, `master`, `trunk` that exists under `refs/heads/` (one `for-each-ref` the service already runs for branches, 13.2), else `None` as today. It mirrors jj's default `trunk()` revset (the `main`/`master`/`trunk` bookmark), never runs `jj`, writes nothing. A user who changed `trunk()` in their jj config is not followed (Q1). Tests: unit and integration on the plain-git reproduction. The base being a bookmark that jj moves only on `jj git fetch` or `jj bookmark set` means "ahead" counts from the last fetched trunk, which is also what the branch base means in a git repository whose main worktree is not pulled.
- **F2, a note on the main worktree's Stage/Commit in a jj repository (B2): about 1 pt.** The service already knows the project's root; it marks the project as jj (`.jj/` present) and the Changes panel says "This is a jj repository: jj rewrites git's index; commit with jj" next to Stage/Commit of the main worktree only. Hiding the buttons instead is a UI decision (Q2).
- **F3, B3 if the check shows it:** a note at worktree creation in a jj repository whose `.claude/worktrees/` is not ignored: about 1 pt. Moving worktrees out of the repository would change #7: not proposed.
- **Not recommended now:** running `jj` (`jj log -r 'trunk()' --ignore-working-copy`) to honour the user's `trunk()` (+2 pts, a new program to find, time-limit and fake in tests; without `--ignore-working-copy` jj would snapshot and write, against #52); showing jj changes (`@`, change ids, descriptions) instead of git's view (a new model, 8+ pts); jj workspaces as worktrees (a second worktree system next to #4's, 8+ pts).

## 6. To check with a real `jj` before building F1–F3

With the human's approval to install `jj` (for instance a release binary in a temporary folder, not `brew install`), in a temporary `HOME`: (1) `jj git init --colocate` then `jj new`/`jj commit`: `HEAD` detached, the index reset, which files under `.git/` are written; (2) a `git worktree add` under `.claude/worktrees/x` with files in it, not ignored, then `jj status`: whether jj snapshots them (B3); (3) a `git commit` in the main worktree, then `jj log`: how jj takes it (B2); (4) whether the installed version's `jj workspace add` still makes a workspace without `.git`.

## 7. Questions for the human

- **Q1.** Build F1 (the `main`/`master`/`trunk` fallback for a detached main worktree in a jj repository), and as a 28.x/29.x task? Recommended: yes, without running `jj`.
- **Q2.** For B2, a note next to Stage/Commit of a jj repository's main worktree (F2), the buttons hidden there, or nothing? Recommended: the note.
- **Q3.** May an agent install `jj` (a release binary in a temporary folder) for the checks of section 6, before F1–F3? Recommended: yes, once, before F1.
