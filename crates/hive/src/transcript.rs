//! A subagent's conversation for the app's read-only view (6.10), from the transcript Claude
//! Code writes beside its agent's: `<session id>/subagents/agent-<agent_id>.jsonl` next to
//! `<session id>.jsonl`. Transcripts are only read, never written.

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
const READ_LIMIT: u64 = 8 * 1024 * 1024;
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
    let file = File::open(&real)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("the transcript is not a file"));
    }
    Ok(file)
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
        let mut file = match open_inside(&self.root, &self.path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), false)),
            file => file?,
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
        }
        let start = self.offset.max(len.saturating_sub(READ_LIMIT));
        let skipped = start > self.offset;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(len - start).read_to_end(&mut bytes)?;
        // A line still being written waits for the next read; one longer than the limit is
        // skipped (what is left of it does not parse).
        let whole = match bytes.iter().rposition(|&b| b == b'\n') {
            Some(end) => end + 1,
            None if bytes.len() as u64 == READ_LIMIT => bytes.len(),
            None => 0,
        };
        self.offset = start + whole as u64;
        let mut entries = entries(&bytes[..whole]);
        let over = entries.len().saturating_sub(MAX_ENTRIES);
        entries.drain(..over);
        Ok((entries, skipped || over > 0))
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
        let Control::Transcript {
            entries, truncated, ..
        } = watch(&f).start()
        else {
            panic!("expected a transcript")
        };
        assert!(truncated);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].text, "1");
        // Exactly the cap is not truncated.
        std::fs::write(&f.log, all[1..].iter().map(|t| said(t)).collect::<String>()).unwrap();
        let Control::Transcript { truncated, .. } = watch(&f).start() else {
            panic!("expected a transcript")
        };
        assert!(!truncated);
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
        append(&f.log, &"y".repeat(READ_LIMIT as usize));
        assert_eq!(watch.poll(), None);
        append(&f.log, &format!("\n{}", said("after")));
        assert_eq!(watch.poll(), appended(&["after"]));
    }

    #[test]
    fn only_a_file_inside_the_root_is_read() {
        let f = fixture();
        let outside = f.root.parent().unwrap().join("secret.jsonl");
        std::fs::write(&outside, said("secret")).unwrap();
        std::os::unix::fs::symlink(&outside, &f.log).unwrap();
        let mut watch = watch(&f);
        let Control::Error { message } = watch.start() else {
            panic!("expected an error")
        };
        assert_eq!(
            message,
            "cannot read the subagent's transcript: the transcript is outside Claude's projects folder"
        );
        assert_eq!(watch.poll(), None);
        // A folder is not a transcript.
        std::fs::remove_file(&f.log).unwrap();
        std::fs::create_dir(&f.log).unwrap();
        let Control::Error { message } = watch.start() else {
            panic!("expected an error")
        };
        assert!(
            message.ends_with("the transcript is not a file"),
            "{message}"
        );
        // Without the root, nothing is inside it: nothing is read.
        std::fs::remove_dir(&f.log).unwrap();
        append(&f.log, &said("x"));
        let mut rootless = Watch::new("s".into(), "a".into(), f.log.clone(), "/nope".into());
        assert_eq!(rootless.start(), transcript(vec![], false));
        assert_eq!(
            watch.start(),
            transcript(vec![entry(TranscriptRole::User, "x", None)], false)
        );
    }
}
