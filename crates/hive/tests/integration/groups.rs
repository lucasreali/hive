//! A folder of repositories followed as a group (14.1): its repositories come and go with
//! the folder, and its own terminals and agents are placed in it.

use hive_protocol::{Control, Project, Role};
use serde_json::json;

use crate::agents::hook;
use crate::common::Conn;
use crate::worktree::Repo;

/// The ids of `projects`, a group's marked `+`, a repository of a group's with its group.
fn shown(projects: &[Project]) -> Vec<String> {
    let shown = |p: &Project| match (&p.parent, p.group) {
        (_, true) => format!("+{}", p.id),
        (Some(group), _) => format!("{} in {group}", p.id),
        (None, _) => p.id.clone(),
    };
    projects.iter().map(shown).collect()
}

/// The next `projects`, as [`shown`].
async fn listed(app: &mut Conn) -> Vec<String> {
    match app.control().await {
        (0, Control::Projects { projects }) => shown(&projects),
        other => panic!("expected projects, got {other:?}"),
    }
}

#[tokio::test]
async fn a_group_follows_its_folder_and_holds_its_own_agents() {
    let repo = Repo::new();
    std::fs::create_dir(repo.env.path("home/work")).unwrap();
    let work = repo.env.path("home/work").canonicalize().unwrap();
    let init = |name: &str| {
        let dir = work.join(name);
        std::fs::create_dir(&dir).unwrap();
        repo.git_in(&dir, &["init", "-q", "-b", "main"]);
        dir.display().to_string()
    };
    let (api, web) = (init("api"), init("web"));
    std::fs::create_dir(work.join("docs")).unwrap();
    std::fs::create_dir(work.join("scratch")).unwrap();
    let g = work.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;

    // Added: the list with its repositories, then the group.
    app.send(0, Control::AddProject { path: g.clone() }).await;
    let inside = |ids: &[&str]| {
        let ids = ids.iter().map(|id| format!("{id} in {g}"));
        [format!("+{g}")].into_iter().chain(ids).collect::<Vec<_>>()
    };
    assert_eq!(listed(&mut app).await, inside(&[&api, &web]));
    let (0, Control::ProjectAdded { project }) = app.control().await else {
        panic!("expected project_added");
    };
    assert_eq!(shown(&[project]), [format!("+{g}")]);
    // A group's folder has no files panel: neither watched nor its ignored folders opened.
    let refused = Control::Error {
        message: format!("{g} is not a worktree of a followed project"),
    };
    let watch = Control::WatchWorktree {
        path: g.clone(),
        base: hive_protocol::DiffBase::Head,
    };
    app.send(0, watch).await;
    assert_eq!(app.control().await, (0, refused.clone()));
    let folders = vec!["node_modules".to_owned()];
    let expand = Control::ExpandIgnored {
        path: g.clone(),
        folders,
    };
    app.send(0, expand).await;
    assert_eq!(app.control().await, (0, refused));

    // A repository made in the folder joins it, one removed leaves it, without a refresh.
    let tools = init("tools");
    assert_eq!(listed(&mut app).await, inside(&[&api, &tools, &web]));
    std::fs::remove_dir_all(&tools).unwrap();
    assert_eq!(listed(&mut app).await, inside(&[&api, &web]));
    // So does a folder already there that becomes a repository, or stops being one.
    let scratch = work.join("scratch");
    repo.git_in(&scratch, &["init", "-q", "-b", "main"]);
    let scratch = scratch.display().to_string();
    assert_eq!(listed(&mut app).await, inside(&[&api, &scratch, &web]));
    std::fs::remove_dir_all(work.join("scratch/.git")).unwrap();
    assert_eq!(listed(&mut app).await, inside(&[&api, &web]));

    // A terminal opened on the group runs in its folder; its agent shows under the group.
    assert_eq!(app.open_terminal(1, &work).await, Some(g.clone()));
    app.input(1, "pwd\n").await;
    app.output_until(1, &format!("{g}\r\n")).await;
    let docs = work.join("docs").display().to_string();
    let start = json!({"session_id": "s1", "cwd": docs});
    let seen = hook(&repo, &mut app, "1", "SessionStart", start).await;
    let placed = Control::AgentDetected {
        id: "s1".into(),
        project: Some(g.clone()),
        worktree: Some(g.clone()),
        cwd: Some(docs.clone()),
    };
    assert_eq!(seen[0], (1, placed));

    // Its repository alone cannot be removed; the group goes with its repositories, once no
    // terminal works in it.
    let remove = |id: &str| Control::RemoveProject { id: id.into() };
    app.send(0, remove(&api)).await;
    let (0, Control::RemoveProjectFailed { message, .. }) = app.control().await else {
        panic!("expected remove_project_failed");
    };
    let said = format!("{api} is in the group {g}: remove the group to stop following it");
    assert_eq!(message, said);
    app.send(1, Control::CloseTerminal).await;
    while !matches!(app.control().await, (1, Control::TerminalExited { .. })) {}
    app.send(0, remove(&g)).await;
    let removed: Vec<Control> = [&api, &web, &g]
        .map(|id| Control::ProjectRemoved { id: id.clone() })
        .into();
    let mut seen = Vec::new();
    while seen.len() < removed.len() {
        seen.push(app.control().await.1);
    }
    assert_eq!(seen, removed);
    drop(app);
    assert!(daemon.wait_exit().success());
}
