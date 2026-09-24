//! Wire protocol between the Hive app, the `hive` service and the CLI.
//!
//! Every message is a frame `[type: u8][channel: u32][length: u32][payload]`,
//! big-endian. Control frames carry one JSON [`Control`] message; terminal
//! frames carry raw PTY bytes. Channel 0 is the connection itself; terminals
//! use channels from 1 up.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{Decoder, Encoder};

/// Bumped on every incompatible change to frames or control messages.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest payload accepted in either direction.
pub const MAX_PAYLOAD: usize = 4_194_304; // 4 MiB

const HEADER_LEN: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Control = 0,
    Terminal = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameType,
    pub channel: u32,
    pub payload: Bytes,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame payload of {0} bytes exceeds the limit of {MAX_PAYLOAD} bytes")]
    Oversized(usize),
    #[error("unknown frame type {0}")]
    UnknownType(u8),
    #[error("expected a control frame")]
    NotControl,
    #[error("invalid control message: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Frame {
    pub fn control(channel: u32, message: &Control) -> Self {
        // Cannot fail: control messages hold only strings, numbers and JSON values.
        let payload = Bytes::from(serde_json::to_vec(message).unwrap_or_default());
        Self {
            kind: FrameType::Control,
            channel,
            payload,
        }
    }

    pub fn terminal(channel: u32, payload: impl Into<Bytes>) -> Self {
        Self {
            kind: FrameType::Terminal,
            channel,
            payload: payload.into(),
        }
    }

    pub fn to_control(&self) -> Result<Control, FrameError> {
        match self.kind {
            FrameType::Control => Ok(serde_json::from_slice(&self.payload)?),
            FrameType::Terminal => Err(FrameError::NotControl),
        }
    }
}

/// Length-prefixed frame codec. Never panics on malformed input.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrameCodec;

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, FrameError> {
        let Some(mut header) = src.get(..HEADER_LEN) else {
            return Ok(None);
        };
        let kind = match header.get_u8() {
            0 => FrameType::Control,
            1 => FrameType::Terminal,
            other => return Err(FrameError::UnknownType(other)),
        };
        let channel = header.get_u32();
        let len = header.get_u32() as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::Oversized(len));
        }
        if src.len() < HEADER_LEN + len {
            return Ok(None);
        }
        src.advance(HEADER_LEN);
        let payload = src.split_to(len).freeze();
        Ok(Some(Frame {
            kind,
            channel,
            payload,
        }))
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = FrameError;

    fn encode(&mut self, frame: Frame, dst: &mut BytesMut) -> Result<(), FrameError> {
        let len = frame.payload.len();
        if len > MAX_PAYLOAD {
            return Err(FrameError::Oversized(len));
        }
        dst.put_u8(frame.kind as u8);
        dst.put_u32(frame.channel);
        dst.put_u32(len as u32);
        dst.extend_from_slice(&frame.payload);
        Ok(())
    }
}

/// Who opened the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The desktop app, through `hive bridge`. Its connection owns the service lifetime.
    App,
    /// `hive hook`: sends one event and disconnects.
    Hook,
}

/// Control messages. Terminal-scoped messages use the frame channel as terminal id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// First message from every client.
    Hello {
        protocol: u32,
        version: String,
        role: Role,
    },
    /// Handshake accepted. `distro` is the service's WSL distribution (`WSL_DISTRO_NAME`);
    /// optional, so adding it kept protocol 1 compatible.
    Welcome {
        version: String,
        #[serde(default)]
        distro: Option<String>,
    },
    /// Handshake refused; the connection is closed after this message.
    VersionMismatch {
        protocol: u32,
        version: String,
    },
    OpenTerminal {
        cwd: String,
        cols: u16,
        rows: u16,
    },
    TerminalOpened,
    Resize {
        cols: u16,
        rows: u16,
    },
    CloseTerminal,
    TerminalExited {
        code: Option<i32>,
    },
    /// Raw hook payload from `hive hook`, tagged with the terminal it came from.
    Hook {
        event: String,
        terminal_id: Option<String>,
        payload: serde_json::Value,
    },
    /// A provider event translated to the internal model.
    Agent(AgentEvent),
    /// A `claude` runs in this terminal without Hive's hooks: its state is not observed.
    UnhookedAgent,
    /// An agent started in this terminal (the frame channel is its `HIVE_TERMINAL_ID`). It is
    /// placed by its own `cwd`, not the terminal's (#19): `project` and `worktree` are the ids
    /// of the followed worktree containing `cwd`, or `None` outside every followed project.
    AgentDetected {
        /// The agent's session id.
        id: String,
        project: Option<String>,
        worktree: Option<String>,
        cwd: Option<String>,
    },
    /// The agent's displayed state (after "the most urgent wins") and its live subagents.
    /// `urgency` and `pending` are `state`'s (`AgentState::urgency`/`pending`), so the app
    /// can roll agents up and count them without its own table.
    /// Sent on the agent's terminal channel whenever it changes, and for every live agent
    /// right after the app's `Welcome`.
    AgentState {
        id: String,
        state: AgentState,
        urgency: u8,
        pending: bool,
        subagents: Vec<SubagentState>,
    },
    /// The agent's session ended, or its terminal exited.
    AgentRemoved {
        id: String,
    },
    /// App → service: every project with its worktrees, answered by `Projects`. Sent after
    /// the handshake and on an explicit refresh.
    ListProjects,
    Projects {
        projects: Vec<Project>,
    },
    /// App → service: follow the git repository containing `path` (#4). Answered by
    /// `ProjectAdded` (also when it is already followed) or `AddProjectFailed`.
    AddProject {
        path: String,
    },
    ProjectAdded {
        project: Project,
    },
    AddProjectFailed {
        path: String,
        error: ProjectError,
        /// Readable explanation, shown as is.
        message: String,
    },
    /// App → service: the local and remote branches of a followed project, answered by
    /// `Branches`.
    ListBranches {
        project: String,
    },
    Branches {
        project: String,
        /// Short names, e.g. `main`, sorted by git.
        local: Vec<String>,
        /// e.g. `origin/main`; remote `HEAD` symrefs are left out.
        remote: Vec<String>,
        /// The branch checked out in the main worktree: the base when none is given.
        current: Option<String>,
        /// Why the branches could not be listed.
        error: Option<String>,
    },
    /// App → service: checks a new worktree's name with the CLI's rule (#33), answered by
    /// `WorktreeNameValidated`. Sent as the user types.
    ValidateWorktreeName {
        project: String,
        name: String,
    },
    WorktreeNameValidated {
        project: String,
        name: String,
        /// Where the worktree would go, relative to the project, e.g. `.claude/worktrees/x/`.
        folder: String,
        /// The branch it would get, e.g. `worktree-x`.
        branch: String,
        /// Why the name is refused, worded as `hive worktree create` says it.
        error: Option<String>,
    },
    /// App → service: `hive worktree create` for a followed project. Answered by
    /// `WorktreeCreated` or `CreateWorktreeFailed`.
    CreateWorktree {
        project: String,
        name: String,
        /// Local or remote branch to start from; the main worktree's HEAD when `None`.
        base: Option<String>,
    },
    WorktreeCreated {
        /// The project with its updated worktrees.
        project: Project,
        /// The new worktree's path (also its id).
        path: String,
        /// What the CLI prints on stderr, e.g. a competing `WorktreeCreate` hook.
        notes: Vec<String>,
    },
    CreateWorktreeFailed {
        project: String,
        name: String,
        message: String,
    },
    Error {
        message: String,
    },
}

impl Control {
    pub fn hello(role: Role, version: &str) -> Self {
        Self::Hello {
            protocol: PROTOCOL_VERSION,
            version: version.to_owned(),
            role,
        }
    }
}

/// A git repository inside WSL that the app follows (#4). Paths are the service's, never
/// derived by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Stable id: the repository's top-level path.
    pub id: String,
    pub name: String,
    pub path: String,
    /// Every worktree from `git worktree list` (#8), the main one first.
    pub worktrees: Vec<Worktree>,
    /// Why the worktrees could not be listed (e.g. the folder was moved).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    /// Stable id: the worktree's path.
    pub id: String,
    /// Shown in the sidebar: the folder name for a Claude worktree, else the branch.
    pub name: String,
    pub path: String,
    /// `None` when detached.
    pub branch: Option<String>,
    /// The repository's main worktree.
    pub main: bool,
    /// Follows Claude's convention: `<repo>/.claude/worktrees/<name>` (#7).
    pub claude: bool,
}

/// Why a folder cannot be added as a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectError {
    /// Empty or only whitespace.
    EmptyPath,
    NotAbsolute,
    /// Missing or unreadable.
    NotFound,
    NotADirectory,
    NotAGitRepository,
    /// The project list could not be saved.
    Storage,
}

/// Provider-independent agent event, produced by an adapter from a raw hook payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Adapter that produced the event, e.g. `claude-code`.
    pub provider: String,
    /// `HIVE_TERMINAL_ID` of the terminal the agent runs in.
    pub terminal_id: Option<String>,
    pub session_id: Option<String>,
    /// Set when the event comes from a subagent.
    pub subagent: Option<Subagent>,
    /// Working directory reported by the agent; places it under a worktree.
    pub cwd: Option<String>,
    pub kind: EventKind,
    /// The provider payload, unchanged.
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subagent {
    pub id: String,
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    PromptSubmitted,
    ToolStarted {
        tool: Option<String>,
    },
    ToolFinished {
        tool: Option<String>,
    },
    ToolFailed {
        tool: Option<String>,
    },
    PermissionRequested {
        tool: Option<String>,
    },
    Notification {
        notification: Notification,
    },
    /// The agent finished its turn.
    TurnFinished,
    /// The turn ended because of an error (API, auth, limits).
    TurnFailed {
        error: Option<String>,
    },
    SubagentStarted,
    SubagentStopped,
    SessionEnded {
        reason: Option<String>,
    },
    /// Hive created (or reused) the worktree `name` at `path` for the agent.
    WorktreeCreated {
        name: Option<String>,
        path: Option<String>,
    },
    /// Hive removed the worktree at `path` for the agent.
    WorktreeRemoved {
        path: Option<String>,
    },
    /// Any provider event without an internal meaning yet.
    Other {
        event: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notification {
    PermissionPrompt,
    ElicitationDialog,
    IdlePrompt,
    AgentNeedsInput,
    Other(String),
}

/// Visual state of an agent ("Mapeamento de estados" in `docs/hive.md`). Declared from least
/// to most urgent, so the most urgent of several states is their `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Ended,
    Idle,
    Working,
    WithSubagents,
    WaitingYou,
    Error,
    WaitingPermission,
}

impl AgentState {
    /// Higher is more urgent: the declaration order, 0 (ended) to 6 (waiting for permission).
    pub fn urgency(self) -> u8 {
        self as u8
    }

    /// Needs the user (the "N pending" counter and F8): waiting for permission, error
    /// (urgency "alta") and waiting for you ("média").
    pub fn pending(self) -> bool {
        self >= AgentState::WaitingYou
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentState {
    pub id: String,
    pub agent_type: Option<String>,
    pub state: AgentState,
    /// The worktree it works in when that is its own (not its agent's): the worktree's id,
    /// shown nested under the subagent instead of at project level (#22).
    pub worktree: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(frame: Frame) -> BytesMut {
        let mut buf = BytesMut::new();
        FrameCodec.encode(frame, &mut buf).unwrap();
        buf
    }

    #[test]
    fn terminal_frame_round_trips_with_big_endian_header() {
        let frame = Frame::terminal(0x0102_0304, &b"ls\r"[..]);
        let mut buf = encode(frame.clone());
        assert_eq!(&buf[..HEADER_LEN], &[1, 1, 2, 3, 4, 0, 0, 0, 3]);
        assert_eq!(FrameCodec.decode(&mut buf).unwrap(), Some(frame));
        assert!(buf.is_empty());
    }

    #[test]
    fn control_frame_round_trips() {
        let msg = Control::hello(Role::App, "0.1.0");
        let mut buf = encode(Frame::control(0, &msg));
        assert_eq!(buf[0], 0);
        let frame = FrameCodec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.to_control().unwrap(), msg);
        assert_eq!(
            msg,
            Control::Hello {
                protocol: PROTOCOL_VERSION,
                version: "0.1.0".into(),
                role: Role::App
            }
        );
    }

    #[test]
    fn control_messages_are_tagged_json() {
        let frame = Frame::control(3, &Control::Resize { cols: 80, rows: 24 });
        assert_eq!(
            &frame.payload[..],
            br#"{"type":"resize","cols":80,"rows":24}"#
        );
    }

    #[test]
    fn welcome_without_a_distro_still_decodes() {
        let frame = Frame {
            kind: FrameType::Control,
            channel: 0,
            payload: Bytes::from_static(br#"{"type":"welcome","version":"0.1.0"}"#),
        };
        let welcome = Control::Welcome {
            version: "0.1.0".into(),
            distro: None,
        };
        assert_eq!(frame.to_control().unwrap(), welcome);
    }

    #[test]
    fn agent_event_round_trips_through_a_control_frame() {
        let msg = Control::Agent(AgentEvent {
            provider: "claude-code".into(),
            terminal_id: Some("3".into()),
            session_id: Some("s".into()),
            subagent: Some(Subagent {
                id: "a".into(),
                agent_type: None,
            }),
            cwd: None,
            kind: EventKind::Notification {
                notification: Notification::Other("x".into()),
            },
            raw: serde_json::json!({"k": [1, 2]}),
        });
        let frame = Frame::control(0, &msg);
        assert_eq!(frame.to_control().unwrap(), msg);
    }

    #[test]
    fn agent_messages_are_tagged_json() {
        let detected = Control::AgentDetected {
            id: "s".into(),
            project: Some("/r".into()),
            worktree: None,
            cwd: Some("/r/x".into()),
        };
        assert_eq!(
            &Frame::control(1, &detected).payload[..],
            br#"{"type":"agent_detected","id":"s","project":"/r","worktree":null,"cwd":"/r/x"}"#
        );
        let removed = Control::AgentRemoved { id: "s".into() };
        assert_eq!(Frame::control(1, &removed).to_control().unwrap(), removed);
    }

    #[test]
    fn agent_state_is_tagged_json() {
        let msg = Control::AgentState {
            id: "s".into(),
            state: AgentState::WaitingPermission,
            urgency: 6,
            pending: true,
            subagents: vec![SubagentState {
                id: "a".into(),
                agent_type: None,
                state: AgentState::WithSubagents,
                worktree: Some("/r/.claude/worktrees/w".into()),
            }],
        };
        assert_eq!(
            &Frame::control(1, &msg).payload[..],
            br#"{"type":"agent_state","id":"s","state":"waiting_permission","urgency":6,"pending":true,"subagents":[{"id":"a","agent_type":null,"state":"with_subagents","worktree":"/r/.claude/worktrees/w"}]}"#
        );
        assert_eq!(Frame::control(1, &msg).to_control().unwrap(), msg);
    }

    #[test]
    fn agent_states_are_ordered_by_urgency() {
        use AgentState::*;
        let most_urgent_first = [
            WaitingPermission,
            Error,
            WaitingYou,
            WithSubagents,
            Working,
            Idle,
            Ended,
        ];
        assert!(most_urgent_first.windows(2).all(|w| w[0] > w[1]));
        let urgency: Vec<u8> = most_urgent_first.iter().map(|s| s.urgency()).collect();
        assert_eq!(urgency, [6, 5, 4, 3, 2, 1, 0]);
        let pending: Vec<bool> = most_urgent_first.iter().map(|s| s.pending()).collect();
        assert_eq!(pending, [true, true, true, false, false, false, false]);
        let json = serde_json::to_string(&most_urgent_first).unwrap();
        assert_eq!(
            json,
            r#"["waiting_permission","error","waiting_you","with_subagents","working","idle","ended"]"#
        );
    }

    #[test]
    fn project_messages_are_tagged_json() {
        let worktree = Worktree {
            id: "/r".into(),
            name: "main".into(),
            path: "/r".into(),
            branch: Some("main".into()),
            main: true,
            claude: false,
        };
        let msg = Control::ProjectAdded {
            project: Project {
                id: "/r".into(),
                name: "r".into(),
                path: "/r".into(),
                worktrees: vec![worktree],
                error: None,
            },
        };
        assert_eq!(Frame::control(0, &msg).to_control().unwrap(), msg);
        let failed = Control::AddProjectFailed {
            path: "x".into(),
            error: ProjectError::NotAGitRepository,
            message: "m".into(),
        };
        assert_eq!(
            &Frame::control(0, &failed).payload[..],
            br#"{"type":"add_project_failed","path":"x","error":"not_a_git_repository","message":"m"}"#
        );
    }

    #[test]
    fn worktree_messages_are_tagged_json() {
        let create = Control::CreateWorktree {
            project: "/r".into(),
            name: "x".into(),
            base: None,
        };
        assert_eq!(
            &Frame::control(0, &create).payload[..],
            br#"{"type":"create_worktree","project":"/r","name":"x","base":null}"#
        );
        let branches = Control::Branches {
            project: "/r".into(),
            local: vec!["main".into()],
            remote: vec!["origin/main".into()],
            current: Some("main".into()),
            error: None,
        };
        assert_eq!(Frame::control(0, &branches).to_control().unwrap(), branches);
    }

    #[test]
    fn terminal_frame_is_not_control() {
        assert!(matches!(
            Frame::terminal(1, "x").to_control(),
            Err(FrameError::NotControl)
        ));
    }

    #[test]
    fn invalid_control_json_is_an_error() {
        let frame = Frame {
            kind: FrameType::Control,
            channel: 0,
            payload: Bytes::from_static(b"{"),
        };
        assert!(matches!(frame.to_control(), Err(FrameError::Json(_))));
    }

    #[test]
    fn partial_frames_wait_for_more_bytes() {
        let full = encode(Frame::terminal(7, &b"hello"[..]));
        for cut in 0..full.len() {
            let mut buf = BytesMut::from(&full[..cut]);
            assert_eq!(FrameCodec.decode(&mut buf).unwrap(), None, "cut at {cut}");
            assert_eq!(buf.len(), cut, "partial input must not be consumed");
        }
    }

    #[test]
    fn two_frames_in_one_buffer_decode_in_order() {
        let mut buf = encode(Frame::terminal(1, "a"));
        buf.extend_from_slice(&encode(Frame::terminal(2, "b")));
        assert_eq!(
            FrameCodec.decode(&mut buf).unwrap(),
            Some(Frame::terminal(1, "a"))
        );
        assert_eq!(
            FrameCodec.decode(&mut buf).unwrap(),
            Some(Frame::terminal(2, "b"))
        );
        assert_eq!(FrameCodec.decode(&mut buf).unwrap(), None);
    }

    #[test]
    fn zero_length_payload_is_a_valid_frame() {
        let mut buf = encode(Frame::terminal(9, Bytes::new()));
        assert_eq!(buf.len(), HEADER_LEN);
        assert_eq!(
            FrameCodec.decode(&mut buf).unwrap(),
            Some(Frame::terminal(9, Bytes::new()))
        );
    }

    #[test]
    fn payload_at_the_limit_is_accepted() {
        let frame = Frame::terminal(1, vec![0; MAX_PAYLOAD]);
        let mut buf = encode(frame.clone());
        assert_eq!(FrameCodec.decode(&mut buf).unwrap(), Some(frame));
    }

    #[test]
    fn oversized_length_is_rejected_before_buffering() {
        let mut buf = BytesMut::from(&[1, 0, 0, 0, 1][..]);
        buf.put_u32(MAX_PAYLOAD as u32 + 1);
        let err = FrameCodec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::Oversized(n) if n == MAX_PAYLOAD + 1));
    }

    #[test]
    fn oversized_payload_is_not_encoded() {
        let mut buf = BytesMut::new();
        let err = FrameCodec
            .encode(Frame::terminal(1, vec![0; MAX_PAYLOAD + 1]), &mut buf)
            .unwrap_err();
        assert!(matches!(err, FrameError::Oversized(n) if n == MAX_PAYLOAD + 1));
        assert!(buf.is_empty());
    }

    #[test]
    fn unknown_type_is_rejected() {
        let mut buf = BytesMut::from(&[2, 0, 0, 0, 1, 0, 0, 0, 0][..]);
        assert!(matches!(
            FrameCodec.decode(&mut buf),
            Err(FrameError::UnknownType(2))
        ));
    }

    #[test]
    fn errors_have_readable_messages() {
        assert_eq!(
            FrameError::Oversized(5).to_string(),
            format!("frame payload of 5 bytes exceeds the limit of {MAX_PAYLOAD} bytes")
        );
        assert_eq!(
            FrameError::UnknownType(7).to_string(),
            "unknown frame type 7"
        );
    }
}
