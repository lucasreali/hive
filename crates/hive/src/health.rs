//! Worktree health for the sidebar: files changed, commits ahead of and behind the branch of
//! the main worktree, whether everything is merged there, and when the last commit was made.
//! It only reads: Hive never merges anything (#12).

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use hive_protocol::{Project, Worktree, WorktreeStatus};

use crate::{changes, git};

/// How often every followed worktree is checked again.
pub const INTERVAL: Duration = Duration::from_secs(30);
/// The longest one git command may take here (e.g. on a hung network drive).
const TIME_LIMIT: Duration = Duration::from_secs(10);

/// The status of `project`'s worktree `w`, `None` when git fails. The main worktree is
/// counted against nothing; the others against its branch, when it has one.
pub fn of(project: &Project, w: &Worktree) -> Option<WorktreeStatus> {
    let main = project.worktrees.iter().find(|w| w.main);
    let base = main.and_then(|m| m.branch.as_deref()).filter(|_| !w.main);
    read(Path::new(&w.path), base).ok()
}

/// Gives every worktree of `project` its status.
pub fn fill(project: &mut Project) {
    let statuses: Vec<_> = project.worktrees.iter().map(|w| of(project, w)).collect();
    for (w, status) in project.worktrees.iter_mut().zip(statuses) {
        w.status = status;
    }
}

fn read(dir: &Path, base: Option<&str>) -> io::Result<WorktreeStatus> {
    let changes = changes::parse_status(&git(dir, &changes::STATUS)?).len() as u64;
    let [seconds] = numbers(&git(dir, &["log", "-1", "--format=%ct", "HEAD", "--"])?)?;
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
        changes,
        ahead,
        behind,
        // Nothing ahead: every commit of HEAD is on the base branch, which is what
        // `git merge-base --is-ancestor HEAD <base>` would say, without another git run.
        merged: ahead == Some(0),
        last_commit_ms: seconds.saturating_mul(1000),
    })
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
    git::output_within(dir, args, &[0], TIME_LIMIT)
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn run(dir: &Path, args: &[&str]) {
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

    fn commit(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), name).unwrap();
        run(dir, &["add", name]);
        run(dir, &["commit", "-q", "-m", name]);
    }

    fn worktree(path: &Path, branch: Option<&str>, main: bool) -> Worktree {
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

    fn status(changes: u64, counts: Option<(u64, u64)>) -> Option<WorktreeStatus> {
        Some(WorktreeStatus {
            changes,
            ahead: counts.map(|c| c.0),
            behind: counts.map(|c| c.1),
            merged: counts.is_some_and(|c| c.0 == 0),
            last_commit_ms: 1_700_000_000_000,
        })
    }

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
                status(3, None),
                status(0, Some((2, 1))),
                status(2, Some((0, 1))),
                status(0, Some((0, 1))),
                None,
            ]
        );

        // With the main worktree detached there is nothing to count against; with no main
        // worktree listed, neither.
        run(&root, &["switch", "-q", "--detach"]);
        project.worktrees[0].branch = None;
        assert_eq!(of(&project, &project.worktrees[1]), status(0, None));
        project.worktrees.remove(0);
        assert_eq!(of(&project, &project.worktrees[0]), status(0, None));
        // A base branch that no longer exists is an error.
        let err = read(&wt("ahead"), Some("missing")).unwrap_err().to_string();
        assert!(err.starts_with("git rev-list"), "{err}");
    }

    #[test]
    fn a_repository_without_commits_has_no_status() {
        let tmp = tempfile::tempdir().unwrap();
        run(tmp.path(), &["init", "-q"]);
        let err = read(tmp.path(), None).unwrap_err().to_string();
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
        assert!(sent.changed("/w", &status(1, None)));
        assert!(!sent.changed("/w", &status(1, None)));
        assert!(sent.changed("/w", &status(2, None)));
        assert!(sent.changed("/x", &status(2, None)), "another path");
    }
}
