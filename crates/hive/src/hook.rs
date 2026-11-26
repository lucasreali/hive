//! `hive hook <event>`: forwards one Claude Code hook call to the service.
//!
//! Never delays or breaks the agent: prints nothing on stdout (some events add
//! stdout to the agent's context), gives up after [`SEND_TIMEOUT`], and the
//! command always exits 0.

use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::SinkExt;
use hive_protocol::{Control, Frame, FrameCodec, Role};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::codec::FramedWrite;

use crate::VERSION;
use crate::paths::Paths;

/// Largest hook input forwarded. Even fully escaped it stays under the frame limit.
pub const MAX_INPUT: usize = 524_288; // 512 KiB

/// Largest `PostToolUse` input read (27.5): it is [slimmed](crate::slim) before it is sent, and
/// an `Edit`'s `originalFile` may hold a file of up to about 10 MB, twice with a `Write`.
pub const MAX_TOOL_INPUT: usize = 32 << 20; // 32 MiB

/// How much of a payload too large to read is scanned for its ids.
const HEAD: usize = 4096;

const SEND_TIMEOUT: Duration = Duration::from_millis(200);

pub async fn run(event: &str, record: Option<&Path>, paths: &Paths, input: impl AsyncRead + Unpin) {
    // Stamped first, so reading a large payload cannot put this call after a later one.
    let sent_ns = monotonic_ns();
    let tool = event == "PostToolUse";
    let payload = read_payload(input, if tool { MAX_TOOL_INPUT } else { MAX_INPUT }).await;
    let terminal_id = std::env::var("HIVE_TERMINAL_ID").ok();
    if let Some(file) = record
        && let Err(err) = append_record(file, event, terminal_id.as_deref(), &payload)
    {
        eprintln!(
            "hive: cannot record the hook call in {}: {err}",
            file.display()
        );
    }
    let payload = if tool {
        crate::slim::slim(payload)
    } else {
        payload
    };
    forward(paths, event, terminal_id, payload, sent_ns).await;
}

/// Sends one hook call stamped `sent_ns` ([`monotonic_ns`]) to the service, giving up after
/// [`SEND_TIMEOUT`]; errors are ignored. `hive worktree hook-create`/`hook-remove` also report
/// their work through here.
pub async fn forward(
    paths: &Paths,
    event: &str,
    terminal_id: Option<String>,
    payload: Value,
    sent_ns: u64,
) {
    let hook = Control::Hook {
        event: event.to_owned(),
        terminal_id,
        payload,
        sent_ns,
    };
    let _ = send(paths, 0, &hook).await;
}

/// The hook JSON of at most `most` bytes; invalid JSON is kept as a string, oversized input is
/// replaced by a marker that keeps the `session_id` and `tool_name` found in its first bytes,
/// so the agent's state still follows the event.
async fn read_payload(input: impl AsyncRead + Unpin, most: usize) -> Value {
    let mut buf = Vec::new();
    let read = input.take(most as u64 + 1).read_to_end(&mut buf).await;
    if read.is_err() || buf.len() > most {
        let mut marker = crate::slim::ids(&buf[..buf.len().min(HEAD)]);
        let error = format!("hook input unreadable or larger than {} KiB", most >> 10);
        marker.insert("hive_error".to_owned(), Value::String(error));
        return Value::Object(marker);
    }
    serde_json::from_slice(&buf)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&buf).into_owned()))
}

/// `CLOCK_MONOTONIC` in ns: one clock for every process on the machine, so it orders hook
/// calls made by different `hive hook` processes (0 if it cannot be read).
#[cfg(unix)]
pub fn monotonic_ns() -> u64 {
    nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC).map_or(0, |t| {
        (t.tv_sec() as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(t.tv_nsec() as u64)
    })
}

#[cfg(windows)]
pub use crate::windows::monotonic_ns;

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
    let mut out =
        crate::mode::private(std::fs::File::options().create(true).append(true)).open(file)?;
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
pub(crate) async fn send(paths: &Paths, channel: u32, message: &Control) -> io::Result<()> {
    let sent = async {
        // Only to our own service (`Paths::connect`).
        let stream = paths.connect().await?;
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
        assert_eq!(
            read_payload(&br#"{"a": 1}"#[..], MAX_INPUT).await,
            json!({"a": 1})
        );
    }

    #[tokio::test]
    async fn invalid_json_is_kept_as_text() {
        assert_eq!(
            read_payload(&b"not json"[..], MAX_INPUT).await,
            json!("not json")
        );
    }

    #[tokio::test]
    async fn input_at_the_limit_is_kept() {
        let text = "x".repeat(MAX_INPUT - 2);
        let input = format!("\"{text}\"");
        assert_eq!(read_payload(input.as_bytes(), MAX_INPUT).await, json!(text));
    }

    #[tokio::test]
    async fn oversized_input_is_replaced_keeping_its_ids() {
        let input = format!("\"{}\"", "x".repeat(MAX_INPUT - 1));
        let payload = read_payload(input.as_bytes(), MAX_INPUT).await;
        assert_eq!(
            payload,
            json!({ "hive_error": "hook input unreadable or larger than 512 KiB" })
        );
        // SYNTHETIC payload shape.
        let input = format!(
            r#"{{"session_id":"s-1","tool_name":"Write","tool_input":{{"content":"{}"}}}}"#,
            "y".repeat(HEAD * 2)
        );
        let payload = read_payload(input.as_bytes(), HEAD).await;
        assert_eq!(
            payload,
            json!({ "hive_error": "hook input unreadable or larger than 4 KiB",
                "session_id": "s-1", "tool_name": "Write" })
        );
        // Only the head is scanned.
        let late = format!(r#"{{"x":"{}","session_id":"s-2"}}"#, "y".repeat(HEAD));
        let payload = read_payload(late.as_bytes(), HEAD).await;
        assert_eq!(payload.get("session_id"), None);
    }

    /// The slimmed `PostToolUse` of a large edit stays far under the frame limit, while the
    /// raw one would be cut to a marker at [`MAX_INPUT`].
    #[tokio::test]
    async fn a_large_post_tool_use_is_read_and_slimmed() {
        let original: String = (0..200_000).map(|i| format!("line {i}\n")).collect();
        // SYNTHETIC payload shape, made-up text.
        let payload = json!({
            "session_id": "s-1", "tool_name": "Edit", "hook_event_name": "PostToolUse",
            "tool_input": {"file_path": "/r/a.txt", "old_string": "line 5", "new_string": "LINE 5"},
            "tool_response": {"filePath": "/r/a.txt", "originalFile": original,
                "structuredPatch": [{"oldStart": 6, "oldLines": 1, "newStart": 6, "newLines": 1,
                    "lines": ["-line 5", "+LINE 5"]}]}
        });
        let input = payload.to_string();
        assert!(input.len() > MAX_INPUT);
        let read = read_payload(input.as_bytes(), MAX_TOOL_INPUT).await;
        let slimmed = crate::slim::slim(read).to_string();
        assert!(slimmed.len() < 1024, "{slimmed}");
        assert!(slimmed.contains(r#""lines":200000"#), "{slimmed}");
    }

    #[cfg(unix)]
    #[test]
    fn the_monotonic_clock_moves_forward() {
        let before = monotonic_ns();
        std::thread::sleep(Duration::from_millis(2));
        let after = monotonic_ns();
        assert!(before > 0);
        assert!(after >= before + 2_000_000, "{before} {after}");
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
        #[cfg(unix)]
        assert_eq!(
            crate::mode::of(&std::fs::metadata(&file).unwrap()) & 0o777,
            0o600
        );
    }
}
