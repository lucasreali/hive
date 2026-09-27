//! The pull requests view (9.31): a followed project's GitHub repository (from its remotes),
//! its pull requests through `gh` as the project's space account ([`Gh::run`]), and the
//! actions on them. What `gh` prints is untrusted: read with size limits, text made fit to
//! show, links kept only when `https://`. The list is fetched from GitHub at most once per
//! [`INTERVAL`] for a project and account unless forced, and less often after GitHub's rate
//! limit ([`Cache`]).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use hive_protocol::{
    CheckState, Control, MergeMethod, Project, PullAction, PullCheck, PullDetail, PullFile,
    PullNote, PullRepo, PullState, PullSummary, ReviewDecision, ReviewState, SpaceEnv, Worktree,
};
use serde_json::Value;

use crate::adapter::{clip, invisible};
use crate::gh::Gh;
use crate::hook::now_ms;
use crate::projects::Projects;
use crate::{git, worktree};

/// How often the list may be fetched from GitHub without `force`.
pub const INTERVAL: Duration = Duration::from_secs(120);
/// The longest wait after GitHub's rate limit; the wait doubles from [`INTERVAL`].
const BACKOFF_LIMIT: Duration = Duration::from_secs(30 * 60);
/// The list's GraphQL query (one request per refresh).
const QUERY: &str = include_str!("pulls.graphql");
/// Pull requests in each list.
const FIRST: &str = "first=30";
/// The fields of `gh pr view --json` read for the details.
const VIEW_FIELDS: &str = "number,title,body,url,state,isDraft,isCrossRepository,author,headRefName,headRefOid,baseRefName,reviewDecision,reviews,comments,statusCheckRollup,files,additions,deletions,mergeable,updatedAt";
/// Most bytes read from `gh` for the list, the details and an action.
const LIST_OUTPUT: u64 = 1 << 20;
const VIEW_OUTPUT: u64 = 8 << 20;
const ACTION_OUTPUT: u64 = 64 << 10;
/// Most characters of a title, a name or a branch.
const LINE_LIMIT: usize = 256;
/// Most bytes of a description, of a review or comment, and of a URL.
const BODY_LIMIT: usize = 64 << 10;
const NOTE_LIMIT: usize = 16 << 10;
const URL_LIMIT: usize = 2048;
/// Most bytes of an error from `gh`.
const ERROR_LIMIT: usize = 4 << 10;
/// Most reviews and comments (the newest), checks and files in the details.
const NOTES_LIMIT: usize = 50;
const CHECKS_LIMIT: usize = 100;
const FILES_LIMIT: usize = 300;

/// A repository on a GitHub host.
#[derive(Debug, PartialEq, Eq)]
struct Repo {
    host: String,
    owner: String,
    name: String,
}

impl Repo {
    /// `gh`'s `--repo` value.
    fn arg(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.name)
    }
}

/// Answers a request of the pull requests view: the messages for the app. It blocks while
/// `gh` asks GitHub.
pub fn answer(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    request: Control,
) -> Vec<Control> {
    match request {
        Control::ListPulls { project, force } => list(gh, projects, cache, project, force)
            .into_iter()
            .collect(),
        Control::OpenPull { project, number } => vec![open(gh, projects, &project, number)],
        Control::ActOnPull {
            project,
            number,
            action,
        } => act(gh, projects, cache, project, number, action),
        Control::CreatePull {
            worktree,
            title,
            body,
            base,
            draft,
        } => {
            let form = Form {
                title: &title,
                body: &body,
                base: &base,
                draft,
            };
            match create(gh, projects, &worktree, form) {
                Ok((project, number)) => {
                    let message = match number {
                        0 => "Opened the pull request".to_owned(),
                        _ => format!("Opened pull request #{number}"),
                    };
                    let done = Control::PullDone {
                        project: project.clone(),
                        number,
                        message,
                    };
                    let listed = list(gh, projects, cache, project, true);
                    [Some(done), listed].into_iter().flatten().collect()
                }
                Err((project, message)) => vec![Control::PullFailed {
                    project,
                    number: None,
                    message,
                }],
            }
        }
        _ => Vec::new(),
    }
}

/// The project `id`'s `pulls`: the last one while it is recent (unless `force`d) or GitHub's
/// rate limit holds, none while a fetch runs (it answers), else a new one from GitHub.
fn list(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    id: String,
    force: bool,
) -> Option<Control> {
    let failed = |error: String, fetched_ms: u64| Control::Pulls {
        project: id.clone(),
        repo: None,
        mine: Vec::new(),
        review: Vec::new(),
        fetched_ms,
        error: Some(error),
    };
    let project = match projects.followed(&id) {
        Ok(project) => project,
        Err(err) => return Some(failed(err.to_string(), 0)),
    };
    let env = projects.space_env(&id);
    let key = key(&id, &env);
    let cache = || cache.lock().unwrap_or_else(PoisonError::into_inner);
    match cache().plan(&key, force, Instant::now()) {
        Plan::Send(reply) => return Some(*reply),
        Plan::Wait => return None,
        Plan::Fetch => {}
    }
    let reply = match fetch(gh, &env, &project) {
        Ok((repo, mine, review)) => Control::Pulls {
            project: id.clone(),
            repo: Some(repo),
            mine,
            review,
            fetched_ms: now_ms(),
            error: None,
        },
        Err(error) => failed(error, now_ms()),
    };
    Some(cache().done(&key, reply, Instant::now()))
}

/// The list's key in the [`Cache`]: the project and the space's account.
fn key(id: &str, env: &SpaceEnv) -> String {
    format!("{id}\n{:?}\n{:?}", env.gh_config_dir, env.gh_account)
}

/// The repository, the account's pull requests and those asking for its review, in one
/// GraphQL request.
fn fetch(
    gh: &Gh,
    env: &SpaceEnv,
    project: &Project,
) -> Result<(PullRepo, Vec<PullSummary>, Vec<PullSummary>), String> {
    let host = host(env);
    let root = Path::new(&project.path);
    let repo = remote(root, host)?;
    let search = |filter: &str| {
        let Repo { owner, name, .. } = &repo;
        format!("repo:{owner}/{name} is:pr {filter} sort:updated-desc")
    };
    let fields = [
        format!("query={QUERY}"),
        format!("owner={}", repo.owner),
        format!("name={}", repo.name),
        format!("mine={}", search("author:@me")),
        format!("review={}", search("is:open review-requested:@me")),
    ];
    let mut args = vec!["api", "graphql", "--hostname", host, "-F", FIRST];
    for field in &fields {
        args.extend(["-f", field.as_str()]);
    }
    let out = run(gh, env, root, &args, LIST_OUTPUT)?;
    parse_list(&out, &project.worktrees)
}

/// The `pull` message for pull request `number` of the project `id`.
fn open(gh: &Gh, projects: &Projects, id: &str, number: u64) -> Control {
    let pull = located(projects, id).and_then(|(project, env, repo)| {
        let (number, repo) = (number.to_string(), repo.arg());
        let args = [
            "pr",
            "view",
            &number,
            "--repo",
            &repo,
            "--json",
            VIEW_FIELDS,
        ];
        let out = run(gh, &env, Path::new(&project.path), &args, VIEW_OUTPUT)?;
        parse_view(&out, &project.worktrees)
    });
    let (pull, error) = match pull {
        Ok(pull) => (Some(pull), None),
        Err(error) => (None, Some(error)),
    };
    Control::Pull {
        project: id.to_owned(),
        number,
        pull,
        error,
    }
}

/// Acts on pull request `number`, then sends it and the list again; a checkout answers the
/// new worktree instead.
fn act(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    id: String,
    number: u64,
    action: PullAction,
) -> Vec<Control> {
    let done = located(projects, &id).and_then(|(project, env, repo)| {
        let (n, repo_arg) = (number.to_string(), repo.arg());
        let pr = |verb: &'static str| vec!["pr", verb, &n, "--repo", &repo_arg];
        let (args, message) = match &action {
            PullAction::Ready => (pr("ready"), format!("#{number} is ready for review")),
            PullAction::Close => (pr("close"), format!("Closed pull request #{number}")),
            PullAction::Merge { method, head } => {
                check_head(head)?;
                let mut args = pr("merge");
                args.extend([merge_flag(*method), "--match-head-commit", head]);
                (args, format!("Merged pull request #{number}"))
            }
            PullAction::Checkout => return checkout(gh, projects, &env, &repo, &project, number),
        };
        run(gh, &env, Path::new(&project.path), &args, ACTION_OUTPUT)?;
        Ok(Control::PullDone {
            project: id.clone(),
            number,
            message,
        })
    });
    let done = match done {
        Ok(done) => done,
        Err(message) => {
            let number = Some(number);
            return vec![Control::PullFailed {
                project: id,
                number,
                message,
            }];
        }
    };
    let pull = matches!(done, Control::PullDone { .. }).then(|| open(gh, projects, &id, number));
    let listed = list(gh, projects, cache, id, true);
    [Some(done), pull, listed].into_iter().flatten().collect()
}

/// Checks pull request `number` out into a new worktree `pr-<number>` (4.x's worktree flow,
/// then `gh pr checkout` in it, on the pull request's own branch). The `worktree-pr-<number>`
/// branch the flow made goes; so does the worktree when `gh` fails.
fn checkout(
    gh: &Gh,
    projects: &Projects,
    env: &SpaceEnv,
    repo: &Repo,
    project: &Project,
    number: u64,
) -> Result<Control, String> {
    let name = format!("pr-{number}");
    let created = projects.create_worktree(&project.id, &name, None);
    let (made, created) = created.map_err(|err| err.to_string())?;
    let root = Path::new(&project.path);
    let args = ["pr", "checkout", &number.to_string(), "--repo", &repo.arg()];
    let checked = run(gh, env, &created.path, &args, ACTION_OUTPUT);
    if checked.is_err() {
        let _ = worktree::remove_path(root, &created.path, true);
    }
    let _ = git::output(root, &["branch", "-D", &format!("worktree-{name}")], &[0]);
    checked?;
    Ok(Control::WorktreeCreated {
        // On the pull request's branch now (the project as made, should it be unfollowed).
        project: projects.followed(&project.id).unwrap_or(made),
        path: created.path.to_string_lossy().into_owned(),
        notes: created.notes,
    })
}

/// What `create_pull` asks for.
struct Form<'a> {
    title: &'a str,
    body: &'a str,
    base: &'a str,
    draft: bool,
}

/// Opens a pull request from the branch of `worktree`: its project and number, or the
/// project (empty when not followed) and why not.
fn create(
    gh: &Gh,
    projects: &Projects,
    worktree: &str,
    form: Form,
) -> Result<(String, u64), (String, String)> {
    let (project, wt) = projects
        .holding(worktree)
        .map_err(|err| (String::new(), err.to_string()))?;
    let id = project.id.clone();
    let fail = |message: &str| Err((id.clone(), message.to_owned()));
    let Some(branch) = wt.branch else {
        return fail("This worktree is on no branch");
    };
    let title = form.title.trim();
    if title.is_empty() || title.chars().count() > LINE_LIMIT {
        return fail("A title of 1 to 256 characters is needed");
    }
    if form.body.len() > BODY_LIMIT {
        return fail("The description is over 64 KiB");
    }
    let base = form.base.trim();
    if base.is_empty() || base.len() > LINE_LIMIT || base.starts_with('-') {
        return fail("The base branch is not a branch name");
    }
    let env = projects.space_env(&id);
    let repo = remote(Path::new(&project.path), host(&env)).map_err(|err| (id.clone(), err))?;
    let fields = [
        repo.arg(),
        format!("--head={branch}"),
        format!("--base={base}"),
        format!("--title={title}"),
        format!("--body={}", form.body),
    ];
    let mut args = vec!["pr", "create", "--repo"];
    args.extend(fields.iter().map(String::as_str));
    if form.draft {
        args.push("--draft");
    }
    let out = run(gh, &env, Path::new(&wt.path), &args, ACTION_OUTPUT);
    let out = out.map_err(|err| (id.clone(), err))?;
    // gh prints the new pull request's link.
    let link = String::from_utf8_lossy(&out);
    let number = link.trim().rsplit_once("/pull/").map(|(_, n)| n.parse());
    Ok((id, number.and_then(Result::ok).unwrap_or(0)))
}

/// The followed project `id`, its space's environment and its GitHub repository.
fn located(projects: &Projects, id: &str) -> Result<(Project, SpaceEnv, Repo), String> {
    let project = projects.followed(id).map_err(|err| err.to_string())?;
    let env = projects.space_env(id);
    let repo = remote(Path::new(&project.path), host(&env))?;
    Ok((project, env, repo))
}

/// The GitHub host of a space: its account's, else github.com.
fn host(env: &SpaceEnv) -> &str {
    env.gh_account
        .as_ref()
        .map_or("github.com", |account| account.host.as_str())
}

/// `gh <args>` as the space runs it. An error (`gh`'s stderr) is cut at [`ERROR_LIMIT`] and
/// names the command by its first two words: the rest can be long (the query, a description).
fn run(gh: &Gh, env: &SpaceEnv, cwd: &Path, args: &[&str], limit: u64) -> Result<Vec<u8>, String> {
    let short: Vec<&str> = args.iter().take(2).copied().collect();
    gh.run(env, cwd, args, limit).map_err(|err| {
        let err = err.replacen(&args.join(" "), &short.join(" "), 1);
        multiline(&err, ERROR_LIMIT)
    })
}

/// The GitHub repository of the project at `root`, from its remotes on `host`.
fn remote(root: &Path, host: &str) -> Result<Repo, String> {
    let args = ["config", "--get-regexp", r"^remote\..*\.(url|gh-resolved)$"];
    // A repository git cannot read has no remote either.
    let out = git::output(root, &args, &[0, 1]).unwrap_or_default();
    pick_remote(&String::from_utf8_lossy(&out), host)
        .ok_or_else(|| format!("No remote of this repository is on {host}"))
}

/// The repository `gh` takes as the base one, from `git config --get-regexp` lines: the
/// remote `gh repo set-default` chose (`gh-resolved = base`), else `upstream`, `github`,
/// `origin`, else the first; only remotes on `host`.
fn pick_remote(config: &str, host: &str) -> Option<Repo> {
    let mut base = None;
    let mut found = Vec::new();
    for (key, value) in config.lines().filter_map(|line| line.split_once(' ')) {
        let Some(key) = key.strip_prefix("remote.") else {
            continue;
        };
        if let Some(name) = key.strip_suffix(".gh-resolved") {
            base = base.or((value == "base").then_some(name));
        } else if let Some(name) = key.strip_suffix(".url")
            && let Some(repo) = parse_url(value, host)
        {
            found.push((name, repo));
        }
    }
    let rank = |name: &str| match name {
        _ if Some(name) == base => 0,
        "upstream" => 1,
        "github" => 2,
        "origin" => 3,
        _ => 4,
    };
    let best = found.into_iter().min_by_key(|(name, _)| rank(name));
    best.map(|(_, repo)| repo)
}

/// The repository a remote URL names on `host`: `https://[user@]host/owner/name[.git]`,
/// `ssh://`, `git://`, or `[user@]host:owner/name[.git]`.
fn parse_url(url: &str, host: &str) -> Option<Repo> {
    let (authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let known = ["https", "http", "ssh", "git", "git+ssh", "ssh+git"];
            known.contains(&scheme).then_some(())?;
            rest.split_once('/')?
        }
        None => url.split_once(':')?,
    };
    let at = authority.rsplit('@').next().unwrap_or_default();
    let at = at.split(':').next().unwrap_or_default();
    at.eq_ignore_ascii_case(host).then_some(())?;
    let path = path.trim_end_matches('/');
    let (owner, name) = path.strip_suffix(".git").unwrap_or(path).split_once('/')?;
    let valid = |part: &str| {
        let chars = |b: u8| b.is_ascii_alphanumeric() || b"-_.".contains(&b);
        !matches!(part, "" | "." | "..")
            && part.len() <= 100
            && !part.starts_with('-')
            && part.bytes().all(chars)
    };
    (valid(owner) && valid(name)).then(|| Repo {
        host: host.to_owned(),
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// The string at `pointer` in `value`; empty when there is none.
fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn number(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn items<'a>(value: &'a Value, pointer: &str) -> &'a [Value] {
    let list = value.pointer(pointer).and_then(Value::as_array);
    list.map_or(&[], Vec::as_slice)
}

/// A line of untrusted text to show ([`clip`]).
fn line(value: &Value, pointer: &str) -> String {
    clip(text(value, pointer), LINE_LIMIT)
}

/// Untrusted text to show (Markdown, `gh`'s errors): its lines and tabs kept, other control and invisible
/// characters dropped, cut at `max` bytes (then ending with "…").
fn multiline(text: &str, max: usize) -> String {
    let keep = |c: &char| matches!(c, '\n' | '\t') || !invisible(*c);
    let mut clean: String = text.chars().filter(keep).collect();
    if clean.len() > max {
        let end = (0..=max).rev().find(|&i| clean.is_char_boundary(i));
        clean.truncate(end.unwrap_or(0));
        clean.push('…');
    }
    clean
}

/// A link to open: `https://` only, without spaces or invisible characters.
fn link(url: &str) -> Option<String> {
    let clean = !url.chars().any(|c| c.is_whitespace() || invisible(c));
    (url.starts_with("https://") && url.len() <= URL_LIMIT && clean).then(|| url.to_owned())
}

/// The list query's answer: the repository and both lists.
fn parse_list(
    out: &[u8],
    worktrees: &[Worktree],
) -> Result<(PullRepo, Vec<PullSummary>, Vec<PullSummary>), String> {
    let answer: Value =
        serde_json::from_slice(out).map_err(|err| format!("gh gave no list: {err}"))?;
    let repo = answer
        .pointer("/data/repository")
        .filter(|repo| repo.is_object())
        .ok_or("gh gave no repository")?;
    let allowed = [
        ("/mergeCommitAllowed", MergeMethod::Merge),
        ("/squashMergeAllowed", MergeMethod::Squash),
        ("/rebaseMergeAllowed", MergeMethod::Rebase),
    ];
    let merge_methods = allowed
        .into_iter()
        .filter(|(pointer, _)| repo.pointer(pointer) == Some(&Value::Bool(true)))
        .map(|(_, method)| method)
        .collect();
    let repo = PullRepo {
        name: line(repo, "/nameWithOwner"),
        url: link(text(repo, "/url")).unwrap_or_default(),
        merge_methods,
        default_merge: match text(repo, "/viewerDefaultMergeMethod") {
            "MERGE" => Some(MergeMethod::Merge),
            "SQUASH" => Some(MergeMethod::Squash),
            "REBASE" => Some(MergeMethod::Rebase),
            _ => None,
        },
    };
    let pulls = |pointer: &str| {
        let nodes = items(&answer, pointer).iter();
        nodes.filter_map(|node| summary(node, worktrees)).collect()
    };
    Ok((repo, pulls("/data/mine/nodes"), pulls("/data/review/nodes")))
}

/// A pull request of the list query or of `gh pr view` (whose checks are counted apart).
fn summary(node: &Value, worktrees: &[Worktree]) -> Option<PullSummary> {
    let pull = number(node, "/number");
    (pull > 0).then_some(())?;
    let flag = |pointer: &str| node.pointer(pointer).and_then(Value::as_bool);
    let state = match (text(node, "/state"), flag("/isDraft")) {
        ("MERGED", _) => PullState::Merged,
        ("CLOSED", _) => PullState::Closed,
        (_, Some(true)) => PullState::Draft,
        _ => PullState::Open,
    };
    let branch = text(node, "/headRefName");
    // Only the repository's own branches are the worktrees' (a fork's `main` is not ours).
    let own = flag("/isCrossRepository") == Some(false);
    let on_branch = |w: &&Worktree| own && w.branch.as_deref() == Some(branch);
    let commit = items(node, "/commits/nodes").last();
    let rollup = commit.map(|commit| text(commit, "/commit/statusCheckRollup/state"));
    Some(PullSummary {
        number: pull,
        title: line(node, "/title"),
        url: link(text(node, "/url")).unwrap_or_default(),
        state,
        branch: clip(branch, LINE_LIMIT),
        base: line(node, "/baseRefName"),
        author: line(node, "/author/login"),
        review: match text(node, "/reviewDecision") {
            "APPROVED" => Some(ReviewDecision::Approved),
            "CHANGES_REQUESTED" => Some(ReviewDecision::ChangesRequested),
            "REVIEW_REQUIRED" => Some(ReviewDecision::ReviewRequired),
            _ => None,
        },
        checks: rollup.and_then(check_state),
        updated_at: line(node, "/updatedAt"),
        worktree: worktrees.iter().find(on_branch).map(|w| w.id.clone()),
    })
}

/// A state or conclusion as GitHub names them (check runs, commit statuses and their rollup).
fn check_state(state: &str) -> Option<CheckState> {
    match state {
        "SUCCESS" => Some(CheckState::Passing),
        "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
            Some(CheckState::Failing)
        }
        "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "WAITING" | "REQUESTED" => {
            Some(CheckState::Running)
        }
        "NEUTRAL" | "SKIPPED" | "STALE" => Some(CheckState::Skipped),
        _ => None,
    }
}

/// `gh pr view --json`'s answer.
fn parse_view(out: &[u8], worktrees: &[Worktree]) -> Result<PullDetail, String> {
    let view: Value =
        serde_json::from_slice(out).map_err(|err| format!("gh gave no pull request: {err}"))?;
    let mut summary = summary(&view, worktrees).ok_or("gh gave no pull request")?;
    let checks: Vec<PullCheck> = items(&view, "/statusCheckRollup")
        .iter()
        .take(CHECKS_LIMIT)
        .map(check)
        .collect();
    // Like GitHub's rollup: any failing one wins, then any still running.
    let order = [
        CheckState::Failing,
        CheckState::Running,
        CheckState::Passing,
        CheckState::Skipped,
    ];
    summary.checks = order
        .into_iter()
        .find(|s| checks.iter().any(|c| c.state == *s));
    let files = items(&view, "/files").iter().take(FILES_LIMIT);
    let files = files.map(|file| PullFile {
        path: line(file, "/path"),
        additions: number(file, "/additions"),
        deletions: number(file, "/deletions"),
    });
    Ok(PullDetail {
        summary,
        body: multiline(text(&view, "/body"), BODY_LIMIT),
        head: line(&view, "/headRefOid"),
        additions: number(&view, "/additions"),
        deletions: number(&view, "/deletions"),
        conflicts: text(&view, "/mergeable") == "CONFLICTING",
        notes: notes(&view),
        checks,
        files: files.collect(),
    })
}

/// A check run (`CheckRun`) or a commit status (`StatusContext`).
fn check(item: &Value) -> PullCheck {
    let run = text(item, "/__typename") == "CheckRun";
    let (name, state, url) = match run {
        true => ("/name", "/conclusion", "/detailsUrl"),
        false => ("/context", "/state", "/targetUrl"),
    };
    let running = run && text(item, "/status") != "COMPLETED";
    let state = match running {
        true => CheckState::Running,
        false => check_state(text(item, state)).unwrap_or(CheckState::Skipped),
    };
    let workflow = line(item, "/workflowName");
    PullCheck {
        name: line(item, name),
        workflow: (!workflow.is_empty()).then_some(workflow),
        state,
        url: link(text(item, url)),
    }
}

/// Reviews with a verdict or a text, and comments not hidden, oldest first; the newest
/// [`NOTES_LIMIT`].
fn notes(view: &Value) -> Vec<PullNote> {
    let reviews = items(view, "/reviews").iter().filter_map(|review| {
        let verdict = match text(review, "/state") {
            "APPROVED" => ReviewState::Approved,
            "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
            "COMMENTED" => ReviewState::Commented,
            "DISMISSED" => ReviewState::Dismissed,
            _ => return None,
        };
        let empty = text(review, "/body").trim().is_empty();
        (verdict != ReviewState::Commented || !empty)
            .then(|| note(review, "/submittedAt", Some(verdict)))
    });
    let comments = items(view, "/comments").iter();
    let shown = comments.filter(|c| c.pointer("/isMinimized") != Some(&Value::Bool(true)));
    let mut notes: Vec<PullNote> = reviews
        .chain(shown.map(|c| note(c, "/createdAt", None)))
        .collect();
    notes.sort_by(|a, b| a.at.cmp(&b.at));
    notes.split_off(notes.len().saturating_sub(NOTES_LIMIT))
}

fn note(item: &Value, at: &str, review: Option<ReviewState>) -> PullNote {
    PullNote {
        author: line(item, "/author/login"),
        body: multiline(text(item, "/body"), NOTE_LIMIT),
        at: line(item, at),
        review,
    }
}

/// A head commit as the app got it: 40 (SHA-1) or 64 (SHA-256) hexadecimal digits.
fn check_head(head: &str) -> Result<(), String> {
    let hex = head.bytes().all(|b| b.is_ascii_hexdigit());
    match hex && matches!(head.len(), 40 | 64) {
        true => Ok(()),
        false => Err("The pull request's head is not a commit".to_owned()),
    }
}

fn merge_flag(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => "--merge",
        MergeMethod::Squash => "--squash",
        MergeMethod::Rebase => "--rebase",
    }
}

/// The last list of each project and account, and when GitHub may be asked again.
#[derive(Default)]
pub struct Cache(HashMap<String, Entry>);

#[derive(Default)]
struct Entry {
    last: Option<Control>,
    asked: Option<Instant>,
    /// A fetch runs: it answers every request made meanwhile.
    busy: bool,
    /// After GitHub's rate limit: no fetch before `retry`, and how long it waited.
    retry: Option<Instant>,
    backoff: Duration,
}

/// What a list request does.
#[derive(Debug, PartialEq)]
enum Plan {
    Send(Box<Control>),
    Fetch,
    Wait,
}

impl Cache {
    /// What a list request for `key` does at `now`: a fetch only when none runs, and the last
    /// list is older than [`INTERVAL`] or `force`d, and GitHub's rate limit is not holding.
    fn plan(&mut self, key: &str, force: bool, now: Instant) -> Plan {
        let entry = self.0.entry(key.to_owned()).or_default();
        let limited = entry.retry.is_some_and(|retry| now < retry);
        let fresh = entry.asked.is_some_and(|asked| now - asked < INTERVAL);
        match &entry.last {
            _ if entry.busy => Plan::Wait,
            Some(last) if limited || (fresh && !force) => Plan::Send(Box::new(last.clone())),
            _ => {
                entry.busy = true;
                Plan::Fetch
            }
        }
    }

    /// Keeps the list fetched for `key` at `now`. After GitHub's rate limit (its error says
    /// so) the next fetch waits twice as long as the last one, from [`INTERVAL`] up to
    /// [`BACKOFF_LIMIT`], and the error tells when.
    fn done(&mut self, key: &str, mut reply: Control, now: Instant) -> Control {
        let entry = self.0.entry(key.to_owned()).or_default();
        entry.busy = false;
        entry.asked = Some(now);
        let limited = match &mut reply {
            Control::Pulls {
                error: Some(error), ..
            } if error.to_ascii_lowercase().contains("rate limit") => Some(error),
            _ => None,
        };
        entry.backoff = match limited {
            Some(_) => (entry.backoff * 2).clamp(INTERVAL, BACKOFF_LIMIT),
            None => Duration::ZERO,
        };
        entry.retry = limited.is_some().then(|| now + entry.backoff);
        if let Some(error) = limited {
            let minutes = entry.backoff.as_secs() / 60;
            *error = format!("GitHub's rate limit: Hive asks again in {minutes} min. {error}");
        }
        entry.last = Some(reply.clone());
        reply
    }
}

#[cfg(test)]
mod tests;
