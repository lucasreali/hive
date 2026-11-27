//! What `hive hook` forwards of a `PostToolUse` (27.5): only the fields the service reads, and,
//! for a file edit, the edit turned into line hashes (`hive_edit`). The file's text in the
//! payload (`originalFile`, `content`, the patch) never crosses the socket.
//!
//! The payload is untrusted: every number is bounded and every id checked here, and the
//! service checks the file itself again before believing any of it.

use serde_json::{Map, Value, json};

use crate::anchor;

/// Top-level fields of a `PostToolUse` the service reads (`adapter`, `states`, `transcript`).
const KEPT: &[&str] = &[
    "session_id",
    "transcript_path",
    "cwd",
    "permission_mode",
    "hook_event_name",
    "agent_id",
    "agent_type",
    "teammate_name",
    "tool_name",
    "background_tasks",
];

/// Fields of `tool_response` the service reads: the background task a tool started.
const KEPT_RESPONSE: &[&str] = &["backgroundTaskId", "taskId", "agentId"];

/// Longest `file_path` believed.
const PATH_LIMIT: usize = 4096;
/// Most hunks of a patch read; the rest are left out.
const MAX_HUNKS: usize = 4096;
/// Most patch lines read in all; the hunks past them are left out.
const MAX_PATCH_LINES: usize = 200_000;
/// Most lines a run keeps the hashes of; a longer run keeps only its range.
pub const RUN_HASHES: usize = 5000;
/// Most hashed lines sent in all, so the frame stays well under 512 KiB; the runs past it keep
/// only their range.
const SENT_HASHES: usize = 40_000;
/// Most lines of a file hashed (`originalFile`, a `Write`'s content).
const MAX_FILE_LINES: usize = 1_000_000;
/// Longest `agent_type` kept, in characters.
const AGENT_TYPE_LIMIT: usize = 64;
/// Longest background task id kept, in bytes (as `states` keeps them).
const ID_LIMIT: usize = 256;
/// Most runs sent; the service refuses an edit with more.
const MAX_RUNS: usize = 4096;

/// The slimmed `PostToolUse` payload: [`KEPT`] fields, `tool_use_id` when it is a plain id,
/// `tool_response`'s [`KEPT_RESPONSE`] fields, and `hive_edit` for a file edit. Anything that is
/// not a JSON object is kept as it is (the adapter reads nothing from it).
pub fn slim(payload: Value) -> Value {
    let Value::Object(mut raw) = payload else {
        return payload;
    };
    let mut out = Map::new();
    for key in KEPT {
        if let Some(value) = raw.remove(*key) {
            out.insert((*key).to_owned(), value);
        }
    }
    if let Some(Value::String(agent_type)) = out.get_mut("agent_type") {
        *agent_type = agent_type.chars().take(AGENT_TYPE_LIMIT).collect();
    }
    if let Some(id) = raw.remove("tool_use_id").filter(plain_id) {
        out.insert("tool_use_id".to_owned(), id);
    }
    let response = raw.remove("tool_response");
    if let Some(Value::Object(response)) = &response {
        let kept: Map<String, Value> = KEPT_RESPONSE
            .iter()
            .filter_map(|k| {
                let id = response
                    .get(*k)?
                    .as_str()
                    .filter(|id| id.len() <= ID_LIMIT)?;
                Some(((*k).to_owned(), Value::String(id.to_owned())))
            })
            .collect();
        out.insert("tool_response".to_owned(), Value::Object(kept));
    }
    let tool = out.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let input = raw.get("tool_input").and_then(Value::as_object);
    let response = response.as_ref().and_then(Value::as_object);
    let edit = input.zip(response).and_then(|(i, r)| edit(tool, i, r));
    let background = input.and_then(|input| background(tool, input));
    if let Some(edit) = edit {
        out.insert("hive_edit".to_owned(), edit);
    }
    if let Some(background) = background {
        out.insert("tool_input".to_owned(), background);
    }
    fitting(out)
}

/// What a Bash `run_in_background` call's `tool_input` keeps (28.8): its `command` and
/// `description`, cut at [`crate::background::COMMAND_CHARS`] and
/// [`crate::background::DESCRIPTION_CHARS`] characters (the service cleans them). `None` for any
/// other call, whose input never crosses the socket.
fn background(tool: &str, input: &Map<String, Value>) -> Option<Value> {
    if tool != "Bash" || input.get("run_in_background") != Some(&Value::Bool(true)) {
        return None;
    }
    let mut kept = Map::new();
    kept.insert("run_in_background".to_owned(), Value::Bool(true));
    let limits = [
        ("command", crate::background::COMMAND_CHARS),
        ("description", crate::background::DESCRIPTION_CHARS),
    ];
    for (key, most) in limits {
        if let Some(text) = input.get(key).and_then(Value::as_str) {
            kept.insert(
                key.to_owned(),
                Value::String(text.chars().take(most).collect()),
            );
        }
    }
    Some(Value::Object(kept))
}

/// `slimmed` as sent, at most [`crate::hook::MAX_INPUT`] once written: without `hive_edit` when
/// it does not fit (the edit is lost, not the event), else only its ids.
fn fitting(mut slimmed: Map<String, Value>) -> Value {
    let size = |m: &Map<String, Value>| serde_json::to_vec(m).map_or(usize::MAX, |v| v.len());
    if size(&slimmed) > crate::hook::MAX_INPUT {
        slimmed.remove("hive_edit");
    }
    if size(&slimmed) > crate::hook::MAX_INPUT {
        slimmed.retain(|k, v| {
            matches!(k.as_str(), "session_id" | "agent_id" | "tool_name") && plain_id(v)
        });
        slimmed.insert(
            "hive_error".to_owned(),
            "hook input too large once slimmed".into(),
        );
    }
    Value::Object(slimmed)
}

/// `[A-Za-z0-9_-]{1,64}`, as Claude's `toolu_…` ids are.
pub(crate) fn plain_id(id: &Value) -> bool {
    id.as_str().is_some_and(|id| {
        (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

/// A file edit's `hive_edit`, or `None` for another tool or a payload of the wrong shape:
///
/// `{file, kind: "edit"|"create"|"update"|"notebook", before, lines, whole, added, user_modified}`
///
/// `added` holds one `[start, count, hashes]` per run of new lines (1-based start, hashes
/// [packed](anchor::pack), empty past [`RUN_HASHES`] or [`SENT_HASHES`]); `whole` says the run
/// is the whole file because Claude Code gave no patch (a new file, or a `Write` whose diff it
/// skipped); `before` is the [digest](anchor::digest) of the file before the edit, `lines` its
/// line count after, when known.
fn edit(tool: &str, input: &Map<String, Value>, response: &Map<String, Value>) -> Option<Value> {
    let path_key = if tool == "NotebookEdit" {
        "notebook_path"
    } else {
        "file_path"
    };
    let file = input.get(path_key)?.as_str()?;
    let named = response.get("filePath").and_then(Value::as_str);
    if file.len() > PATH_LIMIT || named.is_some_and(|named| named != file) {
        return None;
    }
    let user_modified = response.get("userModified").and_then(Value::as_bool) == Some(true);
    let original =
        text(response, "originalFile").and_then(|t| anchor::hashes(t.as_bytes(), MAX_FILE_LINES));
    let before = original.as_ref().map(|h| anchor::digest(h).to_string());
    let patch = response.get("structuredPatch").and_then(Value::as_array);
    let kind = match tool {
        "Edit" | "MultiEdit" => "edit",
        "Write" if text(response, "type") == Some("create") => "create",
        "Write" => "update",
        "NotebookEdit" => "notebook",
        _ => return None,
    };
    let whole = kind == "create" || (kind == "update" && patch.is_none_or(Vec::is_empty));
    let (runs, lines) = if kind == "notebook" {
        (Vec::new(), None)
    } else if whole {
        let content = text(input, "content").or_else(|| text(response, "content"))?;
        let hashes = anchor::hashes(content.as_bytes(), MAX_FILE_LINES)?;
        let lines = hashes.len();
        let runs = if lines == 0 {
            Vec::new()
        } else {
            vec![Run { start: 1, hashes }]
        };
        (runs, Some(lines))
    } else {
        let (runs, delta) = hunks(patch?);
        let lines = original.as_ref().map(|o| o.len() as i64 + delta);
        (runs, lines.and_then(|l| usize::try_from(l).ok()))
    };
    Some(json!({
        "file": file,
        "kind": kind,
        "before": before,
        "lines": lines,
        "whole": whole,
        "added": added(runs),
        "user_modified": user_modified,
    }))
}

fn text<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

/// A run of new lines: where it starts (1-based) and each line's hash.
struct Run {
    start: usize,
    hashes: Vec<u64>,
}

/// The `[start, count, packed]` entries, keeping hashes until [`SENT_HASHES`] are used.
fn added(runs: Vec<Run>) -> Value {
    let mut budget = SENT_HASHES;
    let entries = runs.into_iter().map(|run| {
        let count = run.hashes.len();
        let kept = count <= RUN_HASHES && count <= budget;
        let packed = if kept {
            budget -= count;
            anchor::pack(&run.hashes)
        } else {
            String::new()
        };
        json!([run.start, count, packed])
    });
    Value::Array(entries.collect())
}

/// The runs of `+` lines of a `structuredPatch`, with their new-side line numbers, and the
/// change in line count. A hunk whose lines disagree with its counts, or of the wrong shape, is
/// left out; so are the hunks past [`MAX_HUNKS`] or [`MAX_PATCH_LINES`].
fn hunks(patch: &[Value]) -> (Vec<Run>, i64) {
    let mut runs: Vec<Run> = Vec::new();
    let mut delta = 0i64;
    let mut read = 0usize;
    for hunk in patch.iter().take(MAX_HUNKS) {
        let Some(hunk) = Hunk::of(hunk) else { continue };
        read += hunk.lines.len();
        if read > MAX_PATCH_LINES {
            break;
        }
        let Some(added) = hunk.added() else { continue };
        delta += hunk.new_lines as i64 - hunk.old_lines as i64;
        for (line, hash) in added {
            let full = runs.len() == MAX_RUNS;
            match runs.last_mut() {
                Some(run) if run.start + run.hashes.len() == line => run.hashes.push(hash),
                _ if full => {}
                _ => runs.push(Run {
                    start: line,
                    hashes: vec![hash],
                }),
            }
        }
    }
    (runs, delta)
}

/// One hunk of a `structuredPatch`.
struct Hunk<'a> {
    old_lines: u64,
    new_start: u64,
    new_lines: u64,
    lines: &'a [Value],
}

impl<'a> Hunk<'a> {
    fn of(hunk: &'a Value) -> Option<Self> {
        let number = |key: &str| hunk.get(key)?.as_u64();
        Some(Hunk {
            old_lines: number("oldLines")?,
            // Past any file's length, so line numbers never overflow.
            new_start: number("newStart").filter(|&s| s <= MAX_FILE_LINES as u64)?,
            new_lines: number("newLines")?,
            lines: hunk.get("lines")?.as_array()?,
        })
    }

    /// The new-side line number and hash of each `+` line, when the lines match the counts.
    fn added(&self) -> Option<Vec<(usize, u64)>> {
        let (mut old, mut new) = (0u64, 0u64);
        let mut added = Vec::new();
        let start = usize::try_from(self.new_start).ok()?;
        for line in self.lines {
            let line = line.as_str()?;
            match line.as_bytes().first() {
                Some(b' ') => {
                    old += 1;
                    new += 1;
                }
                Some(b'-') => old += 1,
                Some(b'+') => {
                    added.push((
                        start + new as usize,
                        anchor::line_hash(&line.as_bytes()[1..]),
                    ));
                    new += 1;
                }
                Some(b'\\') => {}
                _ => return None,
            }
        }
        (old == self.old_lines && new == self.new_lines).then_some(added)
    }
}

/// `session_id`, `agent_id` and `tool_name` of a payload too large to read, from a bounded scan of its
/// first bytes: the first `"key": "value"` of each, when the value is a plain id.
pub fn ids(head: &[u8]) -> Map<String, Value> {
    let text = String::from_utf8_lossy(head);
    let mut found = Map::new();
    for key in ["session_id", "agent_id", "tool_name"] {
        let Some(at) = text.find(&format!("\"{key}\"")) else {
            continue;
        };
        let rest = text[at + key.len() + 2..].trim_start();
        let Some(rest) = rest.strip_prefix(':').map(str::trim_start) else {
            continue;
        };
        let Some((value, _)) = rest.strip_prefix('"').and_then(|r| r.split_once('"')) else {
            continue;
        };
        let value = Value::String(value.to_owned());
        if plain_id(&value) {
            found.insert(key.to_owned(), value);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // SYNTHETIC: the shapes below follow Claude Code's hooks and Agent SDK reference
    // (`FileEditOutput`, `FileWriteOutput`), not a recorded payload; the text is made up.

    fn edit_payload(response: Value) -> Value {
        json!({
            "session_id": "0b5c9e7e-1111-4c2d-9f00-aaaaaaaaaaaa",
            "transcript_path": "/home/u/.claude/projects/p/s.jsonl",
            "cwd": "/repo",
            "permission_mode": "default",
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_use_id": "toolu_01ABCdef",
            "tool_input": {
                "file_path": "/repo/src/a.rs",
                "old_string": "fn old() {}",
                "new_string": "fn new() {}\nfn more() {}",
                "replace_all": false
            },
            "tool_response": response,
        })
    }

    fn one_hunk() -> Value {
        json!({
            "filePath": "/repo/src/a.rs",
            "oldString": "fn old() {}",
            "newString": "fn new() {}\nfn more() {}",
            "originalFile": "use x;\nfn old() {}\nfn keep() {}\n",
            "structuredPatch": [{
                "oldStart": 1, "oldLines": 3, "newStart": 1, "newLines": 4,
                "lines": [" use x;", "-fn old() {}", "+fn new() {}", "+fn more() {}", " fn keep() {}"]
            }],
            "userModified": false,
            "replaceAll": false,
            "gitDiff": {"patch": "…"}
        })
    }

    fn packed(lines: &[&str]) -> String {
        anchor::pack(
            &lines
                .iter()
                .map(|l| anchor::line_hash(l.as_bytes()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn an_edit_keeps_the_fields_read_and_its_added_lines() {
        let slimmed = slim(edit_payload(one_hunk()));
        let before =
            anchor::digest(&anchor::hashes(b"use x;\nfn old() {}\nfn keep() {}\n", 10).unwrap());
        assert_eq!(
            slimmed,
            json!({
                "session_id": "0b5c9e7e-1111-4c2d-9f00-aaaaaaaaaaaa",
                "transcript_path": "/home/u/.claude/projects/p/s.jsonl",
                "cwd": "/repo",
                "permission_mode": "default",
                "hook_event_name": "PostToolUse",
                "tool_name": "Edit",
                "tool_use_id": "toolu_01ABCdef",
                "tool_response": {},
                "hive_edit": {
                    "file": "/repo/src/a.rs",
                    "kind": "edit",
                    "before": before.to_string(),
                    "lines": 4,
                    "whole": false,
                    "added": [[2, 2, packed(&["fn new() {}", "fn more() {}"])]],
                    "user_modified": false
                }
            })
        );
    }

    #[test]
    fn replace_all_gives_one_run_per_place() {
        let mut response = one_hunk();
        response["originalFile"] = json!("a\nx\nb\nx\nc\n");
        response["structuredPatch"] = json!([
            {"oldStart": 1, "oldLines": 3, "newStart": 1, "newLines": 3,
             "lines": [" a", "-x", "+y", " b"]},
            {"oldStart": 4, "oldLines": 2, "newStart": 4, "newLines": 2,
             "lines": ["-x", "+y", " c", "\\ No newline at end of file"]},
        ]);
        response["userModified"] = json!(true);
        let edit = &slim(edit_payload(response))["hive_edit"];
        assert_eq!(
            edit["added"],
            json!([[2, 1, packed(&["y"])], [4, 1, packed(&["y"])]])
        );
        assert_eq!(edit["lines"], 5);
        assert_eq!(edit["user_modified"], true);
    }

    #[test]
    fn runs_join_across_deleted_lines_and_split_at_context() {
        let patch = [
            json!({"oldStart": 1, "oldLines": 3, "newStart": 10, "newLines": 4,
            "lines": ["+a", "-b", "+c", " d", "-e", "+f"]}),
        ];
        let (runs, delta) = hunks(&patch);
        let runs: Vec<(usize, usize)> = runs.iter().map(|r| (r.start, r.hashes.len())).collect();
        assert_eq!(runs, [(10, 2), (13, 1)]);
        assert_eq!(delta, 1);
    }

    #[test]
    fn a_hunk_that_disagrees_with_its_counts_or_shape_is_left_out() {
        let good =
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": ["+a"]});
        for bad in [
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 2, "lines": ["+a"]}),
            json!({"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": ["+a"]}),
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": ["*a"]}),
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": [""]}),
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": [3]}),
            json!({"oldStart": 1, "oldLines": 0, "newStart": -1, "newLines": 1, "lines": ["+a"]}),
            json!({"oldStart": 1, "newStart": 1, "newLines": 1, "lines": ["+a"]}),
            json!({"oldStart": 1, "oldLines": 0, "newLines": 1, "lines": ["+a"]}),
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "lines": ["+a"]}),
            json!({"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": "+a"}),
        ] {
            let (runs, delta) = hunks(&[bad.clone(), good.clone()]);
            assert_eq!(runs.len(), 1, "{bad}");
            assert_eq!((runs[0].start, delta), (1, 1), "{bad}");
        }
    }

    #[test]
    fn hunks_past_the_caps_are_left_out() {
        let hunk = |start: u64| json!({"oldStart": start, "oldLines": 0, "newStart": start, "newLines": 1, "lines": ["+a"]});
        let many: Vec<Value> = (1..=MAX_HUNKS as u64 + 1).map(|i| hunk(i * 2)).collect();
        assert_eq!(hunks(&many).0.len(), MAX_HUNKS);
        let lines = vec![json!(" a"); MAX_PATCH_LINES - 1];
        let big = json!({"oldStart": 1, "oldLines": MAX_PATCH_LINES - 1, "newStart": 1,
            "newLines": MAX_PATCH_LINES - 1, "lines": lines});
        assert_eq!(hunks(&[big.clone(), hunk(1)]).0.len(), 1);
        assert_eq!(hunks(&[big, hunk(1), hunk(5)]).0.len(), 1);
    }

    fn write_payload(kind: &str, patch: Value, original: Value) -> Value {
        json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_input": {"file_path": "/repo/new.txt", "content": "one line here\ntwo\n"},
            "tool_response": {"type": kind, "filePath": "/repo/new.txt",
                "content": "one line here\ntwo\n", "structuredPatch": patch,
                "originalFile": original}
        })
    }

    #[test]
    fn a_new_file_is_all_the_sessions() {
        let edit = &slim(write_payload("create", json!([]), Value::Null))["hive_edit"];
        assert_eq!(
            *edit,
            json!({"file": "/repo/new.txt", "kind": "create", "before": null, "lines": 2,
                "whole": true, "added": [[1, 2, packed(&["one line here", "two"])]],
                "user_modified": false})
        );
    }

    #[test]
    fn a_write_with_no_patch_sends_the_whole_file_and_one_with_a_patch_its_runs() {
        let edit = &slim(write_payload("update", json!([]), json!("old\n")))["hive_edit"];
        assert_eq!(edit["kind"], "update");
        assert_eq!(edit["whole"], true);
        assert_eq!(edit["added"][0][1], 2);
        let no_patch = &slim(write_payload("update", Value::Null, json!("old\n")))["hive_edit"];
        assert_eq!(no_patch["whole"], true);
        let patch = json!([{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 2,
            "lines": ["-old", "+one line here", "+two"]}]);
        let edit = &slim(write_payload("update", patch, json!("old\n")))["hive_edit"];
        assert_eq!(edit["whole"], false);
        assert_eq!(edit["lines"], 2);
        assert_eq!(
            edit["added"],
            json!([[1, 2, packed(&["one line here", "two"])]])
        );
        // The response's content stands in for a missing input's; an empty file has no run.
        let mut payload = write_payload("create", json!([]), Value::Null);
        payload["tool_input"]["content"] = Value::Null;
        payload["tool_response"]["content"] = json!("");
        let edit = &slim(payload)["hive_edit"];
        assert_eq!(
            (edit["added"].clone(), edit["lines"].clone()),
            (json!([]), json!(0))
        );
        let mut payload = write_payload("create", json!([]), Value::Null);
        payload["tool_input"]["content"] = Value::Null;
        payload["tool_response"]["content"] = Value::Null;
        assert_eq!(slim(payload).get("hive_edit"), None);
    }

    #[test]
    fn an_edit_without_the_original_file_has_no_line_count() {
        let mut response = one_hunk();
        response["originalFile"] = Value::Null;
        let edit = &slim(edit_payload(response))["hive_edit"];
        assert_eq!(
            (edit["before"].clone(), edit["lines"].clone()),
            (Value::Null, Value::Null)
        );
        assert_eq!(edit["added"][0][1], 2);
    }

    #[test]
    fn long_runs_keep_only_their_range() {
        let run = |n: usize| Run {
            start: 1,
            hashes: vec![7; n],
        };
        let sent = added(vec![run(RUN_HASHES + 1), run(RUN_HASHES), run(2)]);
        assert_eq!(sent[0], json!([1, RUN_HASHES + 1, ""]));
        assert_eq!(sent[1][2], json!(anchor::pack(&[7; RUN_HASHES])));
        assert_eq!(sent[2][2], json!(anchor::pack(&[7, 7])));
        let many: Vec<Run> = (0..SENT_HASHES / RUN_HASHES + 1)
            .map(|_| run(RUN_HASHES))
            .collect();
        let sent = added(many);
        let empty = sent
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r[2] == "")
            .count();
        assert_eq!(empty, 1);
        let exact: Vec<Run> = (0..SENT_HASHES / RUN_HASHES)
            .map(|_| run(RUN_HASHES))
            .collect();
        assert!(added(exact).as_array().unwrap().iter().all(|r| r[2] != ""));
    }

    #[test]
    fn a_notebook_edit_is_a_file_level_record() {
        let payload = json!({"tool_name": "NotebookEdit",
            "tool_input": {"notebook_path": "/repo/n.ipynb", "new_source": "x"},
            "tool_response": {"new_source": "x"}});
        assert_eq!(
            slim(payload)["hive_edit"],
            json!({"file": "/repo/n.ipynb", "kind": "notebook", "before": null, "lines": null,
                "whole": false, "added": [], "user_modified": false})
        );
    }

    #[test]
    fn other_tools_failures_and_mismatched_paths_have_no_edit() {
        let bash = json!({"tool_name": "Bash", "tool_use_id": "toolu_2",
            "tool_input": {"command": "sed -i s/a/b/ x", "file_path": "/repo/x"},
            "tool_response": {"stdout": "x".repeat(1000), "backgroundTaskId": "b1", "interrupted": false}});
        assert_eq!(
            slim(bash),
            json!({"tool_name": "Bash", "tool_use_id": "toolu_2",
                "tool_response": {"backgroundTaskId": "b1"}})
        );
        let failed = json!({"tool_name": "Edit", "tool_input": {"file_path": "/repo/x"},
            "tool_response": "Error: string not found"});
        assert_eq!(slim(failed), json!({"tool_name": "Edit"}));
        let mut other = edit_payload(one_hunk());
        other["tool_response"]["filePath"] = json!("/repo/other.rs");
        assert_eq!(slim(other).get("hive_edit"), None);
        let mut long = edit_payload(one_hunk());
        let path = format!("/{}", "a".repeat(PATH_LIMIT));
        long["tool_input"]["file_path"] = json!(path);
        long["tool_response"]["filePath"] = json!(path);
        assert_eq!(slim(long).get("hive_edit"), None);
        let mut at_limit = edit_payload(one_hunk());
        let path = format!("/{}", "a".repeat(PATH_LIMIT - 1));
        at_limit["tool_input"]["file_path"] = json!(path);
        at_limit["tool_response"]["filePath"] = Value::Null;
        assert_eq!(slim(at_limit)["hive_edit"]["file"], json!(path));
        let mut no_path = edit_payload(one_hunk());
        no_path["tool_input"]["file_path"] = json!(3);
        assert_eq!(slim(no_path).get("hive_edit"), None);
        let mut no_patch = edit_payload(one_hunk());
        no_patch["tool_response"]["structuredPatch"] = json!("x");
        assert_eq!(slim(no_patch).get("hive_edit"), None);
        let mut no_input = edit_payload(one_hunk());
        no_input["tool_input"] = json!("x");
        assert_eq!(slim(no_input).get("hive_edit"), None);
        let mut unknown = edit_payload(one_hunk());
        unknown["tool_name"] = json!("Read");
        assert_eq!(slim(unknown).get("hive_edit"), None);
        assert_eq!(slim(json!("text")), json!("text"));
    }

    #[test]
    fn a_background_bash_call_keeps_its_command_and_description_cut() {
        // SYNTHETIC: the Bash `tool_input` of Claude Code's tools reference (28.8).
        let payload = |input: Value| {
            json!({"tool_name": "Bash", "tool_input": input,
                "tool_response": {"stdout": "", "backgroundTaskId": "b1"}})
        };
        let input = json!({"command": "npm run dev", "description": "Start the server",
            "run_in_background": true, "timeout": 5000, "dangerouslyDisableSandbox": true});
        assert_eq!(
            slim(payload(input)),
            json!({"tool_name": "Bash", "tool_response": {"backgroundTaskId": "b1"},
                "tool_input": {"command": "npm run dev", "description": "Start the server",
                    "run_in_background": true}})
        );
        let long = json!({"command": "é".repeat(crate::background::COMMAND_CHARS + 1),
            "description": "d".repeat(crate::background::DESCRIPTION_CHARS + 1),
            "run_in_background": true});
        let kept = &slim(payload(long))["tool_input"];
        let chars = |key: &str| kept[key].as_str().unwrap().chars().count();
        assert_eq!(chars("command"), crate::background::COMMAND_CHARS);
        assert_eq!(chars("description"), crate::background::DESCRIPTION_CHARS);
        // Text of the wrong type is left out; the call still counts.
        let odd = json!({"command": 3, "run_in_background": true});
        assert_eq!(
            slim(payload(odd))["tool_input"],
            json!({"run_in_background": true})
        );
        // A foreground call, another tool or `run_in_background` not exactly true keeps none.
        for input in [
            json!({"command": "ls"}),
            json!({"command": "ls", "run_in_background": false}),
            json!({"command": "ls", "run_in_background": "true"}),
        ] {
            assert_eq!(slim(payload(input)).get("tool_input"), None);
        }
        let mut monitor = payload(json!({"command": "tail -f x", "run_in_background": true}));
        monitor["tool_name"] = json!("Monitor");
        assert_eq!(slim(monitor).get("tool_input"), None);
    }

    #[test]
    fn ids_and_agent_types_are_checked() {
        let mut payload = edit_payload(one_hunk());
        payload["tool_use_id"] = json!("toolu_$(rm)");
        payload["agent_type"] = json!("x".repeat(100));
        let slimmed = slim(payload);
        assert_eq!(slimmed.get("tool_use_id"), None);
        assert_eq!(slimmed["agent_type"], json!("x".repeat(AGENT_TYPE_LIMIT)));
        assert!(plain_id(&json!("a-b_C9")));
        assert!(plain_id(&json!("a".repeat(64))));
        assert!(!plain_id(&json!("a".repeat(65))));
        assert!(!plain_id(&json!("")));
        assert!(!plain_id(&json!("a.b")));
        assert!(!plain_id(&json!(4)));
    }

    #[test]
    fn the_ids_of_a_payload_too_large_to_read_come_from_its_head() {
        let head = br#"{"session_id" : "abc-1","transcript_path":"/x","tool_name":"Write","tool_input":{"content":"\"session_id\": \"other\""#;
        assert_eq!(
            Value::Object(ids(head)),
            json!({"session_id": "abc-1", "tool_name": "Write"})
        );
        assert_eq!(
            Value::Object(ids(br#"{"session_id":"a b","tool_name":7}"#)),
            json!({})
        );
        assert_eq!(Value::Object(ids(br#"{"session_id""#)), json!({}));
        assert_eq!(Value::Object(ids(br#"{"session_id":"abc"#)), json!({}));
        assert_eq!(Value::Object(ids(br#"{"tool_name" "x"}"#)), json!({}));
        assert_eq!(
            Value::Object(ids(br#"{"agent_id":"a1b2","x":1}"#)),
            json!({"agent_id": "a1b2"})
        );
    }

    #[test]
    fn only_short_string_task_ids_are_kept() {
        let long = "t".repeat(ID_LIMIT + 1);
        let payload = json!({"tool_name": "Bash", "tool_response": {
            "backgroundTaskId": long, "taskId": 4, "agentId": "a".repeat(ID_LIMIT)}});
        assert_eq!(
            slim(payload)["tool_response"],
            json!({"agentId": "a".repeat(ID_LIMIT)})
        );
    }

    #[test]
    fn runs_past_the_services_cap_and_far_line_numbers_are_left_out() {
        let hunk = |start: usize| json!({"oldStart": start, "oldLines": 0, "newStart": start, "newLines": 1, "lines": ["+a"]});
        // One hunk of MAX_RUNS + 1 runs, a context line between each.
        let lines: Vec<&str> = (0..=MAX_RUNS).flat_map(|_| ["+a", " b"]).collect();
        let n = lines.len() as u64;
        let one = json!({"oldStart": 1, "oldLines": n / 2, "newStart": 1, "newLines": n,
            "lines": lines});
        let (runs, delta) = hunks(&[one]);
        assert_eq!(runs.len(), MAX_RUNS);
        assert_eq!(runs[MAX_RUNS - 1].start, 2 * MAX_RUNS - 1);
        assert_eq!(delta, n as i64 / 2);
        let far = [hunk(MAX_FILE_LINES + 1), hunk(MAX_FILE_LINES)];
        let (runs, _) = hunks(&far);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, MAX_FILE_LINES);
    }

    #[test]
    fn a_slimmed_payload_of_exactly_the_limit_is_sent_whole() {
        let size = |p: &Value| serde_json::to_vec(p).unwrap().len();
        let limit = crate::hook::MAX_INPUT;
        // `cwd` padded so the slimmed payload is exactly the limit, edit included.
        let mut payload = edit_payload(one_hunk());
        payload["cwd"] = json!("");
        let pad = limit - size(&slim(payload.clone()));
        payload["cwd"] = json!("c".repeat(pad));
        let slimmed = slim(payload.clone());
        assert_eq!(size(&slimmed), limit);
        assert!(slimmed.get("hive_edit").is_some());
        // Without its edit, exactly the limit: the rest is sent, not only the ids.
        let mut bare = slimmed;
        let edit_len = size(&bare);
        bare.as_object_mut().unwrap().remove("hive_edit");
        let edit_len = edit_len - size(&bare);
        payload["cwd"] = json!("c".repeat(pad + edit_len));
        let slimmed = slim(payload);
        assert_eq!(slimmed.get("hive_edit"), None);
        assert_eq!(slimmed.get("hive_error"), None);
        assert_eq!(size(&slimmed), limit);
    }

    #[test]
    fn a_slimmed_payload_too_large_to_send_loses_its_edit_then_all_but_its_ids() {
        let big = |n: usize| json!(vec![json!({"id": "x".repeat(100)}); n]);
        let mut payload = edit_payload(one_hunk());
        payload["agent_id"] = json!("a1");
        payload["background_tasks"] = big(10);
        assert!(slim(payload.clone()).get("hive_edit").is_some());
        // Over the limit with the edit, within it without: the edit goes.
        let size = |p: &Value| serde_json::to_vec(p).unwrap().len();
        payload["background_tasks"] = big(0);
        let mut bare = slim(payload.clone());
        let with_edit = size(&bare);
        bare.as_object_mut().unwrap().remove("hive_edit");
        let without = size(&bare);
        // Each task adds 110 bytes (109 and a comma), the first 109.
        let tasks = (crate::hook::MAX_INPUT - without + 1) / 110;
        assert!(without + tasks * 110 - 1 <= crate::hook::MAX_INPUT);
        assert!(with_edit + tasks * 110 - 1 > crate::hook::MAX_INPUT);
        payload["background_tasks"] = big(tasks);
        let slimmed = slim(payload.clone());
        assert_eq!(slimmed.get("hive_edit"), None);
        assert_eq!(slimmed.get("hive_error"), None);
        assert!(slimmed.to_string().len() <= crate::hook::MAX_INPUT);
        // Still over: only the ids stay.
        payload["background_tasks"] = big(crate::hook::MAX_INPUT / 100);
        assert_eq!(
            slim(payload),
            json!({"session_id": "0b5c9e7e-1111-4c2d-9f00-aaaaaaaaaaaa", "agent_id": "a1",
                "tool_name": "Edit", "hive_error": "hook input too large once slimmed"})
        );
    }
}
