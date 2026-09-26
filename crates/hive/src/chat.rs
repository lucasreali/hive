//! The in-app chat (7.3, design in `docs/spike/chat.md`): the real `claude` in headless mode,
//! `claude -p` speaking stream-json on pipes (no PTY), in its own process group. Everything it
//! prints is untrusted (model and tool output, file contents): each line is bounded, read as
//! JSON and turned into the `chat_*` messages; unknown types and fields are ignored.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use hive_protocol::{
    AgentEvent, ChatEntry, ChatEntryKind, ChatImage, ChatMode, Control, EventKind, ToolStatus,
};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

use crate::adapter::clip;

/// Longest stdout line read; a longer one is skipped (spike 5.2).
pub const MAX_LINE: usize = 16 << 20;
/// Longest text of an entry sent to the app (spike 5.5).
const MAX_TEXT: usize = 64 << 10;
/// Longest one-line summary of a tool call, in characters.
const MAX_SUMMARY: usize = 500;
/// Longest user turn.
const MAX_TURN: usize = 1 << 20;
/// Longest id (`request_id`, `tool_use_id`, `session_id`) or name taken from the stream.
const MAX_ID: usize = 128;
/// Most entries in one `chat_entries`.
const MAX_ENTRIES: usize = 200;
/// Most JSON bytes of entries in one `chat_entries`, well under a frame (`MAX_PAYLOAD`).
const MAX_BATCH: usize = 3 << 20;
/// Most slash commands kept.
const MAX_COMMANDS: usize = 500;
/// Most tool calls of a turn waiting for their result.
const MAX_TOOLS: usize = 1024;
/// Bytes of stderr kept for `chat_closed`.
const STDERR_TAIL: usize = 4 << 10;
/// Each step of closing a chat waits this long for claude to exit (spike 5.6).
pub const GRACE: Duration = Duration::from_secs(5);
/// The same when the service ends (#18).
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
/// Live text is sent to the app at most this often per chat (spike 4.14).
pub const LIVE_EVERY: Duration = Duration::from_millis(50);

/// `claude --permission-mode`'s value for `mode`.
pub fn mode_arg(mode: ChatMode) -> &'static str {
    match mode {
        ChatMode::Default => "default",
        ChatMode::AcceptEdits => "acceptEdits",
        ChatMode::Plan => "plan",
    }
}

/// The mode `claude` reports (`system/init`); `None` for any other (never offered).
fn mode_of(arg: &str) -> Option<ChatMode> {
    [ChatMode::Default, ChatMode::AcceptEdits, ChatMode::Plan]
        .into_iter()
        .find(|&mode| mode_arg(mode) == arg)
}

/// `claude`'s arguments (spike 3.1): headless, stream-json both ways, permission prompts on
/// stdio, Hive's hooks (`settings`, as the wrapper passes them), resuming `resume`.
pub fn args(settings: &Path, mode: ChatMode, resume: Option<&str>) -> Vec<OsString> {
    let fixed = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--replay-user-messages",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        mode_arg(mode),
        "--settings",
    ];
    let mut args: Vec<OsString> = fixed.into_iter().map(OsString::from).collect();
    args.push(settings.into());
    if let Some(id) = resume {
        args.extend(["--resume".into(), id.into()]);
    }
    args
}

/// A session id as `claude --resume` takes it: a UUID (checked before it is ever passed).
pub fn is_session(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// An id from the stream: 1 to [`MAX_ID`] printable ASCII characters.
fn id_of(value: &Value) -> Option<&str> {
    let id = value.as_str()?;
    let fits = (1..=MAX_ID).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic());
    fits.then_some(id)
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// `text` cut at `max` bytes (on a character boundary), saying how much was left out.
fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let end = text.floor_char_boundary(max);
    let more = (text.len() - end).div_ceil(1024);
    format!("{}… ({more} KiB more)", &text[..end])
}

/// A message's content blocks.
fn blocks(content: &Value) -> &[Value] {
    content.as_array().map_or(&[], Vec::as_slice)
}

/// One line for a tool call: its command, path or pattern, else its input as JSON.
fn summary(tool: &str, input: &Value) -> String {
    let key = match tool {
        "Bash" => Some("command"),
        "Read" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => Some("file_path"),
        "Grep" | "Glob" => Some("pattern"),
        "WebFetch" => Some("url"),
        "WebSearch" => Some("query"),
        "Agent" | "Task" => Some("description"),
        _ => None,
    };
    let text = key.and_then(|key| input[key].as_str());
    clip(
        &text.map_or_else(|| input.to_string(), str::to_owned),
        MAX_SUMMARY,
    )
}

/// A tool result's text; images are not shown yet (7.3g).
fn output(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    let parts: Vec<&str> = blocks(content)
        .iter()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => block["text"].as_str(),
            Some("image") => Some("(image not shown)"),
            _ => None,
        })
        .collect();
    parts.join("\n")
}

/// Tokens in the context of an API call: its input, read from and written to the cache.
fn context(usage: &Value) -> u64 {
    [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ]
    .iter()
    .filter_map(|key| usage[key].as_u64())
    .fold(0, u64::saturating_add)
}

/// The footer of a turn: its time, output tokens and how full the context is.
fn usage(result: &Value, context: u64) -> String {
    let secs = result["duration_ms"].as_u64().unwrap_or_default() as f64 / 1000.0;
    let output = result["usage"]["output_tokens"]
        .as_u64()
        .unwrap_or_default();
    let mut text = format!("{secs:.1} s · {output} output tokens");
    let models = result["modelUsage"].as_object().into_iter().flatten();
    let window = models
        .filter_map(|(_, u)| u["contextWindow"].as_u64())
        .max();
    if let Some(window) = window.filter(|&w| w > 0) {
        let percent = context.saturating_mul(100) / window;
        text.push_str(&format!(" · {percent}% context"));
    }
    text
}

/// `secs` since the epoch as a UTC time of day.
fn time_of_day(secs: u64) -> String {
    format!("{:02}:{:02} UTC", secs / 3600 % 24, secs / 60 % 60)
}

/// Splits entries into `chat_entries` messages of at most `most` entries and `bytes` bytes.
fn batches(chat: u32, entries: Vec<ChatEntry>, most: usize, bytes: usize) -> Vec<Control> {
    let mut batches: Vec<(Vec<ChatEntry>, usize)> = Vec::new();
    for entry in entries {
        let len = serde_json::to_vec(&entry).map_or(0, |json| json.len());
        match batches.last_mut() {
            Some((batch, size)) if batch.len() < most && *size + len <= bytes => {
                batch.push(entry);
                *size += len;
            }
            _ => batches.push((vec![entry], len)),
        }
    }
    let message = |(entries, _)| Control::ChatEntries {
        chat,
        entries,
        replace_last: false,
    };
    batches.into_iter().map(message).collect()
}

/// What a chat does after an input: messages for the app, lines for claude, and the end of a
/// turn for the agent states (spike 5.4).
#[derive(Debug, Default, PartialEq)]
pub struct Out {
    pub app: Vec<Control>,
    pub write: Vec<Value>,
    /// Fed to the agent's state like a hook event.
    pub turn: Option<AgentEvent>,
}

/// The entry growing from a content block's deltas (spike 4.14), until its `assistant` message.
#[derive(Debug)]
struct Live {
    /// The block's `index` in its message.
    index: Option<u64>,
    entry: ChatEntry,
    /// The text so far, kept up to a little over [`MAX_TEXT`].
    text: String,
    /// The app has the entry.
    shown: bool,
    /// It grew since it was last sent.
    changed: bool,
}

/// A chat's conversation state, built from claude's stdout (pure: no process).
#[derive(Debug)]
pub struct Stream {
    chat: u32,
    cwd: String,
    last_entry: u32,
    last_request: u32,
    /// Our `initialize` request's id, until it is answered.
    initialize: Option<String>,
    /// Tool calls waiting for their result, by `tool_use_id`: the result updates the entry.
    tools: HashMap<String, ChatEntry>,
    session: Option<String>,
    model: Option<String>,
    mode: ChatMode,
    busy: bool,
    compacting: bool,
    retry: Option<String>,
    /// The turn already showed its error (`assistant.error`): its result adds none.
    failed: bool,
    /// Context tokens of the last main-thread API call.
    context: u64,
    /// The entry growing live, if any.
    live: Option<Live>,
    /// When live text was last sent.
    sent_at: Option<Instant>,
}

impl Stream {
    /// The chat on channel `chat` in `cwd`, resuming `session` if given.
    pub fn new(chat: u32, cwd: String, mode: ChatMode, session: Option<String>) -> Self {
        Self {
            chat,
            cwd,
            last_entry: 0,
            last_request: 0,
            initialize: None,
            tools: HashMap::new(),
            session,
            model: None,
            mode,
            busy: false,
            compacting: false,
            retry: None,
            failed: false,
            context: 0,
            live: None,
            sent_at: None,
        }
    }

    /// A control request of ours, with the next id.
    fn request(&mut self, request: Value) -> Value {
        self.last_request += 1;
        let id = format!("hive-{}", self.last_request);
        json!({"type": "control_request", "request_id": id, "request": request})
    }

    /// The first line claude reads (as the SDK sends it); `chat_opened` follows its answer.
    pub fn initialize(&mut self) -> Value {
        let request = self.request(json!({"subtype": "initialize", "hooks": null}));
        self.initialize = request["request_id"].as_str().map(str::to_owned);
        request
    }

    fn entry(&mut self, kind: ChatEntryKind, text: &str, parent: Option<&str>) -> ChatEntry {
        self.last_entry += 1;
        ChatEntry {
            id: self.last_entry,
            kind,
            text: cut(text, MAX_TEXT),
            tool: None,
            parent: parent.map(str::to_owned),
            status: None,
            output: None,
            image: None,
        }
    }

    pub fn status(&self) -> Control {
        Control::ChatStatus {
            chat: self.chat,
            busy: self.busy,
            mode: self.mode,
            model: self.model.clone(),
            retry: self.retry.clone(),
            compacting: self.compacting,
            session: self.session.clone(),
        }
    }

    /// Runs `change`, then gives the app its entries and the status when it changed.
    fn changed(&mut self, change: impl FnOnce(&mut Self, &mut Vec<ChatEntry>, &mut Out)) -> Out {
        let before = self.status();
        let (mut entries, mut out) = (Vec::new(), Out::default());
        change(self, &mut entries, &mut out);
        out.app
            .extend(batches(self.chat, entries, MAX_ENTRIES, MAX_BATCH));
        let after = self.status();
        if after != before {
            out.app.push(after);
        }
        out
    }

    /// A user turn. Images are refused until 7.3g.
    pub fn send(&mut self, text: &str, images: &[ChatImage]) -> Out {
        self.changed(|chat, entries, out| {
            let refused = if !images.is_empty() {
                Some("Images cannot be sent yet.")
            } else if text.len() > MAX_TURN {
                Some("The message is longer than 1 MiB.")
            } else {
                None
            };
            if let Some(why) = refused {
                return entries.push(chat.entry(ChatEntryKind::Error, why, None));
            }
            out.write.push(json!({
                "type": "user",
                "message": {"role": "user", "content": text},
                "parent_tool_use_id": null,
                "session_id": "default",
            }));
            entries.push(chat.entry(ChatEntryKind::User, text, None));
            chat.busy = true;
        })
    }

    /// Stops the running turn (spike 4.13); its `result` follows.
    pub fn interrupt(&mut self) -> Out {
        let interrupt = self.request(json!({"subtype": "interrupt"}));
        Out {
            write: vec![interrupt],
            ..Out::default()
        }
    }

    pub fn set_mode(&mut self, mode: ChatMode) -> Out {
        let request = json!({"subtype": "set_permission_mode", "mode": mode_arg(mode)});
        let line = self.request(request);
        let mut out = self.changed(|chat, _, _| chat.mode = mode);
        out.write.push(line);
        out
    }

    /// One stdout line; `None` for one longer than [`MAX_LINE`], which was skipped.
    pub fn line(&mut self, line: Option<&[u8]>) -> Out {
        self.changed(
            |chat, entries, out| match line.map(serde_json::from_slice) {
                Some(Ok(message)) => chat.message(&message, entries, out),
                // Not JSON (e.g. a stray print): nothing to show.
                Some(Err(_)) => {}
                None => {
                    let note = "A message too large to show was left out.";
                    entries.push(chat.entry(ChatEntryKind::Note, note, None));
                }
            },
        )
    }

    fn message(&mut self, message: &Value, entries: &mut Vec<ChatEntry>, out: &mut Out) {
        let parent = id_of(&message["parent_tool_use_id"]);
        match (text(&message["type"]), text(&message["subtype"])) {
            ("control_response", _) => self.answered(&message["response"], entries, out),
            ("control_request", _) => self.asked(message, entries, out),
            ("system", "init") => self.init(message),
            ("system", "status") => self.compacting = message["status"] == "compacting",
            ("system", "compact_boundary") => {
                let tokens = message["compact_metadata"]["pre_tokens"].as_u64();
                let text = match tokens {
                    Some(tokens) => format!("Conversation compacted ({}k tokens)", tokens / 1000),
                    None => "Conversation compacted".to_owned(),
                };
                entries.push(self.entry(ChatEntryKind::Divider, &text, None));
            }
            ("system", "conversation_reset") | ("conversation_reset", _) => {
                if let Some(id) = id_of(&message["new_conversation_id"]) {
                    self.session = Some(id.to_owned());
                }
                let divider = "Conversation cleared";
                entries.push(self.entry(ChatEntryKind::Divider, divider, None));
            }
            ("system", "api_retry") => {
                let count = |key: &str| message[key].as_u64().unwrap_or_default();
                let (attempt, most) = (count("attempt"), count("max_retries"));
                self.retry = Some(format!("Retrying {attempt}/{most}…"));
            }
            ("system", "informational") => {
                let note = text(&message["content"]);
                entries.push(self.entry(ChatEntryKind::Note, note, None));
            }
            ("assistant", _) => self.assistant(message, parent, entries),
            ("user", _) => self.user(message, parent, entries),
            ("result", _) => self.result(message, entries, out),
            ("rate_limit_event", _) => self.rate_limit(&message["rate_limit_info"], entries),
            // Main thread only; a subagent's would be ignored.
            ("stream_event", _) if parent.is_none() => self.streamed(&message["event"]),
            // `tool_progress`, hooks, tasks, …
            _ => {}
        }
    }

    /// The answer to one of our requests: `initialize`'s opens the chat.
    fn answered(&mut self, response: &Value, entries: &mut Vec<ChatEntry>, out: &mut Out) {
        let ours = self.initialize.as_deref();
        if ours.is_none_or(|id| response["request_id"] != id) {
            // Interrupt receipts and mode changes need nothing.
            return;
        }
        self.initialize = None;
        if response["subtype"] == "error" {
            let error = format!("claude did not start: {}", text(&response["error"]));
            entries.push(self.entry(ChatEntryKind::Error, &error, None));
        }
        let names = blocks(&response["response"]["commands"]).iter();
        let names = names.filter_map(|command| id_of(&command["name"]));
        out.app.push(Control::ChatOpened {
            chat: self.chat,
            cwd: self.cwd.clone(),
            session: self.session.clone(),
            model: self.model.clone(),
            mode: self.mode,
            commands: names.take(MAX_COMMANDS).map(str::to_owned).collect(),
            // Only in `system/init`, which comes with the first turn (7.3i).
            api_key_source: None,
        });
    }

    /// A request to us. Permission prompts, questions and plans are denied until 7.3e, which
    /// answers them here; any other (SDK hooks, SDK MCP servers: we register none) is refused.
    fn asked(&mut self, message: &Value, entries: &mut Vec<ChatEntry>, out: &mut Out) {
        let Some(id) = id_of(&message["request_id"]) else {
            return;
        };
        let request = &message["request"];
        let response = if request["subtype"] == "can_use_tool" {
            let tool = clip(text(&request["tool_name"]), MAX_ID);
            let note = format!("{tool} was denied: Hive cannot answer permission requests yet.");
            entries.push(self.entry(ChatEntryKind::Note, &note, None));
            let why = "Denied: the app cannot answer permission requests yet.";
            let deny = json!({"behavior": "deny", "message": why, "interrupt": false});
            json!({"subtype": "success", "request_id": id, "response": deny})
        } else {
            json!({"subtype": "error", "request_id": id, "error": "not supported"})
        };
        out.write
            .push(json!({"type": "control_response", "response": response}));
    }

    fn init(&mut self, message: &Value) {
        if let Some(id) = id_of(&message["session_id"]) {
            self.session = Some(id.to_owned());
        }
        if let Some(model) = id_of(&message["model"]) {
            self.model = Some(model.to_owned());
        }
        if let Some(mode) = message["permissionMode"].as_str().and_then(mode_of) {
            self.mode = mode;
        }
    }

    /// One entry per content block: text, thinking, tool call (spike 4.2–4.4).
    fn assistant(&mut self, message: &Value, parent: Option<&str>, entries: &mut Vec<ChatEntry>) {
        self.retry = None;
        // e.g. `model_not_found`, `rate_limit`: its text is the error.
        let failed = message["error"].is_string();
        self.failed |= failed;
        let message = &message["message"];
        if parent.is_none() {
            self.context = context(&message["usage"]);
        }
        for block in blocks(&message["content"]) {
            let (assistant, thinking) = (ChatEntryKind::Assistant, ChatEntryKind::Thinking);
            let entry = match block["type"].as_str() {
                Some("text") if failed => {
                    let error = ChatEntryKind::Error;
                    self.block(assistant, error, text(&block["text"]), parent)
                }
                Some("text") => self.block(assistant, assistant, text(&block["text"]), parent),
                Some("thinking") => {
                    self.block(thinking, thinking, text(&block["thinking"]), parent)
                }
                Some("tool_use") => self.tool(block, parent),
                // `redacted_thinking` has nothing to read.
                _ => continue,
            };
            entries.push(entry);
        }
    }

    /// A text or thinking block's entry (`kind`); the entry that grew live from it (as
    /// `streamed`) gives its id, so the app replaces it.
    fn block(
        &mut self,
        streamed: ChatEntryKind,
        kind: ChatEntryKind,
        text: &str,
        parent: Option<&str>,
    ) -> ChatEntry {
        let live = (self.live).take_if(|live| parent.is_none() && live.entry.kind == streamed);
        match live {
            Some(live) => ChatEntry {
                kind,
                text: cut(text, MAX_TEXT),
                ..live.entry
            },
            None => self.entry(kind, text, parent),
        }
    }

    /// A text or thinking delta grows its block's live entry, sent by [`Self::flush`]; other
    /// events are ignored (`input_json_delta`: the tool row comes with its `assistant`).
    fn streamed(&mut self, event: &Value) {
        let delta = &event["delta"];
        let (kind, piece) = match (text(&event["type"]), text(&delta["type"])) {
            ("content_block_delta", "text_delta") => (ChatEntryKind::Assistant, &delta["text"]),
            ("content_block_delta", "thinking_delta") => {
                (ChatEntryKind::Thinking, &delta["thinking"])
            }
            _ => return,
        };
        let index = event["index"].as_u64();
        let mut live = match self.live.take() {
            Some(live) if live.index == index && live.entry.kind == kind => live,
            // A new block (one left without its `assistant` stays as it was last sent).
            _ => Live {
                index,
                entry: self.entry(kind, "", None),
                text: String::new(),
                shown: false,
                changed: false,
            },
        };
        if live.text.len() <= MAX_TEXT {
            live.text.push_str(text(piece));
        }
        live.changed = true;
        self.live = Some(live);
    }

    /// When the live text should be sent: at `now` if none was sent in the last
    /// [`LIVE_EVERY`], else when that ends; `None` when it has not changed.
    pub fn due(&self, now: Instant) -> Option<Instant> {
        self.live.as_ref().filter(|live| live.changed)?;
        Some(self.sent_at.map_or(now, |at| (at + LIVE_EVERY).max(now)))
    }

    /// Sends the live text if it is due at `now`: the entry replaces the app's last one when it
    /// is that one already (`replace_last`), else its own `id`.
    pub fn flush(&mut self, now: Instant) -> Out {
        let (mut out, due, last) = (Out::default(), self.due(now), self.last_entry);
        let Some(live) = self
            .live
            .as_mut()
            .filter(|_| due.is_some_and(|due| due <= now))
        else {
            return out;
        };
        live.entry.text = cut(&live.text, MAX_TEXT);
        let replace_last = live.shown && live.entry.id == last;
        (live.shown, live.changed) = (true, false);
        self.sent_at = Some(now);
        out.app.push(Control::ChatEntries {
            chat: self.chat,
            entries: vec![live.entry.clone()],
            replace_last,
        });
        out
    }

    fn tool(&mut self, block: &Value, parent: Option<&str>) -> ChatEntry {
        let name = clip(text(&block["name"]), MAX_ID);
        let mut entry = self.entry(
            ChatEntryKind::Tool,
            &summary(&name, &block["input"]),
            parent,
        );
        entry.tool = Some(name);
        entry.status = Some(ToolStatus::Running);
        if let Some(id) = id_of(&block["id"])
            && self.tools.len() < MAX_TOOLS
        {
            self.tools.insert(id.to_owned(), entry.clone());
        }
        entry
    }

    /// Tool results update their call's entry; a subagent's prompt is a user entry. Replays of
    /// our own turns were shown when sent.
    fn user(&mut self, message: &Value, parent: Option<&str>, entries: &mut Vec<ChatEntry>) {
        if message["isReplay"] == true {
            return;
        }
        let content = &message["message"]["content"];
        if let Some(text) = content.as_str() {
            return entries.push(self.entry(ChatEntryKind::User, text, parent));
        }
        for block in blocks(content) {
            match block["type"].as_str() {
                Some("text") => {
                    entries.push(self.entry(ChatEntryKind::User, text(&block["text"]), parent));
                }
                Some("tool_result") => {
                    let call = id_of(&block["tool_use_id"]).and_then(|id| self.tools.remove(id));
                    let Some(mut entry) = call else { continue };
                    let failed = block["is_error"] == true;
                    let status = if failed {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Ok
                    };
                    entry.status = Some(status);
                    entry.output = Some(cut(&output(&block["content"]), MAX_TEXT));
                    entries.push(entry);
                }
                _ => {}
            }
        }
    }

    /// The end of a turn (spike 4.10, 4.12, 4.13): an error or "Interrupted", then the usage.
    fn result(&mut self, message: &Value, entries: &mut Vec<ChatEntry>, out: &mut Out) {
        // Live text cut by the end of the turn stays as it grew.
        if let Some(mut live) = self.live.take().filter(|live| live.changed) {
            live.entry.text = cut(&live.text, MAX_TEXT);
            entries.push(live.entry);
        }
        let interrupted = text(&message["terminal_reason"]).starts_with("aborted");
        let failed =
            !interrupted && (message["is_error"] == true || message["subtype"] != "success");
        if interrupted {
            entries.push(self.entry(ChatEntryKind::Note, "Interrupted", None));
        } else if failed && !self.failed {
            let errors = blocks(&message["errors"]).iter().filter_map(Value::as_str);
            let errors: Vec<&str> = errors.collect();
            let error = match (errors.join("\n"), message["result"].as_str()) {
                (errors, _) if !errors.is_empty() => errors,
                (_, Some(result)) => result.to_owned(),
                _ => format!(
                    "The turn failed ({}).",
                    clip(text(&message["subtype"]), MAX_ID)
                ),
            };
            entries.push(self.entry(ChatEntryKind::Error, &error, None));
        }
        let footer = usage(message, self.context);
        entries.push(self.entry(ChatEntryKind::Usage, &footer, None));
        let kind = match failed {
            true => EventKind::TurnFailed { error: None },
            false => EventKind::TurnFinished,
        };
        out.turn = self
            .session
            .clone()
            .map(|id| agent_event(self.chat, id, kind));
        self.busy = false;
        self.compacting = false;
        self.retry = None;
        self.failed = false;
        // Calls cut by the end of the turn get no result.
        self.tools.clear();
    }

    fn rate_limit(&mut self, info: &Value, entries: &mut Vec<ChatEntry>) {
        if info["status"] != "rejected" {
            return;
        }
        let mut error = "Usage limit reached".to_owned();
        if let Some(at) = info["resetsAt"].as_u64() {
            error.push_str(&format!("; it resets at {}", time_of_day(at)));
        }
        error.push('.');
        entries.push(self.entry(ChatEntryKind::Error, &error, None));
    }
}

/// A chat's event for the agent states, as a hook would send it from its terminal.
fn agent_event(chat: u32, session: String, kind: EventKind) -> AgentEvent {
    AgentEvent {
        provider: "claude-code".to_owned(),
        terminal_id: Some(chat.to_string()),
        session_id: Some(session),
        subagent: None,
        cwd: None,
        kind,
        activity: None,
        raw: Value::Null,
    }
}

/// Where a chat opens, kept while the human confirms its project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub cwd: String,
    pub project: String,
    pub resume: Option<String>,
    pub mode: ChatMode,
}

/// A running chat, as kept in the service registry.
pub struct Chat {
    pub stream: Stream,
    /// Its space's Claude config folder (see `Terminal::claude_dir`).
    pub claude_dir: Option<String>,
    /// Lines to claude's stdin; dropped (stdin closes) when the chat is closed.
    stdin: Option<mpsc::UnboundedSender<Value>>,
    /// claude's process group: its pid, as the system gave it. A field, not a method, so no
    /// mutant can turn it into 0 (the service's own group) or 1 (every process).
    pub group: i32,
    /// The app closed it: its exit is not a failure.
    pub closing: bool,
}

/// A started chat's output side and process.
pub struct Pipes {
    pub child: Child,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
}

impl Chat {
    /// Runs `program` (the real `claude`, see [`args`]) in `cwd` with pipes, in a process
    /// group of its own, with `env` (as a terminal's) and `HIVE_TERMINAL_ID` = the chat's
    /// channel, so its hooks reach the service like a terminal's. Sends `initialize`.
    pub fn start(
        mut stream: Stream,
        program: &Path,
        args: &[OsString],
        cwd: &str,
        env: &[(&'static str, String)],
    ) -> io::Result<(Self, Pipes)> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .envs(env.iter().cloned())
            .env("HIVE_TERMINAL_ID", stream.chat.to_string())
            // Hooks come from `--settings`, as the wrapper passes them (#21).
            .env("HIVE_WRAPPED", "1")
            .env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        // Always there: every stream is piped and the child was not waited for yet.
        let missing = io::ErrorKind::BrokenPipe;
        let stdin = child.stdin.take().ok_or(missing)?;
        let stdout = child.stdout.take().ok_or(missing)?;
        let stderr = child.stderr.take().ok_or(missing)?;
        let group = child.id().ok_or(missing)? as i32;
        let (lines, input) = mpsc::unbounded_channel();
        tokio::spawn(feed(stdin, input));
        let _ = lines.send(stream.initialize());
        let chat = Self {
            stream,
            claude_dir: None,
            stdin: Some(lines),
            group,
            closing: false,
        };
        Ok((
            chat,
            Pipes {
                child,
                stdout,
                stderr,
            },
        ))
    }

    /// Writes `out`'s lines to claude; returns the rest for the service.
    pub fn run(&mut self, mut out: Out) -> Out {
        for line in out.write.drain(..) {
            if let Some(stdin) = &self.stdin {
                let _ = stdin.send(line);
            }
        }
        out
    }

    /// Closes claude's stdin (it ends after the running turn). Returns whether it was open.
    pub fn close(&mut self) -> bool {
        self.closing = true;
        self.stdin.take().is_some()
    }
}

/// Writes lines to claude until the chat drops its sender. A write error (claude gone) drops
/// the line: the process's exit ends the chat.
async fn feed(mut stdin: ChildStdin, mut lines: mpsc::UnboundedReceiver<Value>) {
    while let Some(line) = lines.recv().await {
        let mut bytes = line.to_string().into_bytes();
        bytes.push(b'\n');
        let _ = stdin.write_all(&bytes).await;
    }
}

/// Reads the next line (without its newline) into `buf`: `Some(true)`, or `Some(false)` for a
/// line longer than `max`, skipped whole (`buf` left empty); `None` at the end.
pub async fn next_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<Option<bool>> {
    buf.clear();
    let limit = max as u64 + 1;
    if (&mut *reader).take(limit).read_until(b'\n', buf).await? == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        return Ok(Some(true));
    }
    if buf.len() <= max {
        // The last line, without a newline.
        return Ok(Some(true));
    }
    // The rest of the line, in bounded reads, up to its newline or the end.
    loop {
        buf.clear();
        let read = (&mut *reader).take(limit).read_until(b'\n', buf).await?;
        if matches!((read, buf.last()), (0, _) | (_, Some(b'\n'))) {
            buf.clear();
            return Ok(Some(false));
        }
    }
}

/// The last [`STDERR_TAIL`] bytes of claude's stderr, read until it closes.
pub async fn tail(mut stderr: impl AsyncRead + Unpin) -> Vec<u8> {
    let (mut tail, mut buf) = (Vec::new(), [0; 4096]);
    while let Ok(n @ 1..) = stderr.read(&mut buf).await {
        tail.extend_from_slice(&buf[..n]);
        let extra = tail.len().saturating_sub(STDERR_TAIL);
        tail.drain(..extra);
    }
    tail
}

/// Why a chat ended, for `chat_closed`: nothing when the app closed it or claude exited
/// cleanly; else its last stderr lines, or how it exited.
pub fn ended(status: Option<ExitStatus>, closing: bool, stderr: &[u8]) -> Option<String> {
    if closing || status.is_some_and(|s| s.success()) {
        return None;
    }
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    Some(match status {
        _ if !stderr.is_empty() => stderr,
        Some(status) => format!("claude ended ({status})"),
        None => "claude ended".to_owned(),
    })
}

/// Ends the process group `group` (spike 5.6), stdin being closed already: SIGINT, SIGTERM,
/// then SIGKILL, each after `grace` if the group is still there.
pub async fn stop(group: i32, grace: Duration) {
    let group = Pid::from_raw(group);
    for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGKILL] {
        let gone = async {
            while killpg(group, None).is_ok() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        if tokio::time::timeout(grace, gone).await.is_ok() {
            return;
        }
        let _ = killpg(group, signal);
    }
}

/// Ends every chat's group when the service ends (#18).
pub async fn end(groups: Vec<i32>) {
    let stopping: Vec<_> = groups
        .into_iter()
        .map(|group| tokio::spawn(stop(group, SHUTDOWN_GRACE)))
        .collect();
    for task in stopping {
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests;
