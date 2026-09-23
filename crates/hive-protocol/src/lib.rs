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
