//! What a remote service may cost the app (26.19, `docs/spike/ssh.md` §8): a remote is someone
//! else's machine, so each of its links has caps the local service's has not. Past one, the
//! link is dropped and the UI told why (`host_link` `disconnected`, kind `limit`); its text is
//! clipped before anything reads it.
//!
//! - **Messages:** at most [`RATE`] control or data frames, and [`RATE_BYTES`] bytes of them, a
//!   second; at most [`ALERTS`] that would show an OS notification in [`ALERT_WINDOW`]. One the
//!   UI takes is read as its type ([`typed`]): a field of another type ends the link, an
//!   unknown one is dropped.
//! - **Lists:** at most [`PROJECTS`] projects, [`SESSIONS`] sessions, [`COMMITS`] commits, and
//!   [`LIST`] items in any other list of a message.
//! - **Terminal output:** at most [`UNACKED`] bytes of one terminal's output the UI has not
//!   acknowledged (9.19), the service stopping at 512 KiB and a read, and [`LINK_UNACKED`] of
//!   all of a link's terminals.
//! - **Queue:** at most [`QUEUE_FRAMES`] frames, or [`QUEUE_BYTES`] bytes, waiting to be written
//!   to a remote that stopped reading them.
//! - **Text:** titles, badges, errors and the like ([`CLIPPED`]) cut at [`TEXT`] characters.
//!
//! A frame over `MAX_PAYLOAD` or of an unknown type breaks the stream (the codec), which is
//! retried as a network failure, as before.

use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

use hive_protocol::Control;
use serde_json::Value;
use tokio::sync::Notify;

/// The most control and data frames a remote sends in [`RATE_WINDOW`].
pub(crate) const RATE: usize = 1000;
const RATE_WINDOW: Duration = Duration::from_secs(1);
/// The most bytes of control and data frames a remote sends in [`RATE_WINDOW`].
pub(crate) const RATE_BYTES: usize = 16 << 20;
/// The most messages that show an OS notification a remote sends in [`ALERT_WINDOW`].
pub(crate) const ALERTS: usize = 20;
const ALERT_WINDOW: Duration = Duration::from_secs(10);
/// The most projects a remote's `projects` lists.
pub(crate) const PROJECTS: usize = 1000;
/// The most sessions a remote's `sessions` lists (its service sends at most 500).
pub(crate) const SESSIONS: usize = 1000;
/// The most commits a remote's `commits` lists (its service sends at most 500, 27.2).
pub(crate) const COMMITS: usize = 1000;
/// The most items of any other list in a remote's message (its service lists at most 50,000
/// files).
pub(crate) const LIST: usize = 50_000;
/// The most bytes of one remote terminal's output the UI has not acknowledged.
pub(crate) const UNACKED: u64 = 16 << 20;
/// The most bytes of all of a remote link's terminals' output the UI has not acknowledged.
pub(crate) const LINK_UNACKED: u64 = 64 << 20;
/// The most frames, and bytes, waiting to be written to a remote.
pub(crate) const QUEUE_FRAMES: usize = 1024;
pub(crate) const QUEUE_BYTES: usize = 64 << 20;
/// The most agents a link keeps as making quitting ask (`Link::at_risk`), and the longest id
/// of one, in bytes: a session id is 36.
pub(crate) const AGENTS: usize = 1024;
pub(crate) const ID: usize = 256;
/// The most characters of a remote's text in [`CLIPPED`] fields.
pub(crate) const TEXT: usize = 1024;
/// The fields of a remote's messages that hold text shown as is (titles, badges, errors), at
/// any depth.
const CLIPPED: &[&str] = &[
    "title", "text", "message", "error", "reason", "activity", "name", "account", "subject",
    "author",
];
/// Fields clipped only in one type's messages: `body` is also a pull request's description.
const CLIPPED_IN: &[(&str, &str)] = &[("terminal_notification", "body")];

/// Why the link was dropped once its remote stopped reading what the app sends.
pub(crate) const QUEUE_FULL: &str = "stopped reading what the app sends";

/// A count over a time window: it starts again at the first count past the window.
#[derive(Debug, Default)]
struct Window {
    start: Option<Instant>,
    total: usize,
}

impl Window {
    /// Adds `n` at `now` to a window `span` long; the total in it.
    fn add(&mut self, now: Instant, span: Duration, n: usize) -> usize {
        match self.start {
            Some(start) if now.duration_since(start) < span => {
                self.total = self.total.saturating_add(n);
            }
            _ => (self.start, self.total) = (Some(now), n),
        }
        self.total
    }
}

/// Counts a remote's frames, their bytes and its notifications: more than [`RATE`] frames or
/// [`RATE_BYTES`] bytes in a [`RATE_WINDOW`], or more than [`ALERTS`] in an [`ALERT_WINDOW`],
/// is a flood.
#[derive(Debug, Default)]
pub(crate) struct Rate {
    frames: Window,
    bytes: Window,
    alerts: Window,
}

impl Rate {
    /// One frame of `len` bytes read at `now`; an error once there were too many.
    pub(crate) fn count(&mut self, now: Instant, len: usize) -> Result<(), String> {
        if self.frames.add(now, RATE_WINDOW, 1) > RATE {
            return Err(format!("sent more than {RATE} messages in a second"));
        }
        if self.bytes.add(now, RATE_WINDOW, len) > RATE_BYTES {
            return Err("sent more than 16 MiB of messages in a second".to_owned());
        }
        Ok(())
    }

    /// `message`, read at `now`, would show an OS notification ([`alerting`]); an error once
    /// there were too many.
    pub(crate) fn alert(&mut self, now: Instant, message: &Value) -> Result<(), String> {
        let n = usize::from(alerting(message));
        if self.alerts.add(now, ALERT_WINDOW, n) > ALERTS {
            return Err(format!(
                "sent more than {ALERTS} notifications in 10 seconds"
            ));
        }
        Ok(())
    }
}

/// Whether the UI shows an OS notification for `message` (`src/notify.ts`): a command that
/// finished, a program's notification, a usage alert, or an agent's state that says `notify`.
pub(crate) fn alerting(message: &Value) -> bool {
    let kind = message["type"].as_str().unwrap_or_default();
    let always = ["command_finished", "terminal_notification", "usage_alert"];
    always.contains(&kind) || message["notify"] == true
}

/// A remote's `message` for the UI as its own type would write it (`Control`): a field of
/// another type is an error, an unknown field is dropped. Its `channel` and `host` are kept.
pub(crate) fn typed(message: Value) -> Result<Value, String> {
    let kind = message["type"].as_str().unwrap_or_default().to_owned();
    let (channel, host) = (message["channel"].clone(), message["host"].clone());
    let control = serde_json::from_value::<Control>(message);
    let control = control.map_err(|_| format!("sent a {kind} the app cannot read"))?;
    let mut typed = serde_json::to_value(control).unwrap_or_default();
    typed["channel"] = channel;
    typed["host"] = host;
    Ok(typed)
}

/// The frames a connection's writer has not written yet; capped on a remote's link
/// (`limited`), where going past the cap wakes [`Queue::full`].
#[derive(Debug, Default)]
pub(crate) struct Queue {
    limited: bool,
    frames: AtomicUsize,
    bytes: AtomicUsize,
    /// Woken when a frame did not fit: the link ends ([`QUEUE_FULL`]).
    pub(crate) full: Notify,
}

impl Queue {
    pub(crate) fn new(limited: bool) -> Self {
        Self {
            limited,
            ..Self::default()
        }
    }

    /// A frame of `len` bytes queued; false (and not counted) when it does not fit.
    pub(crate) fn push(&self, len: usize) -> bool {
        let frames = self.frames.fetch_add(1, SeqCst) + 1;
        let bytes = self.bytes.fetch_add(len, SeqCst) + len;
        if self.limited && (frames > QUEUE_FRAMES || bytes > QUEUE_BYTES) {
            self.pop(len);
            self.full.notify_one();
            return false;
        }
        true
    }

    /// A frame of `len` bytes written.
    pub(crate) fn pop(&self, len: usize) {
        self.frames.fetch_sub(1, SeqCst);
        self.bytes.fetch_sub(len, SeqCst);
    }
}

/// Why a remote terminal's output that the UI has not acknowledged, `unacked` bytes of it and
/// `all` of its link's terminals, drops the link: `None` while within [`UNACKED`] and
/// [`LINK_UNACKED`].
pub(crate) fn unacked(unacked: u64, all: u64) -> Option<String> {
    let over = unacked > UNACKED || all > LINK_UNACKED;
    over.then(|| "sent more terminal output than the app could show".to_owned())
}

/// Checks a remote's control `message` against the list caps, then clips its text.
pub(crate) fn check(message: &mut Value) -> Result<(), String> {
    let kind = message["type"].as_str().unwrap_or_default().to_owned();
    for (list, most) in [
        ("projects", PROJECTS),
        ("sessions", SESSIONS),
        ("commits", COMMITS),
    ] {
        let len = message[list].as_array().map_or(0, Vec::len);
        if kind == list && len > most {
            return Err(format!("sent {len} {list}, over the {most} the app takes"));
        }
    }
    walk(message, &kind, "")
}

/// Checks every list in `value` (a field `key` of a `kind` message) and clips its text.
fn walk(value: &mut Value, kind: &str, key: &str) -> Result<(), String> {
    match value {
        Value::Array(items) if items.len() > LIST => Err(format!(
            "sent a list of {} items, over the {LIST} the app takes",
            items.len()
        )),
        Value::Array(items) => items.iter_mut().try_for_each(|item| walk(item, kind, key)),
        Value::Object(fields) => fields
            .iter_mut()
            .try_for_each(|(key, value)| walk(value, kind, key)),
        Value::String(text) if CLIPPED.contains(&key) || CLIPPED_IN.contains(&(kind, key)) => {
            clip(text);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Cuts `text` at [`TEXT`] characters, marked with an ellipsis.
fn clip(text: &mut String) {
    if let Some((end, _)) = text.char_indices().nth(TEXT) {
        text.truncate(end);
        text.push('…');
    }
}

#[cfg(test)]
mod tests;
