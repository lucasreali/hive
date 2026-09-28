use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::Duration;

use hive_protocol::SpaceEnv;
use serde_json::json;

use super::*;
use crate::pulls::INTERVAL;

const LIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/run-list.json"
);
const VIEW: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/run-view.json"
);
const LOG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/gh/job-log.txt");
/// The recorded run (a nightly `mutants-full`) and its first failed job.
const RUN: u64 = 36312609118;
const FAILED_JOB: u64 = 108601869461;

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
fn the_recorded_list_gives_each_run_its_state_and_worktree() {
    let out = std::fs::read(LIST).unwrap();
    let runs = parse_list(&out, &[worktree("/r/a", "feature")]).unwrap();
    assert_eq!(runs.len(), 5);
    assert_eq!(
        runs[0],
        RunSummary {
            id: 36337160818,
            number: 192,
            workflow: "ci".into(),
            title: "Commit 1".into(),
            branch: "feature".into(),
            event: "push".into(),
            state: CheckState::Running,
            status: "queued".into(),
            url: "https://github.com/o/r/actions/runs/36337160818".into(),
            created_at: "2026-09-27T17:29:59Z".into(),
            started_at: "2026-09-27T17:29:59Z".into(),
            updated_at: "2026-09-27T17:29:59Z".into(),
            worktree: Some("/r/a".into()),
        }
    );
    let done = &runs[2];
    assert_eq!(
        (done.state, done.status.as_str(), done.branch.as_str()),
        (CheckState::Passing, "success", "main")
    );
    assert_eq!(done.worktree, None);
    // Not a list, or a run without its id.
    let err = parse_list(b"{}", &[]).unwrap_err();
    assert_eq!(err, "gh gave no runs");
    assert!(
        parse_list(b"nope", &[])
            .unwrap_err()
            .starts_with("gh gave no runs: ")
    );
    let bare =
        json!([{}, {"databaseId": 1, "status": "completed", "conclusion": "", "event": "Push!"}]);
    let runs = parse_list(bare.to_string().as_bytes(), &[]).unwrap();
    assert_eq!(runs.len(), 1);
    // Completed without a conclusion: its status shows; an odd event does not.
    assert_eq!(
        (
            runs[0].state,
            runs[0].status.as_str(),
            runs[0].event.as_str()
        ),
        (CheckState::Skipped, "completed", "")
    );
}

#[test]
fn the_recorded_run_has_its_jobs_and_steps() {
    let out = std::fs::read(VIEW).unwrap();
    let run = parse_view(&out, &[worktree("/r", "main")]).unwrap();
    assert_eq!(run.summary.id, RUN);
    assert_eq!(
        (run.summary.state, run.summary.event.as_str()),
        (CheckState::Failing, "schedule")
    );
    assert_eq!(run.summary.worktree.as_deref(), Some("/r"));
    let names: Vec<&str> = run.jobs.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "mutants / plan",
            "mutants / shard (17)",
            "mutants / gather",
            "report"
        ]
    );
    let shard = &run.jobs[1];
    assert_eq!(
        (shard.id, shard.state, shard.status.as_str()),
        (FAILED_JOB, CheckState::Failing, "failure")
    );
    assert_eq!(
        shard.url,
        format!("https://github.com/o/r/actions/runs/{RUN}/job/{FAILED_JOB}")
    );
    assert_eq!(
        (shard.started_at.as_str(), shard.completed_at.as_str()),
        ("2026-09-27T10:31:38Z", "2026-09-27T10:52:38Z")
    );
    assert_eq!(shard.steps.len(), 10);
    assert_eq!(
        shard.steps[5],
        RunStep {
            number: 6,
            name: "Run args=()".into(),
            state: CheckState::Failing,
            status: "failure".into(),
        }
    );
    assert_eq!(
        (shard.steps[4].state, shard.steps[4].status.as_str()),
        (CheckState::Skipped, "skipped")
    );
    assert_eq!(run.jobs[3].state, CheckState::Passing);
    // Capped: jobs and steps.
    let steps: Vec<Value> = (0..STEPS_LIMIT + 1).map(|n| json!({"number": n})).collect();
    let job = json!({"steps": steps});
    let jobs: Vec<Value> = (0..JOBS_LIMIT + 1).map(|_| job.clone()).collect();
    let view = json!({"databaseId": 1, "jobs": jobs});
    let run = parse_view(view.to_string().as_bytes(), &[]).unwrap();
    assert_eq!(run.jobs.len(), JOBS_LIMIT);
    assert_eq!(run.jobs[0].steps.len(), STEPS_LIMIT);
    assert_eq!(run.jobs[0].state, CheckState::Running);
    assert_eq!(
        parse_view(b"{}", &[]).unwrap_err(),
        "gh gave no run".to_owned()
    );
    assert!(
        parse_view(b"[", &[])
            .unwrap_err()
            .starts_with("gh gave no run: ")
    );
}

#[test]
fn states_and_words_come_from_github_status_and_conclusion() {
    let state = |status: &str, conclusion: &str| {
        progress(&json!({"status": status, "conclusion": conclusion}))
    };
    for (status, conclusion, shown, expected) in [
        ("completed", "success", "success", CheckState::Passing),
        ("completed", "failure", "failure", CheckState::Failing),
        ("completed", "timed_out", "timed_out", CheckState::Failing),
        (
            "completed",
            "startup_failure",
            "startup_failure",
            CheckState::Failing,
        ),
        ("completed", "cancelled", "cancelled", CheckState::Skipped),
        ("in_progress", "", "in_progress", CheckState::Running),
        ("queued", "success", "queued", CheckState::Running),
    ] {
        assert_eq!(
            state(status, conclusion),
            (shown.to_owned(), expected),
            "{status} {conclusion}"
        );
    }
    let longest = "a_".repeat(WORD_LIMIT / 2);
    assert_eq!(word(&longest), longest);
    assert_eq!(word(&format!("{longest}a")), "");
    for odd in ["Success", "in progress", "x;y", "é"] {
        assert_eq!(word(odd), "", "{odd}");
    }
}

#[test]
fn a_job_log_shows_its_failed_step_without_escapes() {
    let raw = std::fs::read_to_string(LOG).unwrap();
    let log = log_tail(&raw);
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines[0], "✓ built in 3.50s");
    assert_eq!(lines[1], "Run args=()");
    assert_eq!(lines[2], "args=()");
    assert!(lines.contains(&"Found 86 mutants to test"));
    assert!(lines.contains(&"MISSED   src-tauri/src/lib.rs:747:9: replace commands::check_update with () in 4s build + 0s test"), "{log}");
    // Up to the error: the upload and the cleanup after it are left out.
    assert_eq!(
        lines.last(),
        Some(&"##[error]Process completed with exit code 2.")
    );
    for gone in [
        "\u{1b}",
        "##[endgroup]",
        "##[end-action",
        "##[start-action",
        "2026-09-27T",
    ] {
        assert!(!log.contains(gone), "{gone}");
    }
    // No error: the end as it is; a line without a timestamp stays whole.
    let plain = "2026-09-27T10:52:36.1366194Z Post job cleanup.\nno time here\n2026-09-27T10:52:36.1366194Z\n";
    assert_eq!(log_tail(plain), "Post job cleanup.\nno time here\n");
    // A carriage return redraws the line; tabs stay, other controls go.
    assert_eq!(log_tail("10%\r50%\r100%\ta\u{7}b"), "100%\tab");
}

#[test]
fn ansi_escapes_are_stripped() {
    for (text, plain) in [
        ("\u{1b}[31m\u{1b}[1mred\u{1b}[0m", "red"),
        ("\u{1b}[38;5;13mx\u{1b}[0m y", "x y"),
        ("\u{1b}]8;;https://x\u{7}link\u{1b}]8;;\u{7}", "link"),
        ("\u{1b}]0;title\u{1b}\\after", "after"),
        ("\u{1b}]0;never ended", ""),
        ("a\u{1b}=b", "ab"),
        ("end\u{1b}", "end"),
    ] {
        assert_eq!(strip_ansi(text), plain, "{text:?}");
    }
}

#[test]
fn a_long_log_keeps_its_last_whole_lines() {
    // 64 lines of 1023 characters are the limit exactly, with their line ends.
    let line = "y".repeat(1023);
    let mut raw = vec!["x".repeat(60)];
    raw.extend(std::iter::repeat_n(line.clone(), 64));
    let log = log_tail(&raw.join("\n"));
    assert_eq!(log.lines().count(), 64);
    assert!(log.lines().all(|l| l == line));
    assert!(log.len() <= LOG_LIMIT);
    // A line longer than the limit is left out.
    let huge = format!("{}\nlast", "z".repeat(LOG_LIMIT));
    assert_eq!(log_tail(&huge), "last");
}

fn empty(error: Option<&str>) -> Control {
    Control::Runs {
        project: "/r".into(),
        branch: None,
        runs: vec![],
        fetched_ms: 1,
        error: error.map(Into::into),
    }
}

#[test]
fn the_rate_limit_holds_runs_as_pull_requests() {
    let mut cache = Cache::default();
    let now = Instant::now();
    assert_eq!(cache.plan("k", false, now), Plan::Fetch);
    let reply = cache.done("k", empty(Some("HTTP 403: API rate limit exceeded")), now);
    let error = "GitHub's rate limit: Hive asks again in 2 min. HTTP 403: API rate limit exceeded";
    assert_eq!(reply, empty(Some(error)));
    let held = cache.plan("k", true, now + INTERVAL - Duration::from_millis(1));
    assert_eq!(held, Plan::Send(Box::new(reply)));
}

/// A followed repository with a GitHub remote and a fake `gh` in the same temporary folder:
/// it records every call's arguments and folder in `gh.log`, answers with the recordings,
/// and refuses run 13.
struct Setup {
    tmp: tempfile::TempDir,
    projects: Projects,
    id: String,
    gh: Gh,
    cache: Mutex<Cache>,
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &["remote", "add", "origin", "git@github.com:o/r.git"],
    );
    let data = tmp.path().join("data");
    let projects = Projects::load(data.join("spaces.json"), &data.join("projects.json"));
    let id = projects.add(&root.display().to_string()).unwrap().id;
    let dir = tmp.path().display();
    let script = format!(
        r#"#!/bin/sh
for a in "$@"; do printf '%s\037' "$a" >> '{dir}/gh.log'; done
printf '%s\036' "$PWD" >> '{dir}/gh.log'
[ "$3" = 13 ] && {{ echo 'run 13 cannot be cancelled since it has already completed' >&2; exit 1; }}
case "$1 $2" in
  "run list") [ -f '{dir}/fail' ] && {{ cat '{dir}/fail' >&2; exit 1; }}; cat '{LIST}' ;;
  "run view") cat '{VIEW}' ;;
  "api --hostname") case "$4" in
      */jobs/2/logs) seq 1 300000 ;;
      *) cat '{LOG}' ;;
    esac ;;
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

    fn list(&self, branch: Option<&str>, force: bool) -> Vec<Control> {
        self.answer(Control::ListRuns {
            project: self.id.clone(),
            branch: branch.map(Into::into),
            force,
        })
    }

    fn act(&self, run: u64, action: RunAction) -> Vec<Control> {
        self.answer(Control::ActOnRun {
            project: self.id.clone(),
            run,
            action,
            branch: Some("feature".into()),
        })
    }
}

#[test]
fn the_list_is_asked_per_branch_once_per_interval() {
    let setup = setup();
    let id = setup.id.as_str();
    let [
        Control::Runs {
            branch: None,
            runs,
            error: None,
            fetched_ms,
            ..
        },
    ] = &setup.list(None, false)[..]
    else {
        panic!()
    };
    assert_eq!(runs.len(), 5);
    assert!(*fetched_ms > 0);
    let list = ["run", "list", "--repo", "github.com/o/r", "--limit", "30"];
    let json = ["--json", FIELDS, id];
    assert_eq!(setup.calls(), [[&list[..], &json[..]].concat()]);
    // Within the interval: the same answer, GitHub not asked; another branch is its own list.
    setup.list(None, false);
    assert_eq!(setup.calls().len(), 1);
    let [Control::Runs { branch, .. }] = &setup.list(Some("feature"), false)[..] else {
        panic!()
    };
    assert_eq!(branch.as_deref(), Some("feature"));
    let only = ["--branch=feature"];
    assert_eq!(setup.calls()[1], [&list[..], &only[..], &json[..]].concat());
    // While a fetch runs, another request waits for its answer.
    let key = format!("{}\nNone", key(id, &SpaceEnv::default()));
    let running = setup.cache.lock().unwrap().plan(&key, true, Instant::now());
    assert_eq!(running, Plan::Fetch);
    assert_eq!(setup.list(None, true), []);
    setup
        .cache
        .lock()
        .unwrap()
        .done(&key, empty(None), Instant::now());
    // On demand: asked again; gh's refusal is shown, GitHub's rate limit held.
    std::fs::write(setup.tmp.path().join("fail"), "API rate limit exceeded\n").unwrap();
    let [
        Control::Runs {
            error: Some(error),
            runs,
            ..
        },
    ] = &setup.list(None, true)[..]
    else {
        panic!()
    };
    assert!(runs.is_empty());
    assert_eq!(
        error,
        "GitHub's rate limit: Hive asks again in 2 min. gh run list failed: API rate limit exceeded"
    );
    setup.list(None, true);
    assert_eq!(setup.calls().len(), 3);
}

#[test]
fn a_run_and_its_failed_job_log_are_asked_for() {
    let setup = setup();
    let id = setup.id.clone();
    let [
        Control::Run {
            run: RUN,
            detail: Some(detail),
            error: None,
            ..
        },
    ] = &setup.answer(Control::OpenRun {
        project: id.clone(),
        run: RUN,
    })[..]
    else {
        panic!()
    };
    assert_eq!(detail.jobs.len(), 4);
    let fields = format!("{FIELDS},jobs");
    let run = RUN.to_string();
    let view = [
        "run",
        "view",
        &run,
        "--repo",
        "github.com/o/r",
        "--json",
        &fields,
        &id,
    ];
    assert_eq!(setup.calls(), [view]);

    let [
        Control::JobLog {
            job: FAILED_JOB,
            log: Some(log),
            error: None,
            ..
        },
    ] = &setup.answer(Control::OpenJobLog {
        project: id.clone(),
        job: FAILED_JOB,
    })[..]
    else {
        panic!()
    };
    assert!(log.ends_with("##[error]Process completed with exit code 2."));
    let path = format!("repos/o/r/actions/jobs/{FAILED_JOB}/logs");
    assert_eq!(
        setup.calls()[1],
        ["api", "--hostname", "github.com", &path, &id]
    );
    // A big log: its last lines only.
    let [Control::JobLog { log: Some(log), .. }] = &setup.answer(Control::OpenJobLog {
        project: id.clone(),
        job: 2,
    })[..] else {
        panic!()
    };
    assert!(
        log.len() <= LOG_LIMIT && log.len() > LOG_LIMIT - 8,
        "{}",
        log.len()
    );
    assert!(log.ends_with("\n299999\n300000"));
    assert!(log.lines().next().unwrap().parse::<u64>().is_ok());
}

#[test]
fn re_run_and_cancel_send_the_right_arguments() {
    let setup = setup();
    let id = setup.id.clone();
    let repo = "github.com/o/r";
    for (action, args, message) in [
        (
            RunAction::Rerun { failed: false },
            vec!["run", "rerun", "7", "--repo", repo],
            "Re-running every job",
        ),
        (
            RunAction::Rerun { failed: true },
            vec!["run", "rerun", "7", "--repo", repo, "--failed"],
            "Re-running the failed jobs",
        ),
        (
            RunAction::Cancel,
            vec!["run", "cancel", "7", "--repo", repo],
            "Cancelling the run",
        ),
    ] {
        let replies = setup.act(7, action);
        let done = Control::RunDone {
            project: id.clone(),
            run: 7,
            message: message.into(),
        };
        assert_eq!(replies[0], done);
        assert!(matches!(&replies[1], Control::Run { run: 7, .. }));
        assert!(matches!(
            &replies[2],
            Control::Runs { branch: Some(b), error: None, .. } if b == "feature"
        ));
        let calls = setup.calls();
        let call = &calls[calls.len() - 3];
        assert_eq!(call[..call.len() - 1], args);
        // The list is asked again, forced.
        assert_eq!(&calls[calls.len() - 1][..2], ["run", "list"]);
    }
    // A refused one shows gh's message; nothing else is asked.
    let calls = setup.calls().len();
    let failed = Control::RunFailed {
        project: id,
        run: 13,
        message: "gh run cancel failed: run 13 cannot be cancelled since it has already completed"
            .into(),
    };
    assert_eq!(setup.act(13, RunAction::Cancel), [failed]);
    assert_eq!(setup.calls().len(), calls + 1);
}

#[test]
fn only_followed_projects_with_a_github_remote_are_asked_about() {
    let setup = setup();
    let other = setup.tmp.path().display().to_string();
    let refused = format!("{other} is not a followed project");
    let listed = setup.answer(Control::ListRuns {
        project: other.clone(),
        branch: None,
        force: false,
    });
    let expected = Control::Runs {
        project: other.clone(),
        branch: None,
        runs: vec![],
        fetched_ms: 0,
        error: Some(refused.clone()),
    };
    assert_eq!(listed, [expected]);
    let open = setup.answer(Control::OpenRun {
        project: other.clone(),
        run: 1,
    });
    let run = Control::Run {
        project: other.clone(),
        run: 1,
        detail: None,
        error: Some(refused.clone()),
    };
    assert_eq!(open, [run]);
    let log = setup.answer(Control::OpenJobLog {
        project: other.clone(),
        job: 1,
    });
    let expected = Control::JobLog {
        project: other.clone(),
        job: 1,
        log: None,
        error: Some(refused.clone()),
    };
    assert_eq!(log, [expected]);
    let act = setup.answer(Control::ActOnRun {
        project: other.clone(),
        run: 1,
        action: RunAction::Cancel,
        branch: None,
    });
    let failed = Control::RunFailed {
        project: other,
        run: 1,
        message: refused,
    };
    assert_eq!(act, [failed]);
    assert_eq!(setup.answer(Control::ListProjects), []);
    // No GitHub remote.
    git(Path::new(&setup.id), &["remote", "remove", "origin"]);
    let [
        Control::Runs {
            error: Some(error), ..
        },
    ] = &setup.list(None, true)[..]
    else {
        panic!()
    };
    assert_eq!(error, "No remote of this repository is on github.com");
    assert_eq!(setup.calls().len(), 0);
}
