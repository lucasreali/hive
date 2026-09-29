//! Worktree health for the sidebar: files and lines changed, commits ahead of and behind the branch of
//! the main worktree, whether everything is merged there, and when the last commit was made.
//! It only reads: Hive never merges anything (#12).

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use hive_protocol::{Project, Worktree, WorktreeStatus};

use crate::changes::{self, Totals};
use crate::git;

/// How often every followed worktree is checked again.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// The status of `project`'s worktree `w`, `None` when git fails. The main worktree is
/// counted against nothing; the others against its branch, when it has one. `totals`: its
/// changes against `HEAD` when already counted (`changes::answer`), so git is not run again.
pub fn of(project: &Project, w: &Worktree, totals: Option<Totals>) -> Option<WorktreeStatus> {
    read(Path::new(&w.path), branch(project, w), totals).ok()
}

/// The branch `project`'s worktree `w` is counted against: the main worktree's, for any other
/// worktree, when it has one. The Changes panel's `branch` base (9.11) compares with it too.
pub fn branch<'a>(project: &'a Project, w: &Worktree) -> Option<&'a str> {
    let main = project.worktrees.iter().find(|w| w.main);
    main.and_then(|m| m.branch.as_deref()).filter(|_| !w.main)
}

/// Gives every worktree of `project` its status.
pub fn fill(project: &mut Project) {
    let statuses: Vec<_> = project
        .worktrees
        .iter()
        .map(|w| of(project, w, None))
        .collect();
    for (w, status) in project.worktrees.iter_mut().zip(statuses) {
        w.status = status;
    }
}

fn read(dir: &Path, base: Option<&str>, totals: Option<Totals>) -> io::Result<WorktreeStatus> {
    let [seconds] = numbers(&git(dir, &["log", "-1", "--format=%ct", "HEAD", "--"])?)?;
    let totals = match totals {
        Some(totals) => totals,
        None => count(dir)?,
    };
    let (ahead, behind) = match base {
        Some(base) => {
            // A full ref name: a branch can never be read as an option.
            let range = format!("HEAD...refs/heads/{base}");
            let args = ["rev-list", "--left-right", "--count", &range, "--"];
            let [ahead, behind] = numbers(&git(dir, &args)?)?;
            (Some(ahead), Some(behind))
        }
        None => (None, None),
    };
    Ok(WorktreeStatus {
        changes: totals.files,
        added: totals.added,
        removed: totals.removed,
        ahead,
        behind,
        // Nothing ahead: every commit of HEAD is on the base branch, which is what
        // `git merge-base --is-ancestor HEAD <base>` would say, without another git run.
        merged: ahead == Some(0),
        last_commit_ms: seconds.saturating_mul(1000),
    })
}

/// The worktree's changes against `HEAD`, as `changes::list` totals them. A `git diff` that
/// fails or times out leaves the files counted, with no lines: it never hides the status.
fn count(dir: &Path) -> io::Result<Totals> {
    let entries = changes::parse_status(&git(dir, &changes::STATUS)?);
    let Ok(numstat) = git(dir, &changes::numstat("HEAD")) else {
        return Ok(Totals {
            files: entries.len() as u64,
            ..Totals::default()
        });
    };
    Ok(changes::totals(
        dir,
        &entries,
        &changes::parse_numstat(&numstat),
    ))
}

/// Exactly `N` numbers separated by white space.
fn numbers<const N: usize>(out: &[u8]) -> io::Result<[u64; N]> {
    let text = String::from_utf8_lossy(out);
    let bad = || io::Error::other(format!("unexpected git output: {:?}", text.trim()));
    let parsed: Vec<u64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| bad())?;
    parsed.try_into().map_err(|_| bad())
}

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    git::output_within(dir, args, &[0], git::TIME_LIMIT)
}

/// The last status sent to the app, by worktree path, so only changes are sent again.
#[derive(Debug, Default)]
pub struct Sent(HashMap<String, Option<WorktreeStatus>>);

impl Sent {
    /// Remembers `status` as sent for `path`; true when it is not the one sent last.
    pub fn changed(&mut self, path: &str, status: &Option<WorktreeStatus>) -> bool {
        self.0.insert(path.to_owned(), status.clone()).as_ref() != Some(status)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[cfg(unix)]
    use std::path::PathBuf;

    pub(crate) fn run(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_COMMITTER_DATE", "1700000000 +0000")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    pub(crate) fn commit(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), name).unwrap();
        run(dir, &["add", name]);
        run(dir, &["commit", "-q", "-m", name]);
    }

    #[cfg(unix)]
    pub(crate) fn worktree(path: &Path, branch: Option<&str>, main: bool) -> Worktree {
        let path = path.display().to_string();
        Worktree {
            id: path.clone(),
            name: String::new(),
            path,
            branch: branch.map(Into::into),
            main,
            claude: false,
            status: None,
        }
    }

    fn status(totals: (u64, u64, u64), counts: Option<(u64, u64)>) -> Option<WorktreeStatus> {
        let (changes, added, removed) = totals;
        Some(WorktreeStatus {
            changes,
            added,
            removed,
            ahead: counts.map(|c| c.0),
            behind: counts.map(|c| c.1),
            merged: counts.is_some_and(|c| c.0 == 0),
            last_commit_ms: 1_700_000_000_000,
        })
    }

    #[cfg(unix)]
    #[test]
    fn worktrees_are_counted_against_the_main_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let root: PathBuf = tmp.path().canonicalize().unwrap().join("r");
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        commit(&root, "a");
        let wt = |name: &str| root.join(".claude/worktrees").join(name);
        for name in ["ahead", "merged", "detached"] {
            let path = wt(name).display().to_string();
            run(&root, &["worktree", "add", "-q", "-b", name, &path]);
        }
        run(&wt("detached"), &["switch", "-q", "--detach"]);
        commit(&wt("ahead"), "b");
        commit(&wt("ahead"), "c");
        commit(&root, "d");
        std::fs::write(wt("merged").join("new"), "").unwrap();
        std::fs::write(wt("merged").join("a"), "changed").unwrap();

        let mut project = Project {
            id: String::new(),
            name: String::new(),
            path: String::new(),
            worktrees: vec![
                worktree(&root, Some("main"), true),
                worktree(&wt("ahead"), Some("ahead"), false),
                worktree(&wt("merged"), Some("merged"), false),
                worktree(&wt("detached"), None, false),
                worktree(&wt("gone"), Some("gone"), false),
            ],
            error: None,
        };
        fill(&mut project);
        let got: Vec<_> = project.worktrees.iter().map(|w| w.status.clone()).collect();
        assert_eq!(
            got,
            [
                // The linked worktrees inside it are untracked folders, as `changes` lists them.
                status((3, 0, 0), None),
                status((0, 0, 0), Some((2, 1))),
                // An empty new file, and "a" changed on its one line.
                status((2, 1, 1), Some((0, 1))),
                status((0, 0, 0), Some((0, 1))),
                None,
            ]
        );

        // With the main worktree detached there is nothing to count against; with no main
        // worktree listed, neither.
        run(&root, &["switch", "-q", "--detach"]);
        project.worktrees[0].branch = None;
        assert_eq!(
            of(&project, &project.worktrees[1], None),
            status((0, 0, 0), None)
        );
        // Changes already counted are not counted again.
        let counted = Totals {
            files: 9,
            added: 8,
            removed: 7,
        };
        assert_eq!(
            of(&project, &project.worktrees[1], Some(counted)),
            status((9, 8, 7), None)
        );
        project.worktrees.remove(0);
        assert_eq!(
            of(&project, &project.worktrees[0], None),
            status((0, 0, 0), None)
        );
        // A base branch that no longer exists is an error.
        let err = read(&wt("ahead"), Some("missing"), None)
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("git rev-list"), "{err}");
    }

    #[test]
    fn lines_are_counted_against_head_as_the_changes_total_them() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("m"), "1\n2\n3\n").unwrap();
        std::fs::write(dir.join("d"), "x\ny\n").unwrap();
        std::fs::write(dir.join("t"), b"\0a").unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-q", "-m", "c"]);
        let totals = |files, added, removed| Totals {
            files,
            added,
            removed,
        };
        // A clean worktree.
        assert_eq!(count(dir).unwrap(), Totals::default());

        // Modified (one line changed, one added), deleted, untracked (three lines, the last
        // unterminated), and binary files, tracked or not, which add no lines.
        std::fs::write(dir.join("m"), "1\nX\n3\n4\n").unwrap();
        std::fs::remove_file(dir.join("d")).unwrap();
        std::fs::write(dir.join("u"), "a\nb\nc").unwrap();
        std::fs::write(dir.join("t"), b"\0b").unwrap();
        std::fs::write(dir.join("bin"), b"\0\x01").unwrap();
        assert_eq!(count(dir).unwrap(), totals(5, 5, 3));
        // Staged or not, the same: against HEAD.
        run(dir, &["add", "m"]);
        assert_eq!(count(dir).unwrap(), totals(5, 5, 3));
        // What the Changes panel totals against HEAD.
        let listed = changes::list(dir, None).unwrap();
        let panel = totals(listed.changed, listed.added, listed.removed);
        assert_eq!(panel, totals(5, 5, 3));
        let status = read(dir, None, None).unwrap();
        assert_eq!((status.changes, status.added, status.removed), (5, 5, 3));
    }

    #[test]
    fn a_failed_diff_keeps_the_status_without_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run(dir, &["init", "-q", "-b", "main"]);
        commit(dir, "m");
        std::fs::write(dir.join("m"), "changed\n").unwrap();
        std::fs::write(dir.join("u"), "a\nb\n").unwrap();
        // `git status` compares object ids; `git diff` must read the old blob, now gone.
        let blob = crate::git::output(dir, &["rev-parse", "HEAD:m"], &[0]).unwrap();
        let blob = String::from_utf8(blob).unwrap();
        let (fan, rest) = blob.trim().split_at(2);
        std::fs::remove_file(dir.join(".git/objects").join(fan).join(rest)).unwrap();
        assert!(git(dir, &changes::numstat("HEAD")).is_err());

        // The files are still counted, and the rest of the status is kept.
        assert_eq!(read(dir, None, None).ok(), status((2, 0, 0), None));
    }

    #[test]
    fn a_repository_without_commits_has_no_status() {
        let tmp = tempfile::tempdir().unwrap();
        run(tmp.path(), &["init", "-q"]);
        let err = read(tmp.path(), None, None).unwrap_err().to_string();
        assert!(err.starts_with("git log"), "{err}");
    }

    #[test]
    fn git_output_must_hold_exactly_the_numbers_asked() {
        assert_eq!(numbers::<2>(b"2\t1\n").unwrap(), [2, 1]);
        assert_eq!(numbers::<1>(b"17\n").unwrap(), [17]);
        let err = numbers::<2>(b"2\n").unwrap_err();
        assert_eq!(err.to_string(), r#"unexpected git output: "2""#);
        assert!(numbers::<1>(b"x").is_err());
        assert!(numbers::<1>(b"-1").is_err());
    }

    #[test]
    fn only_a_different_status_counts_as_changed() {
        let mut sent = Sent::default();
        assert!(sent.changed("/w", &None), "new");
        assert!(!sent.changed("/w", &None));
        assert!(sent.changed("/w", &status((1, 1, 0), None)));
        assert!(!sent.changed("/w", &status((1, 1, 0), None)));
        assert!(sent.changed("/w", &status((2, 1, 0), None)));
        // The same files with other lines.
        assert!(sent.changed("/w", &status((2, 3, 0), None)));
        assert!(sent.changed("/w", &status((2, 3, 1), None)));
        assert!(sent.changed("/x", &status((2, 3, 1), None)), "another path");
    }
}
