//! `hive hook <event>`: forwards one Claude Code hook call to the service.
//!
//! Never delays or breaks the agent: prints nothing on stdout (some events add
//! stdout to the agent's context), gives up after [`SEND_TIMEOUT`], and the
//! command always exits 0.

use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::SinkExt;
use hive_protocol::{Control, Frame, FrameCodec, Role};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UnixStream;
use tokio_util::codec::FramedWrite;

use crate::VERSION;
use crate::paths::Paths;

/// Largest hook input forwarded. Even fully escaped it stays under the frame limit.
pub const MAX_INPUT: usize = 524_288; // 512 KiB

const SEND_TIMEOUT: Duration = Duration::from_millis(200);

pub async fn run(event: &str, record: Option<&Path>, paths: &Paths, input: impl AsyncRead + Unpin) {
    let payload = read_payload(input).await;
    let terminal_id = std::env::var("HIVE_TERMINAL_ID").ok();
    if let Some(file) = record
        && let Err(err) = append_record(file, event, terminal_id.as_deref(), &payload)
    {
        eprintln!(
            "hive: cannot record the hook call in {}: {err}",
            file.display()
        );
    }
    forward(paths, event, terminal_id, payload).await;
}

/// Sends one hook call to the service, giving up after [`SEND_TIMEOUT`]; errors are ignored.
/// `hive worktree hook-create`/`hook-remove` also report their work through here.
pub async fn forward(paths: &Paths, event: &str, terminal_id: Option<String>, payload: Value) {
    let hook = Control::Hook {
        event: event.to_owned(),
        terminal_id,
        payload,
    };
    let _ = send(paths, 0, &hook).await;
}

/// The hook JSON; invalid JSON is kept as a string, oversized input is replaced by a marker.
async fn read_payload(input: impl AsyncRead + Unpin) -> Value {
    let mut buf = Vec::new();
    let read = input.take(MAX_INPUT as u64 + 1).read_to_end(&mut buf).await;
    if read.is_err() || buf.len() > MAX_INPUT {
        return json!({ "hive_error": "hook input unreadable or larger than 512 KiB" });
    }
    serde_json::from_slice(&buf)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&buf).into_owned()))
}

/// The wall clock, in ms since the Unix epoch (0 before it).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_millis() as u64)
}

/// Appends one JSON line: time, event, terminal and the raw payload.
fn append_record(
    file: &Path,
    event: &str,
    terminal_id: Option<&str>,
    payload: &Value,
) -> io::Result<()> {
    let line = json!({ "ts_ms": now_ms(), "event": event, "terminal_id": terminal_id, "payload": payload });
    let mut out = std::fs::File::options()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(file)?;
    // One write per line keeps concurrent hook calls from interleaving.
    out.write_all(format!("{line}\n").as_bytes())
}

/// `hive badge`: sets (or, with an empty text, clears) the label of the terminal `terminal`.
/// Unlike a hook, a failure is the user's to see.
pub async fn badge(paths: &Paths, terminal: u32, text: String) -> io::Result<()> {
    send(paths, terminal, &Control::Badge { text })
        .await
        .map_err(|err| io::Error::other(format!("cannot reach the Hive service: {err}")))
}

/// Opens a hook-role connection and sends one message on `channel`, giving up after
/// [`SEND_TIMEOUT`].
async fn send(paths: &Paths, channel: u32, message: &Control) -> io::Result<()> {
    let sent = async {
        let stream = UnixStream::connect(paths.socket()).await?;
        let mut writer = FramedWrite::new(stream, FrameCodec);
        let hello = Control::hello(Role::Hook, VERSION);
        writer.send(Frame::control(0, &hello)).await?;
        writer.send(Frame::control(channel, message)).await
    };
    match tokio::time::timeout(SEND_TIMEOUT, sent).await {
        Ok(result) => result.map_err(io::Error::other),
        Err(elapsed) => Err(io::Error::new(io::ErrorKind::TimedOut, elapsed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn json_input_is_parsed() {
        assert_eq!(read_payload(&br#"{"a": 1}"#[..]).await, json!({"a": 1}));
    }

    #[tokio::test]
    async fn invalid_json_is_kept_as_text() {
        assert_eq!(read_payload(&b"not json"[..]).await, json!("not json"));
    }

    #[tokio::test]
    async fn input_at_the_limit_is_kept() {
        let text = "x".repeat(MAX_INPUT - 2);
        let input = format!("\"{text}\"");
        assert_eq!(read_payload(input.as_bytes()).await, json!(text));
    }

    #[tokio::test]
    async fn oversized_input_is_replaced() {
        let input = format!("\"{}\"", "x".repeat(MAX_INPUT - 1));
        let payload = read_payload(input.as_bytes()).await;
        assert_eq!(
            payload,
            json!({ "hive_error": "hook input unreadable or larger than 512 KiB" })
        );
    }

    #[test]
    fn records_are_appended_as_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hooks.jsonl");
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        append_record(&file, "Stop", Some("3"), &json!({"k": "v"})).unwrap();
        append_record(&file, "SessionEnd", None, &json!("raw")).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0]["ts_ms"].as_u64().unwrap() >= before);
        assert_eq!(lines[0]["event"], "Stop");
        assert_eq!(lines[0]["terminal_id"], "3");
        assert_eq!(lines[0]["payload"], json!({"k": "v"}));
        assert_eq!(lines[1]["terminal_id"], Value::Null);
        assert_eq!(lines[1]["payload"], "raw");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
