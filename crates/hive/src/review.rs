//! Review worktrees (27.4): a worktree on a new branch `review-<name>` from the tip of the
//! reviewed worktree's branch, where the user (or an agent of theirs, #52) tries and fixes things
//! without touching the original. "Bring changes back" brings its commits into the reviewed
//! branch, only on the user's click (#12); "Discard review" removes it.
//!
//! Which worktree reviews which is kept by the service in `reviews.json`, next to the spaces file
//! in Hive's folder (22.1), never inside `.git` (#56); a link goes once either worktree is gone.
//! Git always runs with separate arguments; every branch is named in full (`refs/heads/…`) and
//! every commit by its id, so no name is ever read as an option.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hive_protocol::{BRING_BACK_SHOWN, Project, ReviewCommit, ReviewPlan, Worktree};
use serde::{Deserialize, Serialize};

use crate::git::{self, Stdout};
use crate::projects::Projects;
use crate::worktree::{self, Head};
use crate::wrapper::write_atomic;

/// What a review's worktree and branch are named: `review-<name>`.
pub const PREFIX: &str = "review-";
/// Largest links file read or written.
const FILE_LIMIT: u64 = 1024 * 1024;
/// Most review links kept: one more is refused.
pub const LINKS_LIMIT: usize = 256;
/// Most bytes of the reviewed worktree's name in a review's name.
const NAME_LIMIT: usize = 64;
/// Names tried for a review (`review-x`, `review-x-2`, …) before giving up.
const TRIES: usize = 100;
/// Most bytes of a commit's subject in a plan.
const SUMMARY_LIMIT: usize = 200;
/// The longest a merge (with the repository's hooks) may take.
const MERGE_TIME: Duration = Duration::from_secs(120);
/// Most bytes read from one git command's stdout.
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// Most bytes of what git said kept in an error: its end, where git says why it stopped.
const SAID_LIMIT: usize = 64 * 1024;

/// A review worktree and the worktree it reviews, both of the repository `project`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Link {
    project: String,
    review: String,
    reviewed: String,
}

/// The review links, as saved in their file.
pub struct Reviews {
    file: PathBuf,
    links: Mutex<Vec<Link>>,
}

impl Reviews {
    /// The links saved in `file`; none when it is missing, and none (with a warning) when it
    /// cannot be read: the next save replaces it.
    pub fn load(file: PathBuf) -> Self {
        let read = std::fs::File::open(&file)
            .and_then(|mut f| git::read_limited(&mut f, FILE_LIMIT))
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other));
        let links = match read {
            Ok(links) => links,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                eprintln!("hive: warning: {} is ignored: {err}", file.display());
                Vec::new()
            }
        };
        Self {
            file,
            links: Mutex::new(links),
        }
    }

    fn links(&self) -> MutexGuard<'_, Vec<Link>> {
        self.links.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Saves `links`: the file's folder is made when missing, the file is private.
    fn save(&self, links: &[Link]) -> io::Result<()> {
        self.file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        let json = serde_json::to_vec_pretty(links)?;
        write_atomic(&self.file, &json, 0o600)
    }

    /// Whether one more link would be refused.
    pub fn full(&self) -> bool {
        self.links().len() >= LINKS_LIMIT
    }

    /// Links `review` to `reviewed` (worktrees of `project`); kept even when the file cannot be
    /// saved, which is the error.
    pub fn link(&self, project: &str, review: &str, reviewed: &str) -> io::Result<()> {
        let mut links = self.links();
        links.retain(|l| l.review != review);
        if links.len() >= LINKS_LIMIT {
            return Err(io::Error::other(format!(
                "Hive keeps at most {LINKS_LIMIT} reviews: discard some first"
            )));
        }
        links.push(Link {
            project: project.to_owned(),
            review: review.to_owned(),
            reviewed: reviewed.to_owned(),
        });
        self.save(&links)
    }

    /// Marks each worktree of `project` that reviews another of its worktrees with it.
    pub fn mark(&self, project: &mut Project) {
        let links = self.links();
        let paths: Vec<String> = project.worktrees.iter().map(|w| w.path.clone()).collect();
        for w in &mut project.worktrees {
            let link = links
                .iter()
                .find(|l| l.project == project.id && l.review == w.path);
            w.review_of = link
                .map(|l| l.reviewed.clone())
                .filter(|reviewed| paths.contains(reviewed));
        }
    }

    /// Drops the links of `project` (listed by git as `worktrees`) whose review or reviewed
    /// worktree is gone; saved only when one went (a failure is a warning).
    pub fn prune(&self, project: &str, worktrees: &[Worktree]) {
        let mut links = self.links();
        let there = |path: &str| worktrees.iter().any(|w| w.path == path);
        let before = links.len();
        links.retain(|l| l.project != project || (there(&l.review) && there(&l.reviewed)));
        if links.len() != before
            && let Err(err) = self.save(&links)
        {
            eprintln!("hive: warning: the reviews could not be saved: {err}");
        }
    }

    /// The worktree `from` moved to `to` (a rename): its links follow.
    pub fn rename(&self, from: &str, to: &str) {
        let mut links = self.links();
        let mut moved = false;
        for l in links.iter_mut() {
            for path in [&mut l.review, &mut l.reviewed] {
                if path == from {
                    *path = to.to_owned();
                    moved = true;
                }
            }
        }
        if moved && let Err(err) = self.save(&links) {
            eprintln!("hive: warning: the reviews could not be saved: {err}");
        }
    }
}

/// `review-<name>` for the worktree `name`: lowercase letters, digits, `_` and single `-`
/// (anything else becomes a `-`), at most [`NAME_LIMIT`] bytes of `name`. Always a valid
/// worktree name (it starts with a letter) and branch name (no `.`, no trailing `-`).
pub fn review_name(name: &str) -> String {
    let mut out = String::from(PREFIX);
    for c in name.chars().map(|c| c.to_ascii_lowercase()) {
        if out.len() >= PREFIX.len() + NAME_LIMIT {
            break;
        }
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// The first of `review-<name>`, `review-<name>-2`, … whose folder and branch are free in the
/// repository at `root`.
fn free_name(root: &Path, name: &str) -> io::Result<String> {
    let name = review_name(name);
    (1..=TRIES)
        .map(|i| match i {
            1 => name.clone(),
            i => format!("{name}-{i}"),
        })
        .find(|n| worktree::check_name(root, n).is_ok() && !worktree::has_branch(root, n))
        .ok_or_else(|| io::Error::other(format!("{name} and {TRIES} more names are taken")))
}

/// Creates the review of the worktree `path` of a followed project, linked to it. Answers the
/// project with its worktrees (the review marked) and what the CLI notes about the new one.
pub fn create(projects: &Projects, path: &str) -> io::Result<(Project, worktree::Created)> {
    let (owner, reviewed) = projects.holding(path)?;
    let Some(branch) = reviewed.branch else {
        return Err(io::Error::other(format!(
            "{} is detached: there is no branch to review",
            reviewed.name
        )));
    };
    if projects.reviews().full() {
        return Err(io::Error::other(format!(
            "Hive keeps at most {LINKS_LIMIT} reviews: discard some first"
        )));
    }
    let root = Path::new(&owner.id);
    let name = free_name(root, &reviewed.name)?;
    let start = format!("refs/heads/{branch}");
    let head = Head::New {
        branch: &name,
        start: Some(&start),
    };
    let (_, mut created) = projects.create_worktree_on(&owner.id, &name, head)?;
    let review = created.path.to_string_lossy();
    if let Err(err) = projects.reviews().link(&owner.id, &review, path) {
        created
            .notes
            .push(format!("the review link could not be saved: {err}"));
    }
    Ok((projects.followed(&owner.id)?, created))
}

/// The followed project holding the review `path`, the review and the worktree it reviews.
fn pair(projects: &Projects, path: &str) -> io::Result<(Project, Worktree, Worktree)> {
    let (owner, review) = projects.holding(path)?;
    let reviewed = review
        .review_of
        .as_deref()
        .and_then(|of| owner.worktrees.iter().find(|w| w.path == of).cloned())
        .ok_or_else(|| io::Error::other(format!("{} is not a review", review.name)))?;
    Ok((owner, review, reviewed))
}

/// The branch `w` is on.
fn branch_of(w: &Worktree) -> io::Result<String> {
    w.branch
        .clone()
        .ok_or_else(|| io::Error::other(format!("{} is detached: it is on no branch", w.name)))
}

/// `git <args>` in `dir` within [`git::TIME_LIMIT`]: these run while worktree changes wait.
fn read(dir: &Path, args: &[&str], ok: &[i32]) -> io::Result<Vec<u8>> {
    git::output_within(dir, args, ok, git::TIME_LIMIT)
}

/// [`read`], its output as text without the last newline.
fn text(dir: &Path, args: &[&str]) -> io::Result<String> {
    let out = read(dir, args, &[0])?;
    Ok(String::from_utf8_lossy(&out).trim_end().to_owned())
}

/// The commit id `branch` points at.
fn commit(dir: &Path, branch: &str) -> io::Result<String> {
    let name = format!("refs/heads/{branch}^{{commit}}");
    text(dir, &["rev-parse", "--verify", "--quiet", &name])
}

/// How many commits `range` (`<a>..<b>`, of commit ids) has.
fn count(dir: &Path, range: &str) -> io::Result<u64> {
    let out = text(dir, &["rev-list", "--count", range])?;
    out.parse().map_err(io::Error::other)
}

/// What "Bring changes back" of the review `path` would do.
pub fn plan(projects: &Projects, path: &str) -> io::Result<ReviewPlan> {
    let (_, review, reviewed) = pair(projects, path)?;
    let (branch, into) = (branch_of(&review)?, branch_of(&reviewed)?);
    let dir = PathBuf::from(&reviewed.path);
    let dir = dir.as_path();
    let (tip, base) = (commit(dir, &branch)?, commit(dir, &into)?);
    let range = format!("{base}..{tip}");
    let shown = format!("--max-count={BRING_BACK_SHOWN}");
    let log = read(
        dir,
        &["log", "-z", "--format=%H%x1f%s", &shown, &range],
        &[0],
    )?;
    Ok(ReviewPlan {
        reviewed: reviewed.path,
        into,
        fast_forward: count(dir, &format!("{tip}..{base}"))? == 0,
        base,
        branch,
        commits: parse_log(&log),
        total: count(dir, &range)?,
        tip,
    })
}

/// Parses `git log -z --format=%H%x1f%s`, each subject cut at [`SUMMARY_LIMIT`] bytes.
fn parse_log(out: &[u8]) -> Vec<ReviewCommit> {
    String::from_utf8_lossy(out)
        .split('\0')
        .filter_map(|entry| {
            let (id, summary) = entry.split_once('\x1f')?;
            let mut end = summary.len().min(SUMMARY_LIMIT);
            while !summary.is_char_boundary(end) {
                end -= 1;
            }
            Some(ReviewCommit {
                id: id.to_owned(),
                summary: summary[..end].to_owned(),
            })
        })
        .collect()
}

/// Refuses the worktree `w` when it has uncommitted changes to tracked files.
fn clean(w: &Worktree) -> io::Result<()> {
    // Untracked files are no changes git could lose: a merge that would write over one stops
    // (and says so), and a review's folder is one in the main worktree.
    let args = ["status", "--porcelain", "-z", "--untracked-files=no"];
    match read(Path::new(&w.path), &args, &[0])?.is_empty() {
        true => Ok(()),
        false => Err(io::Error::other(format!(
            "{} has uncommitted changes: commit or discard them first",
            w.name
        ))),
    }
}

/// What the user saw of a [`ReviewPlan`] when they clicked "Bring back" (#12).
#[derive(Debug, Clone, Copy)]
pub struct Shown<'a> {
    pub into: &'a str,
    pub base: &'a str,
    pub tip: &'a str,
}

/// Brings the review `path`'s commits into its reviewed branch, in the reviewed worktree, with
/// `env` (the space's git identity, 21.5): only while both worktrees are clean and the plan is
/// still the one `shown`. Answers the project, the commits brought and whether it was a
/// fast-forward.
pub fn bring_back(
    projects: &Projects,
    path: &str,
    shown: Shown,
    env: &[(&str, String)],
) -> io::Result<(Project, u64, bool)> {
    let (owner, review, reviewed) = pair(projects, path)?;
    clean(&review)?;
    clean(&reviewed)?;
    let plan = plan(projects, path)?;
    let moved = match (plan.into == shown.into, plan.base == shown.base) {
        (false, _) => Some(format!("{} is now on {}", reviewed.name, plan.into)),
        (true, false) => Some(format!("{} changed", plan.into)),
        _ => (plan.tip != shown.tip).then(|| format!("{} changed", plan.branch)),
    };
    if let Some(moved) = moved {
        return Err(io::Error::other(format!(
            "{moved} since the commits were shown: look again"
        )));
    }
    if plan.total == 0 {
        return Err(io::Error::other(format!(
            "nothing to bring back: every commit of {} is on {}",
            plan.branch, plan.into
        )));
    }
    let merged = merge(Path::new(&reviewed.path), &plan, env, MERGE_TIME);
    projects.forget();
    merged?;
    Ok((projects.followed(&owner.id)?, plan.total, plan.fast_forward))
}

/// `git merge` of `plan.tip` in `dir`: `--ff-only` for a fast-forward, else `--no-ff` with a
/// message. A merge that stops (a conflict, a hook) is aborted, and what git said is the error.
fn merge(dir: &Path, plan: &ReviewPlan, env: &[(&str, String)], time: Duration) -> io::Result<()> {
    let message = format!("Merge review {} into {}", plan.branch, plan.into);
    let args: Vec<&str> = match plan.fast_forward {
        true => vec!["merge", "--ff-only", &plan.tip],
        false => vec!["merge", "--no-ff", "--no-edit", "-m", &message, &plan.tip],
    };
    let args: Vec<&OsStr> = args.into_iter().map(OsStr::new).collect();
    let mut command = git::command(dir)?;
    command
        .args(&args)
        .envs(env.iter().cloned())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_MERGE_AUTOEDIT", "no");
    let keep = Stdout::Max(OUTPUT_LIMIT);
    let (said, killed) = match git::ran(command, "git", &args, &[], keep, Some(time)) {
        Ok(ran) if ran.code == Some(0) => return Ok(()),
        // Git says what stopped it on both: a conflict on stdout, a refusal on stderr.
        Ok(ran) => (
            String::from_utf8_lossy(&[ran.out, ran.err].concat()).into_owned(),
            false,
        ),
        Err(err) => (err.to_string(), true),
    };
    let merging = read(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[0, 1])?;
    // Stopped by Hive after its commit (a hook after it, past `time`): done all the same.
    let done = killed && count(dir, &format!("HEAD..{}", plan.tip))? == 0;
    if !merging.is_empty() {
        // A conflict, a merge stopped before its commit, or what one stopped after it left:
        // `--abort` goes back to `HEAD`.
        git::output(dir, &["merge", "--abort"], &[0])?;
    } else if killed && !done {
        // Stopped by Hive before git wrote `MERGE_HEAD`: its files go back to before, which
        // was clean. (A merge git refused itself changed nothing, and is left alone: what was
        // staged meanwhile stays.)
        git::output(dir, &["reset", "--merge"], &[0])?;
    }
    if done {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "git merge stopped; nothing was changed:\n{}",
        tail(said.trim(), SAID_LIMIT)
    )))
}

/// The last `limit` bytes of `text` at most, from a character's start.
fn tail(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Removes the review `path` as [`Projects::remove_worktree`] does (`force`, `proc`, `before`),
/// then deletes its `review-…` branch when every commit of it is on the reviewed branch.
pub fn discard(
    projects: &Projects,
    path: &str,
    force: bool,
    proc: crate::procs::Source,
    before: impl FnOnce(&str) -> io::Result<()>,
) -> io::Result<Project> {
    let (owner, review, reviewed) = pair(projects, path)?;
    let root = Path::new(&owner.id);
    let branch = review.branch.filter(|b| b.starts_with(PREFIX));
    let tip = branch.as_deref().and_then(|b| commit(root, b).ok());
    let project = projects.remove_worktree(path, force, proc, before)?;
    if let (Some(branch), Some(tip), Some(into)) = (branch, tip, reviewed.branch) {
        let merged = commit(root, &into).and_then(|into| count(root, &format!("{into}..{tip}")));
        if merged.is_ok_and(|n| n == 0) {
            // Only while it is still at `tip`: a commit made meanwhile keeps it. The worktree is
            // gone already, and a branch kept loses nothing: a failure is not one of the discard.
            let name = format!("refs/heads/{branch}");
            let _ = git::output(root, &["update-ref", "-d", &name, &tip], &[0]);
        }
    }
    Ok(project)
}

#[cfg(test)]
mod tests;
