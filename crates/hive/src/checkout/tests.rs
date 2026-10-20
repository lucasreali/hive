#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::Command;
#[cfg(unix)]
use std::sync::Mutex;

#[cfg(unix)]
use hive_protocol::PullAction;
use serde_json::json;

use super::*;

fn repo() -> Repo {
    Repo {
        host: "github.com".into(),
        owner: "o".into(),
        name: "r".into(),
    }
}

/// `git <args>` in `dir`, without the user's own configuration; its stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn titles_become_lowercase_words_within_the_limit() {
    assert_eq!(
        named("issue", 42, "Fix: the login crash! 🚀"),
        "42-fix-the-login-crash"
    );
    assert_eq!(
        named("issue", 7, "Café ../../etc/passwd"),
        "7-caf-etc-passwd"
    );
    assert_eq!(named("issue", 7, "a\u{0}b\nc\u{202e}d"), "7-a-b-c-d");
    assert_eq!(named("issue", 7, "--force"), "7-force");
    // Nothing usable: the number alone, named.
    assert_eq!(named("issue", 9, "🚀 — ✨"), "issue-9");
    assert_eq!(named("issue", 9, ""), "issue-9");
    assert_eq!(named("pr", 12, "feature/Login"), "pr-12-feature-login");
    assert_eq!(named("pr", 12, "..."), "pr-12");
    // Long titles are cut at the limit, never ending with `-`.
    let long = named("issue", 123, &"word ".repeat(40));
    assert_eq!(long.len(), NAME_LIMIT - 1);
    assert!(long.starts_with("123-word-word-") && long.ends_with("-word"));
    let exact = named("issue", 1, &"a".repeat(100));
    assert_eq!(exact, format!("1-{}", "a".repeat(NAME_LIMIT - 2)));
    let fork = named("pr", 1, &"b".repeat(100));
    assert_eq!(fork, format!("pr-1-{}", "b".repeat(NAME_LIMIT - 5)));
    assert_eq!(words("A b", 3), "a-b");
    assert_eq!(words("A b", 2), "a");
    for title in ["x", "Fix it", &"y z ".repeat(50)] {
        worktree::validate_name(&named("issue", 5, title)).unwrap();
    }
}

#[test]
fn gh_answers_are_read_for_the_number_asked() {
    let pull = json!({
        "number": 12,
        "headRefName": "feature",
        "headRefOid": "a".repeat(40),
        "isCrossRepository": false,
    });
    let out = serde_json::to_vec(&pull).unwrap();
    let head = PullHead {
        branch: "feature".into(),
        commit: "a".repeat(40),
        fork: false,
    };
    assert_eq!(parse_pull(&out, 12), Ok(head));
    assert_eq!(
        parse_pull(&out, 13),
        Err("gh gave another pull request than #13".into())
    );
    // Unknown is taken for a fork.
    let fork = parse_pull(br#"{"number":3,"isCrossRepository":true}"#, 3).unwrap();
    assert_eq!((fork.fork, fork.branch.as_str()), (true, ""));
    assert!(parse_pull(br#"{"number":3}"#, 3).unwrap().fork);
    assert!(
        parse_pull(br#"{"number":3,"isCrossRepository":"no"}"#, 3)
            .unwrap()
            .fork
    );
    let err = parse_pull(b"nope", 3).unwrap_err();
    assert!(err.starts_with("gh gave no pull request: "), "{err}");
    let issue = br#"{"number":42,"title":"Fix it"}"#;
    assert_eq!(parse_issue(issue, 42), Ok("Fix it".into()));
    assert_eq!(
        parse_issue(issue, 4),
        Err("gh gave another issue than #4".into())
    );
    assert_eq!(parse_issue(br#"{"number":4}"#, 4), Ok(String::new()));
}

#[test]
fn the_remote_is_the_one_naming_the_repository() {
    let config = "remote.fork.url https://github.com/me/r\nremote.origin.pushurl x\nremote.origin.url git@github.com:o/r.git\nremote.up.url https://github.com/o/r\n";
    assert_eq!(named_remote(config, &repo()).as_deref(), Some("origin"));
    assert_eq!(
        named_remote("remote.fork.url https://github.com/me/r\n", &repo()),
        None
    );
    assert_eq!(
        named_remote("x.origin.url https://github.com/o/r\n", &repo()),
        None
    );
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q"]);
    assert_eq!(
        remote_name(tmp.path(), &repo()),
        Err("No remote of this repository is github.com/o/r".into())
    );
    git(
        tmp.path(),
        &["remote", "add", "gh", "https://github.com/o/r.git"],
    );
    assert_eq!(remote_name(tmp.path(), &repo()), Ok("gh".into()));
}

#[test]
fn branch_names_from_github_are_checked_by_git() {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q"]);
    let dir = tmp.path();
    let longest = "b".repeat(worktree::BRANCH_LIMIT);
    for good in ["feature", "feat/login-2", "é", longest.as_str()] {
        worktree::check_branch(dir, good).unwrap();
    }
    for bad in [
        "", "-x", "--help", "a b", "a..b", "x.lock", "a:b", "HEAD", "@{-1}",
    ] {
        let err = worktree::check_branch(dir, bad).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("{bad:?} is not a branch name git accepts")
        );
    }
    // Too long: refused without being repeated.
    let err = worktree::check_branch(dir, &format!("{longest}b")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "A branch name over 255 bytes is not accepted"
    );
    assert!(!worktree::has_branch(dir, "main"));
    git(dir, &["commit", "-q", "--allow-empty", "-m", "x"]);
    git(dir, &["branch", "-M", "main"]);
    assert!(worktree::has_branch(dir, "main"));
    assert!(!worktree::has_branch(dir, "mai"));
    // Git accepts `@{-1}` once a branch was checked out before, as that branch's name.
    git(dir, &["checkout", "-q", "-b", "other"]);
    git(dir, &["checkout", "-q", "main"]);
    let out = git(dir, &["check-ref-format", "--branch", "@{-1}"]);
    assert_eq!(out, "other\n");
    let err = worktree::check_branch(dir, "@{-1}").unwrap_err();
    assert_eq!(
        err.to_string(),
        r#""@{-1}" is not a branch name git accepts"#
    );
}

#[test]
fn fetches_carry_the_space_identity_without_a_token_gh_cannot_give() {
    let tmp = tempfile::tempdir().unwrap();
    let gh = Gh {
        program: tmp.path().join("no-gh").into(),
        path: tmp.path().into(),
    };
    let mut env = SpaceEnv {
        git_name: Some("Me".into()),
        ..SpaceEnv::default()
    };
    let identity = vec![
        ("GIT_AUTHOR_NAME", "Me".to_owned()),
        ("GIT_COMMITTER_NAME", "Me".to_owned()),
    ];
    assert_eq!(fetch_vars(&gh, &env), identity);
    env.gh_account = Some(hive_protocol::GhAccount {
        host: "github.com".into(),
        login: "me".into(),
    });
    assert_eq!(fetch_vars(&gh, &env), identity);
}

#[cfg(unix)]
#[test]
fn a_failed_create_on_an_existing_branch_keeps_the_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(dir.join(".worktreeinclude"), ".env\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "x"]);
    git(dir, &["branch", "kept"]);
    let env = dir.join(".env");
    std::fs::write(&env, "SECRET=1").unwrap();
    std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o000)).unwrap();
    let made = worktree::create_on(dir, "kept", Head::Existing("kept"));
    std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o600)).unwrap();
    let err = made.unwrap_err();
    assert!(err.to_string().contains("Permission denied"), "{err}");
    assert!(!dir.join(".claude/worktrees/kept").exists());
    assert!(worktree::has_branch(dir, "kept"));
}

/// A followed repository whose remote `origin` names github.com/o/r but is fetched from a
/// bare repository beside it (`url.<bare>.insteadOf`), and a fake `gh` that logs each call's
/// arguments and answers `pr view` / `issue view` from `pr-<n>.json` / `issue-<n>.json`.
#[cfg(unix)]
struct Setup {
    tmp: tempfile::TempDir,
    projects: Projects,
    id: String,
    gh: Gh,
    cache: Mutex<pulls::Cache>,
    /// The remote's `feature` and `refs/pull/14/head`: a commit the repository lacks.
    remote_commit: String,
}

#[cfg(unix)]
fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let other = tmp.path().join("other");
    let bare = tmp.path().join("bare.git");
    for dir in [&root, &other] {
        std::fs::create_dir(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
    }
    git(&root, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git(&other, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
    let remote_commit = git(&other, &["rev-parse", "HEAD"]).trim().to_owned();
    git(tmp.path(), &["init", "-q", "--bare", "bare.git"]);
    let to = bare.to_str().unwrap();
    let refs = [
        "main:refs/heads/feature",
        "main:refs/pull/14/head",
        "main:refs/pull/15/head",
    ];
    git(&other, &[&["push", "-q", to][..], &refs].concat());
    git(&root, &["push", "-q", to, "main:refs/heads/main"]);
    git(
        &root,
        &["remote", "add", "origin", "https://github.com/o/r"],
    );
    let instead = format!("url.{to}.insteadOf");
    git(&root, &["config", &instead, "https://github.com/o/r"]);
    let data = tmp.path().join("data");
    let projects = Projects::load(data.join("spaces.json"), &data.join("projects.json"));
    let id = projects.add(&root.display().to_string()).unwrap().id;
    let dir = tmp.path().display();
    let script = format!(
        r#"#!/bin/sh
for a in "$@"; do printf '%s\037' "$a" >> '{dir}/gh.log'; done
printf '\036' >> '{dir}/gh.log'
[ "$3" = 13 ] && {{ echo 'GraphQL: Could not resolve to a PullRequest' >&2; exit 1; }}
case "$1 $2" in
  "pr view") cat '{dir}/pr-'"$3"'.json' ;;
  "issue view") cat '{dir}/issue-'"$3"'.json' ;;
  *) echo 'not faked' >&2; exit 1 ;;
esac
"#
    );
    let program = tmp.path().join("gh");
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let gh = Gh {
        program: program.into(),
        path: "/usr/bin:/bin".into(),
    };
    Setup {
        tmp,
        projects,
        id,
        gh,
        cache: Mutex::default(),
        remote_commit,
    }
}

#[cfg(unix)]
impl Setup {
    fn root(&self) -> &Path {
        Path::new(&self.id)
    }

    fn pull(&self, number: u64, branch: &str, commit: &str, fork: bool) {
        let pull = json!({
            "number": number,
            "headRefName": branch,
            "headRefOid": commit,
            "isCrossRepository": fork,
        });
        let file = self.tmp.path().join(format!("pr-{number}.json"));
        std::fs::write(file, pull.to_string()).unwrap();
    }

    fn issue(&self, number: u64, title: &str) {
        let issue = json!({ "number": number, "title": title });
        let file = self.tmp.path().join(format!("issue-{number}.json"));
        std::fs::write(file, issue.to_string()).unwrap();
    }

    fn checkout(&self, number: u64) -> Vec<Control> {
        let request = Control::ActOnPull {
            project: self.id.clone(),
            number,
            action: PullAction::Checkout,
        };
        pulls::answer(&self.gh, &self.projects, &self.cache, request)
    }

    fn for_issue(&self, number: u64, base: Option<&str>) -> Control {
        let request = Control::CreateIssueWorktree {
            project: self.id.clone(),
            number,
            base: base.map(Into::into),
        };
        let mut replies = pulls::answer(&self.gh, &self.projects, &self.cache, request);
        assert_eq!(replies.len(), 1, "{replies:?}");
        replies.remove(0)
    }

    /// Each `gh` call so far, as its arguments.
    fn calls(&self) -> Vec<Vec<String>> {
        let log = std::fs::read_to_string(self.tmp.path().join("gh.log")).unwrap_or_default();
        let calls = log.split_terminator('\u{1e}');
        calls
            .map(|c| c.split_terminator('\u{1f}').map(Into::into).collect())
            .collect()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root().join(".claude/worktrees").join(name)
    }

    /// The branch and commit checked out at worktree `name`.
    fn head(&self, name: &str) -> (String, String) {
        let path = self.path(name);
        let branch = git(&path, &["branch", "--show-current"]);
        let commit = git(&path, &["rev-parse", "HEAD"]);
        (branch.trim().to_owned(), commit.trim().to_owned())
    }

    fn branches(&self) -> String {
        git(self.root(), &["branch", "--format=%(refname:short)"])
    }
}

/// The `worktree_created` among `replies`: its project's new worktree branch, path, notes.
#[cfg(unix)]
fn created(replies: &[Control]) -> (Option<String>, String, Vec<String>) {
    let Some(Control::WorktreeCreated {
        project,
        path,
        notes,
    }) = replies.first()
    else {
        panic!("{replies:?}")
    };
    let new = project.worktrees.iter().find(|w| w.path == *path).unwrap();
    (new.branch.clone(), path.clone(), notes.clone())
}

#[cfg(unix)]
fn failed(replies: &[Control]) -> String {
    let Some(Control::PullFailed { message, .. }) = replies.first() else {
        panic!("{replies:?}")
    };
    message.clone()
}

#[cfg(unix)]
#[test]
fn a_pull_request_of_the_repository_is_checked_out_on_its_branch() {
    let setup = setup();
    setup.pull(12, "feature", &"0".repeat(40), false);
    let replies = setup.checkout(12);
    let (branch, path, notes) = created(&replies);
    assert_eq!(branch.as_deref(), Some("feature"));
    assert_eq!(Path::new(&path), setup.path("pr-12"));
    assert_eq!(notes, Vec::<String>::new());
    // Then the list again.
    assert!(matches!(replies[1], Control::Pulls { .. }), "{replies:?}");
    assert_eq!(replies.len(), 2);
    let pr = "number,headRefName,headRefOid,isCrossRepository";
    let view = ["pr", "view", "12", "--repo", "github.com/o/r", "--json", pr];
    assert_eq!(setup.calls()[0], view);
    // On the remote's commit, tracking it.
    assert_eq!(setup.head("pr-12").1, setup.remote_commit);
    let upstream = git(&setup.path("pr-12"), &["rev-parse", "--abbrev-ref", "@{u}"]);
    assert_eq!(upstream, "origin/feature\n");
    // The worktree flow's own refusal for a second checkout; the branch is untouched.
    let message = failed(&setup.checkout(12));
    assert!(
        message.starts_with(r#"worktree "pr-12" already exists"#),
        "{message}"
    );
    assert_eq!(setup.branches(), "feature\nmain\n");
}

#[cfg(unix)]
#[test]
fn a_fork_pull_request_comes_from_the_base_repository_onto_its_own_branch() {
    let setup = setup();
    let main = git(setup.root(), &["rev-parse", "main"]);
    // The fork's `main` never touches the local `main`.
    setup.pull(14, "main", &setup.remote_commit, true);
    let (branch, _, notes) = created(&setup.checkout(14));
    assert_eq!(branch.as_deref(), Some("pr-14-main"));
    assert!(notes.is_empty());
    assert_eq!(setup.head("pr-14").1, setup.remote_commit);
    assert_eq!(git(setup.root(), &["rev-parse", "main"]), main);
    // No remote added for the fork.
    assert_eq!(git(setup.root(), &["remote"]), "origin\n");
    // A branch already there is checked out as it is, and said so.
    git(setup.root(), &["branch", "pr-15-fix", "main"]);
    setup.pull(15, "fix", &setup.remote_commit, true);
    let (branch, _, notes) = created(&setup.checkout(15));
    assert_eq!(branch.as_deref(), Some("pr-15-fix"));
    assert_eq!(
        notes,
        ["branch pr-15-fix already existed: checked out as it is"]
    );
    assert_eq!(setup.head("pr-15").1, main.trim());
    // A head that is not a commit: nothing fetched or made.
    setup.pull(16, "x", "zz", true);
    let message = failed(&setup.checkout(16));
    assert_eq!(message, "The pull request's head is not a commit");
    assert!(!setup.path("pr-16").exists());
}

#[cfg(unix)]
#[test]
fn a_refused_pull_request_leaves_nothing_behind() {
    let setup = setup();
    // gh's own refusal.
    let message = failed(&setup.checkout(13));
    assert_eq!(
        message,
        "gh pr view failed: GraphQL: Could not resolve to a PullRequest"
    );
    // A branch git rejects: nothing fetched.
    for (number, branch) in [(20, "a..b"), (21, "@{-1}"), (22, "-x")] {
        setup.pull(number, branch, &"0".repeat(40), false);
        let message = failed(&setup.checkout(number));
        assert_eq!(
            message,
            format!("{branch:?} is not a branch name git accepts")
        );
    }
    // A branch gone from GitHub: git's message.
    setup.pull(23, "gone", &"0".repeat(40), false);
    let message = failed(&setup.checkout(23));
    assert!(
        message.starts_with(
            "git fetch --no-tags -- origin +refs/heads/gone:refs/remotes/origin/gone failed: "
        ),
        "{message}"
    );
    // Not a GitHub answer.
    std::fs::write(setup.tmp.path().join("pr-24.json"), "{}").unwrap();
    assert_eq!(
        failed(&setup.checkout(24)),
        "gh gave another pull request than #24"
    );
    for name in ["pr-20", "pr-21", "pr-22", "pr-23", "pr-24"] {
        assert!(!setup.path(name).exists(), "{name}");
    }
    assert_eq!(setup.branches(), "main\n");
}

#[cfg(unix)]
#[test]
fn an_issue_gets_a_worktree_on_a_branch_named_from_its_number_and_title() {
    let setup = setup();
    setup.issue(42, "Fix: the login crash! 🚀");
    let reply = setup.for_issue(42, None);
    let (branch, path, notes) = created(std::slice::from_ref(&reply));
    assert_eq!(branch.as_deref(), Some("42-fix-the-login-crash"));
    assert_eq!(Path::new(&path), setup.path("42-fix-the-login-crash"));
    assert!(notes.is_empty());
    let main = git(setup.root(), &["rev-parse", "main"]);
    assert_eq!(setup.head("42-fix-the-login-crash").1, main.trim());
    let view = [
        "issue",
        "view",
        "42",
        "--repo",
        "github.com/o/r",
        "--json",
        "number,title",
    ];
    assert_eq!(setup.calls()[0], view);
    // From a remote base: fetched first.
    setup.issue(7, "");
    let reply = setup.for_issue(7, Some("origin/feature"));
    assert_eq!(created(&[reply]).0.as_deref(), Some("issue-7"));
    assert_eq!(setup.head("issue-7").1, setup.remote_commit);
    // A base that cannot be fetched is a note.
    git(setup.root(), &["remote", "add", "gone", "/nowhere"]);
    git(setup.root(), &["update-ref", "refs/remotes/gone/x", "main"]);
    setup.issue(8, "eight");
    let (_, _, notes) = created(&[setup.for_issue(8, Some("gone/x"))]);
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].starts_with("could not fetch gone/x: "),
        "{notes:?}"
    );
    // The branch kept from an earlier worktree is checked out as it is.
    git(setup.root(), &["branch", "9-nine", "main"]);
    setup.issue(9, "Nine");
    let (branch, _, notes) = created(&[setup.for_issue(9, Some("origin/feature"))]);
    assert_eq!(branch.as_deref(), Some("9-nine"));
    assert_eq!(
        notes,
        ["branch 9-nine already existed: checked out as it is"]
    );
    assert_eq!(setup.head("9-nine").1, main.trim());
}

#[cfg(unix)]
#[test]
fn a_refused_issue_is_answered_with_its_number() {
    let setup = setup();
    let fail = |reply: Control| {
        let Control::CreateWorktreeFailed {
            project,
            name,
            message,
        } = reply
        else {
            panic!("{reply:?}")
        };
        assert_eq!(project, setup.id);
        (name, message)
    };
    let (name, message) = fail(setup.for_issue(13, None));
    assert_eq!(name, "#13");
    assert_eq!(
        message,
        "gh issue view failed: GraphQL: Could not resolve to a PullRequest"
    );
    std::fs::write(setup.tmp.path().join("issue-5.json"), r#"{"number":6}"#).unwrap();
    let (_, message) = fail(setup.for_issue(5, None));
    assert_eq!(message, "gh gave another issue than #5");
    // The worktree flow's refusal: its folder taken.
    setup.issue(11, "x");
    std::fs::create_dir_all(setup.path("11-x")).unwrap();
    let (_, message) = fail(setup.for_issue(11, None));
    assert!(
        message.starts_with(r#"worktree "11-x" already exists"#),
        "{message}"
    );
    // Not a followed project.
    let request = Control::CreateIssueWorktree {
        project: "/nowhere".into(),
        number: 1,
        base: None,
    };
    let replies = pulls::answer(&setup.gh, &setup.projects, &setup.cache, request);
    let [Control::CreateWorktreeFailed { project, .. }] = &replies[..] else {
        panic!("{replies:?}")
    };
    assert_eq!(project, "/nowhere");
    assert_eq!(setup.branches(), "main\n");
}
