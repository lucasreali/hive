//! Staging, committing and pushing from the Changes panel (27.6, D27.3): each one a click of
//! the user's, never the agent's (#52), and nothing merges (#12). Git runs as the executable
//! with separate arguments (`git::command`), as the worktree's space runs it: its git identity
//! (21.5), and for a push its GitHub account's token (9.30), whose credentials stay git's and
//! `gh`'s. The repository's hooks run as git runs them (never skipped), and a push is
//! never forced: no refspec starts with `+` and no force option is ever built
//! ([`push_args`]). Git's own message, hooks' output included, is the error shown.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hive_protocol::{Control, FileStatus, GitAction, SpaceEnv};

use crate::adapter::clip;
use crate::changes;
use crate::gh::Gh;
use crate::git::{self, Stdout};
use crate::projects::Projects;
use crate::pulls::multiline;
use crate::spaces;
use crate::worktree;

/// The longest a commit (its hooks included) or a staging may take.
const COMMIT_TIME: Duration = Duration::from_secs(300);
/// The longest a push (its `pre-push` hook and the network included) may take.
const PUSH_TIME: Duration = Duration::from_secs(300);
/// Most bytes of a commit message.
const MESSAGE_LIMIT: usize = 64 << 10;
/// Most files staged or unstaged at once, and the longest path.
const PATHS_LIMIT: usize = 10_000;
const PATH_LIMIT: usize = 4096;
/// What git and its hooks may print.
const OUTPUT: Stdout = Stdout::Shown(1 << 20);
/// Most bytes of git's message shown.
const ERROR_LIMIT: usize = 4 << 10;
/// Most characters of a commit's subject in its confirmation.
const SUBJECT_LIMIT: usize = 72;

/// Answers a git request of the Changes panel; it blocks while git (and its hooks) run.
pub fn answer(gh: &Gh, projects: &Projects, request: Control) -> Vec<Control> {
    match request {
        Control::StageFiles {
            worktree,
            paths,
            staged,
        } => {
            let action = match staged {
                true => GitAction::Stage,
                false => GitAction::Unstage,
            };
            let done = located(projects, &worktree).and_then(|(dir, _)| {
                // A space's identity is not needed to stage.
                stage(&dir, &paths, staged)
            });
            vec![reply(worktree, action, done)]
        }
        Control::Commit { worktree, message } => {
            let done = located(projects, &worktree)
                .and_then(|(dir, env)| commit(&dir, &message, &spaces::vars(&env)));
            vec![reply(worktree, GitAction::Commit, done)]
        }
        Control::GetPushTarget { worktree } => {
            let found = located(projects, &worktree).and_then(|(dir, _)| target(&dir));
            vec![target_message(worktree, found)]
        }
        Control::Push {
            worktree,
            branch,
            target,
        } => {
            let done = located(projects, &worktree)
                .and_then(|(dir, env)| push(&dir, &remote_env(gh, &env), (&branch, &target)));
            vec![reply(worktree, GitAction::Push, done)]
        }
        _ => Vec::new(),
    }
}

/// `GitDone` or `GitFailed` for `worktree`.
fn reply(worktree: String, action: GitAction, done: Result<String, String>) -> Control {
    match done {
        Ok(message) => Control::GitDone {
            worktree,
            action,
            message,
        },
        Err(message) => Control::GitFailed {
            worktree,
            action,
            message,
        },
    }
}

/// The folder of `path`, a worktree of a followed git project (a folder project's folder,
/// 23.1, is none), and its space's environment.
fn located(projects: &Projects, path: &str) -> Result<(PathBuf, SpaceEnv), String> {
    let (_, worktree) = projects.holding(path).map_err(|err| err.to_string())?;
    Ok((PathBuf::from(worktree.path), projects.space_env(path)))
}

/// What a git call that reaches a remote gets from the space `env`, as its terminals would:
/// its identity and `GH_CONFIG_DIR`, and its GitHub account's token when `gh` gives one (else
/// git asks the user's credentials as it always does, without a prompt).
pub fn remote_env(gh: &Gh, env: &SpaceEnv) -> Vec<(&'static str, String)> {
    let mut vars = spaces::vars(env);
    vars.extend(gh.vars(env).unwrap_or_default());
    vars
}

/// Git's message from `err`: what it (and its hooks) printed, without the command; else the
/// error itself (git could not run, or took too long). Cut at [`ERROR_LIMIT`].
fn why(verb: &str, err: &io::Error) -> String {
    let err = err.to_string();
    let prefix = format!("git {verb} failed: ");
    multiline(err.strip_prefix(&prefix).unwrap_or(&err), ERROR_LIMIT)
}

/// Runs `git <args>` in `dir` with `env`, never prompting, within `time`; `verb` names it in
/// errors (the arguments may hold a long message).
fn run(
    dir: &Path,
    (verb, args): (&str, &[&OsStr]),
    input: &[u8],
    env: &[(&str, String)],
    time: Duration,
) -> Result<Vec<u8>, String> {
    let mut command = git::command(dir).map_err(|err| err.to_string())?;
    worktree::quiet(command.args(args), env);
    let shown = [OsStr::new(verb)];
    git::limited(command, "git", &shown, input, &[0], OUTPUT, Some(time))
        .map_err(|err| why(verb, &err))
}

/// Checks the paths the app sent: 1 to [`PATHS_LIMIT`] of them, each relative to the
/// worktree, at most [`PATH_LIMIT`] bytes, never leaving it nor holding a NUL.
fn check_paths(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() || paths.len() > PATHS_LIMIT {
        return Err(format!(
            "Between 1 and {PATHS_LIMIT} files can be staged at once"
        ));
    }
    let bad = |path: &String| {
        path.is_empty()
            || path.len() > PATH_LIMIT
            || path.contains('\0')
            || Path::new(path).has_root()
            || path.split(['/', '\\']).any(|part| part == "..")
    };
    match paths.iter().find(|path| bad(path)) {
        Some(path) => Err(format!("{path:?} is not a file of this worktree")),
        None => Ok(()),
    }
}

/// Stages (`git add`) or unstages (`git reset`, which also works before the first commit)
/// `paths` of the worktree at `dir`; unstaging a staged rename unstages its old path
/// too. The paths go on stdin as literal pathspecs: a `*` in a name is that file only.
pub fn stage(dir: &Path, paths: &[String], staged: bool) -> Result<String, String> {
    check_paths(paths)?;
    let mut all: Vec<String> = paths.to_vec();
    if !staged {
        let out = git::output(dir, &changes::STATUS, &[0]).map_err(|err| err.to_string())?;
        let renamed = changes::parse_status(&out).into_iter().filter_map(|entry| {
            let (path, status, from) = entry;
            let path = String::from_utf8_lossy(&path).into_owned();
            let from = from.filter(|_| status == FileStatus::Renamed && paths.contains(&path))?;
            Some(String::from_utf8_lossy(&from).into_owned())
        });
        all.extend(renamed.collect::<Vec<_>>());
    }
    let input: Vec<u8> = all
        .iter()
        .flat_map(|p| [p.as_bytes(), b"\0"].concat())
        .collect();
    let from_stdin = ["--pathspec-from-file=-", "--pathspec-file-nul"];
    let verb: &[&str] = match staged {
        true => &["add"],
        false => &["reset", "--quiet"],
    };
    let args = [&["--literal-pathspecs"][..], verb, &from_stdin].concat();
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let verb = if staged { "add" } else { "reset" };
    run(dir, (verb, &args), &input, &[], COMMIT_TIME)?;
    let files = match paths {
        [one] => one.clone(),
        many => format!("{} files", many.len()),
    };
    Ok(match staged {
        true => format!("Staged {files}"),
        false => format!("Unstaged {files}"),
    })
}

/// Commits what is staged in the worktree at `dir` with `message` and `env` (the space's
/// identity); git's own message when it refuses (nothing staged, a hook failed). Answers the
/// new commit and its subject.
pub fn commit(dir: &Path, message: &str, env: &[(&str, String)]) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("A commit message is needed".to_owned());
    }
    if message.len() > MESSAGE_LIMIT {
        return Err("The commit message is over 64 KiB".to_owned());
    }
    if message.contains('\0') {
        return Err("The commit message holds a NUL character".to_owned());
    }
    // One argument: a message starting with `-` is still the message.
    let message = format!("--message={message}");
    let args = [OsStr::new("commit"), OsStr::new(&message)];
    run(dir, ("commit", &args), &[], env, COMMIT_TIME)?;
    let args = ["log", "-1", "--format=%h%x00%s"];
    let out = git::output(dir, &args, &[0]).unwrap_or_default();
    let out = String::from_utf8_lossy(&out);
    let (commit, subject) = out.trim_end().split_once('\0').unwrap_or_default();
    Ok(format!(
        "Committed {commit}: {}",
        clip(subject, SUBJECT_LIMIT)
    ))
}

/// Where a push of a worktree's branch goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The branch, without `refs/heads/`.
    pub branch: String,
    /// Its upstream's remote, or `origin`.
    pub remote: String,
    /// Its upstream's ref on the remote (`refs/heads/<name>`), or the same name.
    pub merge: String,
    /// It has no upstream yet: the push sets it.
    pub set_upstream: bool,
    /// Its commits the remote-tracking branch has not; `None` when that is not known.
    pub ahead: Option<u64>,
}

impl Target {
    /// e.g. `origin/main`.
    pub fn shown(&self) -> String {
        let name = self
            .merge
            .strip_prefix("refs/heads/")
            .unwrap_or(&self.merge);
        format!("{}/{name}", self.remote)
    }
}

/// Where the branch of the worktree at `dir` is pushed: its upstream, or `origin/<branch>`
/// when it has none. Refused on a detached `HEAD`, an upstream that is a local branch or no
/// branch of its remote, and with no upstream nor `origin`.
pub fn target(dir: &Path) -> Result<Target, String> {
    let read = |args: &[&str]| {
        let out = git::output(dir, args, &[0, 1]).map_err(|err| err.to_string())?;
        Ok::<_, String>(String::from_utf8_lossy(out.trim_ascii()).into_owned())
    };
    let head = read(&["symbolic-ref", "--quiet", "HEAD"])?;
    let Some(branch) = head.strip_prefix("refs/heads/").filter(|b| !b.is_empty()) else {
        return Err("This worktree is on no branch (a detached HEAD): nothing to push".to_owned());
    };
    let remote = read(&["config", "--get", &format!("branch.{branch}.remote")])?;
    let merge = read(&["config", "--get", &format!("branch.{branch}.merge")])?;
    if remote.is_empty() || merge.is_empty() {
        let remotes = read(&["remote"])?;
        if !remotes.lines().any(|r| r == "origin") {
            return Err(format!(
                "{branch} has no upstream and this repository has no remote origin"
            ));
        }
        return Ok(Target {
            branch: branch.to_owned(),
            remote: "origin".to_owned(),
            merge: head.clone(),
            set_upstream: true,
            ahead: None,
        });
    }
    if remote == "." {
        return Err(format!(
            "The upstream of {branch} is a local branch: push it in a terminal"
        ));
    }
    let Some(name) = merge.strip_prefix("refs/heads/").filter(|n| !n.is_empty()) else {
        return Err(format!(
            "The upstream of {branch} is not a branch of {remote}"
        ));
    };
    if remote.starts_with('-') {
        return Err(format!("{remote:?} is not a remote git accepts"));
    }
    let tracking = format!("refs/remotes/{remote}/{name}..HEAD");
    let ahead = read(&["rev-list", "--count", &tracking]).ok();
    Ok(Target {
        branch: branch.to_owned(),
        remote,
        merge,
        set_upstream: false,
        ahead: ahead.and_then(|n| n.parse().ok()),
    })
}

/// The `push_target` answer.
fn target_message(worktree: String, found: Result<Target, String>) -> Control {
    match found {
        Ok(target) => Control::PushTarget {
            worktree,
            target: Some(target.shown()),
            set_upstream: target.set_upstream,
            ahead: target.ahead,
            branch: Some(target.branch),
            error: None,
        },
        Err(error) => Control::PushTarget {
            worktree,
            branch: None,
            target: None,
            set_upstream: false,
            ahead: None,
            error: Some(error),
        },
    }
}

/// The arguments of the push to `target`: the branch's full ref to the upstream's, after
/// `--`, so no name is ever read as an option or a `+` (force). Never a force option.
pub fn push_args(target: &Target) -> Vec<String> {
    let mut args = vec!["push".to_owned()];
    if target.set_upstream {
        args.push("--set-upstream".to_owned());
    }
    let refspec = format!("refs/heads/{}:{}", target.branch, target.merge);
    args.extend(["--".to_owned(), target.remote.clone(), refspec]);
    args
}

/// Pushes the branch of the worktree at `dir` to its [`target`] with `env` (the space's,
/// [`remote_env`]), only while the branch and its target are still the `confirmed` ones (the
/// user saw them; something in the worktree may have switched the branch or changed its
/// upstream since); a rejected push is git's message.
pub fn push(dir: &Path, env: &[(&str, String)], confirmed: (&str, &str)) -> Result<String, String> {
    let target = target(dir)?;
    if (target.branch.as_str(), target.shown().as_str()) != confirmed {
        let (branch, to) = confirmed;
        return Err(format!(
            "{branch} → {to} changed since it was shown (now {} → {}): check again",
            target.branch,
            target.shown()
        ));
    }
    let args = push_args(&target);
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run(dir, ("push", &args), &[], env, PUSH_TIME)?;
    let set = match target.set_upstream {
        true => ", now its upstream",
        false => "",
    };
    Ok(format!(
        "Pushed {} to {}{set}",
        target.branch,
        target.shown()
    ))
}

#[cfg(test)]
mod tests;
