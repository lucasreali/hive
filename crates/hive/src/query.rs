//! The CLI's read commands (28.10): `hive agents` asks the running service for its live agents,
//! and `--json` prints them, or `hive worktree list`'s worktrees, in a documented shape
//! (`docs/architecture.md`, "CLI output"). Nothing here starts, stops or talks to an agent.

use std::fmt::Write as _;
use std::io;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, ListedAgent, Role};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::VERSION;
use crate::paths::Paths;
use crate::worktree::Worktree;

/// `hive agents`'s exit code when no service runs (documented): 1 is any other failure.
pub const NOT_RUNNING: u8 = 3;

/// How long `hive agents` waits for the whole answer.
const ANSWER_TIME: Duration = Duration::from_secs(10);

/// Most agents read from one answer; a longer one is refused.
const MAX_AGENTS: usize = 4096;

/// Why `hive agents` got no list.
#[derive(Debug)]
pub enum Failure {
    /// No service could be reached (exit [`NOT_RUNNING`]).
    NotRunning(io::Error),
    /// A service answered, but not with the list (exit 1).
    Other(io::Error),
}

impl Failure {
    /// `hive agents`'s exit code for it.
    pub fn code(&self) -> u8 {
        match self {
            Self::NotRunning(_) => NOT_RUNNING,
            Self::Other(_) => 1,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning(err) => write!(f, "no Hive service is running ({err})"),
            Self::Other(err) => write!(f, "{err}"),
        }
    }
}

/// The running service's live agents, in terminal order.
pub async fn agents(paths: &Paths) -> Result<Vec<ListedAgent>, Failure> {
    let stream = paths.connect().await.map_err(Failure::NotRunning)?;
    answer(stream, ANSWER_TIME).await.map_err(Failure::Other)
}

/// [`ask`], given up after `time`.
async fn answer<S: AsyncRead + AsyncWrite>(
    stream: S,
    time: Duration,
) -> io::Result<Vec<ListedAgent>> {
    let late = |_| {
        let why = "the Hive service did not answer in time";
        io::Error::new(io::ErrorKind::TimedOut, why)
    };
    tokio::time::timeout(time, ask(stream))
        .await
        .map_err(late)?
}

/// Sends `list_agents` on a hook-role connection and reads the answer.
async fn ask<S: AsyncRead + AsyncWrite>(stream: S) -> io::Result<Vec<ListedAgent>> {
    let (read, write) = tokio::io::split(stream);
    let mut reader = FramedRead::new(read, FrameCodec);
    let mut writer = FramedWrite::new(write, FrameCodec);
    let hello = Control::hello(Role::Hook, VERSION);
    writer
        .send(Frame::control(0, &hello))
        .await
        .map_err(io::Error::other)?;
    writer
        .send(Frame::control(0, &Control::ListAgents))
        .await
        .map_err(io::Error::other)?;
    match next(&mut reader).await? {
        Control::Welcome { .. } => {}
        Control::VersionMismatch { version, .. } => {
            return Err(io::Error::other(format!(
                "the Hive service is version {}, this hive is {VERSION}",
                clean(&version)
            )));
        }
        other => return Err(unexpected(&other)),
    }
    let mut agents = Vec::new();
    // Each turn reads a frame, and keeps at most `MAX_AGENTS` agents.
    loop {
        match next(&mut reader).await? {
            Control::AgentListed { .. } if agents.len() >= MAX_AGENTS => {
                return Err(io::Error::other(format!(
                    "the Hive service listed more than {MAX_AGENTS} agents"
                )));
            }
            Control::AgentListed { agent } => agents.push(agent),
            Control::AgentsListed => return Ok(agents),
            other => return Err(unexpected(&other)),
        }
    }
}

/// The next control message; the end of the connection is an error.
async fn next<R: AsyncRead + Unpin>(reader: &mut FramedRead<R, FrameCodec>) -> io::Result<Control> {
    let frame = reader.next().await.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the Hive service closed the connection",
        )
    })?;
    frame
        .map_err(io::Error::other)?
        .to_control()
        .map_err(io::Error::other)
}

fn unexpected(message: &Control) -> io::Error {
    match message {
        Control::Error { message } => {
            io::Error::other(format!("the Hive service refused: {}", clean(message)))
        }
        _ => io::Error::other("unexpected answer from the Hive service"),
    }
}

/// What `hive agents` prints: [`agents_json`] and a newline, or [`agents_text`].
pub fn render(agents: &[ListedAgent], json: bool) -> String {
    if json {
        agents_json(agents) + "\n"
    } else {
        agents_text(agents)
    }
}

/// `hive agents --json`: `{"agents": [...]}`, one line.
pub fn agents_json(agents: &[ListedAgent]) -> String {
    json!({ "agents": agents }).to_string()
}

/// `hive agents`: one line per agent, tab-separated: terminal, state, where it runs (its
/// worktree, else its folder), context used (percent of its window), title and activity, `-`
/// for what is not known. Control characters (a title's or a path's) print as spaces.
pub fn agents_text(agents: &[ListedAgent]) -> String {
    let mut out = String::new();
    for agent in agents {
        let state = serde_json::to_value(agent.state)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let place = agent.worktree.as_ref().or(agent.cwd.as_ref());
        let context = agent.usage.filter(|u| u.context_limit > 0).map(|u| {
            format!(
                "{}%",
                u.context_tokens.saturating_mul(100) / u.context_limit
            )
        });
        let field = |value: Option<&String>| value.map_or_else(|| "-".to_owned(), |v| clean(v));
        let _ = writeln!(
            out,
            "{}\t{state}\t{}\t{}\t{}\t{}",
            agent.terminal,
            field(place),
            field(context.as_ref()),
            field(agent.title.as_ref()),
            field(agent.activity.as_ref()),
        );
    }
    out
}

/// `hive worktree list --json`: `{"worktrees": [{path, branch, bare, prunable}]}`, the main
/// worktree first, one line. A path that is not UTF-8 has its odd bytes replaced (U+FFFD).
pub fn worktrees_json(worktrees: &[Worktree]) -> String {
    let worktrees: Vec<_> = worktrees
        .iter()
        .map(|wt| {
            json!({
                "path": wt.path.to_string_lossy(),
                "branch": wt.branch,
                "bare": wt.bare,
                "prunable": wt.prunable,
            })
        })
        .collect();
    json!({ "worktrees": worktrees }).to_string()
}

/// `text` with its control and invisible characters (a tab, a newline, an escape sequence's
/// ESC, a bidi override, a zero-width space) as spaces, so it stays one field, cannot drive the
/// terminal and shows what it holds.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if crate::adapter::invisible(c) { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_protocol::{AgentState, ListedUsage, PROTOCOL_VERSION};
    use std::path::PathBuf;

    fn agent(terminal: u32) -> ListedAgent {
        ListedAgent {
            id: format!("s{terminal}"),
            terminal,
            project: None,
            worktree: None,
            cwd: None,
            title: None,
            state: AgentState::Working,
            pending: false,
            since_ms: 1,
            activity: None,
            subagents: vec![],
            usage: None,
        }
    }

    /// A service that reads the hello and `list_agents`, then sends `answer` and closes.
    async fn served(answer: Vec<Control>) -> io::Result<Vec<ListedAgent>> {
        let (client, server) = tokio::io::duplex(1 << 20);
        let service = tokio::spawn(async move {
            let (read, write) = tokio::io::split(server);
            let mut reader = FramedRead::new(read, FrameCodec);
            let mut writer = FramedWrite::new(write, FrameCodec);
            let hello = reader.next().await.unwrap().unwrap().to_control().unwrap();
            assert!(matches!(
                hello,
                Control::Hello {
                    role: Role::Hook,
                    ..
                }
            ));
            let ask = reader.next().await.unwrap().unwrap().to_control().unwrap();
            assert_eq!(ask, Control::ListAgents);
            // The client may stop reading early (an error): what it leaves is dropped.
            for message in answer {
                let _ = writer.send(Frame::control(0, &message)).await;
            }
        });
        let result = ask(client).await;
        service.await.unwrap();
        result
    }

    fn welcome() -> Control {
        Control::Welcome {
            version: VERSION.into(),
            distro: None,
            warning: None,
        }
    }

    #[tokio::test]
    async fn the_agents_are_read_until_the_end() {
        let listed = |terminal| Control::AgentListed {
            agent: agent(terminal),
        };
        let answer = vec![welcome(), listed(1), listed(2), Control::AgentsListed];
        assert_eq!(served(answer).await.unwrap(), vec![agent(1), agent(2)]);
        let none = served(vec![welcome(), Control::AgentsListed]).await;
        assert_eq!(none.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn a_refusal_or_a_short_answer_fails() {
        let mismatch = Control::VersionMismatch {
            protocol: PROTOCOL_VERSION,
            version: "9.9.9\u{1b}[2J".into(),
            terminals: None,
        };
        let err = served(vec![mismatch]).await.unwrap_err();
        let expected = format!("the Hive service is version 9.9.9 [2J, this hive is {VERSION}");
        assert_eq!(err.to_string(), expected);
        let refused = Control::Error {
            message: "no\nway".into(),
        };
        let err = served(vec![refused]).await.unwrap_err();
        assert_eq!(err.to_string(), "the Hive service refused: no way");
        // Closed before the end of the list.
        let err = served(vec![welcome()]).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(err.to_string(), "the Hive service closed the connection");
        // Anything else.
        let odd = served(vec![welcome(), Control::Resync]).await.unwrap_err();
        assert_eq!(odd.to_string(), "unexpected answer from the Hive service");
        let odd = served(vec![Control::AgentsListed]).await.unwrap_err();
        assert_eq!(odd.to_string(), "unexpected answer from the Hive service");
    }

    #[tokio::test]
    async fn a_list_too_long_is_refused() {
        let mut answer = vec![welcome()];
        let listed = Control::AgentListed { agent: agent(1) };
        answer.extend(std::iter::repeat_n(listed, MAX_AGENTS));
        answer.push(Control::AgentsListed);
        assert_eq!(served(answer.clone()).await.unwrap().len(), MAX_AGENTS);
        answer.insert(1, Control::AgentListed { agent: agent(1) });
        let err = served(answer).await.unwrap_err();
        let expected = format!("the Hive service listed more than {MAX_AGENTS} agents");
        assert_eq!(err.to_string(), expected);
    }

    #[tokio::test]
    async fn a_silent_service_is_not_waited_for() {
        let (client, _server) = tokio::io::duplex(1024);
        let err = answer(client, Duration::from_millis(20)).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert_eq!(err.to_string(), "the Hive service did not answer in time");
    }

    #[tokio::test]
    async fn garbage_from_the_service_fails() {
        let (client, server) = tokio::io::duplex(1024);
        let service = tokio::spawn(async move {
            let (_read, write) = tokio::io::split(server);
            let mut writer = FramedWrite::new(write, FrameCodec);
            writer.send(Frame::terminal(0, &b"x"[..])).await.unwrap();
        });
        assert!(ask(client).await.is_err());
        service.await.unwrap();
    }

    #[test]
    fn failures_say_what_went_wrong() {
        let gone = io::Error::new(io::ErrorKind::NotFound, "no socket");
        let gone = Failure::NotRunning(gone);
        assert_eq!(gone.to_string(), "no Hive service is running (no socket)");
        assert_eq!(gone.code(), 3);
        let other = Failure::Other(io::Error::other("late"));
        assert_eq!(other.to_string(), "late");
        assert_eq!(other.code(), 1);
    }

    #[test]
    fn render_is_json_or_text() {
        assert_eq!(render(&[], true), "{\"agents\":[]}\n");
        assert_eq!(render(&[agent(1)], false), agents_text(&[agent(1)]));
        assert_eq!(render(&[], false), "");
    }

    #[test]
    fn agents_json_is_the_listed_agents() {
        let mut one = agent(3);
        one.worktree = Some("/r".into());
        one.usage = Some(ListedUsage {
            context_tokens: 50,
            context_limit: 200,
            output_tokens: 9,
        });
        let json: serde_json::Value = serde_json::from_str(&agents_json(&[one])).unwrap();
        assert_eq!(
            json,
            json!({"agents": [{
                "id": "s3", "terminal": 3, "project": null, "worktree": "/r", "cwd": null,
                "title": null, "state": "working", "pending": false, "since_ms": 1,
                "activity": null, "subagents": [],
                "usage": {"context_tokens": 50, "context_limit": 200, "output_tokens": 9},
            }]})
        );
        assert_eq!(agents_json(&[]), r#"{"agents":[]}"#);
    }

    #[test]
    fn agents_text_is_one_line_per_agent() {
        let mut first = agent(1);
        first.cwd = Some("/home/x".into());
        first.title = Some("fix\tlogin\u{1b}[31m".into());
        first.activity = Some("Running\ntests".into());
        first.state = AgentState::WaitingPermission;
        first.usage = Some(ListedUsage {
            context_tokens: 150,
            context_limit: 200,
            output_tokens: 0,
        });
        let mut second = agent(2);
        second.cwd = Some("/home/y".into());
        // A bidi override or a zero-width space shows as a space, not reordering the line.
        second.worktree = Some("/r/w\u{202E}t\u{200B}x".into());
        // No window known: no percent.
        second.usage = Some(ListedUsage {
            context_tokens: 1,
            context_limit: 0,
            output_tokens: 0,
        });
        assert_eq!(
            agents_text(&[first, second, agent(3)]),
            "1\twaiting_permission\t/home/x\t75%\tfix login [31m\tRunning tests\n\
             2\tworking\t/r/w t x\t-\t-\t-\n\
             3\tworking\t-\t-\t-\t-\n"
        );
        assert_eq!(agents_text(&[]), "");
    }

    #[test]
    fn worktrees_json_is_the_worktrees() {
        let worktrees = [
            Worktree {
                path: PathBuf::from("/r"),
                branch: Some("main".into()),
                bare: false,
                prunable: false,
            },
            Worktree {
                path: PathBuf::from("/r/.claude/worktrees/x"),
                branch: None,
                bare: false,
                prunable: true,
            },
        ];
        let json: serde_json::Value = serde_json::from_str(&worktrees_json(&worktrees)).unwrap();
        assert_eq!(
            json,
            json!({"worktrees": [
                {"path": "/r", "branch": "main", "bare": false, "prunable": false},
                {"path": "/r/.claude/worktrees/x", "branch": null, "bare": false, "prunable": true},
            ]})
        );
        assert_eq!(worktrees_json(&[]), r#"{"worktrees":[]}"#);
    }
}
