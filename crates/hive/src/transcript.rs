//! An agent's transcript, the JSONL conversation Claude Code writes (`transcript_path` in its
//! hook payloads): its tokens (6.9) and whether it ends on an interrupt. Transcripts are only
//! read, never written.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use hive_protocol::Control;
use serde_json::Value;

/// Most bytes read at once: the tail at the first read, then the new bytes of each read.
const READ_LIMIT: u64 = 8_388_608; // 8 MiB
/// Longest `transcript_path` kept from a hook payload.
const PATH_LIMIT: usize = 4096;

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

/// The whole lines of the transcript at `path` (inside `root`) written since `offset`, at most
/// its last [`READ_LIMIT`] bytes, and whether it was read again from its start because it
/// shrank. `offset` moves to the end of what was read. A transcript not written yet has no
/// lines.
fn read_lines(root: &Path, path: &Path, offset: &mut u64) -> io::Result<(Vec<u8>, bool)> {
    let mut file = match open_inside(root, path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((Vec::new(), false));
        }
        file => file?,
    };
    let len = file.metadata()?.len();
    let restarted = len < *offset;
    if restarted {
        *offset = 0;
    }
    let start = (*offset).max(len.saturating_sub(READ_LIMIT));
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
    Ok((bytes, restarted))
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
/// window of the Opus 5 and Fable/Mythos 5 families. Opus/Sonnet 4.x are left to the growth
/// rule: Claude Code has run them at 200k unless asked for `[1m]`.
const LONG_MODELS: [&str; 4] = [
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-fable-5",
    "claude-mythos-5",
];
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

    /// The context window: the window of the session's model ([`window_of`]), else
    /// [`CONTEXT_LIMIT`]; at least 1M once the context went past 200k.
    pub fn limit(&self) -> u64 {
        let window = self.model.as_deref().and_then(window_of);
        let window = window.unwrap_or(CONTEXT_LIMIT);
        match self.long {
            true => window.max(LONG_CONTEXT_LIMIT),
            false => window,
        }
    }
}

/// The window of `model` when it is known from its id: 1M for a `[1m]` variant or a model of
/// [`LONG_MODELS`]; the transcripts and hook payloads do not carry it.
fn window_of(model: &str) -> Option<u64> {
    let long = model.ends_with("[1m]") || LONG_MODELS.iter().any(|m| model.starts_with(m));
    long.then_some(LONG_CONTEXT_LIMIT)
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
    /// Read once: what was there before the agent was detected is not news.
    primed: bool,
    /// The last read ended the main conversation on an interrupt (see [`Usage::interrupted`]).
    interrupted: bool,
}

impl Usage {
    /// Reads what the transcript at `path` (inside `root`) gained and returns the agent's new
    /// `agent_usage` when it changed. At first only the last [`READ_LIMIT`] bytes are read,
    /// so the output of a longer transcript's start is not counted.
    pub fn read(&mut self, id: &str, root: &Path, path: &Path) -> Option<Control> {
        self.due = false;
        let (bytes, restarted) = read_lines(root, path, &mut self.offset).ok()?;
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
        let limit = self.tokens.limit();
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

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        log: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let log = root.join("p/s.jsonl");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        Fixture {
            _dir: dir,
            root,
            log,
        }
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

    /// What [`read_lines`] returns, with its bytes as text.
    fn lines(f: &Fixture, offset: &mut u64) -> io::Result<(String, bool)> {
        let (bytes, restarted) = read_lines(&f.root, &f.log, offset)?;
        Ok((String::from_utf8(bytes).unwrap(), restarted))
    }

    #[test]
    fn whole_lines_are_read_as_the_transcript_grows() {
        let f = fixture();
        let mut offset = 0;
        // Not written yet: no lines.
        assert_eq!(lines(&f, &mut offset).unwrap(), (String::new(), false));
        append(&f.log, &said("one"));
        assert_eq!(lines(&f, &mut offset).unwrap(), (said("one"), false));
        // Read up to the end of the line, its line end included.
        assert_eq!(offset, said("one").len() as u64);
        assert_eq!(lines(&f, &mut offset).unwrap(), (String::new(), false));
        // A line still being written waits for its end.
        let two = said("two");
        append(&f.log, &two[..5]);
        assert_eq!(lines(&f, &mut offset).unwrap(), (String::new(), false));
        append(&f.log, &two[5..]);
        assert_eq!(lines(&f, &mut offset).unwrap(), (two, false));
        // A rewritten, shorter transcript is read again from its start.
        std::fs::write(&f.log, said("new")).unwrap();
        assert_eq!(lines(&f, &mut offset).unwrap(), (said("new"), true));
    }

    #[test]
    fn a_big_transcript_is_read_from_its_last_bytes() {
        let f = fixture();
        // A line longer than the read limit, then a short one: the long one is skipped.
        let filler = "x".repeat(READ_LIMIT as usize);
        append(&f.log, &format!("{}{}", said(&filler), said("last")));
        let mut offset = 0;
        let (text, _) = lines(&f, &mut offset).unwrap();
        assert!(text.ends_with(&said("last")));
        assert!(!text.contains(&said(&filler)));
        // The tail of a line longer than the limit, with no line end, is skipped at once.
        let before = offset;
        append(&f.log, &"y".repeat(READ_LIMIT as usize));
        lines(&f, &mut offset).unwrap();
        assert_eq!(offset, before + READ_LIMIT);
        append(&f.log, &format!("\n{}", said("after")));
        let (text, _) = lines(&f, &mut offset).unwrap();
        assert!(text.ends_with(&said("after")));
    }

    #[test]
    fn only_a_file_inside_the_root_is_read() {
        let f = fixture();
        let error = |offset: &mut u64| lines(&f, offset).unwrap_err().to_string();
        let outside = f.root.parent().unwrap().join("secret.jsonl");
        std::fs::write(&outside, said("secret")).unwrap();
        std::os::unix::fs::symlink(&outside, &f.log).unwrap();
        assert_eq!(
            error(&mut 0),
            "the transcript is outside Claude's projects folder"
        );
        // A folder is not a transcript.
        std::fs::remove_file(&f.log).unwrap();
        std::fs::create_dir(&f.log).unwrap();
        assert_eq!(error(&mut 0), "the transcript is not a file");
        // Nor is a FIFO, which is refused without being opened (that would block).
        std::fs::remove_dir(&f.log).unwrap();
        let made = std::process::Command::new("mkfifo").arg(&f.log).status();
        assert!(made.unwrap().success());
        assert_eq!(error(&mut 0), "the transcript is not a file");
        std::fs::remove_file(&f.log).unwrap();
        // Without the root, nothing is inside it: nothing is read.
        append(&f.log, &said("x"));
        let rootless = read_lines(Path::new("/nope"), &f.log, &mut 0).unwrap();
        assert_eq!(rootless, (Vec::new(), false));
        assert_eq!(lines(&f, &mut 0).unwrap(), (said("x"), false));
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
        let context = |n: u64| turn(None, json!({"input_tokens": n}));
        let at = counted(&[context(CONTEXT_LIMIT)]);
        assert_eq!(at.limit(), CONTEXT_LIMIT);
        let past = counted(&[context(CONTEXT_LIMIT + 1), context(10)]);
        assert_eq!((past.context, past.limit()), (10, LONG_CONTEXT_LIMIT));
        // A 1M model stays at 1M.
        let long = counted(&[by("claude-opus-5", CONTEXT_LIMIT + 1)]);
        assert_eq!(long.limit(), LONG_CONTEXT_LIMIT);
    }

    fn by(model: &str, input: u64) -> Value {
        json!({"type": "assistant", "message": {"model": model, "usage": {"input_tokens": input}}})
    }

    #[test]
    fn the_context_limit_is_the_window_of_the_sessions_model() {
        // Claude Code 2.1.283: an Opus 5.5 session at 52.9k is 5% of 1M, a Haiku one 200k.
        let opus = counted(&[by("claude-opus-5-5", 52_900)]);
        assert_eq!(opus.limit(), LONG_CONTEXT_LIMIT);
        assert_eq!(opus.context * 100 / opus.limit(), 5);
        let haiku = counted(&[by("claude-haiku-4-5-20251001", 52_900)]);
        assert_eq!(haiku.limit(), CONTEXT_LIMIT);
        // The last counted message's model; a synthetic one without context is not counted.
        let switched = counted(&[
            by("claude-haiku-4-5-20251001", 10),
            by("claude-sonnet-5", 10),
            by("<synthetic>", 0),
        ]);
        assert_eq!(switched.limit(), LONG_CONTEXT_LIMIT);
        // An id too long is not kept.
        let long = format!("claude-opus-5-{}", "x".repeat(MESSAGE_ID_LIMIT));
        let unknown = counted(&[by("claude-opus-5", 10), by(&long, 10)]);
        assert_eq!(unknown.limit(), LONG_CONTEXT_LIMIT);
        let longest = format!("claude-haiku-{}", "x".repeat(MESSAGE_ID_LIMIT - 13));
        let kept = counted(&[by("claude-opus-5", 10), by(&longest, 10)]);
        assert_eq!(kept.limit(), CONTEXT_LIMIT);
    }

    #[test]
    fn windows_are_known_by_name() {
        // 1M for a `[1m]` variant and the long models, unknown otherwise.
        assert_eq!(window_of("claude-opus-5-5[1m]"), Some(LONG_CONTEXT_LIMIT));
        assert_eq!(window_of("claude-sonnet-4-6[1m]"), Some(LONG_CONTEXT_LIMIT));
        for model in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5-1",
            "claude-mythos-5",
        ] {
            assert_eq!(window_of(model), Some(LONG_CONTEXT_LIMIT), "{model}");
        }
        assert_eq!(window_of("claude-sonnet-4-6"), None);
        assert_eq!(window_of("claude-haiku-4-5-20251001"), None);
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
        let read = |agent: &mut Usage| agent.read("s", &f.root, &f.log);
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
        assert_eq!(read(&mut agent), None);
        // Outside the root, nothing is read.
        let mut outside = Usage::default();
        assert_eq!(outside.read("s", Path::new("/nope"), &f.log), None);
    }

    #[test]
    fn a_1m_session_is_read_against_1m_from_its_first_message() {
        // The screenshot of 2026-09-26: 52.9k on Opus 5.5 showed "ctx 26%" (of 200k), 5% of 1M.
        let f = fixture();
        append(&f.log, &line(by("claude-opus-5-5", 52_900)));
        let mut usage = Usage::default();
        let message = usage.read("s", &f.root, &f.log);
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
            usage.read("s", &f.root, &f.log);
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
