use std::process::Command;

use hive_protocol::Staged;

use super::*;

/// Git as the tests run it: no user or system config, a fixed identity.
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
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// What `commit` and `push` get in the tests: the space's identity, and no user config.
fn env() -> Vec<(&'static str, String)> {
    vec![
        ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
        ("GIT_AUTHOR_NAME", "Me".into()),
        ("GIT_AUTHOR_EMAIL", "me@x".into()),
        ("GIT_COMMITTER_NAME", "Me".into()),
        ("GIT_COMMITTER_EMAIL", "me@x".into()),
    ]
}

/// A repository on `main`, with a first commit when `committed`.
fn repo(committed: bool) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    if committed {
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "first"]);
    }
    (tmp, dir)
}

/// Each file's staged state as the Changes panel gets it.
fn staged(dir: &Path) -> Vec<(String, Option<Staged>)> {
    let listed = changes::list(dir, None).unwrap();
    listed
        .files
        .into_iter()
        .map(|f| (f.path, f.staged))
        .collect()
}

fn paths(names: &[&str]) -> Vec<String> {
    names.iter().map(|&n| n.to_owned()).collect()
}

#[test]
fn files_are_staged_and_unstaged_by_name_only() {
    let (_tmp, dir) = repo(true);
    std::fs::write(dir.join("a.txt"), "b\n").unwrap();
    std::fs::write(dir.join("*.txt"), "star\n").unwrap();
    std::fs::write(dir.join("c.txt"), "c\n").unwrap();
    // `*.txt` is that file only: `c.txt` stays unstaged.
    let done = stage(&dir, &paths(&["*.txt", "a.txt"]), true).unwrap();
    assert_eq!(done, "Staged 2 files");
    assert_eq!(
        staged(&dir),
        [
            ("*.txt".into(), Some(Staged::Yes)),
            ("a.txt".into(), Some(Staged::Yes)),
            ("c.txt".into(), Some(Staged::No)),
        ]
    );
    // Changed again: partly staged.
    std::fs::write(dir.join("a.txt"), "c\n").unwrap();
    assert_eq!(staged(&dir)[1], ("a.txt".into(), Some(Staged::Partly)));
    assert_eq!(
        stage(&dir, &paths(&["a.txt"]), false).unwrap(),
        "Unstaged a.txt"
    );
    assert_eq!(staged(&dir)[1], ("a.txt".into(), Some(Staged::No)));
    // Git's own refusal, as is.
    let err = stage(&dir, &paths(&["nope.txt"]), true).unwrap_err();
    assert!(err.contains("nope.txt"), "{err}");
    // Git's message only: the command is not named.
    assert!(!err.starts_with("git"), "{err}");
}

#[test]
fn a_staged_rename_is_unstaged_whole() {
    let (_tmp, dir) = repo(true);
    std::fs::write(dir.join("c.txt"), "c\n").unwrap();
    git(&dir, &["add", "c.txt"]);
    git(&dir, &["commit", "-q", "-m", "c"]);
    git(&dir, &["mv", "a.txt", "b.txt"]);
    git(&dir, &["mv", "c.txt", "d.txt"]);
    stage(&dir, &paths(&["b.txt"]), false).unwrap();
    // The old name is back in the index, the new one untracked; the other rename stays.
    let status = git(&dir, &["status", "--porcelain"]);
    assert_eq!(status, " D a.txt\nR  c.txt -> d.txt\n?? b.txt\n");
}

#[test]
fn files_are_unstaged_before_the_first_commit() {
    let (_tmp, dir) = repo(false);
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    stage(&dir, &paths(&["new.txt"]), true).unwrap();
    assert_eq!(staged(&dir), [("new.txt".into(), Some(Staged::Yes))]);
    stage(&dir, &paths(&["new.txt"]), false).unwrap();
    assert_eq!(staged(&dir), [("new.txt".into(), Some(Staged::No))]);
}

#[test]
fn paths_from_the_app_are_checked() {
    let (_tmp, dir) = repo(true);
    let refused = |names: Vec<String>| stage(&dir, &names, true).unwrap_err();
    let between = format!("Between 1 and {PATHS_LIMIT} files can be staged at once");
    assert_eq!(refused(Vec::new()), between);
    assert_eq!(refused(vec!["a".into(); PATHS_LIMIT + 1]), between);
    for bad in ["", "/etc/passwd", "../x", "a/../../x", "a\\..\\x", "a\0b"] {
        let err = refused(paths(&[bad]));
        assert_eq!(err, format!("{bad:?} is not a file of this worktree"));
    }
    assert!(refused(vec!["a".repeat(PATH_LIMIT + 1)]).ends_with("is not a file of this worktree"));
    // Just under the limits is accepted (git then says what it finds).
    assert!(check_paths(&vec!["a".repeat(PATH_LIMIT); PATHS_LIMIT]).is_ok());
}

#[test]
fn a_commit_takes_what_is_staged_with_the_spaces_identity() {
    let (_tmp, dir) = repo(true);
    std::fs::write(dir.join("a.txt"), "b\n").unwrap();
    std::fs::write(dir.join("left.txt"), "l\n").unwrap();
    // Nothing staged: git says so (on its stdout).
    let err = commit(&dir, "m", &env()).unwrap_err();
    assert!(err.contains("no changes added to commit"), "{err}");
    stage(&dir, &paths(&["a.txt"]), true).unwrap();
    let message = "-starts with a dash\n\nand a body";
    let done = commit(&dir, message, &env()).unwrap();
    let head = git(&dir, &["log", "-1", "--format=%h %an <%ae> %cn %B"]);
    let (short, rest) = head.split_once(' ').unwrap();
    assert_eq!(rest, "Me <me@x> Me -starts with a dash\n\nand a body\n\n");
    assert_eq!(done, format!("Committed {short}: -starts with a dash"));
    // Only the staged file went in.
    assert_eq!(staged(&dir), [("left.txt".into(), Some(Staged::No))]);
}

#[test]
fn a_commit_message_is_checked() {
    let (_tmp, dir) = repo(true);
    assert_eq!(
        commit(&dir, " \n ", &env()).unwrap_err(),
        "A commit message is needed"
    );
    // At the limit it is a message.
    let most = "m".repeat(MESSAGE_LIMIT);
    let err = commit(&dir, &most, &env()).unwrap_err();
    assert!(err.contains("nothing to commit"), "{err}");
    let long = "m".repeat(MESSAGE_LIMIT + 1);
    assert_eq!(
        commit(&dir, &long, &env()).unwrap_err(),
        "The commit message is over 64 KiB"
    );
    assert_eq!(
        commit(&dir, "a\0b", &env()).unwrap_err(),
        "The commit message holds a NUL character"
    );
}

#[cfg(unix)]
#[test]
fn the_repositorys_hooks_run_and_their_output_is_the_error() {
    let (_tmp, dir) = repo(true);
    let hook = dir.join(".git/hooks/pre-commit");
    executable(
        &hook,
        "#!/bin/sh\necho 'lint: 2 problems'\necho 'hook says no' >&2\nexit 1\n",
    );
    std::fs::write(dir.join("a.txt"), "b\n").unwrap();
    stage(&dir, &paths(&["a.txt"]), true).unwrap();
    let err = commit(&dir, "m", &env()).unwrap_err();
    assert_eq!(err, "lint: 2 problems\nhook says no");
    assert_eq!(git(&dir, &["rev-list", "--count", "HEAD"]), "1\n");
}

/// A repository with a bare `origin` beside it (never a real remote).
fn with_origin() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (tmp, dir) = repo(true);
    let bare = tmp.path().join("origin.git");
    git(tmp.path(), &["init", "-q", "--bare", "origin.git"]);
    git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    (tmp, dir, bare)
}

#[test]
fn a_branch_without_upstream_goes_to_origin_and_gets_it() {
    let (_tmp, dir, bare) = with_origin();
    let found = target(&dir).unwrap();
    assert_eq!(
        found,
        Target {
            branch: "main".into(),
            remote: "origin".into(),
            merge: "refs/heads/main".into(),
            set_upstream: true,
            ahead: None,
        }
    );
    assert_eq!(found.shown(), "origin/main");
    let done = push(&dir, &env(), ("main", "origin/main")).unwrap();
    assert_eq!(done, "Pushed main to origin/main, now its upstream");
    assert_eq!(
        git(&dir, &["config", "branch.main.merge"]),
        "refs/heads/main\n"
    );
    assert_eq!(
        git(&bare, &["rev-parse", "main"]),
        git(&dir, &["rev-parse", "HEAD"])
    );
    // Now it has one: a new commit is 1 ahead of it, and goes there.
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "second"]);
    let found = target(&dir).unwrap();
    assert_eq!((found.set_upstream, found.ahead), (false, Some(1)));
    assert_eq!(
        push(&dir, &env(), ("main", "origin/main")).unwrap(),
        "Pushed main to origin/main"
    );
    assert_eq!(target(&dir).unwrap().ahead, Some(0));
}

#[test]
fn a_push_goes_to_the_upstreams_own_branch() {
    let (_tmp, dir, bare) = with_origin();
    git(&dir, &["switch", "-q", "-c", "local"]);
    git(
        &dir,
        &["push", "-q", "origin", "local:refs/heads/remote-name"],
    );
    git(
        &dir,
        &["branch", "-q", "--set-upstream-to=origin/remote-name"],
    );
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "x"]);
    let found = target(&dir).unwrap();
    assert_eq!(found.shown(), "origin/remote-name");
    assert_eq!(found.ahead, Some(1));
    // Its upstream changed since it was shown: refused, nothing pushed.
    let changed = push(&dir, &env(), ("local", "origin/other")).unwrap_err();
    assert_eq!(
        changed,
        "local → origin/other changed since it was shown (now local → origin/remote-name): check again"
    );
    push(&dir, &env(), ("local", "origin/remote-name")).unwrap();
    assert_eq!(
        git(&bare, &["rev-parse", "remote-name"]),
        git(&dir, &["rev-parse", "HEAD"])
    );
}

#[test]
fn a_rejected_push_is_gits_message_and_never_forced() {
    let (tmp, dir, bare) = with_origin();
    push(&dir, &env(), ("main", "origin/main")).unwrap();
    // Someone else pushed meanwhile.
    let other = tmp.path().join("other");
    git(
        tmp.path(),
        &["clone", "-q", "-b", "main", bare.to_str().unwrap(), "other"],
    );
    git(&other, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
    git(&other, &["push", "-q", "origin", "main"]);
    let theirs = git(&bare, &["rev-parse", "main"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "mine"]);
    let err = push(&dir, &env(), ("main", "origin/main")).unwrap_err();
    assert!(err.contains("[rejected]"), "{err}");
    assert!(!err.starts_with("git push"), "{err}");
    assert_eq!(git(&bare, &["rev-parse", "main"]), theirs);
}

#[test]
fn no_push_command_is_ever_forced() {
    for set_upstream in [false, true] {
        // Even a branch or a remote branch that looks like an option or a force.
        let target = Target {
            branch: "+main".into(),
            remote: "origin".into(),
            merge: "refs/heads/+main".into(),
            set_upstream,
            ahead: None,
        };
        let args = push_args(&target);
        let force = |arg: &&String| {
            arg.starts_with('+')
                || arg.starts_with("--force")
                || arg.starts_with("--mirror")
                || arg.starts_with("--delete")
                || arg.starts_with("--no-verify")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('f'))
        };
        assert_eq!(args.iter().find(force), None, "{args:?}");
        let upstream = ["--set-upstream"].map(String::from);
        let want = [
            &["push".to_owned()][..],
            if set_upstream { &upstream[..] } else { &[] },
            &["--", "origin", "refs/heads/+main:refs/heads/+main"].map(String::from),
        ]
        .concat();
        assert_eq!(args, want);
    }
    // The source holds no force option at all.
    let source = include_str!("../commit.rs");
    for flag in ["--force", "force-with-lease", "--no-verify", "\"-f\""] {
        assert!(!source.contains(flag), "{flag}");
    }
}

#[test]
fn a_push_is_refused_where_it_cannot_go() {
    let (_tmp, dir) = repo(true);
    assert_eq!(
        target(&dir).unwrap_err(),
        "main has no upstream and this repository has no remote origin"
    );
    let head = git(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["switch", "-q", "--detach", head.trim()]);
    let detached = "This worktree is on no branch (a detached HEAD): nothing to push";
    assert_eq!(target(&dir).unwrap_err(), detached);
    assert_eq!(
        push(&dir, &env(), ("main", "origin/main")).unwrap_err(),
        detached
    );
    git(&dir, &["switch", "-q", "main"]);
    git(&dir, &["config", "branch.main.remote", "."]);
    git(&dir, &["config", "branch.main.merge", "refs/heads/other"]);
    assert_eq!(
        target(&dir).unwrap_err(),
        "The upstream of main is a local branch: push it in a terminal"
    );
    git(&dir, &["config", "branch.main.remote", "up"]);
    git(&dir, &["config", "branch.main.merge", "refs/tags/v1"]);
    assert_eq!(
        target(&dir).unwrap_err(),
        "The upstream of main is not a branch of up"
    );
    // An upstream half set (a remote, no branch): none, so origin's.
    git(&dir, &["remote", "add", "origin", "/nowhere"]);
    git(&dir, &["config", "branch.main.remote", "origin"]);
    git(&dir, &["config", "--unset", "branch.main.merge"]);
    assert!(target(&dir).unwrap().set_upstream);
    git(&dir, &["config", "--unset", "branch.main.remote"]);
    git(&dir, &["config", "branch.main.merge", "refs/heads/main"]);
    assert!(target(&dir).unwrap().set_upstream);
    git(&dir, &["config", "branch.main.remote", "-x"]);
    git(&dir, &["config", "branch.main.merge", "refs/heads/main"]);
    assert_eq!(
        target(&dir).unwrap_err(),
        "\"-x\" is not a remote git accepts"
    );
}

#[test]
fn gits_message_loses_only_the_command() {
    let failed = io::Error::other("git push failed: ! [rejected]");
    assert_eq!(why("push", &failed), "! [rejected]");
    let slow = io::Error::other("git push took longer than 300 s");
    assert_eq!(why("push", &slow), "git push took longer than 300 s");
    let long = io::Error::other(format!("git add failed: {}", "x".repeat(ERROR_LIMIT + 9)));
    assert!(why("add", &long).ends_with('…'));
}

#[cfg(unix)]
#[test]
fn a_push_gets_the_spaces_identity_and_its_accounts_token() {
    let tmp = tempfile::tempdir().unwrap();
    // A fake gh answering `gh auth token` (never the real one).
    let program = tmp.path().join("gh");
    executable(&program, "#!/bin/sh\necho tok-123\n");
    let gh = Gh {
        program: program.into(),
        path: "/usr/bin:/bin".into(),
    };
    let account = hive_protocol::SpaceGhAccount {
        account: hive_protocol::GhAccount {
            host: "github.com".into(),
            login: "me".into(),
        },
        config_dir: None,
    };
    let space = SpaceEnv {
        git_name: Some("Me".into()),
        gh_account: Some(account),
        ..SpaceEnv::default()
    };
    let vars = remote_env(&gh, &space);
    assert!(vars.contains(&("GIT_AUTHOR_NAME", "Me".into())), "{vars:?}");
    assert!(vars.contains(&("GH_TOKEN", "tok-123".into())), "{vars:?}");
    assert!(vars.contains(&("GH_HOST", "github.com".into())), "{vars:?}");
    // No account: the identity only.
    let plain = SpaceEnv {
        git_name: Some("Me".into()),
        ..SpaceEnv::default()
    };
    assert_eq!(
        remote_env(&gh, &plain),
        [
            ("GIT_AUTHOR_NAME", "Me".to_owned()),
            ("GIT_COMMITTER_NAME", "Me".to_owned())
        ]
    );
}

#[test]
fn the_service_answers_only_its_git_requests_and_never_for_a_folder_project() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("notes");
    std::fs::create_dir(&folder).unwrap();
    let data = tmp.path().join("data");
    let projects = Projects::load(data.join("spaces.json"), &data.join("projects.json"));
    let id = projects.add(&folder.display().to_string()).unwrap().id;
    let gh = Gh {
        program: "/nonexistent/gh".into(),
        path: "/usr/bin:/bin".into(),
    };
    let unstage = Control::StageFiles {
        worktree: id.clone(),
        paths: vec!["a".into()],
        staged: false,
    };
    assert_eq!(
        answer(&gh, &projects, unstage),
        [Control::GitFailed {
            worktree: id.clone(),
            action: GitAction::Unstage,
            message: format!("{id} is not a worktree of a followed project"),
        }]
    );
    assert_eq!(answer(&gh, &projects, Control::ListProjects), []);
    // A repository's worktree: each request answers as its function does.
    let (_repo, dir) = repo(true);
    let root = projects.add(&dir.display().to_string()).unwrap().id;
    std::fs::write(Path::new(&root).join("a.txt"), "b\n").unwrap();
    let ask = |request| answer(&gh, &projects, request);
    let stage = Control::StageFiles {
        worktree: root.clone(),
        paths: vec!["a.txt".into()],
        staged: true,
    };
    assert_eq!(
        ask(stage),
        [Control::GitDone {
            worktree: root.clone(),
            action: GitAction::Stage,
            message: "Staged a.txt".into(),
        }]
    );
    let commit = Control::Commit {
        worktree: root.clone(),
        message: " ".into(),
    };
    assert_eq!(
        ask(commit),
        [Control::GitFailed {
            worktree: root.clone(),
            action: GitAction::Commit,
            message: "A commit message is needed".into(),
        }]
    );
    let none = "main has no upstream and this repository has no remote origin";
    let target = Control::GetPushTarget {
        worktree: root.clone(),
    };
    assert_eq!(
        ask(target),
        [Control::PushTarget {
            worktree: root.clone(),
            branch: None,
            target: None,
            set_upstream: false,
            ahead: None,
            error: Some(none.into()),
        }]
    );
    let push = Control::Push {
        worktree: root.clone(),
        branch: "main".into(),
        target: "origin/main".into(),
    };
    assert_eq!(
        ask(push),
        [Control::GitFailed {
            worktree: root.clone(),
            action: GitAction::Push,
            message: none.into(),
        }]
    );
}

/// An executable file written by a `sh` of its own: a file this process writes can be held
/// open by a child another test thread forks meanwhile, and running it then fails with
/// "Text file busy" (Linux).
#[cfg(unix)]
fn executable(path: &Path, body: &str) {
    let written = std::process::Command::new("sh")
        .args(["-c", r#"printf '%s' "$1" > "$2" && chmod 755 "$2""#, "sh"])
        .arg(body)
        .arg(path)
        .status()
        .unwrap();
    assert!(written.success());
}
