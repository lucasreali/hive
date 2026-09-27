//! A subagent's conversation for the app's read-only view (6.10), from the transcript Claude
//! Code writes beside its agent's: `<session id>/subagents/agent-<agent_id>.jsonl` next to
//! `<session id>.jsonl`. Transcripts are only read, never written.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use hive_protocol::{Control, TranscriptEntry, TranscriptRole};
use serde_json::Value;

/// Entries sent at most in one message; earlier ones are left out.
pub const MAX_ENTRIES: usize = 200;
/// Most characters kept of an entry's text; with [`MAX_ENTRIES`] a message stays under a frame
/// even when every character needs escaping.
const TEXT_LIMIT: usize = 2000;
/// Most bytes read at once: the tail when watching starts, then the new bytes of each poll.
const READ_LIMIT: u64 = 8_388_608; // 8 MiB
/// Longest `transcript_path` kept from a hook payload.
const PATH_LIMIT: usize = 4096;
/// Longest subagent id accepted.
const ID_LIMIT: usize = 64;

/// The agent's transcript from a hook payload's `transcript_path`, when it is an absolute
/// `.jsonl` path of reasonable length. Where it points is checked when it is read.
pub fn transcript_path(raw: &Value) -> Option<PathBuf> {
    let path = raw.get("transcript_path")?.as_str()?;
    let path = Path::new(path);
    (path.as_os_str().len() <= PATH_LIMIT
        && path.is_absolute()
        && path.extension().is_some_and(|e| e == "jsonl"))
    .then(|| path.to_owned())
}

/// The transcript of the subagent `id` beside the agent's `parent` one, when `id` is an agent
/// id (`[A-Za-z0-9_-]`, at most [`ID_LIMIT`]), so it cannot leave the `subagents` folder.
pub fn subagent_path(parent: &Path, id: &str) -> Option<PathBuf> {
    let valid = !id.is_empty()
        && id.len() <= ID_LIMIT
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then(|| {
        parent
            .with_extension("")
            .join("subagents")
            .join(format!("agent-{id}.jsonl"))
    })
}

/// Opens `path` only when, links resolved, it is a regular file inside `root`.
fn open_inside(root: &Path, path: &Path) -> io::Result<File> {
    let real = path.canonicalize()?;
    if !real.starts_with(root.canonicalize()?) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the transcript is outside Claude's projects folder",
        ));
    }
    // Checked before opening: opening a FIFO would block the service until a writer came.
    // ponytail: a swap between this check and the open needs write access to Claude's
    // folder (the same user); open with O_NONBLOCK | O_NOFOLLOW if that ever matters.
    if !real.metadata()?.is_file() {
        return Err(io::Error::other("the transcript is not a file"));
    }
    File::open(&real)
}

/// The entries of JSONL `records`: the text of user and assistant messages and each tool
/// call (its input as compact JSON), cut at [`TEXT_LIMIT`]. Meta messages, thinking, tool
/// results and lines that are not JSON are left out.
pub fn entries(records: &[u8]) -> Vec<TranscriptEntry> {
    let mut entries = Vec::new();
    for line in records.split(|&b| b == b'\n') {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let role = match record.get("type").and_then(Value::as_str) {
            Some("user") => TranscriptRole::User,
            Some("assistant") => TranscriptRole::Assistant,
            _ => continue,
        };
        if record.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let mut push = |role, text: &str, tool: Option<&str>| {
            let text = text.trim();
            if !text.is_empty() {
                entries.push(TranscriptEntry {
                    role,
                    text: text.chars().take(TEXT_LIMIT).collect(),
                    tool: tool.map(|t| t.chars().take(ID_LIMIT).collect()),
                });
            }
        };
        match record.pointer("/message/content") {
            Some(Value::String(text)) => push(role, text, None),
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    let field = |key: &str| block.get(key).and_then(Value::as_str);
                    match field("type") {
                        Some("text") => push(role, field("text").unwrap_or_default(), None),
                        Some("tool_use") => {
                            let input = block.get("input").map(Value::to_string);
                            let tool = Some(field("name").unwrap_or("tool"));
                            push(TranscriptRole::Tool, &input.unwrap_or_default(), tool);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    entries
}

/// A subagent's transcript being followed: read from its tail, then as it grows.
#[derive(Debug)]
pub struct Watch {
    pub agent: String,
    pub subagent: String,
    path: PathBuf,
    /// Claude's projects folder, which the transcript must stay inside.
    root: PathBuf,
    /// Up to where the transcript was read (always the end of a line).
    offset: u64,
}

impl Watch {
    pub fn new(agent: String, subagent: String, path: PathBuf, root: PathBuf) -> Self {
        Self {
            agent,
            subagent,
            path,
            root,
            offset: 0,
        }
    }

    /// The first message: the conversation so far (none while the transcript is not written
    /// yet), or why it cannot be read.
    pub fn start(&mut self) -> Control {
        match self.read() {
            Ok((entries, truncated)) => Control::Transcript {
                agent: self.agent.clone(),
                subagent: self.subagent.clone(),
                entries,
                truncated,
            },
            Err(err) => Control::Error {
                message: format!("cannot read the subagent's transcript: {err}"),
            },
        }
    }

    /// What was written since the last read, if anything.
    pub fn poll(&mut self) -> Option<Control> {
        let (entries, _) = self.read().ok()?;
        (!entries.is_empty()).then(|| Control::TranscriptAppended {
            agent: self.agent.clone(),
            subagent: self.subagent.clone(),
            entries,
        })
    }

    /// The entries of the whole lines written since `offset`, at most the last
    /// [`READ_LIMIT`] bytes of them and the last [`MAX_ENTRIES`], and whether any were left
    /// out. A transcript that shrank is read again from the start.
    fn read(&mut self) -> io::Result<(Vec<TranscriptEntry>, bool)> {
        let (bytes, skipped, _) = read_lines(&self.root, &self.path, &mut self.offset)?;
        let mut entries = entries(&bytes);
        let over = entries.len().saturating_sub(MAX_ENTRIES);
        entries.drain(..over);
        Ok((entries, skipped || over > 0))
    }
}

/// The whole lines of the transcript at `path` (inside `root`), at most its last
/// [`READ_LIMIT`] bytes, and whether earlier ones were left out: a resumed chat's history.
pub fn tail(root: &Path, path: &Path) -> io::Result<(Vec<u8>, bool)> {
    let (mut bytes, skipped, _) = read_lines(root, path, &mut 0)?;
    if skipped {
        // The tail starts inside a line: it is left out.
        let first = bytes.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
        bytes.drain(..first);
    }
    Ok((bytes, skipped))
}

/// The whole lines of the transcript at `path` (inside `root`) written since `offset`, at most
/// its last [`READ_LIMIT`] bytes; whether earlier ones were skipped, and whether it was read
/// again from its start because it shrank. `offset` moves to the end of what was read. A
/// transcript not written yet has no lines.
fn read_lines(root: &Path, path: &Path, offset: &mut u64) -> io::Result<(Vec<u8>, bool, bool)> {
    let mut file = match open_inside(root, path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((Vec::new(), false, false));
        }
        file => file?,
    };
    let len = file.metadata()?.len();
    let restarted = len < *offset;
    if restarted {
        *offset = 0;
    }
    let start = (*offset).max(len.saturating_sub(READ_LIMIT));
    let skipped = start > *offset;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT).read_to_end(&mut bytes)?;
    // A line still being written waits for the next read; one longer than the limit is
    // skipped (what is left of it does not parse).
    let whole = match bytes.iter().rposition(|&b| b == b'\n') {
        Some(end) => end + 1,
        None if bytes.len() as u64 == READ_LIMIT => bytes.len(),
        None => 0,
    };
    *offset = start + whole as u64;
    bytes.truncate(whole);
    Ok((bytes, skipped, restarted))
}

/// Largest token count taken from one usage field; a larger one counts as 0.
const TOKEN_LIMIT: u64 = 100_000_000;
/// The usual context window, assumed for a model whose window is not known.
pub const CONTEXT_LIMIT: u64 = 200_000;
/// The long context window.
pub const LONG_CONTEXT_LIMIT: u64 = 1_000_000;
/// Models whose window is 1M without the `[1m]` suffix, by model id prefix: Claude Code
/// 2.1.283's `result.modelUsage` gave 1M for a plain `claude-opus-5-5` and `claude-sonnet-5`
/// (200k for `claude-haiku-4-5-20251001`), and the models overview gives 1M as the default
/// window of the Opus 5 and Fable/Mythos 5 families. Opus/Sonnet 4.x are left to [`Windows`]
/// and the growth rule: Claude Code has run them at 200k unless asked for `[1m]`.
const LONG_MODELS: [&str; 4] = [
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-fable-5",
    "claude-mythos-5",
];
/// Most models whose window is remembered.
const MAX_MODELS: usize = 64;
/// Longest message id remembered.
const MESSAGE_ID_LIMIT: usize = 128;

/// A conversation's tokens, from the `message.usage` of its assistant records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    /// The context of the last turn: its input, cache writes and cache reads.
    pub context: u64,
    /// Output tokens of the whole conversation.
    pub output: u64,
    /// The context went past [`CONTEXT_LIMIT`] once.
    long: bool,
    /// The model of the last counted message (`message.model`, never with a `[1m]` suffix).
    model: Option<String>,
    /// The last counted message's id and output: Claude writes a message's content blocks as
    /// separate records repeating its usage, so a message is counted once.
    last: Option<(String, u64)>,
}

impl Tokens {
    /// Counts a transcript record: an assistant message with a usage, outside a subagent's
    /// sidechain. Fields that are missing, not numbers or over [`TOKEN_LIMIT`] count as 0; a
    /// usage without context (an API error) is left out.
    pub fn add(&mut self, record: &Value) {
        let sidechain = record.get("isSidechain").and_then(Value::as_bool) == Some(true);
        if record.get("type").and_then(Value::as_str) != Some("assistant") || sidechain {
            return;
        }
        let Some(usage) = record.pointer("/message/usage") else {
            return;
        };
        let count = |key: &str| {
            let n = usage.get(key).and_then(Value::as_u64);
            n.filter(|&n| n <= TOKEN_LIMIT).unwrap_or(0)
        };
        let context = count("input_tokens")
            + count("cache_creation_input_tokens")
            + count("cache_read_input_tokens");
        if context == 0 {
            return;
        }
        self.context = context;
        self.long |= context > CONTEXT_LIMIT;
        let model = record.pointer("/message/model").and_then(Value::as_str);
        if let Some(model) = model.filter(|m| m.len() <= MESSAGE_ID_LIMIT) {
            self.model = Some(model.to_owned());
        }
        let output = count("output_tokens");
        let id = record.pointer("/message/id").and_then(Value::as_str);
        match (&mut self.last, id.filter(|id| id.len() <= MESSAGE_ID_LIMIT)) {
            (Some((last, counted)), Some(id)) if last == id => {
                self.output = self.output - *counted + output;
                *counted = output;
            }
            (last, id) => {
                self.output += output;
                *last = id.map(|id| (id.to_owned(), output));
            }
        }
    }

    /// The context window: `known`, else the window of the session's model in `windows`,
    /// else [`CONTEXT_LIMIT`]; at least 1M once the context went past 200k.
    pub fn limit(&self, known: Option<u64>, windows: &Windows) -> u64 {
        let model = self.model.as_deref();
        let window = known.or_else(|| model.and_then(|model| windows.of(model)));
        let window = window.unwrap_or(CONTEXT_LIMIT);
        match self.long {
            true => window.max(LONG_CONTEXT_LIMIT),
            false => window,
        }
    }
}

/// Context windows by model id, learned from the `result.modelUsage` of Hive's chats: the
/// transcripts and hook payloads do not carry them.
#[derive(Debug, Default, Clone)]
pub struct Windows(HashMap<String, u64>);

impl Windows {
    /// Remembers `model`'s window: ids up to [`MESSAGE_ID_LIMIT`] bytes, windows up to
    /// [`TOKEN_LIMIT`], at most [`MAX_MODELS`] models.
    pub fn learn(&mut self, model: &str, window: u64) {
        let room = self.0.contains_key(model) || self.0.len() < MAX_MODELS;
        if room && model.len() <= MESSAGE_ID_LIMIT && (1..=TOKEN_LIMIT).contains(&window) {
            self.0.insert(model.to_owned(), window);
        }
    }

    /// The window of `model`: learned, else 1M for a `[1m]` variant or a model of
    /// [`LONG_MODELS`]; unknown otherwise.
    pub fn of(&self, model: &str) -> Option<u64> {
        let long = model.ends_with("[1m]") || LONG_MODELS.iter().any(|m| model.starts_with(m));
        let learned = self.0.get(model).copied();
        learned.or(long.then_some(LONG_CONTEXT_LIMIT))
    }
}

/// An agent's token usage, read from its transcript as it grows.
#[derive(Debug, Default)]
pub struct Usage {
    /// An event said the transcript grew: it is read on the next tick.
    pub due: bool,
    /// Up to where the transcript was read (always the end of a line).
    offset: u64,
    tokens: Tokens,
    /// The context, its limit and the output last sent to the app.
    sent: Option<(u64, u64, u64)>,
    /// The session's window when its chat told it (the current model's `contextWindow` in a
    /// `result`); it wins over the window of the transcript's model.
    pub window: Option<u64>,
    /// Read once: what was there before the agent was detected is not news.
    primed: bool,
    /// The last read ended the main conversation on an interrupt (see [`Usage::interrupted`]).
    interrupted: bool,
}

impl Usage {
    /// Reads what the transcript at `path` (inside `root`) gained and returns the agent's new
    /// `agent_usage` when it changed. At first only the last [`READ_LIMIT`] bytes are read,
    /// so the output of a longer transcript's start is not counted. A window change alone is
    /// news too.
    pub fn read(
        &mut self,
        id: &str,
        root: &Path,
        path: &Path,
        windows: &Windows,
    ) -> Option<Control> {
        self.due = false;
        let (bytes, _, restarted) = read_lines(root, path, &mut self.offset).ok()?;
        if restarted {
            self.tokens = Tokens::default();
        }
        let mut end = None;
        for line in bytes.split(|&b| b == b'\n') {
            if let Ok(record) = serde_json::from_slice::<Value>(line) {
                self.tokens.add(&record);
                end = crate::sessions::ending_of(&record).or(end);
            }
        }
        self.interrupted = self.primed && end == Some(crate::sessions::Ending::Interrupted);
        self.primed = true;
        let limit = self.tokens.limit(self.window, windows);
        let now = (self.tokens.context, limit, self.tokens.output);
        if self.tokens.context == 0 || self.sent == Some(now) {
            return None;
        }
        self.sent = Some(now);
        self.message(id)
    }

    /// Whether the lines the last read found, after the first read, end the main conversation
    /// on the user's interrupt (`[Request interrupted by user…]`) or on a declined tool call;
    /// answered once.
    pub fn interrupted(&mut self) -> bool {
        std::mem::take(&mut self.interrupted)
    }

    /// The last `agent_usage` sent, for a newly connected app.
    pub fn message(&self, id: &str) -> Option<Control> {
        let (context_tokens, context_limit, output_tokens) = self.sent?;
        Some(Control::AgentUsage {
            id: id.to_owned(),
            context_tokens,
            context_limit,
            output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn entry(role: TranscriptRole, text: &str, tool: Option<&str>) -> TranscriptEntry {
        TranscriptEntry {
            role,
            text: text.into(),
            tool: tool.map(Into::into),
        }
    }

    fn line(record: Value) -> String {
        format!("{record}\n")
    }

    #[test]
    fn transcript_path_must_be_an_absolute_jsonl_path() {
        let path = |p: &str| transcript_path(&json!({ "transcript_path": p }));
        assert_eq!(path("/c/p/s.jsonl"), Some(PathBuf::from("/c/p/s.jsonl")));
        assert_eq!(path("c/p/s.jsonl"), None);
        assert_eq!(path("/c/p/s.json"), None);
        assert_eq!(path(&format!("/{}.jsonl", "a".repeat(PATH_LIMIT))), None);
        let longest = format!("/{}.jsonl", "a".repeat(PATH_LIMIT - 7));
        assert_eq!(path(&longest), Some(PathBuf::from(&longest)));
        assert_eq!(transcript_path(&json!({ "transcript_path": 1 })), None);
        assert_eq!(transcript_path(&json!({})), None);
    }

    #[test]
    fn subagent_path_is_beside_the_agents_and_only_for_agent_ids() {
        let parent = Path::new("/c/p/s.jsonl");
        assert_eq!(
            subagent_path(parent, "aB9_-"),
            Some(PathBuf::from("/c/p/s/subagents/agent-aB9_-.jsonl"))
        );
        let longest = "a".repeat(ID_LIMIT);
        assert!(subagent_path(parent, &longest).is_some());
        for bad in ["", "../x", "a/b", "a.b", "a b", &"a".repeat(ID_LIMIT + 1)] {
            assert_eq!(subagent_path(parent, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn entries_keep_messages_and_tool_calls() {
        let records = [
            line(json!({"type": "user", "message": {"content": "  do it  "}})),
            line(json!({"type": "user", "isMeta": true, "message": {"content": "meta"}})),
            line(
                json!({"type": "user", "isMeta": false, "message": {"content": [
                    {"type": "text", "text": "block"},
                    {"type": "tool_result", "content": "out"},
                ]}}),
            ),
            line(json!({"type": "assistant", "message": {"content": [
                {"type": "thinking", "thinking": "hm"},
                {"type": "text", "text": "done"},
                {"type": "text", "text": "  "},
                {"type": "text"},
                {"type": "tool_use", "name": "Bash", "input": {"command": "ls"}},
                {"type": "tool_use"},
            ]}})),
            line(json!({"type": "assistant", "message": {"content": 3}})),
            line(json!({"type": "assistant"})),
            line(json!({"type": "attachment", "message": {"content": "x"}})),
            line(json!({"message": {"content": "x"}})),
            "not json\n".into(),
        ]
        .concat();
        assert_eq!(
            entries(records.as_bytes()),
            vec![
                entry(TranscriptRole::User, "do it", None),
                entry(TranscriptRole::User, "block", None),
                entry(TranscriptRole::Assistant, "done", None),
                entry(TranscriptRole::Tool, r#"{"command":"ls"}"#, Some("Bash")),
            ]
        );
    }

    #[test]
    fn entries_cut_long_texts_and_tool_names() {
        let long = "é".repeat(TEXT_LIMIT + 1);
        let name = "n".repeat(ID_LIMIT + 1);
        let records = line(json!({"type": "assistant", "message": {"content": [
            {"type": "text", "text": long},
            {"type": "tool_use", "name": name, "input": "x"},
        ]}}));
        let got = entries(records.as_bytes());
        assert_eq!(got[0].text, "é".repeat(TEXT_LIMIT));
        assert_eq!(got[1].tool.as_deref(), Some(&*"n".repeat(ID_LIMIT)));
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        log: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let log = root.join("p/s/subagents/agent-a.jsonl");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        Fixture {
            _dir: dir,
            root,
            log,
        }
    }

    fn watch(f: &Fixture) -> Watch {
        Watch::new("s".into(), "a".into(), f.log.clone(), f.root.clone())
    }

    fn append(path: &Path, text: &str) {
        let mut file = File::options()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn said(text: &str) -> String {
        line(json!({"type": "user", "message": {"content": text}}))
    }

    fn transcript(entries: Vec<TranscriptEntry>, truncated: bool) -> Control {
        Control::Transcript {
            agent: "s".into(),
            subagent: "a".into(),
            entries,
            truncated,
        }
    }

    fn appended(texts: &[&str]) -> Option<Control> {
        Some(Control::TranscriptAppended {
            agent: "s".into(),
            subagent: "a".into(),
            entries: texts
                .iter()
                .map(|t| entry(TranscriptRole::User, t, None))
                .collect(),
        })
    }

    #[test]
    fn a_watch_sends_the_conversation_then_what_is_appended() {
        let f = fixture();
        let mut watch = watch(&f);
        // Not written yet: nothing so far, and it shows up once it is.
        assert_eq!(watch.start(), transcript(vec![], false));
        assert_eq!(watch.poll(), None);
        append(&f.log, &said("one"));
        assert_eq!(watch.poll(), appended(&["one"]));
        // Read up to the end of the line, its line end included.
        assert_eq!(watch.offset, said("one").len() as u64);
        assert_eq!(watch.poll(), None);
        // A line still being written waits for its end.
        let two = said("two");
        append(&f.log, &two[..5]);
        assert_eq!(watch.poll(), None);
        append(&f.log, &format!("{}{}", &two[5..], said("three")));
        assert_eq!(watch.poll(), appended(&["two", "three"]));
        // Lines without entries send nothing.
        append(&f.log, "{}\n");
        assert_eq!(watch.poll(), None);
        // A rewritten, shorter transcript is read again from its start.
        std::fs::write(&f.log, said("new")).unwrap();
        assert_eq!(watch.poll(), appended(&["new"]));
    }

    #[test]
    fn a_long_conversation_starts_with_its_last_entries() {
        let f = fixture();
        let all: Vec<String> = (0..=MAX_ENTRIES).map(|i| i.to_string()).collect();
        append(&f.log, &all.iter().map(|t| said(t)).collect::<String>());
        let user = |t: &String| entry(TranscriptRole::User, t, None);
        let last: Vec<TranscriptEntry> = all[1..].iter().map(user).collect();
        assert_eq!(watch(&f).start(), transcript(last.clone(), true));
        // Exactly the cap is not truncated.
        std::fs::write(&f.log, all[1..].iter().map(|t| said(t)).collect::<String>()).unwrap();
        assert_eq!(watch(&f).start(), transcript(last, false));
    }

    #[test]
    fn a_big_transcript_is_read_from_its_last_bytes() {
        let f = fixture();
        // A line longer than the read limit, then a short one: the long one is skipped.
        let filler = "x".repeat(READ_LIMIT as usize);
        append(&f.log, &format!("{}{}", said(&filler), said("last")));
        let mut watch = watch(&f);
        let first = watch.start();
        assert_eq!(
            first,
            transcript(vec![entry(TranscriptRole::User, "last", None)], true)
        );
        // The tail of a line longer than the limit, with no line end, is skipped at once.
        let before = watch.offset;
        append(&f.log, &"y".repeat(READ_LIMIT as usize));
        assert_eq!(watch.poll(), None);
        assert_eq!(watch.offset, before + READ_LIMIT);
        append(&f.log, &format!("\n{}", said("after")));
        assert_eq!(watch.poll(), appended(&["after"]));
    }

    #[test]
    fn a_tail_is_the_last_whole_lines_inside_the_root() {
        let f = fixture();
        append(&f.log, &format!("{}{}", said("one"), said("two")));
        let both = format!("{}{}", said("one"), said("two")).into_bytes();
        assert_eq!(tail(&f.root, &f.log).unwrap(), (both, false));
        let filler = "x".repeat(READ_LIMIT as usize);
        std::fs::write(&f.log, format!("{}{}", said(&filler), said("last"))).unwrap();
        assert_eq!(
            tail(&f.root, &f.log).unwrap(),
            (said("last").into_bytes(), true)
        );
        let other = f.root.parent().unwrap().join("other");
        std::fs::create_dir(&other).unwrap();
        let outside = tail(&other, &f.log).unwrap_err();
        assert_eq!(outside.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn only_a_file_inside_the_root_is_read() {
        let f = fixture();
        let outside = f.root.parent().unwrap().join("secret.jsonl");
        std::fs::write(&outside, said("secret")).unwrap();
        std::os::unix::fs::symlink(&outside, &f.log).unwrap();
        let mut watch = watch(&f);
        let error = |m: &str| Control::Error {
            message: format!("cannot read the subagent's transcript: {m}"),
        };
        let not_a_file = error("the transcript is not a file");
        assert_eq!(
            watch.start(),
            error("the transcript is outside Claude's projects folder")
        );
        assert_eq!(watch.poll(), None);
        // A folder is not a transcript.
        std::fs::remove_file(&f.log).unwrap();
        std::fs::create_dir(&f.log).unwrap();
        assert_eq!(watch.start(), not_a_file);
        // Nor is a FIFO, which is refused without being opened (that would block).
        std::fs::remove_dir(&f.log).unwrap();
        let made = std::process::Command::new("mkfifo").arg(&f.log).status();
        assert!(made.unwrap().success());
        assert_eq!(watch.start(), not_a_file);
        std::fs::remove_file(&f.log).unwrap();
        // Without the root, nothing is inside it: nothing is read.
        append(&f.log, &said("x"));
        let mut rootless = Watch::new("s".into(), "a".into(), f.log.clone(), "/nope".into());
        assert_eq!(rootless.start(), transcript(vec![], false));
        assert_eq!(
            watch.start(),
            transcript(vec![entry(TranscriptRole::User, "x", None)], false)
        );
    }

    fn turn(id: Option<&str>, usage: Value) -> Value {
        json!({"type": "assistant", "message": {"id": id, "usage": usage}})
    }

    fn counted(records: &[Value]) -> Tokens {
        let mut tokens = Tokens::default();
        for record in records {
            tokens.add(record);
        }
        tokens
    }

    #[test]
    fn tokens_are_the_last_context_and_every_output() {
        let usage = |input: u64, output: u64| {
            json!({"input_tokens": input, "cache_creation_input_tokens": 2,
                   "cache_read_input_tokens": 3, "output_tokens": output})
        };
        let tokens = counted(&[
            turn(Some("m1"), usage(10, 1)),
            // The same message's next content block repeats (and updates) its usage.
            turn(Some("m1"), usage(10, 4)),
            turn(Some("m2"), usage(20, 3)),
            // Not counted: a user record, a subagent's, one without usage, an API error.
            json!({"type": "user", "message": {"usage": usage(1, 1)}}),
            json!({"type": "assistant", "isSidechain": true, "message": {"usage": usage(1, 1)}}),
            json!({"type": "assistant", "message": {}}),
            turn(Some("m3"), json!({"input_tokens": 0, "output_tokens": 9})),
        ]);
        assert_eq!((tokens.context, tokens.output), (25, 7));
        // Without an id, every record counts.
        let tokens = counted(&[turn(None, usage(1, 1)), turn(None, usage(1, 1))]);
        assert_eq!(tokens.output, 2);
        // Ids too long are not remembered.
        let long = "i".repeat(MESSAGE_ID_LIMIT + 1);
        let tokens = counted(&[
            turn(Some(&long), usage(1, 1)),
            turn(Some(&long), usage(1, 1)),
        ]);
        assert_eq!(tokens.output, 2);
        let longest = "i".repeat(MESSAGE_ID_LIMIT);
        let tokens = counted(&[
            turn(Some(&longest), usage(1, 1)),
            turn(Some(&longest), usage(1, 1)),
        ]);
        assert_eq!(tokens.output, 1);
    }

    #[test]
    fn tokens_ignore_fields_out_of_bounds() {
        let tokens = counted(&[turn(
            Some("m"),
            json!({"input_tokens": TOKEN_LIMIT, "cache_read_input_tokens": TOKEN_LIMIT + 1,
                   "cache_creation_input_tokens": -1, "output_tokens": "5"}),
        )]);
        assert_eq!((tokens.context, tokens.output), (TOKEN_LIMIT, 0));
    }

    #[test]
    fn the_context_limit_grows_once_the_context_passed_it() {
        let none = Windows::default();
        let context = |n: u64| turn(None, json!({"input_tokens": n}));
        let at = counted(&[context(CONTEXT_LIMIT)]);
        assert_eq!(at.limit(None, &none), CONTEXT_LIMIT);
        let past = counted(&[context(CONTEXT_LIMIT + 1), context(10)]);
        assert_eq!(
            (past.context, past.limit(None, &none)),
            (10, LONG_CONTEXT_LIMIT)
        );
        // A known window grows to 1M too, and a larger one stays.
        assert_eq!(past.limit(Some(CONTEXT_LIMIT), &none), LONG_CONTEXT_LIMIT);
        assert_eq!(past.limit(Some(2_000_000), &none), 2_000_000);
    }

    fn by(model: &str, input: u64) -> Value {
        json!({"type": "assistant", "message": {"model": model, "usage": {"input_tokens": input}}})
    }

    #[test]
    fn the_context_limit_is_the_window_of_the_sessions_model() {
        let none = Windows::default();
        // Claude Code 2.1.283: an Opus 5.5 session at 52.9k is 5% of 1M, a Haiku one 200k.
        let opus = counted(&[by("claude-opus-5-5", 52_900)]);
        assert_eq!(opus.limit(None, &none), LONG_CONTEXT_LIMIT);
        assert_eq!(opus.context * 100 / opus.limit(None, &none), 5);
        let haiku = counted(&[by("claude-haiku-4-5-20251001", 52_900)]);
        assert_eq!(haiku.limit(None, &none), CONTEXT_LIMIT);
        // The last counted message's model; a synthetic one without context is not counted.
        let switched = counted(&[
            by("claude-haiku-4-5-20251001", 10),
            by("claude-sonnet-5", 10),
            by("<synthetic>", 0),
        ]);
        assert_eq!(switched.limit(None, &none), LONG_CONTEXT_LIMIT);
        // An id too long is not kept.
        let long = format!("claude-opus-5-{}", "x".repeat(MESSAGE_ID_LIMIT));
        let unknown = counted(&[by("claude-opus-5", 10), by(&long, 10)]);
        assert_eq!(unknown.limit(None, &none), LONG_CONTEXT_LIMIT);
        let longest = format!("claude-haiku-{}", "x".repeat(MESSAGE_ID_LIMIT - 13));
        let kept = counted(&[by("claude-opus-5", 10), by(&longest, 10)]);
        assert_eq!(kept.limit(None, &none), CONTEXT_LIMIT);
        // The window a chat told wins; a learned one wins over the table.
        assert_eq!(opus.limit(Some(300_000), &none), 300_000);
        let mut learned = Windows::default();
        learned.learn("claude-opus-5-5", 500_000);
        assert_eq!(opus.limit(None, &learned), 500_000);
    }

    #[test]
    fn windows_are_learned_within_bounds_and_known_by_name() {
        let mut windows = Windows::default();
        // Not learned: 1M for a `[1m]` variant and the long models, unknown otherwise.
        assert_eq!(windows.of("claude-opus-5-5[1m]"), Some(LONG_CONTEXT_LIMIT));
        assert_eq!(
            windows.of("claude-sonnet-4-6[1m]"),
            Some(LONG_CONTEXT_LIMIT)
        );
        for model in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5-1",
            "claude-mythos-5",
        ] {
            assert_eq!(windows.of(model), Some(LONG_CONTEXT_LIMIT), "{model}");
        }
        assert_eq!(windows.of("claude-sonnet-4-6"), None);
        assert_eq!(windows.of("claude-haiku-4-5-20251001"), None);
        windows.learn("claude-sonnet-4-6", 200_000);
        assert_eq!(windows.of("claude-sonnet-4-6"), Some(200_000));
        // Refused: an empty or too large window, an id too long.
        windows.learn("zero", 0);
        windows.learn("huge", TOKEN_LIMIT + 1);
        windows.learn("top", TOKEN_LIMIT);
        windows.learn(&"m".repeat(MESSAGE_ID_LIMIT + 1), 5);
        windows.learn(&"m".repeat(MESSAGE_ID_LIMIT), 5);
        assert_eq!((windows.of("zero"), windows.of("huge")), (None, None));
        assert_eq!(windows.of("top"), Some(TOKEN_LIMIT));
        assert_eq!(windows.of(&"m".repeat(MESSAGE_ID_LIMIT + 1)), None);
        assert_eq!(windows.of(&"m".repeat(MESSAGE_ID_LIMIT)), Some(5));
        // At most `MAX_MODELS` models; a known one still changes.
        for n in 0..MAX_MODELS {
            windows.learn(&format!("model-{n}"), 7);
        }
        assert_eq!(windows.0.len(), MAX_MODELS);
        windows.learn("claude-sonnet-4-6", 1_000_000);
        assert_eq!(windows.of("claude-sonnet-4-6"), Some(1_000_000));
    }

    #[test]
    fn usage_is_read_as_the_transcript_grows() {
        let f = fixture();
        let usage = |context_tokens, context_limit, output_tokens| {
            Some(Control::AgentUsage {
                id: "s".into(),
                context_tokens,
                context_limit,
                output_tokens,
            })
        };
        let spent = |id: &str, input: u64, output: u64| {
            line(turn(
                Some(id),
                json!({"input_tokens": input, "output_tokens": output}),
            ))
        };
        let mut agent = Usage {
            due: true,
            ..Usage::default()
        };
        let none = Windows::default();
        let read = |agent: &mut Usage| agent.read("s", &f.root, &f.log, &none);
        // Not written yet, then no usage yet: nothing to send.
        assert_eq!(read(&mut agent), None);
        assert!(!agent.due);
        append(&f.log, &said("hi"));
        assert_eq!(read(&mut agent), None);
        assert_eq!(agent.message("s"), None);
        append(&f.log, &spent("m1", 100, 5));
        assert_eq!(read(&mut agent), usage(100, CONTEXT_LIMIT, 5));
        assert_eq!(agent.message("s"), usage(100, CONTEXT_LIMIT, 5));
        // Unchanged: nothing is sent again.
        append(&f.log, &spent("m2", 100, 0));
        assert_eq!(read(&mut agent), None);
        // A line still being written waits for its end.
        let next = spent("m3", 300_000, 2);
        append(&f.log, &next[..9]);
        assert_eq!(read(&mut agent), None);
        append(&f.log, &next[9..]);
        assert_eq!(read(&mut agent), usage(300_000, LONG_CONTEXT_LIMIT, 7));
        // A rewritten, shorter transcript is counted again from its start.
        std::fs::write(&f.log, spent("m4", 50, 1)).unwrap();
        assert_eq!(read(&mut agent), usage(50, CONTEXT_LIMIT, 1));
        // The window its chat told is news without a new line.
        agent.window = Some(LONG_CONTEXT_LIMIT);
        assert_eq!(read(&mut agent), usage(50, LONG_CONTEXT_LIMIT, 1));
        assert_eq!(read(&mut agent), None);
        // Outside the root, nothing is read.
        let mut outside = Usage::default();
        assert_eq!(outside.read("s", Path::new("/nope"), &f.log, &none), None);
    }

    #[test]
    fn a_1m_session_is_read_against_1m_from_its_first_message() {
        // The screenshot of 2026-09-26: 52.9k on Opus 5.5 showed "ctx 26%" (of 200k), 5% of 1M.
        let f = fixture();
        append(&f.log, &line(by("claude-opus-5-5", 52_900)));
        let mut usage = Usage::default();
        let message = usage.read("s", &f.root, &f.log, &Windows::default());
        let expected = Control::AgentUsage {
            id: "s".into(),
            context_tokens: 52_900,
            context_limit: LONG_CONTEXT_LIMIT,
            output_tokens: 0,
        };
        assert_eq!(message, Some(expected));
    }

    #[test]
    fn an_interrupt_is_news_only_after_the_first_read_and_while_it_is_last() {
        let f = fixture();
        let esc = said("[Request interrupted by user]");
        // What the transcript held before the first read is old news.
        append(&f.log, &esc);
        let mut usage = Usage::default();
        let read = |usage: &mut Usage| {
            usage.read("s", &f.root, &f.log, &Windows::default());
            usage.interrupted()
        };
        assert!(!read(&mut usage));
        append(&f.log, &said("go on"));
        assert!(!read(&mut usage));
        append(&f.log, &said("[Request interrupted by user for tool use]"));
        assert!(read(&mut usage));
        // Answered once; nothing new is no interrupt.
        assert!(!usage.interrupted());
        assert!(!read(&mut usage));
        // A later message, or a subagent's interrupt, leaves the conversation going on.
        let answer = json!({"type": "assistant", "message": {"stop_reason": "tool_use"}});
        let sub = json!({"type": "user", "isSidechain": true,
                         "message": {"content": "[Request interrupted by user]"}});
        append(&f.log, &format!("{esc}{answer}\n{sub}\n"));
        assert!(!read(&mut usage));
        // The interrupt last, after other records in the same read.
        append(&f.log, &format!("{answer}\n{esc}"));
        assert!(read(&mut usage));
    }
}
