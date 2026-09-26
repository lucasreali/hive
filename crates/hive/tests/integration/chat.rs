//! The in-app chat (7.3) through a real daemon, with a fake `claude` on a `PATH` of its own:
//! the real one is never run.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use hive_protocol::{AgentState, ChatAnswer, ChatEntry, ChatEntryKind, ChatMode, Control, Role};
use serde_json::json;

use crate::agents::hook;
use crate::common::Conn;
use crate::worktree::Repo;

const SESSION: &str = "9f1c2b7e-5d3a-4c1e-8b2f-0a6d4e8c1f00";

/// A fake `claude` (`sh`): writes its chat id and arguments to `args`, answers `initialize`
/// with the `text` fixture's first line, replays the rest after the first user turn, fails on
/// a turn holding "crash", and ends when its stdin closes.
fn fake_claude(fake: &Path) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat/text.jsonl");
    format!(
        r#"#!/bin/sh
printf '%s\n' "$HIVE_TERMINAL_ID" "$HIVE_WRAPPED" "$@" > '{args}'
exec 3< '{fixture}'
IFS= read -r line <&3
IFS= read -r request
printf '%s\n' "$line"
while IFS= read -r request; do
    case $request in
    *crash*) echo boom >&2; exit 3 ;;
    *'"type":"user"'*) while IFS= read -r line <&3; do printf '%s\n' "$line"; done ;;
    esac
done
"#,
        args = fake.join("args").display(),
        fixture = fixture.display(),
    )
}

/// The service's whole `PATH`: `git`, `fish` and a fake `claude`.
fn sandbox(repo: &Repo) -> PathBuf {
    let dir = repo.env.path("fake");
    std::fs::create_dir(&dir).unwrap();
    let path = std::env::var_os("PATH").unwrap();
    for tool in ["git", "fish"] {
        let found = std::env::split_paths(&path)
            .map(|d| d.join(tool))
            .find(|p| p.is_file())
            .unwrap();
        std::os::unix::fs::symlink(found, dir.join(tool)).unwrap();
    }
    write_claude(&dir, &fake_claude(&dir));
    dir
}

fn write_claude(dir: &Path, script: &str) {
    let claude = dir.join("claude");
    std::fs::write(&claude, script).unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn open(cwd: &str, resume: Option<&str>, mode: Option<ChatMode>) -> Control {
    Control::OpenChat {
        cwd: cwd.into(),
        resume: resume.map(str::to_owned),
        mode,
    }
}

fn confirm(chat: u32, cwd: &str, accepted: bool) -> Control {
    Control::ConfirmChatFolder {
        chat,
        cwd: cwd.into(),
        accepted: Some(accepted),
    }
}

fn asked(chat: u32, cwd: &str) -> (u32, Control) {
    let ask = Control::ConfirmChatFolder {
        chat,
        cwd: cwd.into(),
        accepted: None,
    };
    (chat, ask)
}

fn closed(chat: u32, error: Option<&str>) -> (u32, Control) {
    let error = error.map(str::to_owned);
    (chat, Control::ChatClosed { chat, error })
}

fn error(chat: u32, message: &str) -> (u32, Control) {
    let message = message.to_owned();
    (chat, Control::Error { message })
}

fn opened(chat: u32, cwd: &str, session: Option<&str>, mode: ChatMode) -> (u32, Control) {
    let opened = Control::ChatOpened {
        chat,
        cwd: cwd.into(),
        session: session.map(str::to_owned),
        model: None,
        mode,
        commands: vec!["compact".into()],
        api_key_source: None,
    };
    (chat, opened)
}

fn entries(chat: u32, entries: Vec<ChatEntry>) -> (u32, Control) {
    let entries = Control::ChatEntries {
        chat,
        entries,
        replace_last: false,
    };
    (chat, entries)
}

fn entry(id: u32, kind: ChatEntryKind, text: &str) -> ChatEntry {
    ChatEntry {
        id,
        kind,
        text: text.into(),
        tool: None,
        parent: None,
        status: None,
        output: None,
        image: None,
    }
}

fn status(chat: u32, busy: bool, mode: ChatMode, model: bool) -> (u32, Control) {
    let status = Control::ChatStatus {
        chat,
        busy,
        mode,
        model: model.then(|| "claude-haiku-4-5".into()),
        retry: None,
        compacting: false,
        session: Some(SESSION.into()),
        api_key_source: None,
    };
    (chat, status)
}

async fn send(app: &mut Conn, chat: u32, text: &str) {
    let text = text.into();
    let send = Control::ChatSend {
        chat,
        text,
        images: vec![],
    };
    app.send(chat, send).await;
}

#[tokio::test]
async fn a_chat_is_allowed_per_project_then_follows_claudes_stream() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let fake = sandbox(&repo);
    let mut daemon = repo.env.daemon_on_path(&fake);
    let mut app = repo.env.connect(Role::App).await;

    // Only in a worktree of an added project.
    app.send(2, open(&root, None, None)).await;
    let outside = format!("{root} is not a worktree of an added project: chats open only there");
    assert_eq!(app.control().await, closed(2, Some(&outside)));
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    let inside = format!("{root}/src");
    app.send(2, open(&inside, None, None)).await;
    let inside = format!("{inside} is not a worktree of an added project: chats open only there");
    assert_eq!(app.control().await, closed(2, Some(&inside)));
    app.send(0, open(&root, None, None)).await;
    assert_eq!(app.control().await, error(0, "chat channels start at 1"));
    app.send(2, open(&root, Some("../x"), None)).await;
    let not_session = Some("not a session id to resume");
    assert_eq!(app.control().await, closed(2, not_session));

    // The first chat in the project asks; a refusal or a close ends it.
    app.send(2, open(&root, None, None)).await;
    assert_eq!(app.control().await, asked(2, &root));
    app.send(2, open(&root, None, None)).await;
    assert_eq!(app.control().await, error(2, "chat 2 is already open"));
    app.send(2, confirm(2, "/elsewhere", true)).await;
    assert_eq!(
        app.control().await,
        error(2, "no chat waits for this folder")
    );
    app.send(2, confirm(2, &root, false)).await;
    assert_eq!(app.control().await, closed(2, None));
    app.send(3, open(&root, None, None)).await;
    assert_eq!(app.control().await, asked(3, &root));
    app.send(3, Control::CloseChat { chat: 3 }).await;
    assert_eq!(app.control().await, closed(3, None));

    // Allowing it saves the settings; when they cannot be saved the chat does not start.
    let config = repo.env.path("config/hive");
    std::fs::write(&config, "").unwrap();
    app.send(3, open(&root, None, None)).await;
    assert_eq!(app.control().await, asked(3, &root));
    app.send(3, confirm(3, &root, true)).await;
    let (
        3,
        Control::ChatClosed {
            error: Some(why), ..
        },
    ) = app.control().await
    else {
        panic!("the chat started without its settings")
    };
    assert!(why.starts_with("Cannot save "), "{why}");
    std::fs::remove_file(&config).unwrap();
    app.send(4, open(&root, Some(SESSION), Some(ChatMode::AcceptEdits)))
        .await;
    assert_eq!(app.control().await, asked(4, &root));
    app.send(4, confirm(4, &root, true)).await;
    let (0, Control::Settings { settings }) = app.control().await else {
        panic!("no settings")
    };
    assert!(settings.projects[&root].chat_confirmed);
    let accept = ChatMode::AcceptEdits;
    assert_eq!(app.control().await, opened(4, &root, Some(SESSION), accept));
    let args = std::fs::read_to_string(fake.join("args")).unwrap();
    let hooks = repo.env.path("data/hive/hive-hooks.json");
    let expected = format!(
        "4\n1\n-p\n--input-format\nstream-json\n--output-format\nstream-json\n--verbose\n\
         --replay-user-messages\n--permission-prompt-tool\nstdio\n--permission-mode\n\
         acceptEdits\n--settings\n{}\n--resume\n{SESSION}\n",
        hooks.display()
    );
    assert_eq!(args, expected);

    // Its hooks reach the service as a terminal's do, and the stream ends its turn.
    let start = json!({"session_id": SESSION, "cwd": root});
    let seen = hook(&repo, &mut app, "4", "SessionStart", start).await;
    assert!(
        matches!(seen[0], (4, Control::AgentDetected { .. })),
        "{seen:?}"
    );
    send(&mut app, 4, "hi").await;
    assert_eq!(
        app.control().await,
        entries(4, vec![entry(1, ChatEntryKind::User, "hi")])
    );
    assert_eq!(app.control().await, status(4, true, accept, false));
    // `system/init` says the mode is `default`.
    let default = ChatMode::Default;
    assert_eq!(app.control().await, status(4, true, default, true));
    let reply = "Git worktrees let one repository have several checkouts at once. \
                 Each one has its own branch and working files.";
    let assistant = entry(2, ChatEntryKind::Assistant, reply);
    assert_eq!(app.control().await, entries(4, vec![assistant]));
    let usage = entry(
        3,
        ChatEntryKind::Usage,
        "2.3 s · 40 output tokens · 0% context",
    );
    assert_eq!(app.control().await, entries(4, vec![usage]));
    assert_eq!(app.control().await, status(4, false, default, true));
    assert!(
        matches!(
            app.control().await,
            (
                4,
                Control::AgentState {
                    state: AgentState::WaitingYou,
                    ..
                }
            )
        ),
        "the turn did not end"
    );

    // Its channel is taken; other requests go to it or are refused.
    app.send(4, open(&root, None, None)).await;
    assert_eq!(app.control().await, error(4, "chat 4 is already open"));
    let terminal = Control::OpenTerminal {
        cwd: root.clone(),
        cols: 80,
        rows: 24,
    };
    app.send(4, terminal).await;
    assert_eq!(app.control().await, error(4, "chat 4 is open"));
    app.send(4, Control::ChatInterrupt { chat: 4 }).await;
    let plan = Control::ChatSetMode {
        chat: 4,
        mode: ChatMode::Plan,
    };
    app.send(4, plan).await;
    assert_eq!(app.control().await, status(4, false, ChatMode::Plan, true));
    let answer = Control::ChatAnswer {
        chat: 4,
        request: "req_1".into(),
        answer: ChatAnswer::Allow,
    };
    app.send(4, answer).await;
    assert_eq!(app.control().await, error(4, "no such pending request"));
    send(&mut app, 9, "hi").await;
    assert_eq!(
        app.control().await,
        error(9, "no chat is open on channel 9")
    );
    app.open_terminal(5, &repo.root).await;
    app.send(5, open(&root, None, None)).await;
    assert_eq!(app.control().await, error(5, "terminal 5 is open"));

    // Closing ends claude (its stdin closes) and its agent.
    app.send(4, Control::CloseChat { chat: 4 }).await;
    app.send(4, Control::CloseChat { chat: 4 }).await;
    let removed = Control::AgentRemoved { id: SESSION.into() };
    assert_eq!(app.control().await, (4, removed));
    assert_eq!(app.control().await, closed(4, None));

    // Allowed once per project; a claude that fails says why.
    app.send(6, open(&root, None, None)).await;
    assert_eq!(app.control().await, opened(6, &root, None, default));
    send(&mut app, 6, "crash").await;
    let user = entry(1, ChatEntryKind::User, "crash");
    assert_eq!(app.control().await, entries(6, vec![user]));
    assert!(matches!(
        app.control().await,
        (6, Control::ChatStatus { busy: true, .. })
    ));
    assert_eq!(app.control().await, closed(6, Some("boom")));

    // No claude, or one that cannot start.
    std::fs::remove_file(fake.join("claude")).unwrap();
    app.send(7, open(&root, None, None)).await;
    assert_eq!(
        app.control().await,
        closed(7, Some("no claude found on PATH"))
    );
    write_claude(&fake, "#!/nonexistent/sh\n");
    app.send(7, open(&root, None, None)).await;
    let (
        7,
        Control::ChatClosed {
            error: Some(why), ..
        },
    ) = app.control().await
    else {
        panic!("claude started")
    };
    assert!(
        why.starts_with(&format!("cannot start claude in {root}: ")),
        "{why}"
    );

    // A chat still open when the app leaves ends with the service.
    write_claude(&fake, &fake_claude(&fake));
    app.send(8, open(&root, None, None)).await;
    assert_eq!(app.control().await, opened(8, &root, None, default));
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// A fake `claude` that answers `initialize` with the `fixture`'s first line, then after the
/// first user turn replays the rest, reading one line from stdin after each request to us and
/// appending it to `answers`.
fn replaying(fake: &Path, fixture: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat");
    format!(
        r#"#!/bin/sh
exec 3< '{fixture}'
IFS= read -r line <&3
IFS= read -r request
printf '%s\n' "$line"
IFS= read -r request
while IFS= read -r line <&3; do
    printf '%s\n' "$line"
    case $line in
    *'"control_request"'*) IFS= read -r answer && printf '%s\n' "$answer" >> '{answers}' ;;
    esac
done
while IFS= read -r request; do :; done
"#,
        fixture = dir.join(format!("{fixture}.jsonl")).display(),
        answers = fake.join("answers").display(),
    )
}

/// Skips the app's messages until one `wanted` returns something for.
async fn until<T>(app: &mut Conn, mut wanted: impl FnMut(&(u32, Control)) -> Option<T>) -> T {
    loop {
        if let Some(found) = wanted(&app.control().await) {
            return found;
        }
    }
}

async fn request_on(app: &mut Conn, chat: u32) -> hive_protocol::ChatRequest {
    until(app, |message| match message {
        (c, Control::ChatRequest { request, .. }) if *c == chat => Some(request.clone()),
        _ => None,
    })
    .await
}

async fn state_on(app: &mut Conn, chat: u32) -> AgentState {
    until(app, |message| match message {
        (c, Control::AgentState { state, .. }) if *c == chat => Some(*state),
        _ => None,
    })
    .await
}

async fn gone_on(app: &mut Conn, chat: u32) -> String {
    until(app, |message| match message {
        (c, Control::ChatRequestGone { request, .. }) if *c == chat => Some(request.clone()),
        _ => None,
    })
    .await
}

fn answer(chat: u32, request: &str, answer: ChatAnswer) -> Control {
    Control::ChatAnswer {
        chat,
        request: request.into(),
        answer,
    }
}

#[tokio::test]
async fn permission_requests_wait_for_the_answer_of_their_own_chat() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let fake = sandbox(&repo);
    write_claude(&fake, &replaying(&fake, "permission"));
    let mut daemon = repo.env.daemon_on_path(&fake);
    let mut app = repo.env.connect(Role::App).await;
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    app.send(2, open(&root, None, None)).await;
    assert_eq!(app.control().await, asked(2, &root));
    app.send(2, confirm(2, &root, true)).await;
    let default = ChatMode::Default;
    let opened_2 = opened(2, &root, None, default);
    until(&mut app, |m| (*m == opened_2).then_some(())).await;
    let start = json!({"session_id": SESSION, "cwd": root});
    hook(&repo, &mut app, "2", "SessionStart", start).await;

    // A permission prompt waits (🟡) for the human.
    send(&mut app, 2, "go").await;
    let asked = request_on(&mut app, 2).await;
    assert_eq!((asked.id.as_str(), asked.tool.as_str()), ("req_1", "Write"));
    assert_eq!(state_on(&mut app, 2).await, AgentState::WaitingPermission);
    app.send(2, answer(2, "req_1", ChatAnswer::Allow)).await;
    assert_eq!(gone_on(&mut app, 2).await, "req_1");
    let asked = request_on(&mut app, 2).await;
    assert_eq!((asked.id.as_str(), asked.tool.as_str()), ("req_2", "Bash"));

    // Another chat cannot answer it, whatever its message says.
    app.send(3, open(&root, None, None)).await;
    let opened_3 = opened(3, &root, None, default);
    until(&mut app, |m| (*m == opened_3).then_some(())).await;
    app.send(3, answer(2, "req_2", ChatAnswer::Allow)).await;
    assert_eq!(app.control().await, error(3, "no such pending request"));
    let deny = ChatAnswer::Deny {
        message: Some("Not now.".into()),
    };
    app.send(2, answer(2, "req_2", deny)).await;
    assert_eq!(gone_on(&mut app, 2).await, "req_2");
    assert_eq!(state_on(&mut app, 2).await, AgentState::Working);
    assert_eq!(state_on(&mut app, 2).await, AgentState::WaitingYou);

    // Closing a chat denies what it waits on.
    send(&mut app, 3, "go").await;
    assert_eq!(request_on(&mut app, 3).await.id, "req_1");
    app.send(3, Control::CloseChat { chat: 3 }).await;
    let closed_3 = closed(3, None);
    until(&mut app, |m| (*m == closed_3).then_some(())).await;
    let answers = std::fs::read_to_string(fake.join("answers")).unwrap();
    let expected = [
        r#"{"response":{"request_id":"req_1","response":{"behavior":"allow","updatedInput":{"content":"hi","file_path":"/home/u/proj/hello.txt"}},"subtype":"success"},"type":"control_response"}"#,
        r#"{"response":{"request_id":"req_2","response":{"behavior":"deny","interrupt":false,"message":"Not now."},"subtype":"success"},"type":"control_response"}"#,
        r#"{"response":{"request_id":"req_1","response":{"behavior":"deny","interrupt":false,"message":"The chat was closed."},"subtype":"success"},"type":"control_response"}"#,
    ];
    assert_eq!(answers.lines().collect::<Vec<_>>(), expected);
    drop(app);
    assert!(daemon.wait_exit().success());
}
