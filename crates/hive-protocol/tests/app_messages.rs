//! The protocol's contract with the app (9.25): one sample of every `Control` message the app
//! receives, written to `app-messages.json` next to this file. `src/protocol.test.ts` runs each
//! through the app's reducer and checks it against the app's types, so a message renamed, or a
//! field renamed, added or removed here fails the frontend's tests until the app follows.
//! Every message also has its row in `docs/architecture.md`'s message catalog (9.26).

use hive_protocol::*;

/// Every `Control` variant, sorted into the ones the app sends and the ones it receives
/// (service → app). It defines `to_app` (no wildcard: a new variant does not compile until it is
/// sorted here) and `VARIANTS`, whose every received one needs a sample in `samples`.
macro_rules! sort {
    (sent: $($sent:ident)|+; received: $($received:ident)|+;) => {
        /// Whether the app receives `message`.
        fn to_app(message: &Control) -> bool {
            match message {
                $(Control::$sent { .. })|+ => false,
                $(Control::$received { .. })|+ => true,
            }
        }
        /// Every variant, by name, with whether the app receives it.
        const VARIANTS: &[(&str, bool)] = &[
            $((stringify!($sent), false),)+
            $((stringify!($received), true),)+
        ];
    };
}

sort! {
    sent:
        Hello
        | OpenTerminal
        | Resize
        | CloseTerminal
        | Ack
        | Hook
        | ListProjects
        | AddProject
        | RemoveProject
        | CreateSpace
        | UpdateSpace
        | DeleteSpace
        | SelectSpace
        | ListGhAccounts
        | SwitchGhAccount
        | ListPulls
        | OpenPull
        | ActOnPull
        | CreatePull
        | ListRuns
        | OpenRun
        | OpenJobLog
        | ActOnRun
        | ListBranches
        | ValidateWorktreeName
        | CreateWorktree
        | RemoveWorktree
        | RenameWorktree
        | WatchWorktree
        | UnwatchWorktree
        | View
        | ListChanges
        | ListSessions
        | LocateSession
        | DeleteSession
        | SearchFiles
        | ListDirs
        | OpenFile
        | SaveFile
        | CreateFile
        | RenameFile
        | MoveFile
        | CreateFolder
        | DeleteFile
        | OpenInEditor
        | GetSettings
        | SetSettings
        | OpenSettingsFile
        | GetDiagnostics;
    received:
        Welcome
        | VersionMismatch
        | TerminalOpened
        | TerminalExited
        | Badge
        | UnhookedAgent
        | AgentDetected
        | AgentState
        | SubagentWorktrees
        | AgentTitle
        | AgentUsage
        | AgentRemoved
        | Projects
        | ProjectAdded
        | AddProjectFailed
        | ProjectRemoved
        | RemoveProjectFailed
        | Spaces
        | SpaceFailed
        | GhAccounts
        | Notice
        | Pulls
        | Pull
        | PullDone
        | PullFailed
        | Runs
        | Run
        | JobLog
        | RunDone
        | RunFailed
        | Branches
        | WorktreeNameValidated
        | WorktreeCreated
        | CreateWorktreeFailed
        | WorktreeRemoved
        | RemoveWorktreeFailed
        | WorktreeRenamed
        | RenameWorktreeFailed
        | WorktreeStatus
        | Files
        | Changes
        | Sessions
        | RestoreSessions
        | SessionLocated
        | SessionDeleted
        | DeleteSessionFailed
        | SearchResults
        | Dirs
        | File
        | FileSaved
        | SaveFailed
        | FileCreated
        | FileRenamed
        | FolderCreated
        | FileDeleted
        | FileOpFailed
        | EditorTarget
        | Settings
        | SettingsFailed
        | Diagnostics
        | Error;
}

fn s(text: &str) -> String {
    text.to_owned()
}

fn project() -> Project {
    Project {
        id: s("/r"),
        name: s("r"),
        path: s("/r"),
        worktrees: vec![Worktree {
            id: s("/r"),
            name: s("main"),
            path: s("/r"),
            branch: Some(s("main")),
            main: true,
            claude: false,
            status: Some(WorktreeStatus {
                changes: 1,
                ahead: None,
                behind: None,
                merged: false,
                last_commit_ms: 2,
            }),
        }],
        error: None,
    }
}

/// One of every message the app receives, with nested values kept small.
fn samples() -> Vec<Control> {
    use Control::*;
    let m = || s("m");
    vec![
        Welcome {
            version: s("0.4.0"),
            distro: Some(s("Ubuntu")),
        },
        VersionMismatch {
            protocol: 1,
            version: s("0.4.0"),
        },
        TerminalOpened {
            worktree: Some(s("/r")),
        },
        TerminalExited { code: Some(0) },
        Badge { text: s("db") },
        UnhookedAgent,
        AgentDetected {
            id: s("s"),
            project: Some(s("/r")),
            worktree: Some(s("/r")),
            cwd: Some(s("/r")),
        },
        AgentState {
            id: s("s"),
            state: hive_protocol::AgentState::Working,
            urgency: 2,
            pending: false,
            interrupted: false,
            alert: None,
            writing: true,
            subagents: vec![SubagentState {
                id: s("a"),
                agent_type: Some(s("Explore")),
                state: hive_protocol::AgentState::Working,
                worktree: None,
                activity: None,
                since_ms: 1,
                writing: false,
            }],
            activity: Some(s("Reading a.rs")),
            since_ms: 1,
        },
        SubagentWorktrees { worktrees: vec![] },
        AgentTitle {
            id: s("s"),
            title: s("t"),
        },
        AgentUsage {
            id: s("s"),
            context_tokens: 1,
            context_limit: 200_000,
            output_tokens: 2,
        },
        AgentRemoved { id: s("s") },
        Projects {
            projects: vec![project()],
        },
        ProjectAdded { project: project() },
        AddProjectFailed {
            path: s("/x"),
            error: ProjectError::NotFound,
            message: m(),
        },
        ProjectRemoved { id: s("/r") },
        RemoveProjectFailed {
            id: s("/r"),
            message: m(),
        },
        Spaces {
            spaces: vec![Space {
                id: s("default"),
                name: s("Default"),
                projects: vec![s("/r")],
                env: SpaceEnv::default(),
            }],
            current: s("default"),
        },
        SpaceFailed { message: m() },
        GhAccounts {
            gh_config_dir: None,
            accounts: vec![],
            problem: None,
        },
        Notice { message: m() },
        Pulls {
            project: s("/r"),
            repo: None,
            mine: vec![],
            review: vec![],
            fetched_ms: 0,
            error: None,
        },
        Pull {
            project: s("/r"),
            number: 1,
            pull: None,
            error: Some(m()),
        },
        PullDone {
            project: s("/r"),
            number: 1,
            message: m(),
        },
        PullFailed {
            project: s("/r"),
            number: None,
            message: m(),
        },
        Runs {
            project: s("/r"),
            branch: None,
            runs: vec![],
            fetched_ms: 0,
            error: None,
        },
        Run {
            project: s("/r"),
            run: 1,
            detail: None,
            error: Some(m()),
        },
        JobLog {
            project: s("/r"),
            job: 1,
            log: Some(s("log")),
            error: None,
        },
        RunDone {
            project: s("/r"),
            run: 1,
            message: m(),
        },
        RunFailed {
            project: s("/r"),
            run: 1,
            message: m(),
        },
        Branches {
            project: s("/r"),
            local: vec![s("main")],
            remote: vec![],
            current: Some(s("main")),
            error: None,
        },
        WorktreeNameValidated {
            project: s("/r"),
            name: s("x"),
            folder: s(".claude/worktrees/x/"),
            branch: s("worktree-x"),
            error: None,
        },
        WorktreeCreated {
            project: project(),
            path: s("/r/.claude/worktrees/x"),
            notes: vec![],
        },
        CreateWorktreeFailed {
            project: s("/r"),
            name: s("x"),
            message: m(),
        },
        WorktreeRemoved {
            project: project(),
            path: s("/r/.claude/worktrees/x"),
        },
        RemoveWorktreeFailed {
            path: s("/r/.claude/worktrees/x"),
            message: m(),
        },
        WorktreeRenamed {
            project: project(),
            from: s("/r/.claude/worktrees/x"),
            path: s("/r/.claude/worktrees/y"),
        },
        RenameWorktreeFailed {
            path: s("/r/.claude/worktrees/x"),
            name: s("y"),
            message: m(),
        },
        WorktreeStatus {
            path: s("/r"),
            status: None,
        },
        Files {
            path: s("/r"),
            files: vec![s("a.rs")],
            truncated: false,
        },
        Changes {
            path: s("/r"),
            base: DiffBase::Head,
            branch: None,
            base_error: None,
            files: vec![],
            added: 0,
            removed: 0,
            error: None,
        },
        Sessions {
            sessions: vec![],
            error: None,
            truncated: false,
        },
        RestoreSessions { sessions: vec![] },
        SessionLocated {
            id: s("s"),
            target: SessionTarget::Log,
            windows_path: None,
            error: Some(m()),
        },
        SessionDeleted { id: s("s") },
        DeleteSessionFailed {
            id: s("s"),
            message: m(),
        },
        SearchResults {
            worktree: s("/r"),
            query: s("q"),
            matches: vec![],
            truncated: false,
            error: None,
        },
        Dirs {
            path: s("/"),
            windows: false,
            linux_path: Some(s("/")),
            parent: None,
            dirs: vec![],
            error: None,
        },
        File {
            worktree: s("/r"),
            path: s("a.rs"),
            content: Some(s("a")),
            base: None,
            version: Some(s("v")),
            binary: false,
            too_large: false,
            error: None,
        },
        FileSaved {
            worktree: s("/r"),
            path: s("a.rs"),
            version: s("v"),
        },
        SaveFailed {
            worktree: s("/r"),
            path: s("a.rs"),
            error: SaveError::Conflict,
            message: m(),
        },
        FileCreated {
            worktree: s("/r"),
            path: s("b.rs"),
        },
        FileRenamed {
            worktree: s("/r"),
            path: s("b.rs"),
            to: s("c.rs"),
        },
        FolderCreated {
            worktree: s("/r"),
            path: s("d"),
        },
        FileDeleted {
            worktree: s("/r"),
            path: s("c.rs"),
        },
        FileOpFailed {
            worktree: s("/r"),
            message: m(),
        },
        EditorTarget {
            worktree: s("/r"),
            path: s("a.rs"),
            windows_path: None,
            error: Some(m()),
        },
        Settings {
            settings: hive_protocol::Settings::default(),
        },
        SettingsFailed { message: m() },
        Diagnostics {
            settings_file: s("/h/.config/hive/settings.json"),
            wrapper: s("/h/.local/share/hive/bin/claude"),
            claude: None,
        },
        Error { message: m() },
    ]
}

/// The fixture is rewritten from `samples`, and the test fails when that changed it: commit the
/// new file, and the app's types with it.
#[test]
fn the_app_messages_fixture_holds_one_sample_of_each_message_the_app_receives() {
    let samples = samples();
    assert!(samples.iter().all(to_app));
    assert!(!to_app(&Control::ListProjects));
    let json = serde_json::to_value(&samples).unwrap();
    let mut types: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    types.sort_unstable();
    types.dedup();
    assert_eq!(types.len(), samples.len(), "one sample per message");
    let mut received: Vec<String> = VARIANTS
        .iter()
        .filter(|(_, to_app)| *to_app)
        .map(|(name, _)| tag(name))
        .collect();
    received.sort_unstable();
    assert_eq!(
        types, received,
        "every message the app receives has a sample"
    );

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/app-messages.json");
    let before = std::fs::read_to_string(path).unwrap_or_default();
    let now = serde_json::to_string_pretty(&samples).unwrap() + "\n";
    std::fs::write(path, &now).unwrap();
    assert!(
        before == now,
        "{path} was stale and has been rewritten: commit it"
    );
}

/// A variant's `"type"`: its name in snake_case, as `#[serde(rename_all = "snake_case")]` gives it.
fn tag(name: &str) -> String {
    let mut tag = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() && !tag.is_empty() {
            tag.push('_');
        }
        tag.push(c.to_ascii_lowercase());
    }
    tag
}

/// Each row of `docs/architecture.md`'s message catalog: the message's `"type"` and whether it
/// goes to the app (its direction ends at the app or a client).
fn catalog() -> Vec<(String, bool)> {
    let doc = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/architecture.md"
    ));
    let table = doc
        .split("### Message catalog")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").find(|part| part.starts_with('|')))
        .unwrap_or_default();
    table
        .lines()
        .filter_map(|row| {
            let mut cells = row.split(" | ");
            let message = cells.next()?.strip_prefix("| `")?;
            let tag = message.split([' ', '`']).next()?;
            let direction = cells.next()?;
            Some((
                tag.to_owned(),
                direction.ends_with("app") || direction.ends_with("client"),
            ))
        })
        .collect()
}

/// The catalog lists every `Control` message once, in the direction `to_app` sorts it, and no
/// message that no longer exists.
#[test]
fn the_architecture_catalog_lists_every_message_exactly() {
    let mut catalog = catalog();
    catalog.sort_unstable();
    let mut code: Vec<(String, bool)> = VARIANTS
        .iter()
        .map(|&(name, to_app)| (tag(name), to_app))
        .collect();
    code.sort_unstable();
    let missing: Vec<_> = code.iter().filter(|m| !catalog.contains(m)).collect();
    let stale: Vec<_> = catalog.iter().filter(|m| !code.contains(m)).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "not in the catalog as (type, to the app): {missing:?}; in the catalog, not in the code: {stale:?}"
    );
    assert_eq!(catalog, code, "one row per message");
    assert_eq!(tag("RenameWorktreeFailed"), "rename_worktree_failed");
}
