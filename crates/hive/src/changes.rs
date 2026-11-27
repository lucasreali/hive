//! What changed in a worktree ("Árvore de arquivos com diff do git"): every file that
//! differs from its base, staged or not, untracked included, with line counts from
//! `git diff --numstat`. The base is `HEAD` (what `git status` shows) or, for a worktree
//! other than the main one, the merge-base with the main worktree's branch (9.11), so the
//! agent's commits stay in view. Git runs as the executable with separate arguments.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use hive_protocol::{
    ChangedFile, CommitInfo, Control, DiffBase, FileStatus, Project, Staged, TurnBase,
};

use crate::git::{self, read_limited};
use crate::projects;
use crate::seen::Seen;
use crate::snapshot::{self, Since, Snapshots, Turn};

/// Untracked files larger than this are not counted (their lines show as unknown).
const UNTRACKED_LIMIT: u64 = 8_388_608; // 8 MiB
/// Most bytes of untracked files read for their line counts in one listing (9.14); past it
/// the rest show unknown counts too. Default pending the human's review.
const UNTRACKED_BUDGET: u64 = 33_554_432; // 32 MiB
/// Git's binary heuristic: a NUL byte in the first 8000 bytes.
pub(crate) const BINARY_PROBE: usize = 8000;
/// Most bytes of JSON for the files of one `changes` message, well under `MAX_PAYLOAD`.
const MESSAGE_BUDGET: usize = 3_145_728; // 3 MiB
/// What a file's seen mark (27.1) may add to its JSON, set after the budget was counted.
const SEEN_ROOM: usize = r#","seen":"changed""#.len();

/// The `git status` whose entries [`parse_status`] reads.
pub const STATUS: [&str; 5] = [
    "status",
    "--porcelain=v2",
    "-z",
    "--untracked-files=all",
    "--find-renames",
];

/// A worktree's changes; `files` may be cut short (`truncated`), the totals never are.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub files: Vec<ChangedFile>,
    pub added: u64,
    pub removed: u64,
    /// How many files were left out to keep the message within its budget.
    pub truncated: usize,
    /// The files `git status` lists: the worktree's status counts them (`health`).
    pub changed: u64,
}

/// A worktree's changes against `HEAD` as its status counts them (`health`): the files
/// `git status` lists and their lines added and removed (a binary file adds none).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    pub files: u64,
    pub added: u64,
    pub removed: u64,
}

/// What a followed worktree's changes are compared with (9.11, 27.2).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Against {
    pub dir: PathBuf,
    /// The merge-base with `branch`, or the commit picked; `None`: `HEAD` (or `since`).
    pub commit: Option<String>,
    /// The agent's last turn, when that is the base.
    pub since: Option<Since>,
    /// The base used: the one asked, or `head` when `error` says why not.
    pub base: DiffBase,
    /// The main worktree's branch the worktree can be compared with.
    pub branch: Option<String>,
    /// Why the base asked is not used but `HEAD` is.
    pub error: Option<String>,
}

/// The base `asked` for the worktree `path` of `projects`: for `branch`, the merge-base of its
/// `HEAD` and the main worktree's branch (the one 6.6 counts against); for a commit, that
/// commit; for a turn, its snapshot in `snapshots`; else `HEAD` and why.
pub fn against(
    projects: &[Project],
    snapshots: &Snapshots,
    path: &str,
    asked: &DiffBase,
) -> io::Result<Against> {
    let (dir, branch) = projects::followed(projects, path)?;
    let kept = match asked {
        DiffBase::Turn { .. } => snapshots.turns(&dir, path),
        _ => Vec::new(),
    };
    Ok(based((dir, branch), snapshots, &kept, asked))
}

/// [`against`] for the worktree at `dir` on `branch`, a turn base found in `kept`, the
/// worktree's turns read once.
fn based(
    (dir, branch): (PathBuf, Option<String>),
    snapshots: &Snapshots,
    kept: &[Turn],
    asked: &DiffBase,
) -> Against {
    let mut against = Against {
        dir,
        branch,
        ..Against::default()
    };
    let found = match asked {
        DiffBase::Head => return against,
        DiffBase::Branch => merge_base(&against.dir, against.branch.as_deref()).map(Some),
        DiffBase::Commit { id } => verify(&against.dir, id).map(Some),
        DiffBase::Turn { session } => match snapshots.since_in(&against.dir, kept, session) {
            Ok(since) => {
                against.since = Some(since);
                Ok(None)
            }
            Err(err) => Err(err.to_string()),
        },
    };
    match found {
        Ok(commit) => (against.commit, against.base) = (commit, asked.clone()),
        Err(error) => against.error = Some(error),
    }
    against
}

/// The commit `asked` compares the worktree `path` of `projects` with: the merge-base or the
/// commit picked; `None` for `HEAD`, for an agent's turn (its snapshot is a tree, no commit),
/// and for a base that falls back to `HEAD`.
pub fn base_commit(projects: &[Project], path: &str, asked: &DiffBase) -> Option<String> {
    let (dir, branch) = projects::followed(projects, path).ok()?;
    match asked {
        DiffBase::Branch => merge_base(&dir, branch.as_deref()).ok(),
        DiffBase::Commit { id } => verify(&dir, id).ok(),
        DiffBase::Head | DiffBase::Turn { .. } => None,
    }
}

/// The merge-base of `HEAD` in `dir` and the main worktree's `branch`, or why there is none.
fn merge_base(dir: &Path, branch: Option<&str>) -> Result<String, String> {
    let name = branch.ok_or(NO_BRANCH)?;
    // A full ref name: a branch can never be read as an option.
    let theirs = format!("refs/heads/{name}");
    match git::output(dir, &["merge-base", "HEAD", &theirs], &[0, 1]) {
        Ok(out) if !out.is_empty() => Ok(lossy(out.trim_ascii())),
        Ok(_) => Err(format!("No commit in common with {name}")),
        Err(err) => Err(err.to_string()),
    }
}

/// The commit `id` from the app: only a full id (40 or 64 hex digits), checked to be a commit
/// of the repository at `dir` (any failure of git's says it is not).
fn verify(dir: &Path, id: &str) -> Result<String, String> {
    let hex = matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit());
    if !hex {
        return Err("Not a commit id".to_owned());
    }
    let rev = format!("{id}^{{commit}}");
    let args = ["rev-parse", "--verify", "--quiet", "--end-of-options", &rev];
    match git::output(dir, &args, &[0, 1]) {
        Ok(out) if !out.is_empty() => Ok(lossy(out.trim_ascii())),
        _ => Err(GONE.to_owned()),
    }
}

/// Why the main worktree, or a worktree of a detached main worktree, has no branch base.
const NO_BRANCH: &str = "No branch to compare with: this is the main worktree, or it is detached";
/// Why a commit picked is not the base (e.g. after a rebase).
const GONE: &str = "This commit is not in the repository";

/// The `changes` answer for the worktree `path` of `projects` against `asked`, each file with
/// its mark of `seen` (27.1), and, when it was listed against `HEAD`, its [`Totals`]. Its
/// `turns` are the agents' of the worktree with a snapshot in `snapshots`, named by `titles` (by
/// session).
pub fn answer(
    projects: &[Project],
    snapshots: &Snapshots,
    path: String,
    asked: &DiffBase,
    titles: &HashMap<String, String>,
    seen: &Seen,
) -> (Control, Option<Totals>) {
    marked(projects, (snapshots, titles), path, asked, seen, None)
}

/// The `mark_seen` answer (27.1): the `files` of the worktree `path` marked seen or not
/// (`marks`) against `asked`, then its `changes`; `error` when the marks cannot be saved.
pub fn mark(
    projects: &[Project],
    turned: (&Snapshots, &HashMap<String, String>),
    path: String,
    asked: &DiffBase,
    seen: &Seen,
    files: (&[String], bool),
) -> Control {
    marked(projects, turned, path, asked, seen, Some(files)).0
}

/// [`answer`], first marking `files` seen or not when given.
fn marked(
    projects: &[Project],
    (snapshots, titles): (&Snapshots, &HashMap<String, String>),
    path: String,
    asked: &DiffBase,
    seen: &Seen,
    files: Option<(&[String], bool)>,
) -> (Control, Option<Totals>) {
    let followed = projects::followed(projects, &path).map(|(dir, branch)| {
        // Read once: the base and the turns listed agree, though a snapshot is taken meanwhile.
        let kept = snapshots.turns(&dir, &path);
        (based((dir, branch), snapshots, &kept, asked), kept)
    });
    match followed {
        Ok((mut against, mut kept)) => {
            let mut listed = listing(&mut against, &mut kept);
            if let Ok(changes) = &mut listed {
                let marking = files
                    .map(|(paths, marks)| seen.mark(&path, &against, &changes.files, paths, marks));
                if let Some(Err(err)) = marking {
                    let message = format!("Cannot mark the files as seen: {err}");
                    return (Control::Error { message }, None);
                }
                seen.apply(&path, &against, &mut changes.files);
            }
            let totals = listed.as_ref().ok().map(|c| Totals {
                files: c.changed,
                added: c.added,
                removed: c.removed,
            });
            let totals = totals.filter(|_| against.base == DiffBase::Head);
            let turns = turns(kept, titles);
            (message(path, Some(against), listed, turns), totals)
        }
        Err(err) => (message(path, None, Err(err), Vec::new()), None),
    }
}

/// The changes against `against`, whose turn base was found in `kept`. A snapshot that cannot
/// be listed falls back to `HEAD` and says why: no longer whole, or no longer kept (dropped
/// since it was read, then also left out of `kept`).
fn listing(against: &mut Against, kept: &mut Vec<Turn>) -> io::Result<Changes> {
    let since = against.since.as_ref();
    let why = match since.map(|since| (since_turn(&against.dir, since), since)) {
        None => return list(&against.dir, against.commit.as_deref()),
        Some((Ok(changes), _)) => return Ok(changes),
        // Dropped (its `SessionEnd`) while this waited for its store.
        Some((Err(err), since)) if snapshot::dropped(&err) => {
            kept.retain(|t| t.session != since.turn.session);
            err.to_string()
        }
        // A blob the user's `git gc` removed: the snapshot is no longer whole.
        Some((Err(err), _)) => format!("The snapshot of this turn is no longer complete: {err}"),
    };
    against.since = None;
    against.base = DiffBase::Head;
    against.error = Some(why);
    list(&against.dir, None)
}

/// The agents of `kept`, the worktree's [`Snapshots::turns`], each named by its title in
/// `titles`, else by the start of its session id.
fn turns(kept: Vec<Turn>, titles: &HashMap<String, String>) -> Vec<TurnBase> {
    kept.into_iter()
        .map(|turn| TurnBase {
            title: titles
                .get(&turn.session)
                .cloned()
                .unwrap_or_else(|| turn.session.chars().take(8).collect()),
            session: turn.session,
            started_ms: turn.started_ms,
            late: turn.late,
        })
        .collect()
}

/// The changes of the worktree at `dir` since the turn `since`: every file that differs from
/// its snapshot, with its lines from numstat, and what of each is staged (27.6, from
/// `git status`: a file the turn changed and that was committed since has nothing to stage).
pub fn since_turn(dir: &Path, since: &Since) -> io::Result<Changes> {
    let (raw, numstat) = since.list()?;
    let counts = parse_numstat(&numstat);
    let staged = parse_staged(&git(dir, &STATUS)?);
    Ok(collect(
        dir,
        parse_raw(&raw),
        (&counts, &staged),
        UNTRACKED_BUDGET,
    ))
}

/// The most commits `commits` lists (the human, 2026-11-26).
pub const COMMITS: usize = 500;
/// The most characters of a commit's author and subject sent.
const AUTHOR_LIMIT: usize = 100;
const SUBJECT_LIMIT: usize = 200;

/// The `commits` answer for the worktree `path` of `projects` (27.2): the last [`COMMITS`] of
/// its `HEAD`, those of its branch first.
pub fn commits(projects: &[Project], path: String) -> Control {
    let listed =
        projects::followed(projects, &path).and_then(|(dir, branch)| log(&dir, branch, COMMITS));
    let (commits, truncated, error) = match listed {
        Ok((commits, truncated)) => (commits, truncated, None),
        Err(err) => (Vec::new(), false, Some(err.to_string())),
    };
    Control::Commits {
        path,
        commits,
        truncated,
        error,
    }
}

/// The last [`COMMITS`] commits of `HEAD` in `dir`, those made since it left the main
/// worktree's `branch` first, and whether more were left out.
fn log(dir: &Path, branch: Option<String>, most: usize) -> io::Result<(Vec<CommitInfo>, bool)> {
    let head = git::output(dir, &["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1])?;
    if head.is_empty() {
        return Ok((Vec::new(), false));
    }
    let fetched = format!("--max-count={}", most + 1);
    let format = "--format=%H%x00%h%x00%an%x00%ct%x00%s";
    let out = git(dir, &["log", "-z", format, &fetched, "HEAD", "--"])?;
    let mine = match merge_base(dir, branch.as_deref()) {
        Ok(base) => git(dir, &["rev-list", &fetched, &format!("{base}..HEAD"), "--"])?,
        Err(_) => Vec::new(),
    };
    let mine = String::from_utf8_lossy(&mine);
    let mine: Vec<&str> = mine.lines().collect();
    let fields: Vec<String> = out.split(|&b| b == 0).map(lossy).collect();
    let mut commits: Vec<CommitInfo> = fields
        .as_chunks::<5>()
        .0
        .iter()
        .map(|c| CommitInfo {
            on_branch: mine.contains(&c[0].as_str()),
            id: c[0].clone(),
            short: c[1].clone(),
            author: c[2].chars().take(AUTHOR_LIMIT).collect(),
            time_ms: c[3].parse::<u64>().unwrap_or(0).saturating_mul(1000),
            subject: c[4].chars().take(SUBJECT_LIMIT).collect(),
        })
        .collect();
    commits.sort_by_key(|c| !c.on_branch);
    let truncated = commits.len() > most;
    commits.truncate(most);
    Ok((commits, truncated))
}

/// The changes of the worktree at `dir` against `commit`, else `HEAD` (the empty tree before
/// the first commit). Against `HEAD` they are what `git status` lists; against another commit,
/// what `git diff` finds from it to the worktree, and the untracked files.
pub fn list(dir: &Path, commit: Option<&str>) -> io::Result<Changes> {
    let out = git(dir, &STATUS)?;
    let status = parse_status(&out);
    let staged = parse_staged(&out);
    let changed = status.len() as u64;
    let (base, entries) = match commit {
        Some(commit) => {
            let mut entries = committed(dir, commit)?;
            entries.extend(status.into_iter().filter(|e| e.1 == FileStatus::Untracked));
            (commit.to_owned(), entries)
        }
        None => (head(dir)?, status),
    };
    let counts = parse_numstat(&git(dir, &numstat(&base))?);
    Ok(Changes {
        changed,
        ..collect(dir, entries, (&counts, &staged), UNTRACKED_BUDGET)
    })
}

/// The `git diff` whose output [`parse_numstat`] reads: from `base` to the worktree.
pub fn numstat(base: &str) -> [&str; 8] {
    [
        "diff",
        "--numstat",
        "-z",
        "--find-renames",
        "--no-ext-diff",
        "--no-textconv",
        base,
        "--",
    ]
}

/// `HEAD`'s commit, or the empty tree before the first commit.
pub(crate) fn head(dir: &Path) -> io::Result<String> {
    let head = git::output(dir, &["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1])?;
    let base = if head.is_empty() {
        git(dir, &["hash-object", "-t", "tree", "/dev/null"])?
    } else {
        head
    };
    Ok(lossy(base.trim_ascii()))
}

/// The tracked files that differ between `commit` and the worktree (committed since, staged
/// or not).
pub fn committed(dir: &Path, commit: &str) -> io::Result<Vec<Entry>> {
    let args = ["diff", "--raw", "-z", "--find-renames", commit, "--"];
    Ok(parse_raw(&git(dir, &args)?))
}

/// Parses `git diff --raw -z --find-renames`: `:<modes> <ids> <status>`, then the path, or
/// for a rename the old path and the new one. Any other status (a type change, an unmerged
/// path) counts as modified.
pub fn parse_raw(out: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut records = out.split(|&b| b == 0);
    while let Some(meta) = records.next() {
        let Some(code) = meta
            .strip_prefix(b":")
            .and_then(|m| m.rsplit(|&b| b == b' ').next())
        else {
            continue;
        };
        let path = records.next().unwrap_or_default().to_vec();
        entries.push(match code.first() {
            Some(b'R') => {
                let to = records.next().unwrap_or_default().to_vec();
                (to, FileStatus::Renamed, Some(path))
            }
            Some(b'A') => (path, FileStatus::Added, None),
            Some(b'D') => (path, FileStatus::Deleted, None),
            _ => (path, FileStatus::Modified, None),
        });
    }
    entries
}

/// The `changes` answer for `path`, listed against `against` when the worktree was found, with
/// the `turns` it can be compared with.
pub fn message(
    path: String,
    against: Option<Against>,
    listed: io::Result<Changes>,
    turns: Vec<TurnBase>,
) -> Control {
    let (changes, error) = match listed {
        Ok(changes) => {
            let error = (changes.truncated > 0)
                .then(|| format!("too many changes: {} files not shown", changes.truncated));
            (changes, error)
        }
        Err(err) => (Changes::default(), Some(err.to_string())),
    };
    let against = against.unwrap_or_default();
    Control::Changes {
        path,
        base: against.base,
        branch: against.branch,
        base_error: against.error,
        files: changes.files,
        added: changes.added,
        removed: changes.removed,
        error,
        turns,
    }
}

/// A status entry: path, status and the path a rename came from.
pub type Entry = (Vec<u8>, FileStatus, Option<Vec<u8>>);

/// Parses `git status --porcelain=v2 -z`. Ignored entries and headers are skipped; an
/// unmerged file counts as modified.
pub fn parse_status(out: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut records = out.split(|&b| b == 0);
    while let Some(record) = records.next() {
        let fields = |n| record.splitn(n, |&b| b == b' ').collect::<Vec<_>>();
        let entry = match record.first() {
            Some(b'?') => fields(2)
                .get(1)
                .map(|path| (path.to_vec(), FileStatus::Untracked, None)),
            Some(b'1') => fields(9).get(8).map(|path| {
                let xy = fields(3)[1];
                (path.to_vec(), ordinary(xy), None)
            }),
            Some(b'2') => fields(10).get(9).map(|path| {
                let from = records.next().map(<[u8]>::to_vec);
                (path.to_vec(), FileStatus::Renamed, from)
            }),
            Some(b'u') => fields(11)
                .get(10)
                .map(|path| (path.to_vec(), FileStatus::Modified, None)),
            _ => None,
        };
        entries.extend(entry);
    }
    entries
}

/// What of each file `git status --porcelain=v2 -z` lists is staged (27.6), from its `XY`
/// (index, worktree) code: an untracked or unmerged file nothing.
pub fn parse_staged(out: &[u8]) -> HashMap<Vec<u8>, Staged> {
    let mut staged = HashMap::new();
    let mut records = out.split(|&b| b == 0);
    while let Some(record) = records.next() {
        let fields = |n| record.splitn(n, |&b| b == b' ').collect::<Vec<_>>();
        // The fields of each kind of record, where its path is, and whether it can be staged.
        let (n, path, stageable) = match record.first() {
            Some(b'1') => (9, 8, true),
            Some(b'2') => {
                // A rename's old path is the next record.
                records.next();
                (10, 9, true)
            }
            Some(b'?') => (2, 1, false),
            Some(b'u') => (11, 10, false),
            _ => continue,
        };
        let fields = fields(n);
        let (Some(xy), Some(path)) = (fields.get(1), fields.get(path)) else {
            continue;
        };
        let state = match (stageable, xy.first(), xy.get(1)) {
            (false, ..) | (_, Some(b'.'), _) => Staged::No,
            (_, _, Some(b'.')) => Staged::Yes,
            _ => Staged::Partly,
        };
        staged.insert(path.to_vec(), state);
    }
    staged
}

/// A changed tracked file's status against `HEAD` from its `XY` (index, worktree) code.
fn ordinary(xy: &[u8]) -> FileStatus {
    if xy.first() == Some(&b'A') {
        FileStatus::Added
    } else if xy.contains(&b'D') {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    }
}

/// Lines added and removed by path, `None` for a binary file.
pub type Counts = HashMap<Vec<u8>, (Option<u64>, Option<u64>)>;

/// Parses `git diff --numstat -z`: path (the new one for a rename) → lines added and
/// removed, `None` for a binary file.
pub fn parse_numstat(out: &[u8]) -> Counts {
    let mut counts = HashMap::new();
    let mut records = out.split(|&b| b == 0);
    while let Some(record) = records.next() {
        let mut fields = record.splitn(3, |&b| b == b'\t');
        let (Some(added), Some(removed), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // A rename leaves the path empty and gives the old and new paths next.
        let path = if path.is_empty() {
            records.next();
            records.next().unwrap_or_default()
        } else {
            path
        };
        counts.insert(path.to_vec(), (number(added), number(removed)));
    }
    counts
}

fn number(field: &[u8]) -> Option<u64> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

/// The lines an entry adds and removes: from `counts`, or for an untracked file counted here
/// ([`count_lines`], from `budget`); `None` for a binary file.
fn lines(
    dir: &Path,
    (path, status, _): &Entry,
    counts: &Counts,
    budget: &mut u64,
) -> (Option<u64>, Option<u64>) {
    match status {
        FileStatus::Untracked => {
            let lines = count_lines(&dir.join(git::os_string(path)), budget);
            (lines, lines.map(|_| 0))
        }
        _ => counts.get(path).copied().unwrap_or((None, None)),
    }
}

/// The [`Totals`] of the status `entries` of the worktree at `dir` with their `counts` against
/// `HEAD`: the same lines [`list`] totals, reading the same untracked files.
pub fn totals(dir: &Path, entries: &[Entry], counts: &Counts) -> Totals {
    let mut budget = UNTRACKED_BUDGET;
    let mut totals = Totals {
        files: entries.len() as u64,
        ..Totals::default()
    };
    for entry in entries {
        let (added, removed) = lines(dir, entry, counts, &mut budget);
        totals.added = totals.added.saturating_add(added.unwrap_or(0));
        totals.removed = totals.removed.saturating_add(removed.unwrap_or(0));
    }
    totals
}

/// Joins the status entries with their line counts (counted here for untracked files, reading
/// at most `budget` bytes of them), sorted by path, and keeps what fits in a message.
fn collect(
    dir: &Path,
    entries: Vec<Entry>,
    (counts, staged): (&Counts, &HashMap<Vec<u8>, Staged>),
    mut budget: u64,
) -> Changes {
    let mut files: Vec<ChangedFile> = entries
        .into_iter()
        .map(|entry| {
            let (added, removed) = lines(dir, &entry, counts, &mut budget);
            let (path, status, from) = entry;
            ChangedFile {
                staged: staged.get(&path).copied(),
                path: lossy(&path),
                status,
                old_path: from.as_deref().map(lossy),
                added,
                removed,
                seen: None,
            }
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut changes = Changes {
        added: files.iter().filter_map(|f| f.added).sum(),
        removed: files.iter().filter_map(|f| f.removed).sum(),
        ..Changes::default()
    };
    let total = files.len();
    let mut budget = MESSAGE_BUDGET;
    for file in files {
        let size = serde_json::to_vec(&file).unwrap_or_default().len() + 1 + SEEN_ROOM;
        let Some(left) = budget.checked_sub(size) else {
            break;
        };
        budget = left;
        changes.files.push(file);
    }
    changes.truncated = total - changes.files.len();
    changes
}

/// Lines of an untracked file, as git would count them once added; `None` for anything but
/// a regular text file of at most [`UNTRACKED_LIMIT`] bytes, and once one does not fit in
/// what is left of `budget` (it is then spent, so no other file is read).
pub fn count_lines(path: &Path, budget: &mut u64) -> Option<u64> {
    let meta = path.symlink_metadata().ok()?;
    if !meta.is_file() || meta.len() > UNTRACKED_LIMIT {
        return None;
    }
    let Some(left) = budget.checked_sub(meta.len()) else {
        *budget = 0;
        return None;
    };
    *budget = left;
    let mut file = open_nonblocking(path).ok()?;
    // Its size as it was measured: a file still growing is counted next time.
    let bytes = read_limited(&mut file, meta.len()).ok()?;
    if bytes[..bytes.len().min(BINARY_PROBE)].contains(&0) {
        return None;
    }
    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
    let unterminated = bytes.last().is_some_and(|&b| b != b'\n');
    Some((lines + usize::from(unterminated)) as u64)
}

/// Opens `path` for reading without ever waiting: a FIFO swapped in for a measured file opens
/// at once and reads as empty or fails, where a blocking open would wait for a writer forever.
/// Nor through a link swapped in for it after its check (`O_NOFOLLOW`, TODO 24.7).
#[cfg(unix)]
fn open_nonblocking(path: &Path) -> io::Result<std::fs::File> {
    use nix::fcntl::OFlag;
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(OFlag::O_NONBLOCK.union(OFlag::O_NOFOLLOW).bits())
        .open(path)
}

#[cfg(windows)]
use crate::windows::open_nonblocking;

fn lossy(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    git::output(dir, args, &[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::tests::{commit, run, worktree};

    fn file(path: &str, status: FileStatus, counts: (Option<u64>, Option<u64>)) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            status,
            old_path: None,
            added: counts.0,
            removed: counts.1,
            staged: None,
            seen: None,
        }
    }

    #[test]
    fn status_entries_are_parsed() {
        let out = b"1 .M N... 100644 100644 100644 aa bb src/a b.rs\0\
1 A. N... 000000 100644 100644 00 bb new.rs\0\
1 AM N... 000000 100644 100644 00 bb new2.rs\0\
1 D. N... 100644 000000 000000 aa 00 gone.rs\0\
1 .D N... 100644 100644 000000 aa aa gone2.rs\0\
2 R. N... 100644 100644 100644 aa aa R100 to name\0from name\0\
u UU N... 100644 100644 100644 100644 a1 a2 a3 both.rs\0\
? odd\xffname\0\
! ignored\0\
# branch.oid x\0\
1 short\0";
        let entries = parse_status(out);
        let got: Vec<_> = entries
            .iter()
            .map(|(p, s, f)| (lossy(p), *s, f.as_deref().map(lossy)))
            .collect();
        use FileStatus::*;
        assert_eq!(
            got,
            vec![
                ("src/a b.rs".into(), Modified, None),
                ("new.rs".into(), Added, None),
                ("new2.rs".into(), Added, None),
                ("gone.rs".into(), Deleted, None),
                ("gone2.rs".into(), Deleted, None),
                ("to name".into(), Renamed, Some("from name".into())),
                ("both.rs".into(), Modified, None),
                ("odd\u{fffd}name".into(), Untracked, None),
            ]
        );
        // The raw bytes of a non-UTF-8 name are kept for reading the file.
        assert_eq!(entries[7].0, b"odd\xffname");
        // What of each is staged: the index side (X) and the worktree side (Y).
        let staged = parse_staged(out);
        let state = |path: &[u8]| staged.get(path).copied();
        use Staged::{No, Partly, Yes};
        assert_eq!(state(b"src/a b.rs"), Some(No));
        assert_eq!(state(b"new.rs"), Some(Yes));
        assert_eq!(state(b"new2.rs"), Some(Partly));
        assert_eq!(state(b"gone.rs"), Some(Yes));
        assert_eq!(state(b"gone2.rs"), Some(No));
        assert_eq!(state(b"to name"), Some(Yes));
        assert_eq!(state(b"from name"), None);
        assert_eq!(state(b"both.rs"), Some(No));
        assert_eq!(state(b"odd\xffname"), Some(No));
        assert_eq!(state(b"ignored"), None);
        assert_eq!(staged.len(), 8);
    }

    #[test]
    fn numstat_is_parsed() {
        let out = b"3\t1\tsrc/a b.rs\0-\t-\timg.png\0\
2\t0\t\0from name\0to name\0\
0\t0\tmode-only\0garbage\0";
        let counts = parse_numstat(out);
        assert_eq!(counts.len(), 4);
        assert_eq!(counts[&b"src/a b.rs".to_vec()], (Some(3), Some(1)));
        assert_eq!(counts[&b"img.png".to_vec()], (None, None));
        assert_eq!(counts[&b"to name".to_vec()], (Some(2), Some(0)));
        assert_eq!(counts[&b"mode-only".to_vec()], (Some(0), Some(0)));
        assert_eq!(number(b"\xff"), None);
    }

    #[cfg(unix)]
    #[test]
    fn untracked_lines_are_counted_like_git() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            std::fs::write(dir.path().join(name), bytes).unwrap();
            dir.path().join(name)
        };
        let count = |path: &Path| {
            let mut all = u64::MAX;
            count_lines(path, &mut all)
        };
        assert_eq!(count(&write("empty", b"")), Some(0));
        assert_eq!(count(&write("two", b"a\nb\n")), Some(2));
        assert_eq!(count(&write("open", b"a\nb")), Some(2));
        assert_eq!(count(&write("bin", b"a\0b\n")), None);
        let mut late_nul = vec![b'a'; BINARY_PROBE];
        late_nul.push(0);
        assert_eq!(count(&write("late", &late_nul)), Some(1));
        let mut at_limit = vec![b'a'; UNTRACKED_LIMIT as usize - 1];
        at_limit.push(b'\n');
        assert_eq!(count(&write("limit", &at_limit)), Some(1));
        at_limit.push(b'\n');
        let big = write("big", &at_limit);
        // Nothing that is not read spends the budget.
        let mut budget = 10;
        assert_eq!(count_lines(&big, &mut budget), None);
        std::os::unix::fs::symlink("two", dir.path().join("link")).unwrap();
        assert_eq!(count_lines(&dir.path().join("link"), &mut budget), None);
        assert_eq!(count_lines(&dir.path().join("missing"), &mut budget), None);
        assert_eq!(count_lines(dir.path(), &mut budget), None);
        // A FIFO is never opened: no writer would ever come.
        let fifo = dir.path().join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
        assert_eq!(count_lines(&fifo, &mut budget), None);
        assert_eq!(budget, 10);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_swapped_in_after_the_check_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret"), "secret").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("secret", &link).unwrap();
        let code = open_nonblocking(&link).unwrap_err().raw_os_error();
        assert_eq!(code, Some(nix::libc::ELOOP));
        assert!(open_nonblocking(&dir.path().join("secret")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_opens_and_reads_without_waiting_for_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
        // On another thread, so a blocking open fails the test instead of hanging it.
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let file = open_nonblocking(&fifo);
            tx.send(file.and_then(|mut f| read_limited(&mut f, 10)).ok())
        });
        let read = rx.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(read, Ok(Some(vec![])), "no writer: empty, at once");
        assert!(reader.join().unwrap().is_ok());
    }

    #[test]
    fn the_budget_stops_reading_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [("a", "1\n2\n"), ("b", "1\n"), ("c", "123\n"), ("d", "1")] {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let untracked = |name: &str| (name.as_bytes().to_vec(), FileStatus::Untracked, None);
        let entries = ["a", "b", "c", "d"].map(untracked).to_vec();
        // `a` and `b` fit exactly; `c` does not, and `d` is not read after it though it would fit.
        let changes = collect(dir.path(), entries, (&HashMap::new(), &HashMap::new()), 7);
        let counted: Vec<_> = changes.files.iter().map(|f| f.added).collect();
        assert_eq!(counted, [Some(2), Some(1), None, None]);
        assert_eq!(changes.added, 3);
    }

    #[test]
    fn files_are_sorted_counted_and_cut_to_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("u.txt"), "x\ny\n").unwrap();
        std::fs::write(dir.path().join("ub"), "\0").unwrap();
        let entries = vec![
            (b"b".to_vec(), FileStatus::Modified, None),
            (b"u.txt".to_vec(), FileStatus::Untracked, None),
            (b"a".to_vec(), FileStatus::Renamed, Some(b"old".to_vec())),
            (b"bin".to_vec(), FileStatus::Modified, None),
            (b"ub".to_vec(), FileStatus::Untracked, None),
        ];
        let counts = HashMap::from([
            (b"b".to_vec(), (Some(3), Some(1))),
            (b"a".to_vec(), (Some(1), Some(2))),
        ]);
        let changes = collect(
            dir.path(),
            entries,
            (&counts, &HashMap::new()),
            UNTRACKED_BUDGET,
        );
        let mut renamed = file("a", FileStatus::Renamed, (Some(1), Some(2)));
        renamed.old_path = Some("old".into());
        assert_eq!(
            changes,
            Changes {
                files: vec![
                    renamed,
                    file("b", FileStatus::Modified, (Some(3), Some(1))),
                    file("bin", FileStatus::Modified, (None, None)),
                    file("u.txt", FileStatus::Untracked, (Some(2), Some(0))),
                    file("ub", FileStatus::Untracked, (None, None)),
                ],
                added: 6,
                removed: 3,
                truncated: 0,
                changed: 0,
            }
        );

        // Each entry is ~1 KiB of JSON: the budget holds some, the totals count all.
        let name = "x".repeat(1000);
        let many: Vec<_> = (0..4000)
            .map(|i| {
                (
                    format!("{i:05}{name}").into_bytes(),
                    FileStatus::Modified,
                    None,
                )
            })
            .collect();
        let counts = many
            .iter()
            .map(|(p, _, _)| (p.clone(), (Some(1), Some(0))))
            .collect();
        let changes = collect(
            dir.path(),
            many,
            (&counts, &HashMap::new()),
            UNTRACKED_BUDGET,
        );
        let shown = changes.files.len();
        assert_eq!(changes.added, 4000);
        assert_eq!(shown + changes.truncated, 4000);
        let json: usize = changes
            .files
            .iter()
            .map(|f| serde_json::to_vec(f).unwrap().len() + 1 + SEEN_ROOM)
            .sum();
        assert!(json <= MESSAGE_BUDGET, "{json}");
        let next = serde_json::to_vec(&file(
            &format!("{shown:05}{name}"),
            FileStatus::Modified,
            (Some(1), Some(0)),
        ))
        .unwrap()
        .len()
            + 1
            + SEEN_ROOM;
        assert!(json + next > MESSAGE_BUDGET, "the next file did not fit");
    }

    #[test]
    fn a_folder_outside_git_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = list(dir.path(), None).unwrap_err().to_string();
        assert!(err.starts_with("git status --porcelain=v2"), "{err}");
    }

    #[test]
    fn raw_diff_entries_are_parsed() {
        let out = b":100644 100644 aa bb M\0src/a b.rs\0\
:000000 100644 00 bb A\0new.rs\0\
:100644 000000 aa 00 D\0gone.rs\0\
:100644 100644 aa bb R087\0from name\0to name\0\
:100644 120000 aa bb T\0link\0\
:000000 000000 00 00 U\0both.rs\0\
garbage\0\
:100644 100644 aa bb M\0odd\xffname\0";
        let entries = parse_raw(out);
        let got: Vec<_> = entries
            .iter()
            .map(|(p, s, f)| (lossy(p), *s, f.as_deref().map(lossy)))
            .collect();
        use FileStatus::*;
        assert_eq!(
            got,
            vec![
                ("src/a b.rs".into(), Modified, None),
                ("new.rs".into(), Added, None),
                ("gone.rs".into(), Deleted, None),
                ("to name".into(), Renamed, Some("from name".into())),
                ("link".into(), Modified, None),
                ("both.rs".into(), Modified, None),
                ("odd\u{fffd}name".into(), Modified, None),
            ]
        );
        assert_eq!(entries[6].0, b"odd\xffname");
    }

    fn rev(dir: &Path) -> String {
        lossy(git(dir, &["rev-parse", "HEAD"]).unwrap().trim_ascii())
    }

    fn project(worktrees: Vec<hive_protocol::Worktree>) -> Project {
        Project {
            id: String::new(),
            name: String::new(),
            path: String::new(),
            worktrees,
            error: None,
            group: false,
            folder: false,
            parent: None,
        }
    }

    fn changes(
        path: &str,
        base: DiffBase,
        base_error: Option<&str>,
        files: Vec<ChangedFile>,
    ) -> Control {
        Control::Changes {
            path: path.into(),
            base,
            branch: Some("main".into()),
            base_error: base_error.map(Into::into),
            added: files.iter().filter_map(|f| f.added).sum(),
            removed: files.iter().filter_map(|f| f.removed).sum(),
            files,
            error: None,
            turns: vec![],
        }
    }

    /// Snapshots kept in `dir`, never used by the test.
    fn no_snapshots(dir: &Path) -> Snapshots {
        Snapshots::new(dir.join("snapshots"))
    }

    #[test]
    fn a_branch_base_keeps_the_branch_commits_in_view() {
        let tmp = tempfile::tempdir().unwrap();
        let top = crate::paths::canonical(tmp.path()).unwrap();
        let (root, w, lone) = (top.join("r"), top.join("w"), top.join("lone"));
        let snapshots = &no_snapshots(&top);
        let titles = &HashMap::new();
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        commit(&root, "a");
        commit(&root, "r");
        let fork = rev(&root);
        run(
            &root,
            &["worktree", "add", "-q", "-b", "w", w.to_str().unwrap()],
        );
        commit(&w, "b");
        run(&w, &["mv", "r", "s"]);
        run(&w, &["commit", "-q", "-m", "s"]);
        std::fs::write(w.join("a"), "a\nmore\n").unwrap();
        std::fs::write(w.join("u"), "u\n").unwrap();
        // The main branch moves on: that is not the branch's work.
        commit(&root, "d");
        // A worktree whose branch shares no commit with main.
        run(
            &root,
            &["worktree", "add", "-q", "--detach", lone.to_str().unwrap()],
        );
        run(&lone, &["switch", "-q", "--orphan", "o"]);
        commit(&lone, "x");
        let mut projects = [project(vec![
            worktree(&root, Some("main"), true),
            worktree(&w, Some("w"), false),
            worktree(&lone, Some("o"), false),
        ])];
        let path = w.display().to_string();
        let seen = Seen::new(top.join("hive/seen.json"));

        assert_eq!(
            against(&projects, snapshots, &path, &DiffBase::Branch).unwrap(),
            Against {
                dir: w.clone(),
                commit: Some(fork.clone()),
                since: None,
                base: DiffBase::Branch,
                branch: Some("main".into()),
                error: None,
            }
        );
        let mut renamed = file("s", FileStatus::Renamed, (Some(0), Some(0)));
        renamed.old_path = Some("r".into());
        let mut modified = file("a", FileStatus::Modified, (Some(2), Some(1)));
        let mut untracked = file("u", FileStatus::Untracked, (Some(1), Some(0)));
        // Only what is left to commit can be staged: not the branch's commits (27.6).
        modified.staged = Some(Staged::No);
        untracked.staged = Some(Staged::No);
        let branch_files = vec![
            modified.clone(),
            file("b", FileStatus::Added, (Some(1), Some(0))),
            renamed,
            untracked.clone(),
        ];
        // The status counts only against HEAD: not the branch's commits.
        let on_branch = changes(&path, DiffBase::Branch, None, branch_files.clone());
        assert_eq!(
            answer(
                &projects,
                snapshots,
                path.clone(),
                &DiffBase::Branch,
                titles,
                &seen
            ),
            (on_branch.clone(), None)
        );
        // Against HEAD, only what is not committed.
        assert_eq!(
            answer(
                &projects,
                snapshots,
                path.clone(),
                &DiffBase::Head,
                titles,
                &seen
            ),
            (
                changes(
                    &path,
                    DiffBase::Head,
                    None,
                    vec![modified.clone(), untracked.clone()]
                ),
                Some(Totals {
                    files: 2,
                    added: 3,
                    removed: 1
                })
            )
        );
        // A commit picked: listed as the branch base is, from that commit.
        let picked = DiffBase::Commit { id: fork.clone() };
        assert_eq!(
            answer(&projects, snapshots, path.clone(), &picked, titles, &seen),
            (changes(&path, picked.clone(), None, branch_files), None)
        );
        // An id that is not a full one, or no commit of the repository: HEAD, and why.
        let short = DiffBase::Commit {
            id: fork[..7].into(),
        };
        let gone = DiffBase::Commit { id: "0".repeat(40) };
        let tree = git(&w, &["rev-parse", "HEAD^{tree}"]).unwrap();
        let tree = DiffBase::Commit {
            id: lossy(tree.trim_ascii()),
        };
        let option = DiffBase::Commit {
            id: format!("--{}", "0".repeat(38)),
        };
        for (base, why) in [
            (short, "Not a commit id"),
            (gone, GONE),
            (tree, GONE),
            (option, "Not a commit id"),
        ] {
            let used = against(&projects, snapshots, &path, &base).unwrap();
            assert_eq!(
                (used.commit, used.base, used.error.as_deref()),
                (None, DiffBase::Head, Some(why))
            );
        }
        // The commit each base compares with (27.5's blame of the base side).
        let commit_of = |base: &DiffBase| base_commit(&projects, &path, base);
        assert_eq!(commit_of(&DiffBase::Branch), Some(fork.clone()));
        assert_eq!(commit_of(&picked), Some(fork.clone()));
        let turn = DiffBase::Turn {
            session: "s".into(),
        };
        assert_eq!((commit_of(&DiffBase::Head), commit_of(&turn)), (None, None));
        assert_eq!(base_commit(&projects, "/nope", &DiffBase::Branch), None);
        // A 64-digit id in a SHA-1 repository is not one of its commits either.
        let long = DiffBase::Commit { id: "a".repeat(64) };
        let used = against(&projects, snapshots, &path, &long).unwrap();
        assert_eq!(used.error.as_deref(), Some(GONE));

        // Marked seen (27.1): the answer carries the marks of the base it was listed against.
        let a = ["a".to_owned()];
        let mut seen_a = modified.clone();
        seen_a.seen = Some(hive_protocol::SeenMark::Seen);
        assert_eq!(
            mark(
                &projects,
                (snapshots, titles),
                path.clone(),
                &DiffBase::Head,
                &seen,
                (&a, true)
            ),
            changes(&path, DiffBase::Head, None, vec![seen_a, untracked.clone()])
        );
        assert_eq!(
            answer(
                &projects,
                snapshots,
                path.clone(),
                &DiffBase::Branch,
                titles,
                &seen
            )
            .0,
            on_branch
        );
        // Marks that cannot be saved say why, and nothing is marked.
        let capped = Seen::with_limit(top.join("hive/capped.json"), 2);
        assert_eq!(
            mark(
                &projects,
                (snapshots, titles),
                path.clone(),
                &DiffBase::Head,
                &capped,
                (&a, true)
            ),
            Control::Error {
                message: "Cannot mark the files as seen: capped.json would be over 2 bytes".into()
            }
        );
        mark(
            &projects,
            (snapshots, titles),
            path.clone(),
            &DiffBase::Head,
            &seen,
            (&a, false),
        );
        assert_eq!(
            answer(
                &projects,
                snapshots,
                path.clone(),
                &DiffBase::Head,
                titles,
                &seen
            )
            .0,
            changes(&path, DiffBase::Head, None, vec![modified, untracked])
        );

        // A detached HEAD still has a merge-base with main.
        run(&w, &["switch", "-q", "--detach"]);
        let detached = against(&projects, snapshots, &path, &DiffBase::Branch).unwrap();
        assert_eq!(detached.commit, Some(fork));

        // No commit in common: HEAD, and why.
        let lone = lone.display().to_string();
        let why = "No commit in common with main";
        assert_eq!(
            answer(
                &projects,
                snapshots,
                lone.clone(),
                &DiffBase::Branch,
                titles,
                &seen
            )
            .0,
            changes(&lone, DiffBase::Head, Some(why), vec![])
        );

        // The main worktree has no branch to compare with.
        let main = root.display().to_string();
        let against_main = against(&projects, snapshots, &main, &DiffBase::Branch).unwrap();
        let expected = (None, None, Some(NO_BRANCH.to_owned()));
        let got = (against_main.commit, against_main.branch, against_main.error);
        assert_eq!(got, expected);

        // A branch git cannot read: HEAD, and git's error.
        projects[0].worktrees[0].branch = Some("gone".into());
        let gone = against(&projects, snapshots, &path, &DiffBase::Branch).unwrap();
        assert_eq!(gone.commit, None);
        let error = gone.error.unwrap_or_default();
        assert!(error.starts_with("git merge-base"), "{error}");

        // A worktree git no longer reads: why, with no totals and nothing marked.
        std::fs::remove_file(w.join(".git")).unwrap();
        let (reply, totals) = answer(
            &projects,
            snapshots,
            path.clone(),
            &DiffBase::Head,
            titles,
            &seen,
        );
        assert_eq!(totals, None);
        let why = list(&w, None).unwrap_err().to_string();
        assert!(why.starts_with("git status --porcelain=v2"), "{why}");
        assert_eq!(
            reply,
            Control::Changes {
                path: path.clone(),
                base: DiffBase::Head,
                branch: Some("gone".into()),
                base_error: None,
                files: vec![],
                added: 0,
                removed: 0,
                error: Some(why),
                turns: vec![],
            }
        );

        // Only a followed worktree.
        assert_eq!(
            answer(
                &projects,
                snapshots,
                "/nope".into(),
                &DiffBase::Branch,
                titles,
                &seen
            ),
            (
                Control::Changes {
                    path: "/nope".into(),
                    base: DiffBase::Head,
                    branch: None,
                    base_error: None,
                    files: vec![],
                    added: 0,
                    removed: 0,
                    error: Some("/nope is not a worktree of a followed project".into()),
                    turns: vec![],
                },
                None
            )
        );
    }

    #[test]
    fn a_turn_base_lists_what_changed_since_the_agents_last_turn() {
        use serde_json::json;
        let tmp = tempfile::tempdir().unwrap();
        let top = crate::paths::canonical(tmp.path()).unwrap();
        let root = top.join("r");
        let snapshots = no_snapshots(&top);
        let seen = Seen::new(top.join("hive/seen.json"));
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        commit(&root, "a");
        commit(&root, "r");
        std::fs::write(root.join("old-untracked"), "o\n").unwrap();
        std::fs::write(root.join("gone"), "g\n").unwrap();
        // Before the turn: what the user or another agent did is not the turn's.
        std::fs::write(root.join("a"), "before\n").unwrap();
        let projects = [project(vec![worktree(&root, None, true)])];
        let path = root.display().to_string();
        let turn = snapshots.take(&root, &path, "s1", 7).unwrap();
        snapshots.started("session-two-long");
        snapshots.edited("session-two-long");
        snapshots.take(&root, &path, "session-two-long", 9).unwrap();
        // The turn: an edit, a delete, a rename, an untracked file changed and a new one.
        std::fs::write(root.join("a"), "before\nafter\n").unwrap();
        std::fs::remove_file(root.join("gone")).unwrap();
        std::fs::rename(root.join("r"), root.join("s")).unwrap();
        std::fs::write(root.join("old-untracked"), "o\nmore\n").unwrap();
        std::fs::write(root.join("new"), "n\n").unwrap();
        let base = DiffBase::Turn {
            session: "s1".into(),
        };
        let titles = HashMap::from([("s1".to_owned(), "claude-3".to_owned())]);
        let (listed, totals) = answer(&projects, &snapshots, path.clone(), &base, &titles, &seen);
        let mut renamed = file("s", FileStatus::Renamed, (Some(0), Some(0)));
        renamed.old_path = Some("r".into());
        // What of each is staged (27.6), from `git status`: none of it; `gone` was untracked,
        // so nothing of it is left to commit.
        renamed.staged = Some(Staged::No);
        let unstaged = |mut f: ChangedFile| {
            f.staged = Some(Staged::No);
            f
        };
        let files = vec![
            unstaged(file("a", FileStatus::Modified, (Some(1), Some(0)))),
            file("gone", FileStatus::Deleted, (Some(0), Some(1))),
            unstaged(file("new", FileStatus::Added, (Some(1), Some(0)))),
            unstaged(file(
                "old-untracked",
                FileStatus::Modified,
                (Some(1), Some(0)),
            )),
            renamed,
        ];
        let turns = vec![
            TurnBase {
                session: "session-two-long".into(),
                title: "session-".into(),
                started_ms: 9,
                late: true,
            },
            TurnBase {
                session: "s1".into(),
                title: "claude-3".into(),
                started_ms: 7,
                late: false,
            },
        ];
        let expected = Control::Changes {
            path: path.clone(),
            base: base.clone(),
            branch: None,
            base_error: None,
            added: 3,
            removed: 1,
            files,
            error: None,
            turns,
        };
        assert_eq!((listed, totals), (expected, None));
        // Marked seen against the turn (27.1): kept under that base, changed by a new edit.
        let a = ["a".to_owned()];
        let turned = (&snapshots, &titles);
        let marked = mark(&projects, turned, path.clone(), &base, &seen, (&a, true));
        // Each file's mark, as the app reads it (`null`: none).
        let marks = |reply: &Control| {
            let reply = serde_json::to_value(reply).unwrap();
            let files = reply["files"].as_array().cloned().unwrap_or_default();
            files.iter().map(|f| f["seen"].clone()).collect::<Vec<_>>()
        };
        let none = serde_json::Value::Null;
        assert_eq!(
            marks(&marked),
            [
                json!("seen"),
                none.clone(),
                none.clone(),
                none.clone(),
                none
            ]
        );
        let at_head = answer(
            &projects,
            &snapshots,
            path.clone(),
            &DiffBase::Head,
            &titles,
            &seen,
        );
        assert!(marks(&at_head.0).iter().all(serde_json::Value::is_null));
        std::fs::write(root.join("a"), "before\nafter\nmore\n").unwrap();
        let again = answer(&projects, &snapshots, path.clone(), &base, &titles, &seen);
        assert_eq!(marks(&again.0)[0], json!("changed"));
        std::fs::write(root.join("a"), "before\nafter\n").unwrap();
        // The same as git's diff of the turn's tree and a snapshot taken now.
        let now = snapshots.take(&root, &path, "s3", 10).unwrap();
        let since = snapshots.since(&root, &path, "s3").unwrap();
        let tree = |args: &[&str]| -> Vec<u8> {
            let mut command = crate::git::command(&root).unwrap();
            let objects = top.join("snapshots");
            let store = std::fs::read_dir(&objects)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            command.env(
                "GIT_OBJECT_DIRECTORY",
                store.path().join("store.git/objects"),
            );
            command.args(args);
            command.output().unwrap().stdout
        };
        let diff = tree(&[
            "diff-tree",
            "-r",
            "-z",
            "--find-renames",
            &turn.tree,
            &now.tree,
        ]);
        let mut entries = parse_raw(&diff);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let listed = parse_raw(&since.raw().unwrap());
        assert!(listed.is_empty(), "nothing changed since s3");
        let (raw, _) = snapshots.since(&root, &path, "s1").unwrap().list().unwrap();
        let mut since_s1 = parse_raw(&raw);
        since_s1.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(since_s1, entries);

        // No snapshot of that session: HEAD, and why.
        let none = DiffBase::Turn {
            session: "other".into(),
        };
        let used = against(&projects, &snapshots, &path, &none).unwrap();
        assert_eq!(
            (used.base, used.error.as_deref()),
            (
                DiffBase::Head,
                Some("No snapshot of this agent's last turn in this worktree")
            )
        );
        // A snapshot no longer whole (its objects pruned by hand): HEAD, and why.
        let store = std::fs::read_dir(top.join("snapshots"))
            .unwrap()
            .next()
            .unwrap();
        let objects = store.unwrap().path().join("store.git/objects");
        std::fs::remove_dir_all(&objects).unwrap();
        std::fs::create_dir_all(objects.join("info")).unwrap();
        let (listed, totals) = answer(&projects, &snapshots, path.clone(), &base, &titles, &seen);
        let listed = serde_json::to_value(&listed).unwrap();
        assert_eq!(listed["base"], "head");
        let error = listed["base_error"].as_str().unwrap_or_default();
        assert!(
            error.starts_with("The snapshot of this turn is no longer complete: "),
            "{error}"
        );
        assert!(totals.is_some());
    }

    #[test]
    fn an_answer_names_the_turns_it_read_though_snapshots_change_meanwhile() {
        let tmp = tempfile::tempdir().unwrap();
        let top = crate::paths::canonical(tmp.path()).unwrap();
        let root = top.join("r");
        let snapshots = no_snapshots(&top);
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        commit(&root, "a");
        let path = root.display().to_string();
        let followed = || (root.clone(), None);
        let turn = |session: &str| DiffBase::Turn {
            session: session.into(),
        };
        let sessions = |kept: &[Turn]| kept.iter().map(|t| t.session.clone()).collect::<Vec<_>>();
        // Read before a snapshot is taken: its turn is neither the base nor listed.
        let before = snapshots.turns(&root, &path);
        snapshots.take(&root, &path, "s1", 7).unwrap();
        let used = based(followed(), &snapshots, &before, &turn("s1"));
        assert_eq!(
            (used.base, used.error.as_deref(), sessions(&before)),
            (DiffBase::Head, Some(snapshot::NO_SNAPSHOT), vec![])
        );
        // Read before its `SessionEnd`: HEAD, why, and no longer listed; the other kept.
        snapshots.take(&root, &path, "s2", 9).unwrap();
        let mut kept = snapshots.turns(&root, &path);
        let mut used = based(followed(), &snapshots, &kept, &turn("s1"));
        assert_eq!(used.base, turn("s1"));
        snapshots.end("s1");
        let listed = listing(&mut used, &mut kept).unwrap();
        assert_eq!(
            (used.base, used.error.as_deref(), sessions(&kept)),
            (
                DiffBase::Head,
                Some(snapshot::NO_SNAPSHOT),
                vec!["s2".to_owned()]
            )
        );
        assert!(used.since.is_none());
        assert_eq!(listed.files, list(&root, None).unwrap().files);
    }

    #[test]
    fn the_commits_are_those_of_head_the_branchs_first() {
        let tmp = tempfile::tempdir().unwrap();
        let top = crate::paths::canonical(tmp.path()).unwrap();
        let (root, w) = (top.join("r"), top.join("w"));
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        let mut projects = [project(vec![
            worktree(&root, Some("main"), true),
            worktree(&w, Some("w"), false),
        ])];
        let main = root.display().to_string();
        // Before the first commit: none.
        assert_eq!(
            commits(&projects, main.clone()),
            Control::Commits {
                path: main.clone(),
                commits: vec![],
                truncated: false,
                error: None,
            }
        );
        commit(&root, "a");
        let long = "s".repeat(300);
        run(&root, &["commit", "-q", "--allow-empty", "-m", &long]);
        run(
            &root,
            &["worktree", "add", "-q", "-b", "w", w.to_str().unwrap()],
        );
        commit(&w, "b");
        let (listed, truncated) = log(&w, Some("main".into()), COMMITS).unwrap();
        assert!(!truncated);
        let got: Vec<_> = listed
            .iter()
            .map(|c| {
                (
                    c.subject.len(),
                    c.on_branch,
                    !c.author.is_empty(),
                    c.time_ms,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (1, true, true, 1_700_000_000_000),
                (200, false, true, 1_700_000_000_000),
                (1, false, true, 1_700_000_000_000)
            ]
        );
        assert_eq!(listed[0].id, rev(&w));
        assert_eq!(listed[0].short, &listed[0].id[..listed[0].short.len()]);
        // The main worktree has no branch: none is its own.
        let (listed, _) = log(&root, None, COMMITS).unwrap();
        assert!(listed.iter().all(|c| !c.on_branch) && listed.len() == 2);
        // Over the cap: cut, and said; the branch's first.
        let (listed, truncated) = log(&w, Some("main".into()), 1).unwrap();
        assert_eq!((listed.len(), truncated), (1, true));
        assert!(listed[0].on_branch);
        let (listed, truncated) = log(&w, Some("main".into()), 3).unwrap();
        assert_eq!((listed.len(), truncated), (3, false));
        // A main branch git cannot read: no commit is the branch's.
        projects[0].worktrees[0].branch = Some("gone".into());
        let (listed, _) = log(&w, Some("gone".into()), COMMITS).unwrap();
        assert!(listed.iter().all(|c| !c.on_branch));
        // Only a followed worktree.
        assert_eq!(
            commits(&projects, "/nope".into()),
            Control::Commits {
                path: "/nope".into(),
                commits: vec![],
                truncated: false,
                error: Some("/nope is not a worktree of a followed project".into()),
            }
        );
    }

    #[test]
    fn the_message_carries_the_changes_or_why_not() {
        let changes = Changes {
            files: vec![file("a", FileStatus::Added, (Some(1), Some(0)))],
            added: 1,
            removed: 0,
            truncated: 0,
            changed: 1,
        };
        let against = Against {
            dir: PathBuf::from("/w"),
            commit: Some("c".into()),
            since: None,
            base: DiffBase::Branch,
            branch: Some("main".into()),
            error: None,
        };
        let turns = vec![TurnBase {
            session: "s".into(),
            title: "t".into(),
            started_ms: 1,
            late: true,
        }];
        assert_eq!(
            message("/w".into(), Some(against), Ok(changes), turns.clone()),
            Control::Changes {
                path: "/w".into(),
                base: DiffBase::Branch,
                branch: Some("main".into()),
                base_error: None,
                files: vec![file("a", FileStatus::Added, (Some(1), Some(0)))],
                added: 1,
                removed: 0,
                error: None,
                turns,
            }
        );
        let cut = Changes {
            truncated: 7,
            added: 5,
            ..Changes::default()
        };
        let head = Against {
            error: Some("why".into()),
            ..Against::default()
        };
        assert_eq!(
            message("/w".into(), Some(head), Ok(cut), vec![]),
            Control::Changes {
                path: "/w".into(),
                base: DiffBase::Head,
                branch: None,
                base_error: Some("why".into()),
                files: vec![],
                added: 5,
                removed: 0,
                error: Some("too many changes: 7 files not shown".into()),
                turns: vec![],
            }
        );
        assert_eq!(
            message("/w".into(), None, Err(io::Error::other("nope")), vec![]),
            Control::Changes {
                path: "/w".into(),
                base: DiffBase::Head,
                branch: None,
                base_error: None,
                files: vec![],
                added: 0,
                removed: 0,
                error: Some("nope".into()),
                turns: vec![],
            }
        );
    }
}
