use std::path::Path;

use hive_protocol::{DiffBase, Worktree};
use serde_json::json;

use super::*;
use crate::adapter::{Adapter, ClaudeCode};
use crate::health::tests::run;

const T0: u64 = 1_699_000_000_000;

fn h(lines: &[&str]) -> Vec<u64> {
    lines
        .iter()
        .map(|l| anchor::line_hash(l.as_bytes()))
        .collect()
}

fn edit(id: u64, runs: &[(usize, &[&str])]) -> Edit {
    Edit {
        id,
        session: "s1".into(),
        agent: None,
        agent_type: None,
        tool: "Edit".into(),
        tool_use_id: None,
        cwd: None,
        ms: T0,
        worktree: "/w".into(),
        path: "a.rs".into(),
        user_modified: false,
        added: runs
            .iter()
            .map(|(start, lines)| (*start, lines.len(), anchor::pack(&h(lines))))
            .collect(),
    }
}

fn map(lines: &[&str], origins: Vec<Origin>) -> FileMap {
    FileMap {
        size: 0,
        mtime_ms: 0,
        used_ms: 0,
        hashes: h(lines),
        origins,
    }
}

use Origin::{Before, Edit as E, Editor, Unknown};

const LONG_A: &str = "let alpha = compute_alpha();";
const LONG_B: &str = "let beta = compute_beta();";
const LONG_C: &str = "let gamma = compute_gamma();";

#[test]
fn a_first_edit_claims_its_lines_and_the_rest_was_there_before() {
    let file = h(&["a", LONG_A, LONG_B, "b"]);
    let e = edit(7, &[(2, &[LONG_A, LONG_B])]);
    let origins = follow(None, &file, Unknown, Some((&e, false)), &[]);
    assert_eq!(origins, [Before, E(7), E(7), Before]);
}

#[test]
fn lines_keep_their_origin_through_later_changes() {
    let old = map(&["a", LONG_A, "b"], vec![Before, E(1), Before]);
    let file = h(&["x", "a", LONG_A, "y", "b"]);
    let origins = follow(Some(&old), &file, Unknown, None, &[]);
    assert_eq!(origins, [Unknown, Before, E(1), Unknown, Before]);
    let saved = follow(Some(&old), &file, Editor(5), None, &[]);
    assert_eq!(saved, [Editor(5), Before, E(1), Editor(5), Before]);
}

#[test]
fn a_run_moved_by_a_formatter_is_found_nearest_its_line() {
    // The edit said line 2, but a hook's formatter put a line before it.
    let file = h(&[
        "a",
        "// added by a formatter",
        LONG_A,
        LONG_B,
        LONG_A,
        LONG_B,
    ]);
    let e = edit(3, &[(2, &[LONG_A, LONG_B])]);
    let origins = follow(None, &file, Unknown, Some((&e, false)), &[]);
    assert_eq!(origins, [Before, Before, E(3), E(3), Before, Before]);
}

#[test]
fn a_run_that_is_not_in_the_file_labels_nothing() {
    let file = h(&["a", "b", "c"]);
    let e = edit(3, &[(2, &[LONG_A])]);
    let origins = follow(None, &file, Unknown, Some((&e, false)), &[]);
    assert_eq!(origins, [Before, Unknown, Before]);
    // Past the file's end it changes nothing.
    let far = edit(4, &[(9, &[LONG_A])]);
    assert_eq!(
        follow(None, &file, Unknown, Some((&far, false)), &[]),
        [Before, Before, Before]
    );
    // With a map, the lines it would have covered that changed stay unknown.
    let old = map(&["a", "B", "c"], vec![E(1), E(1), E(1)]);
    assert_eq!(
        follow(Some(&old), &file, Unknown, Some((&e, false)), &[]),
        [E(1), Unknown, E(1)]
    );
}

#[test]
fn a_run_kept_as_a_range_only_claims_its_changed_lines() {
    let old = map(&["a", "b", "c"], vec![E(1), E(1), E(1)]);
    let file = h(&["a", "B", "C", "D"]);
    let mut e = edit(2, &[]);
    e.added = vec![(2, 2, String::new()), (9, 1, String::new())];
    let origins = follow(Some(&old), &file, Unknown, Some((&e, false)), &[]);
    assert_eq!(origins, [E(1), E(2), E(2), Unknown]);
}

#[test]
fn a_whole_file_write_claims_only_what_changed_since_the_map() {
    let old = map(&["a", LONG_A, "c"], vec![E(1), E(1), Editor(4)]);
    let file = h(&["a", LONG_B, "c", LONG_C]);
    let e = edit(2, &[(1, &["a", LONG_B, "c", LONG_C])]);
    let origins = follow(Some(&old), &file, Unknown, Some((&e, true)), &[]);
    assert_eq!(origins, [E(1), E(2), Editor(4), E(2)]);
    // Without a map every line is the session's, also where a formatter changed some.
    let formatted = h(&["a", LONG_B, "C", LONG_C]);
    let origins = follow(None, &formatted, Unknown, Some((&e, true)), &[]);
    assert_eq!(origins, [E(2), E(2), Before, E(2)]);
}

#[test]
fn text_that_comes_back_is_its_edit_again_but_not_a_lone_brace() {
    let history_edit = edit(5, &[(2, &[LONG_A, LONG_B]), (5, &["}"])]);
    let history = [&history_edit];
    // A `git stash pop` puts the lines back after a shell command removed them.
    let old = map(&["a", "b"], vec![Before, Before]);
    let file = h(&["a", LONG_A, LONG_B, "b", "}"]);
    let origins = follow(Some(&old), &file, Unknown, None, &history);
    assert_eq!(origins, [Before, E(5), E(5), Before, Unknown]);
    // Only unknown lines are re-claimed.
    let old = map(&["a", LONG_A, "b"], vec![Before, Editor(1), Before]);
    let origins = follow(Some(&old), &file, Unknown, None, &history);
    assert_eq!(origins, [Before, Editor(1), E(5), Before, Unknown]);
}

#[test]
fn maps_are_stored_as_runs_and_read_back() {
    let m = map(
        &["a", "b", "c", "d", "e"],
        vec![Before, Before, E(3), Editor(9), Unknown],
    );
    let stored = m.stored();
    assert_eq!(
        stored.origins,
        vec![
            (0, 2, StoredOrigin::Named("before".into())),
            (2, 1, StoredOrigin::Edit(3)),
            (3, 1, StoredOrigin::Named("editor@9".into())),
            (4, 1, StoredOrigin::Named("unknown".into())),
        ]
    );
    assert_eq!(FileMap::from_stored(&stored), Some(m.clone()));
    let empty = map(&[], vec![]);
    assert_eq!(FileMap::from_stored(&empty.stored()), Some(empty));
}

#[test]
fn a_stored_map_whose_runs_do_not_fit_its_lines_is_dropped() {
    let m = map(&["a", "b"], vec![Before, Before]);
    let with = |origins: Vec<(usize, usize, StoredOrigin)>| StoredMap {
        origins,
        ..m.stored()
    };
    let named = |n: &str| StoredOrigin::Named(n.into());
    assert!(FileMap::from_stored(&with(vec![(0, 1, named("before"))])).is_none());
    assert!(FileMap::from_stored(&with(vec![(0, 3, named("before"))])).is_none());
    assert!(FileMap::from_stored(&with(vec![(1, 2, named("before"))])).is_none());
    assert!(FileMap::from_stored(&with(vec![(0, 2, named("later"))])).is_none());
    assert!(FileMap::from_stored(&with(vec![(0, 2, named("editor@x"))])).is_none());
    let hashes = StoredMap {
        hashes: "AAA".into(),
        ..m.stored()
    };
    assert!(FileMap::from_stored(&hashes).is_none());
}

#[test]
fn the_run_around_a_line_has_its_origin() {
    let m = map(&["a", "b", "c", "d"], vec![Before, E(1), E(1), Before]);
    assert_eq!(m.run(0), 0..1);
    assert_eq!(m.run(1), 1..3);
    assert_eq!(m.run(2), 1..3);
    assert_eq!(m.run(3), 3..4);
}

#[test]
fn a_hook_edit_is_checked_before_it_is_believed() {
    let packed = anchor::pack(&[1, 2]);
    let good = json!({"file": "/w/a", "kind": "edit", "whole": false, "user_modified": true,
        "added": [[3, 2, packed]]});
    let claim = claimed(&good).unwrap();
    assert_eq!(claim.added, [(3, 2, packed.clone())]);
    assert!(claim.user_modified && !claim.whole && !claim.file_level);
    let notebook = json!({"file": "/w/n", "kind": "notebook", "added": [], "whole": true});
    let claim = claimed(&notebook).unwrap();
    assert!(claim.file_level && claim.whole && !claim.user_modified);
    // Hashes that do not unpack to the count keep the range only.
    let short = json!({"file": "/w/a", "kind": "create", "added": [[1, 3, packed]]});
    assert_eq!(claimed(&short).unwrap().added, [(1, 3, String::new())]);
    let last = FILE_LINES;
    let at_end = json!({"file": "/w/a", "kind": "edit", "added": [[last, 1, ""]]});
    assert!(claimed(&at_end).is_some());
    let many: Vec<Value> = (0..MAX_RUNS).map(|i| json!([i + 1, 1, ""])).collect();
    assert!(claimed(&json!({"file": "/w/a", "kind": "edit", "added": many})).is_some());
    let too_many: Vec<Value> = (0..=MAX_RUNS).map(|i| json!([i + 1, 1, ""])).collect();
    let long = format!("/{}", "a".repeat(PATH_LIMIT));
    for bad in [
        json!({"file": "a", "kind": "edit", "added": []}),
        json!({"file": long, "kind": "edit", "added": []}),
        json!({"file": 3, "kind": "edit", "added": []}),
        json!({"file": "/w/a", "kind": "delete", "added": []}),
        json!({"file": "/w/a", "added": []}),
        json!({"file": "/w/a", "kind": "edit"}),
        json!({"file": "/w/a", "kind": "edit", "added": too_many}),
        json!({"file": "/w/a", "kind": "edit", "added": [[0, 1, ""]]}),
        json!({"file": "/w/a", "kind": "edit", "added": [[1, 0, ""]]}),
        json!({"file": "/w/a", "kind": "edit", "added": [[last + 1, 1, ""]]}),
        json!({"file": "/w/a", "kind": "edit", "added": [[1, last + 1, ""]]}),
        json!({"file": "/w/a", "kind": "edit", "added": [["1", 1, ""]]}),
        json!({"file": "/w/a", "kind": "edit", "added": [[1, "1", ""]]}),
        json!({"file": "/w/a", "kind": "edit", "added": [[1, 1, 3]]}),
    ] {
        assert!(claimed(&bad).is_none(), "{bad}");
    }
    assert_eq!(
        claimed(&json!({"file": "/w/a", "kind": "edit", "added": [], "whole": true}))
            .map(|c| c.whole),
        Some(true)
    );
}

#[test]
fn paths_are_relative_and_never_in_git() {
    assert_eq!(relative("src/a.rs"), Some("src/a.rs"));
    for bad in [
        "",
        "/a",
        "../a",
        "a/../b",
        "./a",
        ".git/config",
        "a/.GIT/x",
        ".Git",
    ] {
        assert_eq!(relative(bad), None, "{bad}");
    }
    assert_eq!(
        relative(&"a".repeat(PATH_LIMIT)).map(str::len),
        Some(PATH_LIMIT)
    );
    assert_eq!(relative(&"a".repeat(PATH_LIMIT + 1)), None);
    assert_eq!(relative(".github/x"), Some(".github/x"));
}

/// A repository with one commit in a temporary folder, followed as a project.
struct Repo_ {
    _dir: tempfile::TempDir,
    root: PathBuf,
    projects: Vec<Project>,
}

impl Repo_ {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical(dir.path()).unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        let projects = vec![project(&root, &[])];
        Repo_ {
            _dir: dir,
            root,
            projects,
        }
    }

    fn worktree(&self) -> String {
        self.root.display().to_string()
    }

    fn write(&self, rel: &str, lines: &[&str]) {
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn commit(&self, message: &str) {
        run(&self.root, &["add", "-A"]);
        run(&self.root, &["commit", "-q", "-m", message]);
    }
}

fn project(root: &Path, others: &[&Path]) -> Project {
    let mut worktrees = vec![crate::health::tests::worktree(root, Some("main"), true)];
    worktrees.extend(
        others
            .iter()
            .map(|w| crate::health::tests::worktree(w, Some("other"), false)),
    );
    Project {
        id: root.display().to_string(),
        name: "repo".into(),
        path: root.display().to_string(),
        worktrees,
        error: None,
        group: false,
        parent: None,
        folder: false,
    }
}

/// The event `hive hook PostToolUse` forwards for an `Edit` (SYNTHETIC shape, made-up text),
/// slimmed as it is.
fn edited(session: &str, file: &Path, hunk: Value, original: &str) -> AgentEvent {
    let payload = json!({
        "session_id": session,
        "cwd": file.parent().unwrap(),
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_use_id": "toolu_01",
        "tool_input": {"file_path": file, "old_string": "x", "new_string": "y"},
        "tool_response": {"filePath": file, "originalFile": original,
            "structuredPatch": [hunk], "userModified": false},
    });
    ClaudeCode.translate("PostToolUse", None, crate::slim::slim(payload))
}

/// A `Write` creating `file` with `content` (SYNTHETIC shape).
fn created(session: &str, file: &Path, content: &str) -> AgentEvent {
    let payload = json!({
        "session_id": session,
        "agent_id": "a1",
        "agent_type": "general-purpose",
        "tool_name": "Write",
        "tool_use_id": "toolu_02",
        "tool_input": {"file_path": file, "content": content},
        "tool_response": {"type": "create", "filePath": file, "content": content,
            "structuredPatch": [], "originalFile": null},
    });
    ClaudeCode.translate("PostToolUse", None, crate::slim::slim(payload))
}

fn origins(store: &Provenance, repo: &Repo_, rel: &str) -> Vec<Origin> {
    let mut inner = store.inner();
    let repo = store.repo(&mut inner, &repo.worktree(), T0);
    repo.maps[&repo.project][rel].origins.clone()
}

fn no_title(_: &str, _: &str) -> (Option<String>, bool) {
    (None, false)
}

fn ask(store: &Provenance, repo: &Repo_, rel: &str, line: u32, side: LineSide) -> LineOrigin {
    let asked = Asked {
        worktree: repo.worktree(),
        path: rel.into(),
        line,
        side,
        base: DiffBase::Head,
    };
    match store.who_wrote(&repo.projects, &no_title, asked, T0 + 10_000) {
        Control::LineAuthor { origin, .. } => origin,
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_agents_edit_is_recorded_and_its_lines_answer_live() {
    let repo = Repo_::new();
    repo.write("src/a.rs", &["use x;", LONG_A, LONG_B, "}"]);
    let store = Provenance::default();
    let file = repo.root.join("src/a.rs");
    let hunk = json!({"oldStart": 1, "oldLines": 3, "newStart": 1, "newLines": 4,
        "lines": [" use x;", "-old", "+".to_owned() + LONG_A, "+".to_owned() + LONG_B, " }"]});
    store.saw(
        &repo.projects,
        &edited("s-1", &file, hunk, "use x;\nold\n}\n"),
        T0,
    );
    assert_eq!(
        origins(&store, &repo, "src/a.rs"),
        [Before, E(0), E(0), Before]
    );
    let LineOrigin::Session { edit, commit } =
        ask(&store, &repo, "src/a.rs", 3, LineSide::Worktree)
    else {
        panic!()
    };
    assert_eq!(commit, None);
    assert_eq!(
        *edit,
        LineEdit {
            session: "s-1".into(),
            title: None,
            deleted: false,
            project: Some(repo.worktree()),
            worktree: repo.worktree(),
            agent: None,
            agent_type: None,
            tool: "Edit".into(),
            tool_use_id: Some("toolu_01".into()),
            ms: T0,
            user_modified: false,
            start: 2,
            len: 2,
        }
    );
    let stored = &store.inner().repos[&repo.worktree()].edits[0];
    assert_eq!(
        stored.cwd,
        Some(repo.root.join("src").display().to_string())
    );
    assert_eq!(stored.path, "src/a.rs");
}

#[test]
fn a_new_file_is_the_sessions_and_its_subagent_is_named() {
    let repo = Repo_::new();
    let file = repo.root.join("new.txt");
    repo.write("new.txt", &[LONG_A, LONG_B]);
    let store = Provenance::default();
    let titled = |id: &str, cwd: &str| (Some(format!("{id} in {cwd}")), true);
    store.saw(
        &repo.projects,
        &created("s-2", &file, &format!("{LONG_A}\n{LONG_B}\n")),
        T0,
    );
    let asked = Asked {
        worktree: repo.worktree(),
        path: "new.txt".into(),
        line: 1,
        side: LineSide::Worktree,
        base: DiffBase::Head,
    };
    let Control::LineAuthor {
        origin: LineOrigin::Session { edit, .. },
        ..
    } = store.who_wrote(&repo.projects, &titled, asked, T0)
    else {
        panic!()
    };
    assert_eq!(edit.title, Some(format!("s-2 in {}", repo.worktree())));
    assert!(edit.deleted);
    assert_eq!(
        (edit.agent.as_deref(), edit.agent_type.as_deref()),
        (Some("a1"), Some("general-purpose"))
    );
    assert_eq!((edit.tool.as_str(), edit.start, edit.len), ("Write", 1, 2));
}

#[test]
fn edits_outside_followed_worktrees_or_of_other_shapes_are_left_out() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = canonical(elsewhere.path()).unwrap().join("x.txt");
    std::fs::write(&outside, "x\n").unwrap();
    store.saw(&repo.projects, &created("s", &outside, "x\n"), T0);
    // Inside `.git`: never.
    let git_file = repo.root.join(".git/description");
    store.saw(&repo.projects, &created("s", &git_file, "x\n"), T0);
    // A bad session id, a tool that edits nothing, a failed tool.
    repo.write("a.txt", &["x"]);
    let file = repo.root.join("a.txt");
    store.saw(&repo.projects, &created("../s", &file, "x\n"), T0);
    let mut read = created("s", &file, "x\n");
    read.kind = EventKind::ToolFinished {
        tool: Some("Read".into()),
    };
    store.saw(&repo.projects, &read, T0);
    let mut failed = created("s", &file, "x\n");
    failed.kind = EventKind::ToolFailed {
        tool: Some("Write".into()),
    };
    store.saw(&repo.projects, &failed, T0);
    let mut bad = created("s", &file, "x\n");
    bad.raw["hive_edit"]["kind"] = json!("delete");
    store.saw(&repo.projects, &bad, T0);
    let inner = store.inner();
    assert!(
        inner.repos.values().all(|r| r.edits.is_empty()),
        "{:?}",
        inner.repos
    );
    assert!(inner.dirty.is_empty());
}

#[cfg(unix)]
#[test]
fn a_fifo_or_a_link_out_of_the_worktree_is_never_read() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let fifo = repo.root.join("pipe");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
    store.saw(&repo.projects, &created("s", &fifo, "x\n"), T0);
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("t"), "x\n").unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("t"), repo.root.join("link")).unwrap();
    store.saw(
        &repo.projects,
        &created("s", &repo.root.join("link"), "x\n"),
        T0,
    );
    let inner = store.inner();
    assert!(inner.repos.values().all(|r| r.edits.is_empty()));
}

#[test]
fn a_file_past_the_cap_gets_a_file_level_record_only() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let lines: Vec<String> = (0..=FILE_LINES).map(|i| format!("{i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    repo.write("big.txt", &refs);
    let file = repo.root.join("big.txt");
    store.saw(&repo.projects, &created("s", &file, "0\n"), T0);
    let inner = store.inner();
    let r = &inner.repos[&repo.worktree()];
    assert_eq!(r.edits.len(), 1);
    assert!(r.edits[0].added.is_empty());
    assert!(!r.maps[&repo.worktree()].contains_key("big.txt"));
    drop(inner);
    let bytes = "x".repeat(FILE_BYTES as usize + 1);
    std::fs::write(repo.root.join("big.txt"), bytes).unwrap();
    assert!(matches!(look(&repo.root, "big.txt"), Ok(Look::TooLarge)));
    std::fs::write(repo.root.join("big.txt"), "x".repeat(FILE_BYTES as usize)).unwrap();
    assert!(matches!(look(&repo.root, "big.txt"), Ok(Look::Lines(_))));
    assert_eq!(
        look(&repo.root, "gone.txt").err().map(|e| e.kind()),
        Some(io::ErrorKind::NotFound)
    );
}

#[test]
fn lines_are_followed_through_shell_rewrites_editor_saves_and_later_edits() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A, LONG_B]);
    store.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n{LONG_B}\n")),
        T0,
    );
    // A Bash `sed -i` adds a line: unknown, seen at the next tool's end (a stat changed).
    repo.write("a.txt", &["sed line", LONG_A, LONG_B]);
    let mut bash = created("s1", &file, "");
    bash.kind = EventKind::ToolFinished {
        tool: Some("Bash".into()),
    };
    bash.cwd = Some(repo.worktree());
    store.saw(&repo.projects, &bash, T0 + 2000);
    assert_eq!(origins(&store, &repo, "a.txt"), [Unknown, E(0), E(0)]);
    // At most once a second.
    repo.write("a.txt", &["sed line 2", LONG_A, LONG_B]);
    store.saw(&repo.projects, &bash, T0 + 2500);
    assert_eq!(origins(&store, &repo, "a.txt"), [Unknown, E(0), E(0)]);
    // Hive's editor saves it.
    store.saved(&repo.projects, &repo.worktree(), "a.txt", T0 + 3000);
    assert_eq!(
        origins(&store, &repo, "a.txt"),
        [Editor(T0 + 3000), E(0), E(0)]
    );
    // Saving the same text again changes nothing.
    store.saved(&repo.projects, &repo.worktree(), "a.txt", T0 + 4000);
    assert_eq!(
        origins(&store, &repo, "a.txt"),
        [Editor(T0 + 3000), E(0), E(0)]
    );
    assert_eq!(
        ask(&store, &repo, "a.txt", 1, LineSide::Worktree),
        LineOrigin::Editor { ms: T0 + 3000 }
    );
    // A second session edits the same file: both keep their lines.
    repo.write("a.txt", &["sed line 2", LONG_A, LONG_C, LONG_B]);
    let hunk = json!({"oldStart": 2, "oldLines": 2, "newStart": 2, "newLines": 3,
        "lines": [" ".to_owned() + LONG_A, "+".to_owned() + LONG_C, " ".to_owned() + LONG_B]});
    store.saw(&repo.projects, &edited("s2", &file, hunk, ""), T0 + 5000);
    assert_eq!(
        origins(&store, &repo, "a.txt"),
        [Editor(T0 + 3000), E(0), E(1), E(0)]
    );
    // A file deleted meanwhile is no longer followed.
    std::fs::remove_file(&file).unwrap();
    let _ = ask(&store, &repo, "a.txt", 1, LineSide::Worktree);
    assert!(!store.inner().repos[&repo.worktree()].maps[&repo.worktree()].contains_key("a.txt"));
}

#[test]
fn an_editor_save_starts_following_a_file_and_bad_paths_are_ignored() {
    let repo = Repo_::new();
    let store = Provenance::default();
    repo.write("b.txt", &["one"]);
    store.saved(&repo.projects, &repo.worktree(), "b.txt", T0);
    assert_eq!(origins(&store, &repo, "b.txt"), [Before]);
    store.saved(&repo.projects, &repo.worktree(), "../b.txt", T0);
    store.saved(&repo.projects, "/elsewhere", "b.txt", T0);
    assert_eq!(store.inner().repos.len(), 1);
}

#[test]
fn records_survive_a_restart_and_a_malformed_file_is_set_aside() {
    let repo = Repo_::new();
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("provenance");
    let store = Provenance::new(Some(dir.clone()));
    store.flush();
    assert!(!dir.exists(), "nothing to write");
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A]);
    store.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    store.flush();
    let written = dir.join(format!("{}.json", key(&repo.worktree())));
    #[cfg(unix)]
    {
        let mode = |p: &Path| crate::mode::of(&std::fs::metadata(p).unwrap()) & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&written), 0o600);
    }
    let again = Provenance::new(Some(dir.clone()));
    assert_eq!(origins(&again, &repo, "a.txt"), [E(0)]);
    assert_eq!(again.inner().repos[&repo.worktree()].next_id, 1);
    // Malformed: set aside, and Hive starts empty.
    std::fs::write(&written, "{nope").unwrap();
    let fresh = Provenance::new(Some(dir.clone()));
    assert!(fresh.files_changed(&repo.projects, T0).is_empty());
    assert!(written.with_extension("json.bad").exists());
    assert!(!written.exists());
    // Another project's file under this name is not believed either.
    let other = Provenance::new(Some(dir.clone()));
    other.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    other.flush();
    let text = std::fs::read_to_string(&written).unwrap();
    std::fs::write(&written, text.replace(&repo.worktree(), "/elsewhere")).unwrap();
    assert!(
        Provenance::new(Some(dir))
            .files_changed(&repo.projects, T0)
            .is_empty()
    );
}

#[test]
fn a_worktree_gone_drops_its_maps_and_a_project_removed_its_file() {
    let repo = Repo_::new();
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("provenance");
    let store = Provenance::new(Some(dir.clone()));
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A]);
    store.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    store.flush();
    store.worktree_removed(&repo.worktree(), &repo.worktree(), T0);
    {
        let inner = store.inner();
        let r = &inner.repos[&repo.worktree()];
        assert!(r.maps.is_empty());
        assert_eq!(r.edits.len(), 1);
        assert!(inner.dirty.contains(&repo.worktree()));
    }
    store.flush();
    // Unchanged: nothing to write.
    store.worktree_removed(&repo.worktree(), &repo.worktree(), T0);
    assert!(store.inner().dirty.is_empty());
    let written = dir.join(format!("{}.json", key(&repo.worktree())));
    assert!(written.exists());
    store.project_removed(&repo.worktree());
    assert!(!written.exists());
    assert!(store.inner().repos.is_empty());
    // A stored worktree whose folder is gone is dropped on load.
    store.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    store.flush();
    let text = std::fs::read_to_string(&written).unwrap();
    let moved = text.replace(
        &format!("\"worktrees\":{{\"{}\"", repo.worktree()),
        "\"worktrees\":{\"/gone/worktree\"",
    );
    assert_ne!(moved, text);
    std::fs::write(&written, moved).unwrap();
    let again = Provenance::new(Some(dir.clone()));
    assert_eq!(again.files_changed(&repo.projects, T0)["s1"], 1);
    assert!(again.inner().repos[&repo.worktree()].maps.is_empty());
    // Every record gone: the file goes.
    let empty = Provenance::new(Some(dir));
    empty.files_changed(&repo.projects, T0 + MAX_AGE_MS + 1);
    empty.inner().dirty.insert(repo.worktree());
    empty.flush();
    assert!(!written.exists());
}

#[test]
fn old_and_surplus_records_go() {
    let edits = (0..MAX_RECORDS as u64 + 2)
        .map(|i| Edit {
            ms: T0 + i,
            ..edit(i, &[])
        })
        .collect();
    let mut repo = Repo {
        edits,
        ..Repo::default()
    };
    prune(&mut repo, T0);
    assert_eq!(repo.edits.len(), MAX_RECORDS);
    assert_eq!(repo.edits[0].id, 2);
    prune(&mut repo, T0 + MAX_AGE_MS + 2);
    assert_eq!(repo.edits[0].id, 2);
    prune(&mut repo, T0 + MAX_AGE_MS + 3);
    assert_eq!(repo.edits[0].id, 3);
}

#[test]
fn a_repository_past_its_cap_drops_the_oldest_maps_then_records() {
    let mut repo = Repo {
        project: "/p".into(),
        ..Repo::default()
    };
    let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    for (name, used) in [("old.txt", 1), ("new.txt", 2)] {
        let mut m = map(&refs, vec![Before; 200]);
        m.used_ms = used;
        repo.maps
            .entry("/w".into())
            .or_default()
            .insert(name.into(), m);
    }
    repo.edits = (0..20).map(|i| edit(i, &[(1, &refs[..10])])).collect();
    let full = serde_json::to_vec(&stored(&repo)).unwrap().len();
    assert_eq!(fit_within(&mut repo, full).len(), full);
    let json = fit_within(&mut repo, full - 1);
    assert!(json.len() < full);
    assert_eq!(repo.maps["/w"].keys().collect::<Vec<_>>(), ["new.txt"]);
    let json = fit_within(&mut repo, json.len() - 1);
    assert!(repo.maps.is_empty());
    assert_eq!(repo.edits.len(), 20);
    fit_within(&mut repo, json.len() - 1);
    assert_eq!(repo.edits.len(), 18);
    assert_eq!(repo.edits[0].id, 2);
    let json = fit_within(&mut repo, 10);
    assert!(repo.edits.is_empty());
    assert!(json.len() > 10);
}

#[test]
fn past_the_total_cap_the_least_recently_written_files_go() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, size: usize, age_s: u64| {
        let path = dir.path().join(name);
        std::fs::write(&path, vec![b'x'; size]).unwrap();
        let at = std::time::SystemTime::now() - Duration::from_secs(age_s);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    };
    write("a.json", 10, 30);
    write("b.json.bad", 10, 20);
    write("c.json", 10, 10);
    write("d.json", 10, 40);
    write("notes.txt", 100, 50);
    let kept: HashSet<String> = ["d.json".to_owned()].into();
    assert!(evict_files(dir.path(), &kept, 40).is_empty());
    let gone = evict_files(dir.path(), &kept, 20);
    assert_eq!(gone, ["a", "b.json.bad"]);
    assert!(dir.path().join("c.json").exists() && dir.path().join("d.json").exists());
    assert!(evict_files(&dir.path().join("none"), &kept, 0).is_empty());
}

#[test]
fn a_session_lists_the_files_it_changed() {
    let repo = Repo_::new();
    let store = Provenance::default();
    for (i, name) in ["a.txt", "b.txt"].iter().enumerate() {
        repo.write(name, &[LONG_A, LONG_B]);
        let file = repo.root.join(name);
        store.saw(
            &repo.projects,
            &created("s1", &file, &format!("{LONG_A}\n{LONG_B}\n")),
            T0 + i as u64,
        );
    }
    repo.write("a.txt", &["x", LONG_A, LONG_B]);
    let file = repo.root.join("a.txt");
    let hunk = json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": ["+x"]});
    store.saw(
        &repo.projects,
        &edited("s1", &file, hunk.clone(), ""),
        T0 + 5,
    );
    store.saw(&repo.projects, &edited("s2", &file, hunk, ""), T0 + 6);
    let Control::SessionFiles {
        id,
        files,
        truncated,
    } = store.session_files(&repo.projects, "s1".into(), T0)
    else {
        panic!()
    };
    assert_eq!((id.as_str(), truncated), ("s1", false));
    let w = repo.worktree();
    assert_eq!(
        files,
        [
            SessionFile {
                worktree: w.clone(),
                path: "a.txt".into(),
                edits: 2,
                lines_now: 2,
                first_line: Some(2),
                last_ms: T0 + 5
            },
            SessionFile {
                worktree: w,
                path: "b.txt".into(),
                edits: 1,
                lines_now: 2,
                first_line: Some(1),
                last_ms: T0 + 1
            },
        ]
    );
    let counts = store.files_changed(&repo.projects, T0);
    assert_eq!((counts["s1"], counts["s2"]), (2, 1));
    let Control::SessionFiles { files, .. } =
        store.session_files(&repo.projects, "none".into(), T0)
    else {
        panic!()
    };
    assert!(files.is_empty());
}

#[test]
fn at_most_five_hundred_files_are_listed() {
    let store = Provenance::default();
    let projects = vec![project(Path::new("/p"), &[])];
    {
        let mut inner = store.inner();
        let repo = store.repo(&mut inner, "/p", T0);
        repo.edits = (0..=FILES_LIMIT as u64)
            .map(|i| Edit {
                path: format!("f{i}"),
                ms: T0 + i,
                ..edit(i, &[])
            })
            .collect();
    }
    let Control::SessionFiles {
        files, truncated, ..
    } = store.session_files(&projects, "s1".into(), T0)
    else {
        panic!()
    };
    assert!(truncated);
    assert_eq!(files.len(), FILES_LIMIT);
    assert_eq!(files[0].path, format!("f{FILES_LIMIT}"));
    assert_eq!((files[0].lines_now, files[0].first_line), (0, None));
    // Exactly at the cap: all listed.
    store
        .inner()
        .repos
        .get_mut("/p")
        .unwrap()
        .edits
        .truncate(FILES_LIMIT);
    let Control::SessionFiles {
        files, truncated, ..
    } = store.session_files(&projects, "s1".into(), T0)
    else {
        panic!()
    };
    assert!(!truncated);
    assert_eq!(files.len(), FILES_LIMIT);
}

#[test]
fn blame_porcelain_is_read() {
    let out = b"1234abcd 7 9 1\nauthor Ann\nauthor-mail <a@b>\nauthor-time 1700000000\ncommitter-time 1700000100\nsummary fix the thing\nprevious 0000 a.rs\nfilename src/old name.rs\n\tlet x = 1;\n";
    let blamed = parse_blame(out).unwrap();
    assert_eq!(
        blamed,
        Blamed {
            commit: Some(LineCommit {
                id: "1234abcd".into(),
                author: "Ann".into(),
                ms: 1_700_000_000_000,
                summary: "fix the thing".into(),
            }),
            committed_ms: 1_700_000_100_000,
            path: "src/old name.rs".into(),
            line: 7,
            hash: anchor::line_hash(b"let x = 1;"),
        }
    );
    let uncommitted = b"0000000000000000000000000000000000000000 1 1 1\nauthor Not Committed Yet\nfilename a.rs\n\tx\n";
    assert_eq!(parse_blame(uncommitted).unwrap().commit, None);
    assert!(parse_blame(b"abc 1 1 1\nfilename a.rs\n").is_none());
    assert!(parse_blame(b"abc 1 1 1\n\tx\n").is_none());
    assert!(parse_blame(b"abc x 1 1\nfilename a\n\tx\n").is_none());
    assert!(parse_blame(b"abc").is_none());
    assert!(parse_blame(b"").is_none());
}

#[test]
fn after_a_commit_and_in_another_worktree_the_answer_is_probable() {
    let repo = Repo_::new();
    repo.write("a.txt", &["head"]);
    repo.commit("first");
    let store = Provenance::default();
    let file = repo.root.join("a.txt");
    let text = ["head", LONG_A, LONG_B, LONG_C, "}"];
    repo.write("a.txt", &text);
    let hunk = json!({"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 5, "lines": [
        " head", "+".to_owned() + LONG_A, "+".to_owned() + LONG_B, "+".to_owned() + LONG_C, "+}"]});
    store.saw(&repo.projects, &edited("s1", &file, hunk, "head\n"), T0);
    // Not committed yet, and not live (its map dropped, as for a removed worktree):
    store.worktree_removed(&repo.worktree(), &repo.worktree(), T0);
    assert_eq!(
        ask(&store, &repo, "a.txt", 3, LineSide::Worktree),
        LineOrigin::Unknown { commit: None }
    );
    repo.commit("the agent's work");
    let LineOrigin::Session { edit, commit } = ask(&store, &repo, "a.txt", 3, LineSide::Worktree)
    else {
        panic!()
    };
    let commit = commit.unwrap();
    assert_eq!(commit.summary, "the agent's work");
    assert!(commit.ms > 0 && !commit.author.is_empty());
    assert_eq!((edit.session.as_str(), edit.start, edit.len), ("s1", 3, 1));
    // A lone `}` is never matched through a commit; the first line predates the record.
    for line in [5, 1] {
        let LineOrigin::Unknown { commit: Some(c) } =
            ask(&store, &repo, "a.txt", line, LineSide::Worktree)
        else {
            panic!("{line}")
        };
        assert!(!c.id.is_empty());
    }
    // Past the end of the file, or a line 0: unknown.
    assert_eq!(
        ask(&store, &repo, "a.txt", 99, LineSide::Worktree),
        LineOrigin::Unknown { commit: None }
    );
    assert_eq!(
        ask(&store, &repo, "a.txt", 0, LineSide::Worktree),
        LineOrigin::Unknown { commit: None }
    );
    // The diff's old side, at the base: the same answer.
    std::fs::write(&file, "changed\n").unwrap();
    let LineOrigin::Session { edit, .. } = ask(&store, &repo, "a.txt", 2, LineSide::Base) else {
        panic!()
    };
    assert_eq!(edit.start, 2);
    // Records made after the commit (with the slack) are not in it.
    let later = Provenance::default();
    {
        let mut inner = later.inner();
        let r = later.repo(&mut inner, &repo.worktree(), T0);
        let mut e = edit_of(&store, &repo);
        e.ms = 1_700_000_000_000 + COMMIT_SLACK_MS + 1;
        r.edits.push(e);
    }
    assert!(matches!(
        ask(&later, &repo, "a.txt", 2, LineSide::Base),
        LineOrigin::Unknown { commit: Some(_) }
    ));
}

fn edit_of(store: &Provenance, repo: &Repo_) -> Edit {
    store.inner().repos[&repo.worktree()].edits[0].clone()
}

#[test]
fn a_rename_or_a_squash_still_finds_the_session_and_context_ranks_candidates() {
    let repo = Repo_::new();
    let filler: Vec<String> = (0..16)
        .map(|i| format!("unchanged filler line {i}"))
        .collect();
    let filler: Vec<&str> = filler.iter().map(String::as_str).collect();
    repo.write("old.txt", &[&["top", "mid"][..], &filler].concat());
    repo.commit("first");
    let store = Provenance::default();
    // Two sessions wrote the same line in different places of the file.
    let file = repo.root.join("old.txt");
    repo.write(
        "old.txt",
        &[&["top", LONG_A, LONG_B, "mid", LONG_C, LONG_A][..], &filler].concat(),
    );
    let first = json!({"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 3,
        "lines": [" top", "+".to_owned() + LONG_A, "+".to_owned() + LONG_B]});
    store.saw(&repo.projects, &edited("s1", &file, first, "top\n"), T0);
    let second = json!({"oldStart": 4, "oldLines": 0, "newStart": 5, "newLines": 2,
        "lines": ["+".to_owned() + LONG_C, "+".to_owned() + LONG_A]});
    store.saw(&repo.projects, &edited("s2", &file, second, ""), T0 + 1);
    run(&repo.root, &["mv", "old.txt", "new.txt"]);
    repo.commit("squashed and renamed");
    let who = |line| match ask(&store, &repo, "new.txt", line, LineSide::Worktree) {
        LineOrigin::Session { edit, .. } => edit.session,
        other => panic!("{other:?}"),
    };
    assert_eq!(who(2), "s1");
    assert_eq!(who(6), "s2");
    assert_eq!(who(3), "s1");
}

#[test]
fn an_unfollowed_worktree_or_a_bad_path_is_unknown() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let unknown = LineOrigin::Unknown { commit: None };
    assert_eq!(ask(&store, &repo, "../x", 1, LineSide::Worktree), unknown);
    let mut elsewhere = Repo_::new();
    elsewhere.projects.clear();
    assert_eq!(
        ask(&store, &elsewhere, "a.txt", 1, LineSide::Worktree),
        unknown
    );
    // The base side of a file in no commit.
    assert_eq!(ask(&store, &repo, "a.txt", 1, LineSide::Base), unknown);
}

#[test]
fn a_commits_file_gives_the_line_and_its_context() {
    let repo = Repo_::new();
    repo.write("a.txt", &["1", "2", "3", "4", "5", "6", "7", "8"]);
    repo.commit("c");
    let anchor = at_commit(&repo.root, "HEAD", "a.txt", 5).unwrap();
    assert_eq!(anchor.lines, h(&["5"]));
    assert_eq!(
        (anchor.before, anchor.after),
        (h(&["2", "3", "4"]), h(&["6", "7", "8"]))
    );
    assert!(at_commit(&repo.root, "HEAD", "a.txt", 9).is_none());
    assert!(at_commit(&repo.root, "HEAD", "a.txt", 0).is_none());
    assert!(at_commit(&repo.root, "HEAD", "none.txt", 1).is_none());
    std::fs::write(
        repo.root.join("big.txt"),
        "x".repeat(FILE_BYTES as usize + 1),
    )
    .unwrap();
    repo.commit("big");
    assert!(at_commit(&repo.root, "HEAD", "big.txt", 1).is_none());
}

#[test]
fn the_store_key_names_one_file_per_project() {
    assert_eq!(key("/a").len(), 16);
    assert_eq!(key("/a"), key("/a"));
    assert_ne!(key("/a"), key("/b"));
    let projects = vec![
        Project {
            group: true,
            ..project(Path::new("/g"), &[])
        },
        project(Path::new("/p"), &[Path::new("/p/.claude/worktrees/x")]),
    ];
    assert_eq!(
        project_of(&projects, "/p/.claude/worktrees/x").map(|p| p.id.as_str()),
        Some("/p")
    );
    assert!(project_of(&projects, "/g").is_none());
    let _ = Worktree::clone(&projects[1].worktrees[0]);
}

#[test]
fn hostile_numbers_in_a_hook_edit_are_refused() {
    let two = anchor::pack(&[1, 3]);
    let edit = |added: Value| json!({"file": "/w/a", "kind": "edit", "added": added});
    // Overflowing, out of order, overlapping.
    for bad in [
        json!([[u64::MAX, 2, two]]),
        json!([[1, u64::MAX, ""]]),
        json!([[5, 1, ""], [2, 1, ""]]),
        json!([[2, 2, ""], [3, 1, ""]]),
    ] {
        assert!(claimed(&edit(bad.clone())).is_none(), "{bad}");
    }
    assert!(claimed(&edit(json!([[2, 2, ""], [4, 1, ""]]))).is_some());
    // At most MAX_HASHED hashed lines; ranges only do not count.
    let run = |start: usize, n: usize| json!([start, n, anchor::pack(&vec![1; n])]);
    let half = MAX_HASHED / 2;
    assert!(claimed(&edit(json!([run(1, half), run(half + 1, half)]))).is_some());
    assert!(claimed(&edit(json!([run(1, half), run(half + 1, half + 1)]))).is_none());
    assert!(claimed(&edit(json!([run(1, half), [half + 1, half + 1, ""]]))).is_some());
    // A whole file is one run.
    let whole = json!({"file": "/w/a", "kind": "create", "whole": true,
        "added": [[1, 1, ""], [3, 1, ""]]});
    assert!(claimed(&whole).is_none());
}

#[test]
fn runs_outside_a_files_lines_are_left_out() {
    let mut e = edit(1, &[]);
    e.added = vec![
        (0, 1, String::new()),
        (usize::MAX, 2, String::new()),
        (FILE_LINES, 2, String::new()),
        (2, 0, String::new()),
        (FILE_LINES, 1, String::new()),
        (3, 2, anchor::pack(&[7, 8])),
    ];
    let runs: Vec<_> = e.runs().collect();
    assert_eq!(
        runs,
        [(FILE_LINES - 1..FILE_LINES, vec![]), (2..4, vec![7, 8])]
    );
}

#[test]
fn a_stored_map_with_an_overflowing_run_is_dropped() {
    let m = map(&["a"], vec![Before]);
    let hostile = StoredMap {
        origins: vec![
            (0, 1, StoredOrigin::Named("before".into())),
            (1, usize::MAX, StoredOrigin::Named("before".into())),
        ],
        ..m.stored()
    };
    assert!(FileMap::from_stored(&hostile).is_none());
}

#[test]
fn a_loaded_store_keeps_only_sound_records_and_never_reuses_an_id() {
    let repo = Repo_::new();
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("provenance");
    std::fs::create_dir_all(&dir).unwrap();
    let w = repo.worktree();
    let record = |id: u64, path: &str, session: &str| {
        json!({"id": id, "session": session, "tool": "Edit", "ms": T0, "worktree": w,
            "path": path, "added": []})
    };
    let hashes = anchor::pack(&h(&["x"]));
    let file_map = json!({"size": 0, "mtime_ms": 0, "used_ms": 0, "hashes": hashes,
        "origins": [[0, 1, 7]]});
    let mut files = serde_json::Map::new();
    files.insert("a.txt".into(), file_map.clone());
    files.insert(".git/config".into(), file_map);
    let mut worktrees = serde_json::Map::new();
    worktrees.insert(w.clone(), Value::Object(files));
    let stored = json!({"version": 1, "project": w, "next_id": 0,
        "edits": [record(7, "a.txt", "s1"), record(9, ".git/config", "s1"),
            record(8, "b.txt", "../s")],
        "worktrees": worktrees});
    std::fs::write(dir.join(format!("{}.json", key(&w))), stored.to_string()).unwrap();
    let store = Provenance::new(Some(dir));
    let mut inner = store.inner();
    let r = store.repo(&mut inner, &w, T0);
    assert_eq!(r.edits.iter().map(|e| e.id).collect::<Vec<_>>(), [7]);
    assert_eq!(r.next_id, 8);
    assert_eq!(r.maps[&w].keys().collect::<Vec<_>>(), ["a.txt"]);
}

#[test]
fn an_edit_searches_for_at_most_so_many_runs_away_from_their_line() {
    // Every run is somewhere else in the file: the first RECLAIM_RUNS are found.
    let lines: Vec<String> = (0..=RECLAIM_RUNS)
        .map(|i| format!("significant line {i}"))
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut file = vec!["pad"];
    file.extend(&refs);
    let runs: Vec<(usize, &[&str])> = (0..refs.len()).map(|i| (i + 3, &refs[i..=i])).collect();
    let e = edit(1, &runs);
    let origins = follow(None, &h(&file), Unknown, Some((&e, false)), &[]);
    assert_eq!(origins[1..=RECLAIM_RUNS], vec![E(1); RECLAIM_RUNS]);
    assert_eq!(origins[RECLAIM_RUNS + 1], Before);
    // Text that comes back: at most RECLAIM_RUNS runs of the history are tried.
    let old = map(&["pad"], vec![Before]);
    let back = follow(Some(&old), &h(&file), Unknown, None, &[&e]);
    assert_eq!(back[1..=RECLAIM_RUNS], vec![E(1); RECLAIM_RUNS]);
    assert_eq!(back[RECLAIM_RUNS + 1], Unknown);
}

#[test]
fn renames_are_read_from_name_status() {
    let out = b"M\0a.txt\0R087\0old.txt\0new.txt\0C100\0src.txt\0copy.txt\0";
    assert_eq!(parse_renames(out, "new.txt").as_deref(), Some("old.txt"));
    assert_eq!(parse_renames(out, "copy.txt").as_deref(), Some("src.txt"));
    assert_eq!(parse_renames(out, "a.txt"), None);
    assert_eq!(parse_renames(out, "old.txt"), None);
    assert_eq!(parse_renames(b"R087\0old.txt", "new.txt"), None);
    assert_eq!(parse_renames(b"M", "a.txt"), None);
    assert_eq!(parse_renames(b"M\0a.txt", "a.txt"), None);
}

#[test]
fn a_worktree_with_nothing_followed_has_nothing_to_look_at() {
    let store = Provenance::default();
    store.touch("/p", "/p", T0);
    let inner = store.inner();
    assert_eq!(inner.repos["/p"].touched["/p"], T0);
    assert!(inner.repos["/p"].maps.is_empty());
}

#[test]
fn an_unreadable_store_file_is_set_aside() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("provenance");
    let named = dir.join(format!("{}.json", key("/p")));
    std::fs::create_dir_all(&named).unwrap();
    let store = Provenance::new(Some(dir));
    assert!(
        store
            .files_changed(&[project(Path::new("/p"), &[])], T0)
            .is_empty()
    );
    assert!(named.with_extension("json.bad").is_dir());
}

#[cfg(unix)]
#[test]
fn a_store_that_cannot_be_written_keeps_its_records_in_memory() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo_::new();
    let home = tempfile::tempdir().unwrap();
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A]);
    // Its folder cannot be made: a file is in the way.
    std::fs::write(home.path().join("blocked"), "").unwrap();
    let blocked = Provenance::new(Some(home.path().join("blocked/provenance")));
    blocked.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    blocked.flush();
    assert!(blocked.inner().dirty.contains(&repo.worktree()));
    // Its folder cannot be written.
    let dir = home.path().join("provenance");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let read_only = Provenance::new(Some(dir.clone()));
    read_only.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    read_only.flush();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    assert_eq!(read_only.files_changed(&repo.projects, T0)["s1"], 1);
}

#[test]
fn past_the_total_cap_a_loaded_repository_is_dropped_from_memory_too() {
    let repo = Repo_::new();
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("provenance");
    let store = Provenance::new(Some(dir.clone()));
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A]);
    store.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n")),
        T0,
    );
    store.flush();
    let older = dir.join(format!("{}.json", key(&repo.worktree())));
    let at = std::time::SystemTime::now() - Duration::from_secs(60);
    std::fs::File::options()
        .write(true)
        .open(&older)
        .unwrap()
        .set_modified(at)
        .unwrap();
    // Another repository written now pushes the older one out.
    {
        let mut inner = store.inner();
        store
            .repo(&mut inner, "/other", T0)
            .edits
            .push(edit(1, &[]));
        inner.dirty.insert("/other".into());
    }
    store.flush_within(1);
    assert!(!older.exists());
    let inner = store.inner();
    assert!(!inner.repos.contains_key(&repo.worktree()));
    assert!(inner.repos.contains_key("/other"));
}

#[test]
fn a_live_line_without_its_record_or_from_before_goes_through_blame() {
    let repo = Repo_::new();
    repo.write("a.txt", &["head"]);
    repo.commit("first");
    let store = Provenance::default();
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &["head", LONG_A]);
    let hunk = json!({"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 2,
        "lines": [" head", "+".to_owned() + LONG_A]});
    store.saw(&repo.projects, &edited("s1", &file, hunk, "head\n"), T0);
    // Before Hive followed the file: the commit.
    let LineOrigin::Unknown { commit: Some(c) } =
        ask(&store, &repo, "a.txt", 1, LineSide::Worktree)
    else {
        panic!()
    };
    assert_eq!(c.summary, "first");
    // Its record went (age): not committed, so unknown.
    store
        .inner()
        .repos
        .get_mut(&repo.worktree())
        .unwrap()
        .edits
        .clear();
    assert_eq!(
        ask(&store, &repo, "a.txt", 2, LineSide::Worktree),
        LineOrigin::Unknown { commit: None }
    );
}

#[test]
fn a_file_too_large_at_its_commit_is_matched_on_the_line_alone() {
    let repo = Repo_::new();
    let filler = "x".repeat(100);
    let mut lines: Vec<&str> = vec![&filler; (FILE_BYTES as usize) / 100];
    lines.push(LONG_A);
    repo.write("big.txt", &lines);
    repo.commit("big");
    let store = Provenance::default();
    {
        let mut inner = store.inner();
        let r = store.repo(&mut inner, &repo.worktree(), T0);
        r.edits.push(Edit {
            path: "big.txt".into(),
            worktree: repo.worktree(),
            ..edit(0, &[(lines.len(), &[LONG_A])])
        });
    }
    let line = lines.len() as u32;
    let LineOrigin::Session { edit, commit } =
        ask(&store, &repo, "big.txt", line, LineSide::Worktree)
    else {
        panic!()
    };
    assert_eq!(
        (edit.start, commit.map(|c| c.summary)),
        (line, Some("big".into()))
    );
}

/// The caps the human approved (Q6, 2026-11-26) and the commit window.
#[test]
fn the_caps_are_the_approved_ones() {
    assert_eq!(FILE_BYTES, 2_097_152);
    assert_eq!(FILE_LINES, 50_000);
    assert_eq!(MAX_RECORDS, 20_000);
    assert_eq!(MAX_AGE_MS, 7_776_000_000);
    assert_eq!(REPO_BYTES, 8_388_608);
    assert_eq!(TOTAL_BYTES, 67_108_864);
    assert_eq!(COMMIT_SLACK_MS, 300_000);
    assert_eq!(STORE_READ, 16_777_216);
}

#[test]
fn a_hook_edit_path_may_be_as_long_as_the_limit() {
    let at = format!("/{}", "a".repeat(PATH_LIMIT - 1));
    let over = format!("/{}", "a".repeat(PATH_LIMIT));
    let edit = |file: &str| json!({"file": file, "kind": "edit", "added": []});
    assert!(claimed(&edit(&at)).is_some());
    assert!(claimed(&edit(&over)).is_none());
}

#[test]
fn a_file_rewritten_to_the_same_size_is_seen_by_its_time() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A, LONG_B]);
    store.saw(
        &repo.projects,
        &created("s1", &file, &format!("{LONG_A}\n{LONG_B}\n")),
        T0,
    );
    // Same length, other text, a later time.
    let other = LONG_B.replace("beta", "BETA");
    repo.write("a.txt", &[LONG_A, &other]);
    let later = std::time::SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert_eq!(LONG_B.len(), other.len());
    let mut stop = created("s1", &file, "");
    stop.kind = EventKind::TurnFinished;
    stop.cwd = Some(repo.worktree());
    store.saw(&repo.projects, &stop, T0 + 2000);
    assert_eq!(origins(&store, &repo, "a.txt"), [E(0), Unknown]);
    // Neither changed: not read again (a change of text alone is not seen).
    repo.write("a.txt", &[LONG_A, LONG_B]);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(later)
        .unwrap();
    store.saw(&repo.projects, &stop, T0 + 4000);
    assert_eq!(origins(&store, &repo, "a.txt"), [E(0), Unknown]);
    // Another size at the stored time: seen, exactly a second after the last look.
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    repo.write("a.txt", &["new first line", LONG_A, LONG_B]);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    store.saw(&repo.projects, &stop, T0 + 5000);
    // The session's two lines are back together: its own again.
    assert_eq!(origins(&store, &repo, "a.txt"), [Unknown, E(0), E(0)]);
}

#[test]
fn a_records_ids_and_folder_are_checked_again_in_the_service() {
    let repo = Repo_::new();
    let store = Provenance::default();
    let file = repo.root.join("a.txt");
    repo.write("a.txt", &[LONG_A]);
    let mut event = created("s1", &file, &format!("{LONG_A}\n"));
    // A frame not from `hive hook`: its id is not slimmed.
    event.raw["tool_use_id"] = json!("toolu bad");
    event.cwd = Some(repo.worktree());
    store.saw(&repo.projects, &event, T0);
    let e = edit_of(&store, &repo);
    assert_eq!((e.tool_use_id, e.cwd), (None, None));
    // A live line past the file's end goes through blame (uncommitted: unknown).
    assert_eq!(
        ask(&store, &repo, "a.txt", 2, LineSide::Worktree),
        LineOrigin::Unknown { commit: None }
    );
}

#[test]
fn text_from_another_file_or_worktree_is_never_reclaimed() {
    let repo = Repo_::new();
    let other = Repo_::new();
    let mut projects = repo.projects.clone();
    projects[0]
        .worktrees
        .push(crate::health::tests::worktree(&other.root, None, false));
    let store = Provenance::default();
    // An edit of b.txt, and of a.txt in another worktree, both writing LONG_A, LONG_B.
    let text = format!("{LONG_A}\n{LONG_B}\n");
    repo.write("b.txt", &[LONG_A, LONG_B]);
    store.saw(
        &projects,
        &created("s1", &repo.root.join("b.txt"), &text),
        T0,
    );
    other.write("a.txt", &[LONG_A, LONG_B]);
    store.saw(
        &projects,
        &created("s2", &other.root.join("a.txt"), &text),
        T0,
    );
    // a.txt here is followed, then a shell puts the same text in it.
    repo.write("a.txt", &["x"]);
    store.saved(&projects, &repo.worktree(), "a.txt", T0);
    repo.write("a.txt", &["x", LONG_A, LONG_B]);
    let _ = ask(&store, &repo, "a.txt", 1, LineSide::Worktree);
    assert_eq!(origins(&store, &repo, "a.txt"), [Before, Unknown, Unknown]);
    // An agent edit of it: the history is a.txt's of this worktree only.
    repo.write("a.txt", &["y", LONG_A, LONG_B]);
    let hunk = json!({"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1,
        "lines": ["-x", "+y"]});
    store.saw(
        &projects,
        &edited("s3", &repo.root.join("a.txt"), hunk, ""),
        T0,
    );
    assert_eq!(origins(&store, &repo, "a.txt"), [E(2), Unknown, Unknown]);
}
