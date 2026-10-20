//! Worktrees from GitHub (17.24): a pull request checked out into a new worktree on its
//! branch, and a new worktree for an issue on a branch named from its number and title. Hive
//! only prepares the worktree; the human starts `claude`.
//!
//! What GitHub says (titles, branch names, commits) is untrusted: titles are made into
//! lowercase ASCII words and capped, a pull request's branch is checked with
//! `git check-ref-format --branch`, a commit must be hexadecimal, and each goes to git as a
//! separate argument. Nothing is fetched from a fork: its pull request comes from the base
//! repository's `refs/pull/<number>/head`, so no URL from GitHub ever reaches git. A local
//! branch that already exists is checked out as it is, never moved.

use std::path::Path;

use hive_protocol::{Control, Project, SpaceEnv};
use serde_json::Value;

use crate::gh::Gh;
use crate::git;
use crate::projects::Projects;
use crate::pulls::{self, ACTION_OUTPUT, Repo};
use crate::worktree::{self, Head};

/// Longest branch Hive names from an issue's title or a fork's branch.
const NAME_LIMIT: usize = 64;
/// The fields of `gh pr view --json` a checkout reads.
const PULL_FIELDS: &str = "number,headRefName,headRefOid,isCrossRepository";
/// The fields of `gh issue view --json` read.
const ISSUE_FIELDS: &str = "number,title";

/// A pull request's head, as `gh pr view` gives it.
#[derive(Debug, PartialEq, Eq)]
struct PullHead {
    branch: String,
    commit: String,
    /// From another repository (a fork); unknown counts as one.
    fork: bool,
}

/// Checks pull request `number` of `repo` out into a new worktree `pr-<number>`. A branch of
/// the repository itself is fetched into its remote-tracking branch and the worktree gets a
/// local branch of the same name tracking it; a fork's is fetched from `refs/pull/<number>/head`
/// onto a new branch `pr-<number>-<its branch>`.
pub(crate) fn pull(
    gh: &Gh,
    projects: &Projects,
    env: &SpaceEnv,
    repo: &Repo,
    project: &Project,
    number: u64,
) -> Result<Control, String> {
    let root = Path::new(&project.path);
    let (n, repo_arg) = (number.to_string(), repo.arg());
    let args = ["pr", "view", &n, "--repo", &repo_arg, "--json", PULL_FIELDS];
    let head = parse_pull(&pulls::run(gh, env, root, &args, ACTION_OUTPUT)?, number)?;
    let remote = remote_name(root, repo)?;
    let (branch, refspec, start) = match head.fork {
        true => {
            pulls::check_head(&head.commit)?;
            let branch = named("pr", number, &head.branch);
            (branch, format!("refs/pull/{number}/head"), head.commit)
        }
        false => {
            let branch = head.branch;
            let tracking = format!("refs/remotes/{remote}/{branch}");
            (
                branch.clone(),
                format!("+refs/heads/{branch}:{tracking}"),
                tracking,
            )
        }
    };
    worktree::check_branch(root, &branch).map_err(|err| err.to_string())?;
    worktree::fetch(root, &remote, &refspec, &fetch_vars(gh, env)).map_err(shown)?;
    let name = format!("pr-{number}");
    created(projects, project, &name, &branch, Some(&start), None)
}

/// A new worktree for issue `number` of the followed project `id`'s repository, on the
/// branch `<number>-<title>` from `base` (fetched first when remote, as `create_worktree`).
pub(crate) fn issue(
    gh: &Gh,
    projects: &Projects,
    id: &str,
    number: u64,
    base: Option<&str>,
) -> Result<Control, String> {
    let (project, env, repo) = pulls::located(projects, id)?;
    let root = Path::new(&project.path);
    let (n, repo_arg) = (number.to_string(), repo.arg());
    let args = [
        "issue",
        "view",
        &n,
        "--repo",
        &repo_arg,
        "--json",
        ISSUE_FIELDS,
    ];
    let title = parse_issue(&pulls::run(gh, &env, root, &args, ACTION_OUTPUT)?, number)?;
    let branch = named("issue", number, &title);
    worktree::check_branch(root, &branch).map_err(|err| err.to_string())?;
    let fetched = base.and_then(|base| worktree::fetch_base(root, base, &fetch_vars(gh, &env)));
    created(projects, &project, &branch, &branch, base, fetched)
}

/// The worktree `name` of `project` on `branch`: a new one from `start`, or the local branch
/// as it is when it already exists (said in a note, after `fetched`'s).
fn created(
    projects: &Projects,
    project: &Project,
    name: &str,
    branch: &str,
    start: Option<&str>,
    fetched: Option<String>,
) -> Result<Control, String> {
    let root = Path::new(&project.path);
    let exists = worktree::has_branch(root, branch).map_err(|err| err.to_string())?;
    let (head, kept) = match exists {
        true => {
            let note = format!("branch {branch} already existed: checked out as it is");
            (Head::Existing(branch), Some(note))
        }
        false => (Head::New { branch, start }, None),
    };
    let made = projects.create_worktree_on(&project.id, name, head);
    let (project, created) = made.map_err(|err| err.to_string())?;
    Ok(Control::WorktreeCreated {
        project,
        path: created.path.to_string_lossy().into_owned(),
        notes: fetched
            .into_iter()
            .chain(kept)
            .chain(created.notes)
            .collect(),
    })
}

/// What the space's terminals give git to fetch: its identity and its account's token
/// (none when `gh` gives none: git then uses the user's own credentials).
fn fetch_vars(gh: &Gh, env: &SpaceEnv) -> Vec<(&'static str, String)> {
    let mut vars = crate::spaces::vars(env);
    vars.extend(gh.vars(env).unwrap_or_default());
    vars
}

/// A git error fit to show: what a remote prints is untrusted.
fn shown(err: std::io::Error) -> String {
    pulls::multiline(&err.to_string(), pulls::ERROR_LIMIT)
}

/// The name of the remote of the repository at `root` that is `repo`.
fn remote_name(root: &Path, repo: &Repo) -> Result<String, String> {
    let args = ["config", "--get-regexp", r"^remote\..*\.url$"];
    let out = git::output(root, &args, &[0, 1]).unwrap_or_default();
    named_remote(&String::from_utf8_lossy(&out), repo)
        .ok_or_else(|| format!("No remote of this repository is {}", repo.arg()))
}

/// The first remote whose URL, in `git config --get-regexp` lines, names `repo`.
fn named_remote(config: &str, repo: &Repo) -> Option<String> {
    config
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(_, url)| pulls::parse_url(url, &repo.host).as_ref() == Some(repo))
        .find_map(|(key, _)| key.strip_prefix("remote.")?.strip_suffix(".url"))
        .map(str::to_owned)
}

/// `gh pr view`'s answer about pull request `number`.
fn parse_pull(out: &[u8], number: u64) -> Result<PullHead, String> {
    let value = answer(out, number, "pull request")?;
    Ok(PullHead {
        branch: pulls::text(&value, "/headRefName").to_owned(),
        commit: pulls::text(&value, "/headRefOid").to_owned(),
        fork: value.pointer("/isCrossRepository").and_then(Value::as_bool) != Some(false),
    })
}

/// The title in `gh issue view`'s answer about issue `number`.
fn parse_issue(out: &[u8], number: u64) -> Result<String, String> {
    let value = answer(out, number, "issue")?;
    Ok(pulls::text(&value, "/title").to_owned())
}

/// `gh`'s JSON answer, when it is about `what` `number`.
fn answer(out: &[u8], number: u64, what: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_slice(out).map_err(|err| format!("gh gave no {what}: {err}"))?;
    match pulls::number(&value, "/number") == number {
        true => Ok(value),
        false => Err(format!("gh gave another {what} than #{number}")),
    }
}

/// `<number>-<words>` for an issue, `pr-<number>-<words>` for a fork's pull request, from
/// `text`'s ASCII letters and digits ([`words`]), at most [`NAME_LIMIT`] long; without
/// words, `<what>-<number>`.
fn named(what: &str, number: u64, text: &str) -> String {
    let prefix = match what {
        "pr" => format!("pr-{number}"),
        _ => number.to_string(),
    };
    let words = words(text, NAME_LIMIT - prefix.len() - 1);
    match words.is_empty() {
        true => format!("{what}-{number}"),
        false => format!("{prefix}-{words}"),
    }
}

/// `text`'s runs of ASCII letters and digits, lowercase, joined by `-`, cut at `max` bytes
/// (never ending with `-`). Anything else (accents, emoji, `/`, `..`, controls) separates.
fn words(text: &str, max: usize) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let mut joined = words.join("-");
    joined.truncate(max);
    joined.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests;
