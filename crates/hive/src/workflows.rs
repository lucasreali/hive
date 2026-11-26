//! Running a workflow by hand from the Actions view (27.6, D27.3): the project's workflows
//! that take `workflow_dispatch`, read from a worktree's `HEAD` (its files are untrusted YAML,
//! read with size limits), and `gh workflow run` on the branch and inputs the user gave, as the
//! project's space account ([`crate::pulls::run`]). The new run is then looked for, so the
//! view can follow it as 9.32 does.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use hive_protocol::{Control, Workflow, WorkflowInput, WorkflowValue};
use serde_json::Value as Json;
use serde_norway::Value;

use crate::adapter::clip;
use crate::gh::Gh;
use crate::git;
use crate::projects::Projects;
use crate::pulls::{ACTION_OUTPUT, Cache, LINE_LIMIT, located, multiline, run};

/// Where the workflows are.
const FOLDER: &str = ".github/workflows/";
/// Most workflow files read, and the largest one.
const FILES_LIMIT: usize = 100;
const FILE_LIMIT: u64 = 256 << 10;
/// Most inputs of a workflow (GitHub's own limit is 25), and options of a choice.
const INPUTS_LIMIT: usize = 50;
const OPTIONS_LIMIT: usize = 100;
/// Most bytes of an input's description, and of all the values sent (GitHub's own limit is
/// 65,535 characters for the inputs; it also keeps `gh`'s arguments well within the OS's).
const DESCRIPTION_LIMIT: usize = 1024;
const VALUE_LIMIT: usize = 64 << 10;
/// How many times, and how long apart, GitHub is asked for the new run after it was started:
/// it lists a run a few seconds after the event.
const LOOKS: usize = 5;
pub const LOOK_EVERY: Duration = Duration::from_secs(2);

/// Answers `list_workflows` and `run_workflow`; `wait` is how long apart the new run is
/// looked for ([`LOOK_EVERY`]).
pub fn answer(
    gh: &Gh,
    projects: &Projects,
    cache: &Mutex<Cache>,
    request: Control,
    wait: Duration,
) -> Vec<Control> {
    match request {
        Control::ListWorkflows { project, worktree } => {
            let (workflows, error) = match list(projects, &project, &worktree) {
                Ok(workflows) => (workflows, None),
                Err(error) => (Vec::new(), Some(error)),
            };
            vec![Control::Workflows {
                project,
                worktree,
                workflows,
                error,
            }]
        }
        Control::RunWorkflow {
            project,
            workflow,
            branch,
            inputs,
        } => match start(gh, projects, &project, (&workflow, &branch, &inputs), wait) {
            Ok((run, message)) => {
                let started = Control::WorkflowStarted {
                    project: project.clone(),
                    branch: branch.clone(),
                    run,
                    message,
                };
                // Then the run (when found) and the branch's list, as after a re-run.
                let mut replies = vec![started];
                let follow = run.map(|run| Control::OpenRun {
                    project: project.clone(),
                    run,
                });
                let list = Control::ListRuns {
                    project,
                    branch: Some(branch),
                    force: true,
                };
                for request in follow.into_iter().chain([list]) {
                    replies.extend(crate::actions::answer(gh, projects, cache, request));
                }
                replies
            }
            Err(message) => vec![Control::WorkflowFailed { project, message }],
        },
        _ => Vec::new(),
    }
}

/// The workflows of the project `id` that can be run by hand, as its worktree `worktree`'s
/// `HEAD` has them, by file name; none before the first commit.
fn list(projects: &Projects, id: &str, worktree: &str) -> Result<Vec<Workflow>, String> {
    let (project, wt) = projects.holding(worktree).map_err(|err| err.to_string())?;
    if project.id != id {
        return Err(format!("{worktree} is not a worktree of {id}"));
    }
    let dir = Path::new(&wt.path);
    let args = ["ls-tree", "-z", "--name-only", "HEAD", "--", FOLDER];
    // No `HEAD` yet: no workflow.
    let Ok(out) = git::output(dir, &args, &[0]) else {
        return Ok(Vec::new());
    };
    let files = out.split(|&b| b == 0).filter_map(|path| {
        let path = std::str::from_utf8(path).ok()?;
        let file = path.strip_prefix(FOLDER)?;
        check_file(file).ok().map(|()| (path, file))
    });
    let mut workflows: Vec<Workflow> = files
        .take(FILES_LIMIT)
        .filter_map(|(path, file)| {
            let spec = format!("HEAD:{path}");
            let args = ["cat-file", "blob", spec.as_str()].map(std::ffi::OsStr::new);
            let bytes = git::run(dir, &args, &[], &[0], FILE_LIMIT).ok()?;
            parse(file, &bytes)
        })
        .collect();
    workflows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.file.cmp(&b.file)));
    Ok(workflows)
}

/// A workflow file name `gh workflow run` takes: `[A-Za-z0-9._-]`, `.yml` or `.yaml`, never
/// read as an option or a path.
fn check_file(file: &str) -> Result<(), String> {
    let chars = file
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    let named = file.ends_with(".yml") || file.ends_with(".yaml");
    match chars && named && !file.starts_with(['-', '.']) && file.len() <= LINE_LIMIT {
        true => Ok(()),
        false => Err(format!("{file:?} is not a workflow of this repository")),
    }
}

/// The workflow in the YAML `bytes` of `.github/workflows/<file>` when it takes
/// `workflow_dispatch` (`on: workflow_dispatch`, a list holding it, or a map with its inputs).
fn parse(file: &str, bytes: &[u8]) -> Option<Workflow> {
    let yaml: Value = serde_norway::from_slice(bytes).ok()?;
    // YAML 1.1 reads a bare `on` as `true`.
    let on = yaml.get("on").or_else(|| yaml.get(Value::Bool(true)))?;
    let dispatch = |value: &Value| value.as_str() == Some("workflow_dispatch");
    let inputs = match on {
        Value::String(_) if dispatch(on) => None,
        Value::Sequence(events) if events.iter().any(dispatch) => None,
        Value::Mapping(events) => Some(events.get("workflow_dispatch")?),
        _ => return None,
    };
    let inputs = inputs
        .and_then(|d| d.get("inputs"))
        .and_then(Value::as_mapping);
    let inputs = inputs.into_iter().flatten().take(INPUTS_LIMIT);
    let name = yaml
        .get("name")
        .and_then(Value::as_str)
        .map(|n| clip(n, LINE_LIMIT));
    Some(Workflow {
        file: file.to_owned(),
        name: name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| file.to_owned()),
        inputs: inputs
            .filter_map(|(name, spec)| input(name, spec))
            .collect(),
    })
}

/// A `workflow_dispatch` input, when its name is one [`check_name`] accepts.
fn input(name: &Value, spec: &Value) -> Option<WorkflowInput> {
    let name = name.as_str()?;
    check_name(name).ok()?;
    let text = |key: &str| spec.get(key).and_then(scalar);
    let options = spec.get("options").and_then(Value::as_sequence);
    let options = options.into_iter().flatten().filter_map(scalar);
    Some(WorkflowInput {
        name: name.to_owned(),
        description: multiline(&text("description").unwrap_or_default(), DESCRIPTION_LIMIT),
        required: spec.get("required").and_then(Value::as_bool) == Some(true),
        default: text("default").map(|d| clip(&d, LINE_LIMIT)),
        kind: match text("type").as_deref() {
            Some(kind @ ("boolean" | "choice" | "number" | "environment")) => kind.to_owned(),
            _ => "string".to_owned(),
        },
        options: options
            .take(OPTIONS_LIMIT)
            .map(|o| clip(&o, LINE_LIMIT))
            .collect(),
    })
}

/// A string, number or boolean as text.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// An input's name as GitHub allows them: letters, digits, `-` and `_`, starting with a
/// letter or `_`, at most 100 bytes.
fn check_name(name: &str) -> Result<(), String> {
    let first = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    let chars = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b));
    match first && chars && name.len() <= 100 {
        true => Ok(()),
        false => Err(format!("{name:?} is not an input name")),
    }
}

/// Starts the workflow `file` on `branch` with `inputs` (`gh workflow run`), then looks for
/// its run: the first of that workflow and branch that was not there before. Answers the run
/// (none when GitHub has not listed it yet) and what to show.
fn start(
    gh: &Gh,
    projects: &Projects,
    id: &str,
    (file, branch, inputs): (&str, &str, &[WorkflowValue]),
    wait: Duration,
) -> Result<(Option<u64>, String), String> {
    check_file(file)?;
    if inputs.len() > INPUTS_LIMIT {
        return Err(format!("At most {INPUTS_LIMIT} inputs can be given"));
    }
    let mut total = 0;
    for WorkflowValue { name, value } in inputs {
        check_name(name)?;
        if value.contains('\0') {
            return Err(format!("The value of {name} holds a NUL"));
        }
        total += value.len();
    }
    if total > VALUE_LIMIT {
        return Err("The inputs' values are over 64 KiB".to_owned());
    }
    let (project, env, repo) = located(projects, id)?;
    let root = Path::new(&project.path);
    crate::worktree::check_branch(root, branch)?;
    let repo = repo.arg();
    let workflow = format!("--workflow={file}");
    let only = format!("--branch={branch}");
    let runs = [
        "run",
        "list",
        "--repo",
        &repo,
        &workflow,
        &only,
        "--event=workflow_dispatch",
        "--limit",
        "20",
        "--json",
        "databaseId",
    ];
    let listed = || run(gh, &env, root, &runs, ACTION_OUTPUT).map(|out| ids(&out));
    // Without the runs before, a new one cannot be told apart: it is not looked for.
    let before = listed().ok();
    let fields: Vec<String> = inputs
        .iter()
        .map(|WorkflowValue { name, value }| format!("{name}={value}"))
        .collect();
    let refs = format!("--ref={branch}");
    let mut args = vec!["workflow", "run", file, "--repo", &repo, &refs];
    for field in &fields {
        args.extend(["--raw-field", field.as_str()]);
    }
    run(gh, &env, root, &args, ACTION_OUTPUT)?;
    let message = format!("Started {file} on {branch}");
    let Some(before) = before else {
        return Ok((None, message));
    };
    for _ in 0..LOOKS {
        std::thread::sleep(wait);
        let new = listed().ok().and_then(|now| {
            let mut new = now.into_iter().filter(|id| !before.contains(id));
            new.next()
        });
        if new.is_some() {
            return Ok((new, message));
        }
    }
    Ok((None, message))
}

/// The run ids of `gh run list --json databaseId`'s answer, newest first.
fn ids(out: &[u8]) -> Vec<u64> {
    let list: Json = serde_json::from_slice(out).unwrap_or_default();
    let runs = list.as_array().map(Vec::as_slice).unwrap_or_default();
    runs.iter()
        .filter_map(|run| run.get("databaseId").and_then(Json::as_u64))
        .collect()
}

#[cfg(test)]
mod tests;
