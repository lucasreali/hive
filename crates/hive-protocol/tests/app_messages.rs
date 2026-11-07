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
        | StatuslineUsage
        | ListProjects
        | AddProject
        | RemoveProject
        | InitGit
        | CreateProject
        | CreateDir
        | CreateSpace
        | UpdateSpace
        | DeleteSpace
        | SelectSpace
        | ListClaudeLogins
        | ListGhAccounts
        | SwitchGhAccount
        | GetGhIdentity
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
        | CreateIssueWorktree
        | RemoveWorktree
        | RenameWorktree
        | WatchWorktree
        | UnwatchWorktree
        | ExpandIgnored
        | View
        | ListChanges
        | ListScripts
        | ListSessions
        | LocateSession
        | DeleteSession
        | MoveSession
        | SearchFiles
        | ListDirs
        | OpenFile
        | ReadFile
        | OpenTable
        | SaveFile
        | CreateFile
        | RenameFile
        | MoveFile
        | CreateFolder
        | DeleteFile
        | OpenInEditor
        | GetSettings
        | Resync
        | SetSettings
        | CreateAccount
        | RemoveAccount
        | OpenSettingsFile
        | GetDiagnostics
        | GetShared
        | OpenShared
        | ListUninstall
        | Uninstall
        | BrowserEndpoint
        | BrowserReady
        | WatchBrowser
        | BrowserDrawn;
    received:
        Welcome
        | VersionMismatch
        | TerminalOpened
        | TerminalExited
        | Badge
        | UnhookedAgent
        | TerminalProgress
        | CommandFinished
        | TerminalNotification
        | AgentDetected
        | AgentState
        | SubagentWorktrees
        | AgentTitle
        | AgentUsage
        | SessionUsage
        | UsageAlert
        | AgentRemoved
        | Projects
        | ProjectAdded
        | AddProjectFailed
        | ProjectCreated
        | DirCreated
        | CreateDirFailed
        | ProjectRemoved
        | RemoveProjectFailed
        | Spaces
        | SpaceFailed
        | ClaudeLogins
        | GhAccounts
        | GhIdentity
        | GhIdentityFailed
        | Notice
        | Pulls
        | Pull
        | PullDone
        | PullFailed
        | PullEnded
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
        | Scripts
        | Sessions
        | RestoreSessions
        | SessionLocated
        | SessionDeleted
        | DeleteSessionFailed
        | SessionMoved
        | MoveSessionFailed
        | SearchResults
        | Dirs
        | File
        | FileRange
        | Table
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
        | Shared
        | UninstallList
        | Uninstalled
        | BrowserStatus
        | BrowserFrame
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
                added: 3,
                removed: 4,
                ahead: None,
                behind: None,
                merged: false,
                last_commit_ms: 2,
            }),
        }],
        error: None,
        group: false,
        folder: false,
        parent: Some(s("/g")),
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
            warning: Some(s("cannot move Hive's files into /h/.hive")),
        },
        VersionMismatch {
            protocol: 1,
            version: s("0.4.0"),
        },
        TerminalOpened {
            worktree: Some(s("/r")),
            account: s("Work"),
        },
        TerminalExited { code: Some(0) },
        Badge { text: s("db") },
        UnhookedAgent,
        TerminalProgress {
            state: ProgressState::Normal,
            value: 40,
        },
        CommandFinished {
            exit: Some(1),
            secs: 12,
        },
        TerminalNotification {
            title: s("build"),
            body: s("done"),
            notify: true,
        },
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
            notify: false,
            subagents: vec![SubagentState {
                id: s("a"),
                agent_type: Some(s("Explore")),
                state: hive_protocol::AgentState::Working,
                worktree: None,
                activity: None,
                since_ms: 1,
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
        SessionUsage {
            usage: Some(SessionWindow {
                used_percentage: 42,
                resets_at: 1_738_425_600,
            }),
            week: Some(SessionWindow {
                used_percentage: 41,
                resets_at: 1_738_857_600,
            }),
            limit_at: Some(1_738_420_200),
            week_limit_at: None,
        },
        UsageAlert {
            window: UsageWindow::Session,
            threshold: 80,
            resets_at: 1_738_425_600,
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
        ProjectCreated {
            project: project(),
            path: s("/r"),
        },
        DirCreated {
            parent: s("/p"),
            name: s("n"),
        },
        CreateDirFailed {
            parent: s("/p"),
            name: s("n"),
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
                projects: vec![s("/r"), s("/g")],
                groups: vec![s("/g")],
                env: SpaceEnv::default(),
            }],
            current: s("default"),
        },
        SpaceFailed { message: m() },
        ClaudeLogins {
            logins: vec![ClaudeLogin {
                config_dir: None,
                logged_in: true,
            }],
        },
        GhAccounts {
            gh_config_dir: None,
            accounts: vec![],
            problem: None,
        },
        GhIdentity {
            account: GhAccount {
                host: s("github.com"),
                login: s("me"),
            },
            name: s("me"),
            email: s("1+me@users.noreply.github.com"),
        },
        GhIdentityFailed {
            account: GhAccount {
                host: s("github.com"),
                login: s("me"),
            },
            message: m(),
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
        PullEnded {
            project: s("/r"),
            worktree: s("/r/.claude/worktrees/a"),
            number: 1,
            title: s("Fix the login"),
            state: PullState::Merged,
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
            ignored: vec![s(".env"), s("target/")],
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
        Scripts {
            path: s("/r"),
            scripts: vec![Script {
                name: s("dev"),
                command: s("bun run dev"),
                source: ScriptSource::PackageJson,
            }],
            problems: vec![m()],
            port: Some(20_000),
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
        SessionMoved {
            id: s("s"),
            config_dir: Some(s("/w")),
        },
        MoveSessionFailed {
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
            warning: Some(m()),
            claude: false,
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
            media: Some(Media {
                mime: s("image/png"),
                token: s("t"),
                version: s("4-1"),
            }),
        },
        FileRange {
            id: 1,
            size: 4,
            mime: s("image/png"),
            version: s("4-1"),
            error: None,
        },
        Table {
            worktree: s("/r"),
            path: s("a.csv"),
            sheets: vec![Sheet {
                name: s(""),
                rows: vec![vec![s("a"), s("b")], vec![s("1")]],
                total: 3,
                partial: false,
            }],
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
            reveal: false,
            windows_path: None,
            error: Some(m()),
        },
        Settings {
            settings: hive_protocol::Settings::default(),
        },
        SettingsFailed { message: m() },
        Diagnostics {
            settings_file: s("/h/.hive/settings.json"),
            wrapper: s("/h/.hive/bin/claude"),
            claude: None,
            git_error: Some(m()),
        },
        Shared {
            folder: s("/h/.hive/shared"),
            plugin: s("hive-shared"),
            skills: vec![s("review")],
            agents: vec![s("tester")],
            commands: vec![s("about")],
            mcp_servers: vec![s("db")],
            error: Some(m()),
        },
        UninstallList {
            keep_data: false,
            accounts: true,
            items: vec![UninstallItem {
                path: s("/h/.hive/accounts/work"),
                kind: UninstallKind::Account,
            }],
            account_folders: vec![s("/h/.hive/accounts/work")],
        },
        Uninstalled {
            failed: vec![UninstallFailure {
                path: s("/Applications/Hive.app"),
                error: m(),
            }],
        },
        BrowserStatus {
            worktree: s("/r"),
            running: true,
            url: Some(s("http://localhost:5173/")),
            error: None,
        },
        BrowserFrame {
            worktree: s("/r"),
            data: s("/9j/4AAQ"),
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
