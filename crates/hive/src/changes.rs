//! What changed in a worktree ("Árvore de arquivos com diff do git"): every file that
//! differs from its base, staged or not, untracked included, with line counts from
//! `git diff --numstat`. The base is `HEAD` (what `git status` shows) or, for a worktree
//! other than the main one, the merge-base with the main worktree's branch (9.11), so the
//! agent's commits stay in view. Git runs as the executable with separate arguments.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use hive_protocol::{ChangedFile, Control, DiffBase, FileStatus, Project};

use crate::git::{self, read_limited};
use crate::projects;

/// Untracked files larger than this are not counted (their lines show as unknown).
const UNTRACKED_LIMIT: u64 = 8_388_608; // 8 MiB
/// Most bytes of untracked files read for their line counts in one listing (9.14); past it
/// the rest show unknown counts too. Default pending the human's review.
const UNTRACKED_BUDGET: u64 = 33_554_432; // 32 MiB
/// Git's binary heuristic: a NUL byte in the first 8000 bytes.
pub(crate) const BINARY_PROBE: usize = 8000;
/// Most bytes of JSON for the files of one `changes` message, well under `MAX_PAYLOAD`.
const MESSAGE_BUDGET: usize = 3_145_728; // 3 MiB

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

/// What a followed worktree's changes are compared with (9.11).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Against {
    pub dir: PathBuf,
    /// The merge-base with `branch`; `None`: `HEAD`.
    pub commit: Option<String>,
    /// The main worktree's branch the worktree can be compared with.
    pub branch: Option<String>,
    /// Why `branch` was asked but `HEAD` is used.
    pub error: Option<String>,
}

/// The base `asked` for the worktree `path` of `projects`: for `branch`, the merge-base of its
/// `HEAD` and the main worktree's branch (the one 6.6 counts against), else `HEAD` and why.
pub fn against(projects: &[Project], path: &str, asked: DiffBase) -> io::Result<Against> {
    let (dir, branch) = projects::followed(projects, path)?;
    let (commit, error) = match (asked, &branch) {
        (DiffBase::Head, _) => (None, None),
        (DiffBase::Branch, None) => (None, Some(NO_BRANCH.to_owned())),
        (DiffBase::Branch, Some(name)) => {
            // A full ref name: a branch can never be read as an option.
            let theirs = format!("refs/heads/{name}");
            match git::output(&dir, &["merge-base", "HEAD", &theirs], &[0, 1]) {
                Ok(out) if !out.is_empty() => (Some(lossy(out.trim_ascii())), None),
                Ok(_) => (None, Some(format!("No commit in common with {name}"))),
                Err(err) => (None, Some(err.to_string())),
            }
        }
    };
    Ok(Against {
        dir,
        commit,
        branch,
        error,
    })
}

/// Why the main worktree, or a worktree of a detached main worktree, has no branch base.
const NO_BRANCH: &str = "No branch to compare with: this is the main worktree, or it is detached";

/// The `changes` answer for the worktree `path` of `projects` against `asked`, and, when it
/// was listed against `HEAD`, its [`Totals`].
pub fn answer(projects: &[Project], path: String, asked: DiffBase) -> (Control, Option<Totals>) {
    match against(projects, &path, asked) {
        Ok(against) => {
            let listed = list(&against.dir, against.commit.as_deref());
            let totals = listed.as_ref().ok().map(|c| Totals {
                files: c.changed,
                added: c.added,
                removed: c.removed,
            });
            let totals = totals.filter(|_| against.commit.is_none());
            (message(path, Some(against), listed), totals)
        }
        Err(err) => (message(path, None, Err(err)), None),
    }
}

/// The changes of the worktree at `dir` against `commit`, else `HEAD` (the empty tree before
/// the first commit). Against `HEAD` they are what `git status` lists; against another commit,
/// what `git diff` finds from it to the worktree, and the untracked files.
pub fn list(dir: &Path, commit: Option<&str>) -> io::Result<Changes> {
    let status = parse_status(&git(dir, &STATUS)?);
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
        ..collect(dir, entries, &counts, UNTRACKED_BUDGET)
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
fn head(dir: &Path) -> io::Result<String> {
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

/// The `changes` answer for `path`, listed against `against` when the worktree was found.
pub fn message(path: String, against: Option<Against>, listed: io::Result<Changes>) -> Control {
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
        base: match against.commit {
            Some(_) => DiffBase::Branch,
            None => DiffBase::Head,
        },
        branch: against.branch,
        base_error: against.error,
        files: changes.files,
        added: changes.added,
        removed: changes.removed,
        error,
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
fn collect(dir: &Path, entries: Vec<Entry>, counts: &Counts, mut budget: u64) -> Changes {
    let mut files: Vec<ChangedFile> = entries
        .into_iter()
        .map(|entry| {
            let (added, removed) = lines(dir, &entry, counts, &mut budget);
            let (path, status, from) = entry;
            ChangedFile {
                path: lossy(&path),
                status,
                old_path: from.as_deref().map(lossy),
                added,
                removed,
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
        let size = serde_json::to_vec(&file).unwrap_or_default().len() + 1;
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
#[cfg(unix)]
fn open_nonblocking(path: &Path) -> io::Result<std::fs::File> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    let nonblocking = nix::fcntl::OFlag::O_NONBLOCK.bits();
    OpenOptions::new()
        .read(true)
        .custom_flags(nonblocking)
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
        let changes = collect(dir.path(), entries, &HashMap::new(), 7);
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
        let changes = collect(dir.path(), entries, &counts, UNTRACKED_BUDGET);
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
        let changes = collect(dir.path(), many, &counts, UNTRACKED_BUDGET);
        let shown = changes.files.len();
        assert_eq!(changes.added, 4000);
        assert_eq!(shown + changes.truncated, 4000);
        let json: usize = changes
            .files
            .iter()
            .map(|f| serde_json::to_vec(f).unwrap().len() + 1)
            .sum();
        assert!(json <= MESSAGE_BUDGET, "{json}");
        let next = serde_json::to_vec(&file(
            &format!("{shown:05}{name}"),
            FileStatus::Modified,
            (Some(1), Some(0)),
        ))
        .unwrap()
        .len()
            + 1;
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
        }
    }

    #[test]
    fn a_branch_base_keeps_the_branch_commits_in_view() {
        let tmp = tempfile::tempdir().unwrap();
        let top = crate::paths::canonical(tmp.path()).unwrap();
        let (root, w, lone) = (top.join("r"), top.join("w"), top.join("lone"));
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

        assert_eq!(
            against(&projects, &path, DiffBase::Branch).unwrap(),
            Against {
                dir: w.clone(),
                commit: Some(fork.clone()),
                branch: Some("main".into()),
                error: None,
            }
        );
        let mut renamed = file("s", FileStatus::Renamed, (Some(0), Some(0)));
        renamed.old_path = Some("r".into());
        let modified = file("a", FileStatus::Modified, (Some(2), Some(1)));
        let untracked = file("u", FileStatus::Untracked, (Some(1), Some(0)));
        // The status counts only against HEAD: not the branch's commits.
        assert_eq!(
            answer(&projects, path.clone(), DiffBase::Branch),
            (
                changes(
                    &path,
                    DiffBase::Branch,
                    None,
                    vec![
                        modified.clone(),
                        file("b", FileStatus::Added, (Some(1), Some(0))),
                        renamed,
                        untracked.clone(),
                    ]
                ),
                None
            )
        );
        // Against HEAD, only what is not committed.
        assert_eq!(
            answer(&projects, path.clone(), DiffBase::Head),
            (
                changes(&path, DiffBase::Head, None, vec![modified, untracked]),
                Some(Totals {
                    files: 2,
                    added: 3,
                    removed: 1
                })
            )
        );

        // A detached HEAD still has a merge-base with main.
        run(&w, &["switch", "-q", "--detach"]);
        let detached = against(&projects, &path, DiffBase::Branch).unwrap();
        assert_eq!(detached.commit, Some(fork));

        // No commit in common: HEAD, and why.
        let lone = lone.display().to_string();
        let why = "No commit in common with main";
        assert_eq!(
            answer(&projects, lone.clone(), DiffBase::Branch).0,
            changes(&lone, DiffBase::Head, Some(why), vec![])
        );

        // The main worktree has no branch to compare with.
        let main = root.display().to_string();
        let against_main = against(&projects, &main, DiffBase::Branch).unwrap();
        let expected = (None, None, Some(NO_BRANCH.to_owned()));
        let got = (against_main.commit, against_main.branch, against_main.error);
        assert_eq!(got, expected);

        // A branch git cannot read: HEAD, and git's error.
        projects[0].worktrees[0].branch = Some("gone".into());
        let gone = against(&projects, &path, DiffBase::Branch).unwrap();
        assert_eq!(gone.commit, None);
        let error = gone.error.unwrap_or_default();
        assert!(error.starts_with("git merge-base"), "{error}");

        // Only a followed worktree.
        assert_eq!(
            answer(&projects, "/nope".into(), DiffBase::Branch),
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
                },
                None
            )
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
            branch: Some("main".into()),
            error: None,
        };
        assert_eq!(
            message("/w".into(), Some(against), Ok(changes)),
            Control::Changes {
                path: "/w".into(),
                base: DiffBase::Branch,
                branch: Some("main".into()),
                base_error: None,
                files: vec![file("a", FileStatus::Added, (Some(1), Some(0)))],
                added: 1,
                removed: 0,
                error: None,
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
            message("/w".into(), Some(head), Ok(cut)),
            Control::Changes {
                path: "/w".into(),
                base: DiffBase::Head,
                branch: None,
                base_error: Some("why".into()),
                files: vec![],
                added: 5,
                removed: 0,
                error: Some("too many changes: 7 files not shown".into()),
            }
        );
        assert_eq!(
            message("/w".into(), None, Err(io::Error::other("nope"))),
            Control::Changes {
                path: "/w".into(),
                base: DiffBase::Head,
                branch: None,
                base_error: None,
                files: vec![],
                added: 0,
                removed: 0,
                error: Some("nope".into()),
            }
        );
    }
}
