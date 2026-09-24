//! What changed in a worktree ("Árvore de arquivos com diff do git"): every file that
//! differs from `HEAD`, staged or not, untracked included, as `git status` shows them, with
//! line counts from `git diff --numstat`. Git runs as the executable with separate arguments.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::Path;

use hive_protocol::{ChangedFile, Control, FileStatus};

use crate::worktree::{read_limited, run_git};

/// Most bytes read from `git status` or `git diff`.
const GIT_LIMIT: u64 = 16 * 1024 * 1024;
/// Untracked files larger than this are not counted (their lines show as unknown).
const UNTRACKED_LIMIT: u64 = 8 * 1024 * 1024;
/// Git's binary heuristic: a NUL byte in the first 8000 bytes.
const BINARY_PROBE: usize = 8000;
/// Most bytes of JSON for the files of one `changes` message, well under `MAX_PAYLOAD`.
const MESSAGE_BUDGET: usize = 3 * 1024 * 1024;

/// A worktree's changes; `files` may be cut short (`truncated`), the totals never are.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub files: Vec<ChangedFile>,
    pub added: u64,
    pub removed: u64,
    /// How many files were left out to keep the message within its budget.
    pub truncated: usize,
}

/// The changes of the worktree at `dir` against `HEAD` (the empty tree before the first
/// commit).
pub fn list(dir: &Path) -> io::Result<Changes> {
    let status = git(
        dir,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--find-renames",
        ],
    )?;
    let head = git_ok(dir, &["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1])?;
    let base = if head.is_empty() {
        git(dir, &["hash-object", "-t", "tree", "/dev/null"])?
    } else {
        head
    };
    let base = String::from_utf8_lossy(&base).trim().to_owned();
    let numstat = git(
        dir,
        &[
            "diff",
            "--numstat",
            "-z",
            "--find-renames",
            "--no-ext-diff",
            "--no-textconv",
            &base,
            "--",
        ],
    )?;
    Ok(collect(
        dir,
        parse_status(&status),
        &parse_numstat(&numstat),
    ))
}

/// The `changes` answer for `path`.
pub fn message(path: String, listed: io::Result<Changes>) -> Control {
    let (changes, error) = match listed {
        Ok(changes) => {
            let error = (changes.truncated > 0)
                .then(|| format!("too many changes: {} files not shown", changes.truncated));
            (changes, error)
        }
        Err(err) => (Changes::default(), Some(err.to_string())),
    };
    Control::Changes {
        path,
        files: changes.files,
        added: changes.added,
        removed: changes.removed,
        error,
    }
}

/// A status entry: path, status and the path a rename came from.
type Entry = (Vec<u8>, FileStatus, Option<Vec<u8>>);

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

/// Parses `git diff --numstat -z`: path (the new one for a rename) → lines added and
/// removed, `None` for a binary file.
pub fn parse_numstat(out: &[u8]) -> HashMap<Vec<u8>, (Option<u64>, Option<u64>)> {
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

/// Joins the status entries with their line counts (counted here for untracked files),
/// sorted by path, and keeps what fits in a message.
fn collect(
    dir: &Path,
    entries: Vec<Entry>,
    counts: &HashMap<Vec<u8>, (Option<u64>, Option<u64>)>,
) -> Changes {
    let mut files: Vec<ChangedFile> = entries
        .into_iter()
        .map(|(path, status, from)| {
            let (added, removed) = match status {
                FileStatus::Untracked => {
                    let lines = count_lines(&dir.join(os(&path)));
                    (lines, lines.map(|_| 0))
                }
                _ => counts.get(&path).copied().unwrap_or((None, None)),
            };
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
/// a regular text file of at most [`UNTRACKED_LIMIT`] bytes.
pub fn count_lines(path: &Path) -> Option<u64> {
    if !path.symlink_metadata().ok()?.is_file() {
        return None;
    }
    let bytes = read_limited(&mut File::open(path).ok()?, UNTRACKED_LIMIT).ok()?;
    if bytes[..bytes.len().min(BINARY_PROBE)].contains(&0) {
        return None;
    }
    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
    let unterminated = bytes.last().is_some_and(|&b| b != b'\n');
    Some((lines + usize::from(unterminated)) as u64)
}

fn os(path: &[u8]) -> &OsStr {
    std::os::unix::ffi::OsStrExt::from_bytes(path)
}

fn lossy(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    git_ok(dir, args, &[0])
}

fn git_ok(dir: &Path, args: &[&str], ok: &[i32]) -> io::Result<Vec<u8>> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run_git(dir, &args, &[], ok, GIT_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn untracked_lines_are_counted_like_git() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            std::fs::write(dir.path().join(name), bytes).unwrap();
            dir.path().join(name)
        };
        assert_eq!(count_lines(&write("empty", b"")), Some(0));
        assert_eq!(count_lines(&write("two", b"a\nb\n")), Some(2));
        assert_eq!(count_lines(&write("open", b"a\nb")), Some(2));
        assert_eq!(count_lines(&write("bin", b"a\0b\n")), None);
        let mut late_nul = vec![b'a'; BINARY_PROBE];
        late_nul.push(0);
        assert_eq!(count_lines(&write("late", &late_nul)), Some(1));
        let big = File::create(dir.path().join("big")).unwrap();
        big.set_len(UNTRACKED_LIMIT + 1).unwrap();
        assert_eq!(count_lines(&dir.path().join("big")), None);
        let at_limit = File::create(dir.path().join("limit")).unwrap();
        at_limit.set_len(UNTRACKED_LIMIT).unwrap();
        assert_eq!(
            count_lines(&dir.path().join("limit")),
            None,
            "NUL bytes: binary"
        );
        std::os::unix::fs::symlink("two", dir.path().join("link")).unwrap();
        assert_eq!(count_lines(&dir.path().join("link")), None);
        assert_eq!(count_lines(&dir.path().join("missing")), None);
        assert_eq!(count_lines(dir.path()), None);
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
        let changes = collect(dir.path(), entries, &counts);
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
        let changes = collect(dir.path(), many, &counts);
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
    fn the_message_carries_the_changes_or_why_not() {
        let changes = Changes {
            files: vec![file("a", FileStatus::Added, (Some(1), Some(0)))],
            added: 1,
            removed: 0,
            truncated: 0,
        };
        assert_eq!(
            message("/w".into(), Ok(changes)),
            Control::Changes {
                path: "/w".into(),
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
        assert_eq!(
            message("/w".into(), Ok(cut)),
            Control::Changes {
                path: "/w".into(),
                files: vec![],
                added: 5,
                removed: 0,
                error: Some("too many changes: 7 files not shown".into()),
            }
        );
        assert_eq!(
            message("/w".into(), Err(io::Error::other("nope"))),
            Control::Changes {
                path: "/w".into(),
                files: vec![],
                added: 0,
                removed: 0,
                error: Some("nope".into()),
            }
        );
    }
}
