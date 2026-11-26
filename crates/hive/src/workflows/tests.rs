#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::process::Command;

use super::*;

const DISPATCHED: &str = r#"
name: Release
on:
  workflow_dispatch:
    inputs:
      level:
        description: "Which part to bump"
        required: true
        default: patch
        type: choice
        options: [patch, minor, major]
      dry:
        type: boolean
        default: true
      count:
        type: number
        default: 3
      plain: {}
      listed:
        default: [a, b]
      "bad name": {}
  push:
"#;

#[test]
fn a_dispatchable_workflow_gives_its_inputs() {
    let workflow = parse("release.yml", DISPATCHED.as_bytes()).unwrap();
    assert_eq!(workflow.file, "release.yml");
    assert_eq!(workflow.name, "Release");
    let input = |name: &str| workflow.inputs.iter().find(|i| i.name == name).unwrap();
    assert_eq!(
        input("level"),
        &WorkflowInput {
            name: "level".into(),
            description: "Which part to bump".into(),
            required: true,
            default: Some("patch".into()),
            kind: "choice".into(),
            options: vec!["patch".into(), "minor".into(), "major".into()],
        }
    );
    let dry = input("dry");
    assert_eq!(
        (dry.kind.as_str(), dry.default.as_deref()),
        ("boolean", Some("true"))
    );
    assert_eq!(input("count").default.as_deref(), Some("3"));
    let plain = input("plain");
    assert_eq!(
        (
            plain.kind.as_str(),
            plain.required,
            plain.default.as_deref()
        ),
        ("string", false, None)
    );
    // A default that is no scalar is none.
    assert_eq!(input("listed").default, None);
    // An input GitHub would not accept is left out.
    assert_eq!(workflow.inputs.len(), 5);
}

#[test]
fn only_workflows_run_by_hand_are_listed() {
    let parsed = |yaml: &str| parse("w.yml", yaml.as_bytes());
    // The trigger as a word, in a list, or a map without inputs; the file names it otherwise.
    let word = parsed("on: workflow_dispatch").unwrap();
    assert_eq!((word.name.as_str(), word.inputs.len()), ("w.yml", 0));
    assert!(parsed("on: [push, workflow_dispatch]").is_some());
    assert!(parsed("name: ''\non:\n  workflow_dispatch:\n").is_some());
    // YAML 1.1's `on` read as `true`.
    assert!(parsed("true: workflow_dispatch").is_some());
    for not in [
        "on: push",
        "on: [push]",
        "on:\n  push:\n",
        "on: 3",
        "name: x",
        "{",
        "- a",
    ] {
        assert_eq!(parsed(not), None, "{not}");
    }
    // Many inputs or options are cut.
    let many: String = (0..INPUTS_LIMIT + 5)
        .map(|i| format!("      i{i}: {{}}\n"))
        .collect();
    let yaml = format!("on:\n  workflow_dispatch:\n    inputs:\n{many}");
    assert_eq!(parsed(&yaml).unwrap().inputs.len(), INPUTS_LIMIT);
    let options: Vec<String> = (0..OPTIONS_LIMIT + 5).map(|i| i.to_string()).collect();
    let yaml = format!(
        "on:\n  workflow_dispatch:\n    inputs:\n      c:\n        options: [{}]\n",
        options.join(", ")
    );
    assert_eq!(
        parsed(&yaml).unwrap().inputs[0].options.len(),
        OPTIONS_LIMIT
    );
}

#[test]
fn file_and_input_names_are_checked() {
    for good in ["ci.yml", "release-1.yaml", "a_b.yml"] {
        assert_eq!(check_file(good), Ok(()));
    }
    for bad in [
        "-x.yml",
        ".x.yml",
        "ci.json",
        "a b.yml",
        "../ci.yml",
        "a/ci.yml",
    ] {
        assert_eq!(
            check_file(bad),
            Err(format!("{bad:?} is not a workflow of this repository"))
        );
    }
    assert!(check_file(&format!("{}.yml", "a".repeat(LINE_LIMIT))).is_err());
    for good in ["a", "_x", "level-2"] {
        assert_eq!(check_name(good), Ok(()));
    }
    for bad in ["", "2a", "-a", "a b", "a=b", &"a".repeat(101)] {
        assert_eq!(
            check_name(bad),
            Err(format!("{bad:?} is not an input name"))
        );
    }
    assert!(check_name(&"a".repeat(100)).is_ok());
    assert_eq!(
        ids(br#"[{"databaseId":3},{"x":1},{"databaseId":2}]"#),
        [3, 2]
    );
    assert_eq!(ids(b"nope"), Vec::<u64>::new());
}

/// A followed repository whose `HEAD` has workflows, and a fake `gh` that logs each call's
/// arguments and lists runs from `runs` (one id per line, the newest first), refusing a run
/// of `broken.yml`.
#[cfg(unix)]
struct Setup {
    tmp: tempfile::TempDir,
    projects: Projects,
    id: String,
    gh: Gh,
    cache: Mutex<Cache>,
}

#[cfg(unix)]
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

#[cfg(unix)]
fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let flows = root.join(".github/workflows");
    std::fs::create_dir_all(&flows).unwrap();
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
printf '\036' >> '{dir}/gh.log'
case "$1 $2" in
  "run list") [ -f '{dir}/nolist' ] && {{ echo 'HTTP 500' >&2; exit 1; }}; printf '['; sep=''; while read -r id; do printf '%s{{"databaseId":%s}}' "$sep" "$id"; sep=,; done < '{dir}/runs'; echo ']' ;;
  "workflow run") [ "$3" = broken.yml ] && {{ echo 'HTTP 422: Workflow does not have workflow_dispatch trigger' >&2; exit 1; }}
    [ -f '{dir}/next' ] && {{ cat '{dir}/next' '{dir}/runs' > '{dir}/both'; mv '{dir}/both' '{dir}/runs'; }}; true ;;
  "run view") echo '{{}}' ;;
esac
"#
    );
    std::fs::write(tmp.path().join("runs"), "7\n").unwrap();
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

#[cfg(unix)]
impl Setup {
    fn answer(&self, request: Control) -> Vec<Control> {
        answer(
            &self.gh,
            &self.projects,
            &self.cache,
            request,
            Duration::ZERO,
        )
    }

    fn calls(&self) -> Vec<Vec<String>> {
        let log = std::fs::read_to_string(self.tmp.path().join("gh.log")).unwrap_or_default();
        let calls = log.split_terminator('\u{1e}');
        calls
            .map(|c| c.split_terminator('\u{1f}').map(Into::into).collect())
            .collect()
    }

    fn start(&self, file: &str, inputs: &[(&str, &str)]) -> Vec<Control> {
        let inputs = inputs.iter().map(|&(name, value)| WorkflowValue {
            name: name.into(),
            value: value.into(),
        });
        self.answer(Control::RunWorkflow {
            project: self.id.clone(),
            workflow: file.into(),
            branch: "main".into(),
            inputs: inputs.collect(),
        })
    }
}

#[cfg(unix)]
#[test]
fn the_workflows_are_read_from_the_worktrees_head() {
    let setup = setup();
    let root = Path::new(&setup.id);
    let list = || {
        setup.answer(Control::ListWorkflows {
            project: setup.id.clone(),
            worktree: setup.id.clone(),
        })
    };
    let listed = |replies: Vec<Control>| match &replies[..] {
        [
            Control::Workflows {
                workflows, error, ..
            },
        ] => (
            workflows.iter().map(|w| w.file.clone()).collect::<Vec<_>>(),
            error.clone(),
        ),
        other => panic!("{other:?}"),
    };
    // No commit yet: none.
    assert_eq!(listed(list()), (vec![], None));
    let flows = root.join(".github/workflows");
    std::fs::write(flows.join("release.yml"), DISPATCHED).unwrap();
    std::fs::write(flows.join("ci.yml"), "name: ci\non: push\n").unwrap();
    std::fs::write(flows.join("-odd.yml"), "on: workflow_dispatch\n").unwrap();
    std::fs::write(flows.join("big.yml"), "#".repeat(FILE_LIMIT as usize + 1)).unwrap();
    std::fs::write(flows.join("a.yaml"), "on: workflow_dispatch\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "flows"]);
    // Not committed: not on GitHub either.
    std::fs::write(flows.join("new.yml"), "on: workflow_dispatch\n").unwrap();
    // Sorted by name: `a.yaml` is named by its file.
    assert_eq!(
        listed(list()),
        (vec!["release.yml".into(), "a.yaml".into()], None)
    );
    // A worktree of another project, or none.
    let other = Control::ListWorkflows {
        project: "/elsewhere".into(),
        worktree: setup.id.clone(),
    };
    let error = Some(format!("{} is not a worktree of /elsewhere", setup.id));
    assert_eq!(listed(setup.answer(other)), (vec![], error));
    let none = Control::ListWorkflows {
        project: setup.id.clone(),
        worktree: "/nope".into(),
    };
    let error = Some("/nope is not a worktree of a followed project".to_owned());
    assert_eq!(listed(setup.answer(none)), (vec![], error));
    assert_eq!(setup.calls().len(), 0);
}

#[cfg(unix)]
#[test]
fn a_started_workflow_is_followed_to_its_run() {
    let setup = setup();
    std::fs::write(setup.tmp.path().join("next"), "9\n").unwrap();
    let replies = setup.start("release.yml", &[("level", "-minor"), ("note", "a=b")]);
    assert_eq!(
        replies[0],
        Control::WorkflowStarted {
            project: setup.id.clone(),
            branch: "main".into(),
            run: Some(9),
            message: "Started release.yml on main".into(),
        }
    );
    // Then the run and the branch's list, as after a re-run.
    assert!(
        matches!(replies[1], Control::Run { run: 9, .. }),
        "{replies:?}"
    );
    assert!(
        matches!(&replies[2], Control::Runs { branch: Some(b), .. } if b == "main"),
        "{replies:?}"
    );
    let calls = setup.calls();
    let listed = [
        "run",
        "list",
        "--repo",
        "github.com/o/r",
        "--workflow=release.yml",
        "--branch=main",
        "--event=workflow_dispatch",
        "--limit",
        "20",
        "--json",
        "databaseId",
    ];
    assert_eq!(calls[0], listed);
    let run = [
        "workflow",
        "run",
        "release.yml",
        "--repo",
        "github.com/o/r",
        "--ref=main",
        "--raw-field",
        "level=-minor",
        "--raw-field",
        "note=a=b",
    ];
    assert_eq!(calls[1], run);
    assert_eq!(calls[2], listed);
    // GitHub not listing it yet: started, with no run, after a few looks.
    std::fs::remove_file(setup.tmp.path().join("next")).unwrap();
    let before = setup.calls().len();
    let replies = setup.start("release.yml", &[]);
    assert!(
        matches!(&replies[0], Control::WorkflowStarted { run: None, .. }),
        "{replies:?}"
    );
    assert!(matches!(replies[1], Control::Runs { .. }), "{replies:?}");
    assert_eq!(setup.calls().len(), before + 2 + LOOKS + 1);
}

#[cfg(unix)]
#[test]
fn a_start_is_refused_with_ghs_message_or_before_it_runs() {
    let setup = setup();
    let failed = |replies: Vec<Control>| match &replies[..] {
        [Control::WorkflowFailed { message, .. }] => message.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        failed(setup.start("broken.yml", &[])),
        "gh workflow run failed: HTTP 422: Workflow does not have workflow_dispatch trigger"
    );
    let calls = setup.calls().len();
    assert_eq!(
        failed(setup.start("-x.yml", &[])),
        "\"-x.yml\" is not a workflow of this repository"
    );
    assert_eq!(
        failed(setup.start("ci.yml", &[("a b", "x")])),
        "\"a b\" is not an input name"
    );
    let long = "v".repeat(VALUE_LIMIT + 1);
    assert_eq!(
        failed(setup.start("ci.yml", &[("a", &long)])),
        "The inputs' values are over 64 KiB"
    );
    assert_eq!(
        failed(setup.start("ci.yml", &[("a", "x\0y")])),
        "The value of a holds a NUL"
    );
    let many: Vec<(&str, &str)> = vec![("a", "b"); INPUTS_LIMIT + 1];
    assert_eq!(
        failed(setup.start("ci.yml", &many)),
        format!("At most {INPUTS_LIMIT} inputs can be given")
    );
    let bad_branch = setup.answer(Control::RunWorkflow {
        project: setup.id.clone(),
        workflow: "ci.yml".into(),
        branch: "a..b".into(),
        inputs: Vec::new(),
    });
    assert_eq!(
        failed(bad_branch),
        "\"a..b\" is not a branch name git accepts"
    );
    assert_eq!(setup.calls().len(), calls);
    // At the limits: sent.
    let most = "v".repeat(VALUE_LIMIT / INPUTS_LIMIT);
    let mut full: Vec<(&str, &str)> = vec![("a", most.as_str()); INPUTS_LIMIT - 1];
    let rest = "v".repeat(VALUE_LIMIT - most.len() * (INPUTS_LIMIT - 1));
    full.push(("b", rest.as_str()));
    assert!(matches!(
        &setup.start("ci.yml", &full)[0],
        Control::WorkflowStarted { .. }
    ));
    assert_eq!(setup.answer(Control::ListProjects), []);
    // The runs before cannot be listed: started all the same, not looked for.
    std::fs::write(setup.tmp.path().join("nolist"), "").unwrap();
    let calls = setup.calls().len();
    let replies = setup.start("ci.yml", &[]);
    assert!(
        matches!(&replies[0], Control::WorkflowStarted { run: None, .. }),
        "{replies:?}"
    );
    // The list before, the start and the branch's list: no looks.
    assert_eq!(setup.calls().len(), calls + 3);
}
