//! The Actions view (9.32): a followed project's latest GitHub Actions workflow runs, a run's
//! jobs and steps, a failed job's log tail, and re-running or cancelling a run, through `gh`
//! as the project's space account. Built on [`crate::pulls`]: its repository lookup, its
//! `gh` runner, its untrusted-text helpers and its [`Cache`] (the list is fetched at most once
//! per 2 minutes for a project, branch and account unless forced).

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use hive_protocol::{
    CheckState, Control, RunAction, RunDetail, RunJob, RunStep, RunSummary, Worktree,
};
use serde_json::Value;

use crate::adapter::{clip, invisible};
use crate::gh::Gh;
use crate::git::Stdout;
use crate::hook::now_ms;
use crate::projects::Projects;
use crate::pulls::{
    ACTION_OUTPUT, Cache, LINE_LIMIT, Plan, items, key, line, link, located, number, run, text,
};

/// Runs in a list.
const LIMIT: &str = "30";
/// The fields of `gh run list --json`; `gh run view` adds `jobs`.
const FIELDS: &str = "databaseId,number,workflowName,displayTitle,headBranch,event,status,conclusion,createdAt,startedAt,updatedAt,url";
/// Most bytes read from `gh` for the list and a run's jobs.
const LIST_OUTPUT: Stdout = Stdout::Max(1 << 20);
const VIEW_OUTPUT: Stdout = Stdout::Max(4 << 20);
/// The end of a job's log read from GitHub, however long the log is.
const RAW_LOG: Stdout = Stdout::Tail(1 << 20);
/// Most bytes of the log tail sent to the app.
const LOG_LIMIT: usize = 64 << 10;
/// Most jobs of a run (GitHub's own matrix limit) and steps of a job.
const JOBS_LIMIT: usize = 256;
const STEPS_LIMIT: usize = 100;
/// Longest status word (`startup_failure` and the like).
const WORD_LIMIT: usize = 32;

/// Answers a request of the Actions view: the messages for the app. It blocks while `gh`
/// asks GitHub.
pub fn answer(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    request: Control,
) -> Vec<Control> {
    match request {
        Control::ListRuns {
            project,
            branch,
            force,
        } => list(gh, projects, cache, project, branch, force)
            .into_iter()
            .collect(),
        Control::OpenRun { project, run } => vec![open(gh, projects, &project, run)],
        Control::OpenJobLog { project, job } => vec![job_log(gh, projects, project, job)],
        Control::ActOnRun {
            project,
            run,
            action,
            branch,
        } => act(gh, projects, cache, (project, run), action, branch),
        _ => Vec::new(),
    }
}

/// The project `id`'s `runs` on `branch` (all branches without one), as `pulls::list` gives
/// its pull requests: from the cache while recent or rate-limited, none while a fetch runs.
fn list(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    id: String,
    branch: Option<String>,
    force: bool,
) -> Option<Control> {
    let reply = |runs: Result<Vec<RunSummary>, String>, fetched_ms: u64| {
        let (runs, error) = match runs {
            Ok(runs) => (runs, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Control::Runs {
            project: id.clone(),
            branch: branch.clone(),
            runs,
            fetched_ms,
            error,
        }
    };
    let (project, env, repo) = match located(projects, &id) {
        Ok(found) => found,
        Err(error) => return Some(reply(Err(error), 0)),
    };
    let key = format!("{}\n{branch:?}", key(&id, &env));
    let cache = || cache.lock().unwrap_or_else(PoisonError::into_inner);
    match cache().plan(&key, force, Instant::now()) {
        Plan::Send(reply) => return Some(*reply),
        Plan::Wait => return None,
        Plan::Fetch => {}
    }
    let repo = repo.arg();
    let mut args = vec!["run", "list", "--repo", &repo, "--limit", LIMIT];
    let only = branch.as_ref().map(|branch| format!("--branch={branch}"));
    args.extend(only.as_deref());
    args.extend(["--json", FIELDS]);
    let out = run(gh, &env, Path::new(&project.path), &args, LIST_OUTPUT);
    let runs = out.and_then(|out| parse_list(&out, &project.worktrees));
    Some(cache().done(&key, reply(runs, now_ms()), Instant::now()))
}

/// The `run` message for run `id` of the project `project`.
fn open(gh: &Gh, projects: &Projects, project: &str, id: u64) -> Control {
    let detail = located(projects, project).and_then(|(found, env, repo)| {
        let (id, repo) = (id.to_string(), repo.arg());
        let fields = format!("{FIELDS},jobs");
        let args = ["run", "view", &id, "--repo", &repo, "--json", &fields];
        let out = run(gh, &env, Path::new(&found.path), &args, VIEW_OUTPUT)?;
        parse_view(&out, &found.worktrees)
    });
    let (detail, error) = match detail {
        Ok(detail) => (Some(detail), None),
        Err(error) => (None, Some(error)),
    };
    Control::Run {
        project: project.to_owned(),
        run: id,
        detail,
        error,
    }
}

/// The `job_log` message: the end of job `job`'s log ([`log_tail`]).
fn job_log(gh: &Gh, projects: &Projects, project: String, job: u64) -> Control {
    let log = located(projects, &project).and_then(|(found, env, repo)| {
        let path = format!("repos/{}/{}/actions/jobs/{job}/logs", repo.owner, repo.name);
        let args = ["api", "--hostname", &repo.host, &path];
        let out = run(gh, &env, Path::new(&found.path), &args, RAW_LOG)?;
        Ok(log_tail(&String::from_utf8_lossy(&out)))
    });
    let (log, error) = match log {
        Ok(log) => (Some(log), None),
        Err(error) => (None, Some(error)),
    };
    Control::JobLog {
        project,
        job,
        log,
        error,
    }
}

/// Re-runs or cancels run `id`, then sends it and the `branch` list again.
fn act(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    (project, id): (String, u64),
    action: RunAction,
    branch: Option<String>,
) -> Vec<Control> {
    let done = located(projects, &project).and_then(|(found, env, repo)| {
        let (id, repo) = (id.to_string(), repo.arg());
        let (mut args, message) = match action {
            RunAction::Rerun { failed: false } => (vec!["run", "rerun"], "Re-running every job"),
            RunAction::Rerun { failed: true } => {
                (vec!["run", "rerun"], "Re-running the failed jobs")
            }
            RunAction::Cancel => (vec!["run", "cancel"], "Cancelling the run"),
        };
        args.extend([id.as_str(), "--repo", &repo]);
        if action == (RunAction::Rerun { failed: true }) {
            args.push("--failed");
        }
        run(gh, &env, Path::new(&found.path), &args, ACTION_OUTPUT)?;
        Ok(message.to_owned())
    });
    let message = match done {
        Ok(message) => message,
        Err(message) => {
            return vec![Control::RunFailed {
                project,
                run: id,
                message,
            }];
        }
    };
    let done = Control::RunDone {
        project: project.clone(),
        run: id,
        message,
    };
    let detail = open(gh, projects, &project, id);
    let listed = list(gh, projects, cache, project, branch, true);
    [Some(done), Some(detail), listed]
        .into_iter()
        .flatten()
        .collect()
}

/// `gh run list --json`'s answer.
fn parse_list(out: &[u8], worktrees: &[Worktree]) -> Result<Vec<RunSummary>, String> {
    let list: Value =
        serde_json::from_slice(out).map_err(|err| format!("gh gave no runs: {err}"))?;
    let runs = list.as_array().ok_or("gh gave no runs")?.iter();
    Ok(runs.filter_map(|run| summary(run, worktrees)).collect())
}

/// `gh run view --json`'s answer.
fn parse_view(out: &[u8], worktrees: &[Worktree]) -> Result<RunDetail, String> {
    let view: Value =
        serde_json::from_slice(out).map_err(|err| format!("gh gave no run: {err}"))?;
    let summary = summary(&view, worktrees).ok_or("gh gave no run")?;
    let jobs = items(&view, "/jobs").iter().take(JOBS_LIMIT).map(|job| {
        let (status, state) = progress(job);
        let steps = items(job, "/steps").iter().take(STEPS_LIMIT);
        let steps = steps.map(|step| {
            let (status, state) = progress(step);
            RunStep {
                number: number(step, "/number"),
                name: line(step, "/name"),
                state,
                status,
            }
        });
        RunJob {
            id: number(job, "/databaseId"),
            name: line(job, "/name"),
            state,
            status,
            url: link(text(job, "/url")).unwrap_or_default(),
            started_at: line(job, "/startedAt"),
            completed_at: line(job, "/completedAt"),
            steps: steps.collect(),
        }
    });
    Ok(RunDetail {
        summary,
        jobs: jobs.collect(),
    })
}

/// A run of the list or of `gh run view`; none without its id.
fn summary(node: &Value, worktrees: &[Worktree]) -> Option<RunSummary> {
    let id = number(node, "/databaseId");
    (id > 0).then_some(())?;
    let branch = text(node, "/headBranch");
    let (status, state) = progress(node);
    Some(RunSummary {
        id,
        number: number(node, "/number"),
        workflow: line(node, "/workflowName"),
        title: line(node, "/displayTitle"),
        branch: clip(branch, LINE_LIMIT),
        event: word(text(node, "/event")),
        state,
        status,
        url: link(text(node, "/url")).unwrap_or_default(),
        created_at: line(node, "/createdAt"),
        started_at: line(node, "/startedAt"),
        updated_at: line(node, "/updatedAt"),
        worktree: worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(branch))
            .map(|w| w.id.clone()),
    })
}

/// A run's, job's or step's status to show (its conclusion once completed) and its state.
fn progress(node: &Value) -> (String, CheckState) {
    let (status, conclusion) = (text(node, "/status"), text(node, "/conclusion"));
    let state = match (status, conclusion) {
        ("completed", "success") => CheckState::Passing,
        ("completed", "failure" | "timed_out" | "startup_failure") => CheckState::Failing,
        // Cancelled, skipped, neutral, stale, waiting for an approval.
        ("completed", _) => CheckState::Skipped,
        _ => CheckState::Running,
    };
    let shown = match (status, conclusion) {
        ("completed", conclusion) if !conclusion.is_empty() => conclusion,
        _ => status,
    };
    (word(shown), state)
}

/// A word of GitHub's (a status, an event): lowercase letters and `_` only, else nothing.
fn word(text: &str) -> String {
    let valid =
        text.len() <= WORD_LIMIT && text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    match valid {
        true => text.to_owned(),
        false => String::new(),
    }
}

/// What the Actions view shows of a job's log, from its (possibly cut) end: each line without
/// GitHub's timestamp, its ANSI escapes and other control characters, and the runner's
/// grouping markers; up to the last `##[error]` line (the failed step's end, not the cleanup
/// after it) when there is one; the last [`LOG_LIMIT`] bytes of that, whole lines only.
fn log_tail(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        // GitHub starts each line with `2026-09-27T10:33:42.5489270Z `.
        let line = match line.split_at_checked(28) {
            Some((time, rest)) if time.ends_with('Z') => rest.strip_prefix(' ').unwrap_or(rest),
            _ => line,
        };
        // A carriage return redraws the line: only its last text shows.
        let line = line.rsplit('\r').next().unwrap_or_default();
        let line = strip_ansi(line);
        let hidden = ["##[endgroup]", "##[start-action ", "##[end-action "];
        if hidden.iter().any(|marker| line.starts_with(marker)) {
            continue;
        }
        let line = line.strip_prefix("##[group]").unwrap_or(&line);
        lines.push(
            line.chars()
                .filter(|&c| c == '\t' || !invisible(c))
                .collect(),
        );
    }
    if let Some(error) = lines.iter().rposition(|l| l.starts_with("##[error]")) {
        lines.truncate(error + 1);
    }
    let mut size = 0;
    let kept = lines.iter().rev().take_while(|line| {
        size += line.len() + 1;
        size <= LOG_LIMIT
    });
    let start = lines.len() - kept.count();
    lines[start..].join("\n")
}

/// `line` without its ANSI escapes: CSI (`ESC [` … final byte, colours), OSC (`ESC ]` … BEL or
/// `ESC \`, titles and links) and two-character ones.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        let end = match chars.next() {
            Some('[') => chars.find(|c| ('@'..='~').contains(c)),
            Some(']') => chars.find(|c| matches!(c, '\u{7}' | '\u{1b}')),
            _ => None,
        };
        // An OSC ended by `ESC \`.
        if end == Some('\u{1b}') {
            chars.next();
        }
    }
    out
}

#[cfg(test)]
mod tests;
