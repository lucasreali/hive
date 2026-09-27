use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use hive_protocol::GhAccount;
use serde_json::json;

use super::*;

const SEARCH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/pr-search.json"
);
const VIEW: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/pr-view.json"
);
const MISSING: &str = include_str!("../../tests/fixtures/gh/pr-search-missing.json");
const HEAD: &str = "4a75987727d9a40c58546914e9fd74e6485b9443";

fn worktree(id: &str, branch: &str) -> Worktree {
    Worktree {
        id: id.into(),
        name: branch.into(),
        path: id.into(),
        branch: Some(branch.into()),
        main: false,
        claude: false,
        status: None,
    }
}

#[test]
fn remotes_name_the_repository_gh_would_use() {
    let repo = |owner: &str, name: &str| Repo {
        host: "github.com".into(),
        owner: owner.into(),
        name: name.into(),
    };
    for url in [
        "https://github.com/o/r.git",
        "https://user@GitHub.com/o/r/",
        "http://github.com/o/r",
        "ssh://git@github.com:22/o/r.git",
        "git://github.com/o/r",
        "git+ssh://git@github.com/o/r",
        "ssh+git://github.com/o/r",
        "git@github.com:o/r.git",
    ] {
        assert_eq!(parse_url(url, "github.com"), Some(repo("o", "r")), "{url}");
    }
    assert_eq!(
        parse_url("git@github.com:o/.github", "github.com"),
        Some(repo("o", ".github"))
    );
    let long = format!("git@github.com:o/{}", "r".repeat(100));
    assert!(parse_url(&long, "github.com").is_some());
    let longer = format!("git@github.com:o/{}", "r".repeat(101));
    for url in [
        "https://gitlab.com/o/r",
        "https://github.com.evil/o/r",
        "file://github.com/o/r",
        "/home/me/r",
        "git@github.com:o",
        "git@github.com:o/r/x",
        "git@github.com:-o/r",
        "git@github.com:o/..",
        "git@github.com:o/.",
        "git@github.com:/r",
        "git@github.com:o/r;x",
        "https://github.com",
        longer.as_str(),
    ] {
        assert_eq!(parse_url(url, "github.com"), None, "{url}");
    }
    let enterprise = parse_url("https://ghe.example/o/r", "ghe.example").unwrap();
    assert_eq!(enterprise.arg(), "ghe.example/o/r");

    // gh's order: the remote `gh repo set-default` chose, upstream, github, origin, the first.
    let config = "remote.fork.url https://github.com/me/r\nremote.origin.url https://github.com/o/origin\nremote.github.url https://github.com/o/github\nremote.upstream.url https://github.com/o/upstream\nremote.other.url https://gitlab.com/o/x\nbranch.x.remote origin\n";
    let pick = |config: &str| pick_remote(config, "github.com").map(|r| r.name);
    assert_eq!(pick(config).as_deref(), Some("upstream"));
    let chosen = format!("{config}remote.fork.gh-resolved base\nremote.origin.gh-resolved other\n");
    assert_eq!(pick(&chosen).as_deref(), Some("r"));
    let lines: Vec<&str> = config.lines().collect();
    assert_eq!(pick(&lines[..3].join("\n")).as_deref(), Some("github"));
    assert_eq!(pick(&lines[..2].join("\n")).as_deref(), Some("origin"));
    assert_eq!(pick(lines[0]).as_deref(), Some("r"));
    assert_eq!(pick("remote.other.url https://gitlab.com/o/x\nnoise"), None);
}

#[test]
fn the_recorded_list_gives_each_pull_request_its_state_checks_and_worktree() {
    let out = std::fs::read(SEARCH).unwrap();
    let worktrees = [
        worktree("/r/a", "williammartin-ghes-api-telemetry"),
        // The fork's pull request has this branch name too: not this worktree's.
        worktree("/r/b", "document-search-operator-support"),
    ];
    let (repo, mine, review) = parse_list(&out, &worktrees).unwrap();
    assert_eq!(
        repo,
        PullRepo {
            name: "cli/cli".into(),
            url: "https://github.com/cli/cli".into(),
            merge_methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
            default_merge: Some(MergeMethod::Merge),
        }
    );
    assert_eq!(mine.len(), 3);
    assert_eq!(
        mine[0],
        PullSummary {
            number: 14337,
            title: "Disable telemetry for unauthenticated GHES `gh api` requests using absolute hostnames".into(),
            url: "https://github.com/cli/cli/pull/14337".into(),
            state: PullState::Merged,
            branch: "williammartin-ghes-api-telemetry".into(),
            base: "trunk".into(),
            author: "octo-1".into(),
            review: Some(ReviewDecision::Approved),
            checks: Some(CheckState::Passing),
            updated_at: "2026-09-25T08:18:47Z".into(),
            worktree: Some("/r/a".into()),
        }
    );
    assert_eq!(mine[1].worktree, None);
    let first = &review[0];
    assert_eq!(
        (
            first.number,
            first.state,
            first.review,
            first.worktree.as_deref()
        ),
        (
            14519,
            PullState::Open,
            Some(ReviewDecision::ChangesRequested),
            None
        )
    );
}

#[test]
fn a_list_without_its_repository_is_an_error() {
    assert_eq!(
        parse_list(MISSING.as_bytes(), &[]).unwrap_err(),
        "gh gave no repository"
    );
    let err = parse_list(b"not json", &[]).unwrap_err();
    assert!(err.starts_with("gh gave no list: "), "{err}");
    // Only what the repository allows, and no default GitHub does not name.
    let bare = json!({"data": {"repository": {"squashMergeAllowed": true, "rebaseMergeAllowed": "yes", "viewerDefaultMergeMethod": "OTHER"},
        "mine": {"nodes": [{}, {"number": 1, "state": "OPEN", "isDraft": true, "reviewDecision": "REVIEW_REQUIRED"}]}}});
    let (repo, mine, review) = parse_list(bare.to_string().as_bytes(), &[]).unwrap();
    assert_eq!(repo.merge_methods, [MergeMethod::Squash]);
    assert_eq!((repo.default_merge, repo.url.as_str()), (None, ""));
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].state, mine[0].review, mine[0].checks),
        (PullState::Draft, Some(ReviewDecision::ReviewRequired), None)
    );
    assert!(review.is_empty());
    for (method, name) in [
        (MergeMethod::Squash, "SQUASH"),
        (MergeMethod::Rebase, "REBASE"),
    ] {
        let answer = json!({"data": {"repository": {"viewerDefaultMergeMethod": name}}});
        let (repo, _, _) = parse_list(answer.to_string().as_bytes(), &[]).unwrap();
        assert_eq!(repo.default_merge, Some(method));
    }
}

#[test]
fn the_recorded_details_keep_reviews_and_comments_fit_to_show() {
    let out = std::fs::read(VIEW).unwrap();
    let pull = parse_view(&out, &[]).unwrap();
    assert_eq!(pull.summary.number, 14337);
    assert_eq!(pull.summary.checks, Some(CheckState::Passing));
    assert!(
        pull.body
            .starts_with("<!-- a template comment -->\n### Description\n")
    );
    assert_eq!(
        (
            pull.head.as_str(),
            pull.additions,
            pull.deletions,
            pull.conflicts
        ),
        (HEAD, 84, 47, false)
    );
    // The three comments were hidden as spam; the approval without a text stays.
    let notes: Vec<(&str, Option<ReviewState>, &str)> = pull
        .notes
        .iter()
        .map(|n| (n.body.as_str(), n.review, n.at.as_str()))
        .collect();
    assert_eq!(
        notes,
        [
            (
                "Text 1 with `code`.",
                Some(ReviewState::Commented),
                "2026-09-03T13:51:01Z"
            ),
            (
                "Text 2 with `code`.",
                Some(ReviewState::Approved),
                "2026-09-03T15:08:48Z"
            ),
            ("", Some(ReviewState::Approved), "2026-09-03T15:19:49Z"),
            (
                "Text 4 with `code`.",
                Some(ReviewState::Commented),
                "2026-09-25T08:13:42Z"
            ),
        ]
    );
    assert_eq!(pull.notes[0].author, "octo-9");
    assert_eq!(pull.checks.len(), 3);
    assert_eq!(
        pull.checks[1],
        PullCheck {
            name: "lint".into(),
            workflow: Some("Lint".into()),
            state: CheckState::Passing,
            url: Some(
                "https://github.com/cli/cli/actions/runs/33771463812/job/100702465720".into()
            ),
        }
    );
    assert_eq!(
        pull.files[0],
        PullFile {
            path: "pkg/cmd/api/api.go".into(),
            additions: 33,
            deletions: 31,
        }
    );
    assert_eq!(pull.files.len(), 4);
    assert!(
        parse_view(b"[]", &[])
            .unwrap_err()
            .starts_with("gh gave no pull request")
    );
    assert!(
        parse_view(b"{", &[])
            .unwrap_err()
            .starts_with("gh gave no pull request: ")
    );
}

#[test]
fn checks_and_statuses_have_one_state() {
    // Shapes from gh's own `statusCheckRollup` export: check runs and commit statuses.
    let run = |status: &str, conclusion: &str| {
        check(
            &json!({"__typename": "CheckRun", "name": "build", "status": status, "conclusion": conclusion, "detailsUrl": "javascript:x", "workflowName": ""}),
        )
    };
    assert_eq!(
        run("IN_PROGRESS", ""),
        PullCheck {
            name: "build".into(),
            workflow: None,
            state: CheckState::Running,
            url: None,
        }
    );
    assert_eq!(run("COMPLETED", "FAILURE").state, CheckState::Failing);
    assert_eq!(run("COMPLETED", "WHATEVER").state, CheckState::Skipped);
    let status = check(
        &json!({"__typename": "StatusContext", "context": "ci/x", "state": "PENDING", "targetUrl": "https://ci.example/1"}),
    );
    assert_eq!(
        status,
        PullCheck {
            name: "ci/x".into(),
            workflow: None,
            state: CheckState::Running,
            url: Some("https://ci.example/1".into()),
        }
    );
    for (state, expected) in [
        ("SUCCESS", Some(CheckState::Passing)),
        ("ERROR", Some(CheckState::Failing)),
        ("EXPECTED", Some(CheckState::Running)),
        ("NEUTRAL", Some(CheckState::Skipped)),
        ("", None),
    ] {
        assert_eq!(check_state(state), expected, "{state}");
    }
    // The details' overall state: a failing check wins, then a running one, then a passing one.
    let view = |states: &[(&str, &str)]| {
        let checks: Vec<Value> = states
            .iter()
            .map(|(s, c)| json!({"__typename": "CheckRun", "status": s, "conclusion": c}))
            .collect();
        let view = json!({"number": 1, "state": "CLOSED", "statusCheckRollup": checks, "mergeable": "CONFLICTING"});
        parse_view(view.to_string().as_bytes(), &[]).unwrap()
    };
    let done = ("COMPLETED", "SUCCESS");
    let skipped = ("COMPLETED", "SKIPPED");
    assert_eq!(
        view(&[done, ("QUEUED", ""), ("COMPLETED", "CANCELLED")])
            .summary
            .checks,
        Some(CheckState::Failing)
    );
    assert_eq!(
        view(&[done, ("QUEUED", ""), skipped]).summary.checks,
        Some(CheckState::Running)
    );
    assert_eq!(
        view(&[skipped, done]).summary.checks,
        Some(CheckState::Passing)
    );
    let closed = view(&[skipped]);
    assert_eq!(closed.summary.checks, Some(CheckState::Skipped));
    assert_eq!(
        (closed.summary.state, closed.conflicts),
        (PullState::Closed, true)
    );
    assert_eq!(view(&[]).summary.checks, None);
    let many: Vec<(&str, &str)> = vec![done; CHECKS_LIMIT + 1];
    assert_eq!(view(&many).checks.len(), CHECKS_LIMIT);
}

#[test]
fn reviews_and_comments_keep_the_newest() {
    let comment = |i: usize| json!({"body": format!("c{i}"), "createdAt": format!("2026-01-01T00:00:{i:02}Z"), "isMinimized": false});
    let comments: Vec<Value> = (0..NOTES_LIMIT + 1).map(comment).collect();
    let reviews = [
        json!({"state": "PENDING", "body": "draft", "submittedAt": "2026-02-01T00:00:00Z"}),
        json!({"state": "DISMISSED", "body": "", "submittedAt": "2026-02-01T00:00:01Z", "author": {"login": "a\u{202e}b"}}),
        json!({"state": "CHANGES_REQUESTED", "body": "", "submittedAt": "2026-02-01T00:00:02Z"}),
        json!({"state": "COMMENTED", "body": " \n", "submittedAt": "2026-02-01T00:00:03Z"}),
    ];
    let view = json!({"reviews": reviews, "comments": comments});
    let notes = super::notes(&view);
    assert_eq!(notes.len(), NOTES_LIMIT);
    assert_eq!(notes[0].body, "c3");
    let last: Vec<Option<ReviewState>> =
        notes[NOTES_LIMIT - 2..].iter().map(|n| n.review).collect();
    assert_eq!(
        last,
        [
            Some(ReviewState::Dismissed),
            Some(ReviewState::ChangesRequested)
        ]
    );
    assert_eq!(notes[NOTES_LIMIT - 2].author, "ab");
    let files: Vec<Value> = (0..FILES_LIMIT + 1).map(|_| json!({"path": "f"})).collect();
    let view = json!({"number": 2, "files": files});
    assert_eq!(
        parse_view(view.to_string().as_bytes(), &[])
            .unwrap()
            .files
            .len(),
        FILES_LIMIT
    );
}

#[test]
fn untrusted_text_is_made_fit_to_show() {
    assert_eq!(markdown("a\tb\r\n\u{202e}c\u{0}", 100), "a\tb\nc");
    assert_eq!(markdown("abcd", 4), "abcd");
    assert_eq!(markdown("abcde", 4), "abcd…");
    // Never cut inside a character.
    assert_eq!(markdown("aé", 2), "a…");
    assert_eq!(markdown("éé", 1), "…");
    assert_eq!(
        link("https://github.com/o"),
        Some("https://github.com/o".into())
    );
    for bad in [
        "http://x",
        "javascript:alert(1)",
        "https://x y",
        "https://x\u{200b}",
    ] {
        assert_eq!(link(bad), None, "{bad}");
    }
    let long = format!("https://{}", "a".repeat(URL_LIMIT - 8));
    assert!(link(&long).is_some());
    assert_eq!(link(&format!("{long}a")), None);
    let title = json!({"number": 1, "title": "x".repeat(LINE_LIMIT + 1)});
    let pull = summary(&title, &[]).unwrap();
    assert_eq!(pull.title.chars().count(), LINE_LIMIT);
    assert!(summary(&json!({"number": 0}), &[]).is_none());
    assert_eq!(check_head(HEAD), Ok(()));
    assert_eq!(check_head(&"a".repeat(64)), Ok(()));
    for bad in ["", "--delete-branch", &HEAD[1..], &"g".repeat(40)] {
        assert_eq!(check_head(bad), Err(format!("{bad:?} is not a commit")));
    }
}

fn pulls(error: Option<&str>) -> Control {
    Control::Pulls {
        project: "/r".into(),
        repo: None,
        mine: vec![],
        review: vec![],
        fetched_ms: 1,
        error: error.map(Into::into),
    }
}

#[test]
fn the_list_is_fetched_once_per_interval_unless_forced() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    assert_eq!(cache.plan("k", false, start), Plan::Fetch);
    // A fetch runs: every request waits for its answer.
    assert_eq!(cache.plan("k", true, start), Plan::Wait);
    assert_eq!(cache.done("k", pulls(None), start), pulls(None));
    let cached = Plan::Send(Box::new(pulls(None)));
    let interval = INTERVAL.as_millis() as u64;
    assert_eq!(cache.plan("k", false, at(interval - 1)), cached);
    // Another project or account has its own.
    assert_eq!(cache.plan("other", false, start), Plan::Fetch);
    assert_eq!(cache.plan("k", true, at(1)), Plan::Fetch);
    cache.done("k", pulls(None), at(1));
    assert_eq!(cache.plan("k", false, at(interval)), cached);
    assert_eq!(cache.plan("k", false, at(interval + 1)), Plan::Fetch);
    // Any other error is kept for the interval too, with no longer wait.
    let failed = cache.done("k", pulls(Some("gh: Not Found")), at(interval + 1));
    assert_eq!(failed, pulls(Some("gh: Not Found")));
    assert_eq!(cache.plan("k", true, at(interval + 2)), Plan::Fetch);
}

#[test]
fn the_rate_limit_makes_the_next_fetch_wait_longer() {
    let mut cache = Cache::default();
    let mut now = Instant::now();
    let limited = pulls(Some("gh: API rate limit exceeded for user ID 1."));
    for minutes in [2, 4, 8, 16, 30, 30] {
        assert_eq!(cache.plan("k", false, now), Plan::Fetch);
        let reply = cache.done("k", limited.clone(), now);
        let error = format!(
            "GitHub's rate limit: Hive asks again in {minutes} min. gh: API rate limit exceeded for user ID 1."
        );
        assert_eq!(reply, pulls(Some(&error)));
        // Not even on demand before then.
        let wait = Duration::from_secs(minutes * 60);
        let before = cache.plan("k", true, now + wait - Duration::from_millis(1));
        assert_eq!(before, Plan::Send(Box::new(reply)));
        now += wait;
    }
    assert_eq!(cache.plan("k", true, now), Plan::Fetch);
    cache.done("k", pulls(None), now);
    // A list that worked starts the wait over.
    assert_eq!(cache.plan("k", true, now), Plan::Fetch);
    let secondary = pulls(Some("You have exceeded a secondary Rate Limit"));
    let reply = cache.done("k", secondary, now);
    let Control::Pulls {
        error: Some(error), ..
    } = reply
    else {
        panic!()
    };
    assert!(
        error.starts_with("GitHub's rate limit: Hive asks again in 2 min."),
        "{error}"
    );
}

/// A followed repository with a GitHub remote (`remote`), and a fake `gh` in the same
/// temporary folder: it records every call's arguments and folder in `gh.log`, answers the
/// list and the details with the recordings, and refuses pull request 13.
struct Setup {
    tmp: tempfile::TempDir,
    projects: Projects,
    id: String,
    gh: Gh,
    cache: Mutex<Cache>,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
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

fn setup(remote: &str) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    );
    git(&root, &["remote", "add", "origin", remote]);
    let data = tmp.path().join("data");
    let projects = Projects::load(data.join("spaces.json"), &data.join("projects.json"));
    let id = projects.add(&root.display().to_string()).unwrap().id;
    let dir = tmp.path().display();
    let script = format!(
        r#"#!/bin/sh
for a in "$@"; do printf '%s\037' "$a" >> '{dir}/gh.log'; done
printf '%s\036' "$PWD" >> '{dir}/gh.log'
[ "$3" = 13 ] && {{ echo 'GraphQL: Pull request is not mergeable' >&2; exit 1; }}
case "$1 $2" in
  "api graphql") [ -f '{dir}/fail' ] && {{ cat '{dir}/fail' >&2; exit 1; }}; cat '{SEARCH}' ;;
  "pr view") cat '{VIEW}' ;;
  "pr checkout") git checkout -q -b feature ;;
  "pr create") [ "$7" = --title=nolink ] || echo 'https://github.com/o/r/pull/42' ;;
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
    let cache = Mutex::default();
    Setup {
        tmp,
        projects,
        id,
        gh,
        cache,
    }
}

impl Setup {
    fn answer(&self, request: Control) -> Vec<Control> {
        answer(&self.gh, &self.projects, &self.cache, request)
    }

    /// Each `gh` call so far: its arguments, then its folder.
    fn calls(&self) -> Vec<Vec<String>> {
        let log = std::fs::read_to_string(self.tmp.path().join("gh.log")).unwrap_or_default();
        let calls = log.split_terminator('\u{1e}');
        calls
            .map(|c| c.split('\u{1f}').map(Into::into).collect())
            .collect()
    }

    fn list(&self, force: bool) -> Vec<Control> {
        self.answer(Control::ListPulls {
            project: self.id.clone(),
            force,
        })
    }

    fn act(&self, number: u64, action: PullAction) -> Vec<Control> {
        self.answer(Control::ActOnPull {
            project: self.id.clone(),
            number,
            action,
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        Path::new(&self.id).join(name)
    }
}

#[test]
fn the_list_is_one_graphql_request_as_the_space_account() {
    let setup = setup("git@github.com:o/r.git");
    let replies = setup.list(false);
    let [
        Control::Pulls {
            project,
            repo,
            mine,
            review,
            fetched_ms,
            error,
        },
    ] = &replies[..]
    else {
        panic!("{replies:?}")
    };
    assert_eq!((project, error), (&setup.id, &None));
    assert_eq!(repo.as_ref().unwrap().name, "cli/cli");
    assert_eq!((mine.len(), review.len()), (3, 3));
    assert!(*fetched_ms > 0);
    let query = format!("query={QUERY}");
    let expected = [
        "api",
        "graphql",
        "--hostname",
        "github.com",
        "-F",
        "first=30",
        "-f",
        &query,
        "-f",
        "owner=o",
        "-f",
        "name=r",
        "-f",
        "mine=repo:o/r is:pr author:@me sort:updated-desc",
        "-f",
        "review=repo:o/r is:pr is:open review-requested:@me sort:updated-desc",
        &setup.id,
    ];
    assert_eq!(setup.calls(), [expected]);
    // Asked again within the interval: the same answer, GitHub not asked.
    assert_eq!(setup.list(false), replies);
    assert_eq!(setup.calls().len(), 1);
    // On demand: asked again; gh's error is shown without the query.
    std::fs::write(setup.tmp.path().join("fail"), "gh: Something went wrong\n").unwrap();
    let [
        Control::Pulls {
            error: Some(error),
            repo: None,
            ..
        },
    ] = &setup.list(true)[..]
    else {
        panic!()
    };
    assert_eq!(error, "gh api graphql failed: gh: Something went wrong");
    assert_eq!(setup.calls().len(), 2);
}

#[test]
fn a_space_account_on_another_host_needs_a_remote_there() {
    let setup = setup("https://github.com/o/r");
    let env = SpaceEnv {
        gh_account: Some(GhAccount {
            host: "ghe.example".into(),
            login: "me".into(),
        }),
        ..SpaceEnv::default()
    };
    let update = |spaces: &mut crate::spaces::Spaces| spaces.update("default", "Work", env);
    setup.projects.change_spaces(update).unwrap();
    let [
        Control::Pulls {
            error: Some(error), ..
        },
    ] = &setup.list(false)[..]
    else {
        panic!()
    };
    assert_eq!(error, "No remote of this repository is on ghe.example");
    assert_eq!(setup.calls().len(), 0);
}

#[test]
fn only_followed_projects_are_asked_about() {
    let setup = setup("https://github.com/o/r");
    let other = setup.tmp.path().display().to_string();
    let refused = format!("{other} is not a followed project");
    let [
        Control::Pulls {
            error: Some(error),
            fetched_ms: 0,
            ..
        },
    ] = &setup.answer(Control::ListPulls {
        project: other.clone(),
        force: true,
    })[..]
    else {
        panic!()
    };
    assert_eq!(error, &refused);
    let open = setup.answer(Control::OpenPull {
        project: other.clone(),
        number: 1,
    });
    let pull = Control::Pull {
        project: other.clone(),
        number: 1,
        pull: None,
        error: Some(refused.clone()),
    };
    assert_eq!(open, [pull]);
    let act = setup.answer(Control::ActOnPull {
        project: other.clone(),
        number: 1,
        action: PullAction::Close,
    });
    assert_eq!(
        act,
        [Control::PullFailed {
            project: other.clone(),
            number: Some(1),
            message: refused
        }]
    );
    let create = Control::CreatePull {
        worktree: other.clone(),
        title: "t".into(),
        body: String::new(),
        base: "main".into(),
        draft: false,
    };
    let message = format!("{other} is not a worktree of a followed project");
    assert_eq!(
        setup.answer(create),
        [Control::PullFailed {
            project: String::new(),
            number: None,
            message
        }]
    );
    assert_eq!(setup.answer(Control::ListProjects), []);
    // A project with no GitHub remote.
    git(Path::new(&setup.id), &["remote", "remove", "origin"]);
    let [
        Control::Pull {
            error: Some(error), ..
        },
    ] = &setup.answer(Control::OpenPull {
        project: setup.id.clone(),
        number: 1,
    })[..]
    else {
        panic!()
    };
    assert_eq!(error, "No remote of this repository is on github.com");
    assert_eq!(setup.calls().len(), 0);
}

#[test]
fn details_and_actions_send_the_right_arguments() {
    let setup = setup("https://github.com/o/r");
    let open = setup.answer(Control::OpenPull {
        project: setup.id.clone(),
        number: 14337,
    });
    let [
        Control::Pull {
            pull: Some(pull),
            error: None,
            number: 14337,
            ..
        },
    ] = &open[..]
    else {
        panic!("{open:?}")
    };
    assert_eq!(pull.head, HEAD);
    let repo = "github.com/o/r";
    let id = setup.id.as_str();
    assert_eq!(
        setup.calls(),
        [vec![
            "pr",
            "view",
            "14337",
            "--repo",
            repo,
            "--json",
            VIEW_FIELDS,
            id
        ]]
    );

    let merge = PullAction::Merge {
        method: MergeMethod::Squash,
        head: HEAD.into(),
    };
    let replies = setup.act(7, merge);
    assert_eq!(replies.len(), 3);
    let done = Control::PullDone {
        project: setup.id.clone(),
        number: 7,
        message: "Merged pull request #7".into(),
    };
    assert_eq!(replies[0], done);
    assert!(matches!(
        &replies[1],
        Control::Pull {
            number: 7,
            pull: Some(_),
            ..
        }
    ));
    assert!(matches!(&replies[2], Control::Pulls { error: None, .. }));
    let calls = setup.calls();
    assert_eq!(
        calls[1],
        [
            "pr",
            "merge",
            "7",
            "--repo",
            repo,
            "--squash",
            "--match-head-commit",
            HEAD,
            id
        ]
    );
    assert_eq!(&calls[2][..2], ["pr", "view"]);
    assert_eq!(&calls[3][..2], ["api", "graphql"]);
    for (action, args, message) in [
        (
            PullAction::Ready,
            vec!["pr", "ready", "7", "--repo", repo],
            "#7 is ready for review",
        ),
        (
            PullAction::Close,
            vec!["pr", "close", "7", "--repo", repo],
            "Closed pull request #7",
        ),
        (
            PullAction::Merge {
                method: MergeMethod::Merge,
                head: HEAD.into(),
            },
            vec![
                "pr",
                "merge",
                "7",
                "--repo",
                repo,
                "--merge",
                "--match-head-commit",
                HEAD,
            ],
            "Merged pull request #7",
        ),
        (
            PullAction::Merge {
                method: MergeMethod::Rebase,
                head: HEAD.into(),
            },
            vec![
                "pr",
                "merge",
                "7",
                "--repo",
                repo,
                "--rebase",
                "--match-head-commit",
                HEAD,
            ],
            "Merged pull request #7",
        ),
    ] {
        let replies = setup.act(7, action);
        let Control::PullDone { message: said, .. } = &replies[0] else {
            panic!()
        };
        assert_eq!(said, message);
        let calls = setup.calls();
        let call = &calls[calls.len() - 3];
        assert_eq!(call[..call.len() - 1], args);
    }
    // A refused one shows gh's message; nothing else is asked.
    let calls = setup.calls().len();
    let failed = Control::PullFailed {
        project: setup.id.clone(),
        number: Some(13),
        message: "gh pr close failed: GraphQL: Pull request is not mergeable".into(),
    };
    assert_eq!(setup.act(13, PullAction::Close), [failed]);
    assert_eq!(setup.calls().len(), calls + 1);
    // A head that is not a commit never reaches gh.
    let bad = PullAction::Merge {
        method: MergeMethod::Merge,
        head: "--admin".into(),
    };
    let [Control::PullFailed { message, .. }] = &setup.act(7, bad)[..] else {
        panic!()
    };
    assert_eq!(message, r#""--admin" is not a commit"#);
    assert_eq!(setup.calls().len(), calls + 1);
}

#[test]
fn checkout_makes_a_worktree_on_the_pull_request_branch() {
    let setup = setup("https://github.com/o/r");
    let replies = setup.act(12, PullAction::Checkout);
    let path = setup.path(".claude/worktrees/pr-12");
    let Control::WorktreeCreated {
        project,
        path: created,
        notes,
    } = &replies[0]
    else {
        panic!("{replies:?}")
    };
    assert_eq!((created.as_str(), notes.len()), (path.to_str().unwrap(), 0));
    let new = project
        .worktrees
        .iter()
        .find(|w| w.path == *created)
        .unwrap();
    assert_eq!(new.branch.as_deref(), Some("feature"));
    assert!(matches!(&replies[1], Control::Pulls { .. }));
    assert_eq!(replies.len(), 2);
    let calls = setup.calls();
    assert_eq!(
        calls[0],
        [
            "pr",
            "checkout",
            "12",
            "--repo",
            "github.com/o/r",
            created.as_str()
        ]
    );
    // The branch the worktree flow made is gone.
    let branches = git(
        Path::new(&setup.id),
        &["branch", "--format=%(refname:short)"],
    );
    assert_eq!(branches, "feature\nmain\n");

    // gh refuses: neither the worktree nor its branch is left.
    let [
        Control::PullFailed {
            number: Some(13),
            message,
            ..
        },
    ] = &setup.act(13, PullAction::Checkout)[..]
    else {
        panic!()
    };
    assert_eq!(
        message,
        "gh pr checkout failed: GraphQL: Pull request is not mergeable"
    );
    assert!(!setup.path(".claude/worktrees/pr-13").exists());
    let branches = git(
        Path::new(&setup.id),
        &["branch", "--format=%(refname:short)"],
    );
    assert_eq!(branches, "feature\nmain\n");
    // Its folder taken: the worktree flow's own refusal.
    let [Control::PullFailed { message, .. }] = &setup.act(12, PullAction::Checkout)[..] else {
        panic!()
    };
    assert!(
        message.starts_with(r#"worktree "pr-12" already exists"#),
        "{message}"
    );
}

#[test]
fn a_pull_request_opens_from_the_worktree_branch() {
    let setup = setup("https://github.com/o/r");
    let root = Path::new(&setup.id);
    git(root, &["branch", "topic"]);
    let wt = setup.path("wt");
    git(
        root,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "topic"],
    );
    let create = |title: &str, body: &str, base: &str, draft: bool| {
        setup.answer(Control::CreatePull {
            worktree: wt.display().to_string(),
            title: title.into(),
            body: body.into(),
            base: base.into(),
            draft,
        })
    };
    let replies = create(" --title x ", "-x\nbody", " main ", true);
    let done = Control::PullDone {
        project: setup.id.clone(),
        number: 42,
        message: "Opened pull request #42".into(),
    };
    assert_eq!(replies[0], done);
    assert!(matches!(&replies[1], Control::Pulls { .. }));
    let calls = setup.calls();
    let args = [
        "pr",
        "create",
        "--repo",
        "github.com/o/r",
        "--head=topic",
        "--base=main",
        "--title=--title x",
        "--body=-x\nbody",
        "--draft",
    ];
    assert_eq!(calls[0][..calls[0].len() - 1], args);
    assert_eq!(calls[0].last().unwrap(), wt.to_str().unwrap());
    create("t", "", "main", false);
    let calls = setup.calls();
    assert_eq!(calls[2].len(), 9);
    assert_eq!(calls[2][7], "--body=");

    let refused = |title: &str, body: &str, base: &str| {
        let [
            Control::PullFailed {
                project,
                number: None,
                message,
            },
        ] = &create(title, body, base, false)[..]
        else {
            panic!()
        };
        assert_eq!(project, &setup.id);
        message.clone()
    };
    let calls = setup.calls().len();
    assert_eq!(
        refused(" ", "", "main"),
        "A title of 1 to 256 characters is needed"
    );
    assert_eq!(
        refused(&"t".repeat(257), "", "main"),
        "A title of 1 to 256 characters is needed"
    );
    assert_eq!(
        refused("t", &"b".repeat(BODY_LIMIT + 1), "main"),
        "The description is over 64 KiB"
    );
    for base in ["", "-b", &"b".repeat(257)] {
        assert_eq!(
            refused("t", "", base),
            "The base branch is not a branch name"
        );
    }
    assert_eq!(setup.calls().len(), calls);
    let most = create("t", &"b".repeat(BODY_LIMIT), &"b".repeat(256), false);
    assert!(matches!(most[0], Control::PullDone { .. }), "{most:?}");
    // gh answered without a link: opened all the same.
    let done = Control::PullDone {
        project: setup.id.clone(),
        number: 0,
        message: "Opened the pull request".into(),
    };
    assert_eq!(create("nolink", "", "main", false)[0], done);
    git(&wt, &["checkout", "-q", "--detach"]);
    assert_eq!(refused("t", "", "main"), "This worktree is on no branch");
}
