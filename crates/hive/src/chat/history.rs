//! A resumed chat's conversation so far (7.3j), read from its session's transcript
//! ([`crate::transcript::tail`]). Claude Code writes its transcript records with the stream's
//! shapes (`user` / `assistant` messages), so they become entries as the stream's do.

use serde_json::Value;

use super::{ChatEntryKind, Out, Stream};

impl Stream {
    /// The entries of the transcript `records` (JSONL): the main conversation's messages, tool
    /// calls with their results; meta records, subagents' records and anything else are left
    /// out. `truncated`: earlier records were not read, which a note says.
    pub fn history(&mut self, records: &[u8], truncated: bool) -> Out {
        self.changed(|chat, entries, _| {
            if truncated {
                let note = "Earlier messages are not shown.";
                entries.push(chat.entry(ChatEntryKind::Note, note, None));
            }
            for line in records.split(|&b| b == b'\n') {
                let Ok(record) = serde_json::from_slice::<Value>(line) else {
                    continue;
                };
                if record["isMeta"] == true || record["isSidechain"] == true {
                    continue;
                }
                match record["type"].as_str() {
                    Some("assistant") => chat.assistant(&record, None, entries),
                    Some("user") => chat.user(&record, None, entries),
                    _ => {}
                }
            }
            // Calls the transcript left without a result get none now, and an old error does
            // not hide the next turn's.
            chat.tools.clear();
            chat.failed = false;
        })
    }
}

#[cfg(test)]
mod tests {
    use hive_protocol::{ChatEntry, ChatMode, Control, ToolStatus};
    use serde_json::json;

    use super::*;

    fn entries(out: &Out) -> Vec<ChatEntry> {
        let batches = out.app.iter().filter_map(|message| match message {
            Control::ChatEntries {
                chat: 3, entries, ..
            } => Some(entries.clone()),
            _ => None,
        });
        batches.flatten().collect()
    }

    fn entry(id: u32, kind: ChatEntryKind, text: &str) -> ChatEntry {
        ChatEntry {
            id,
            kind,
            text: text.into(),
            tool: None,
            parent: None,
            status: None,
            output: None,
            images: vec![],
        }
    }

    fn lines(records: &[Value]) -> Vec<u8> {
        let lines: Vec<String> = records.iter().map(Value::to_string).collect();
        format!("{}\nnot json\n", lines.join("\n")).into_bytes()
    }

    #[test]
    fn a_transcript_becomes_the_chats_first_entries() {
        let mut stream = Stream::new(3, "/r".into(), ChatMode::Default, None);
        let bash = json!({"type": "tool_use", "id": "t1", "name": "Bash",
                          "input": {"command": "ls"}});
        let records = lines(&[
            json!({"type": "user", "message": {"content": "hi"}}),
            json!({"type": "user", "isMeta": true, "message": {"content": "meta"}}),
            json!({"type": "assistant", "message": {"content": [
                {"type": "thinking", "thinking": "hm"},
                {"type": "text", "text": "hello"},
                bash,
            ]}}),
            json!({"type": "assistant", "isSidechain": true,
                   "message": {"content": [{"type": "text", "text": "sub"}]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "a.txt"},
            ]}}),
            json!({"type": "summary", "summary": "x"}),
        ]);
        let out = stream.history(&records, true);
        let mut tool = entry(5, ChatEntryKind::Tool, "ls");
        tool.tool = Some("Bash".into());
        tool.status = Some(ToolStatus::Running);
        let mut done = tool.clone();
        done.status = Some(ToolStatus::Ok);
        done.output = Some("a.txt".into());
        assert_eq!(
            entries(&out),
            vec![
                entry(1, ChatEntryKind::Note, "Earlier messages are not shown."),
                entry(2, ChatEntryKind::User, "hi"),
                entry(3, ChatEntryKind::Thinking, "hm"),
                entry(4, ChatEntryKind::Assistant, "hello"),
                tool,
                done,
            ]
        );
        assert!(out.write.is_empty() && out.turn.is_none());
        // Live entries go on after the history's.
        let live = stream.send("next", &[]);
        assert_eq!(entries(&live)[0].id, 6);
    }

    #[test]
    fn the_images_sent_come_back_with_their_message() {
        let mut stream = Stream::new(3, "/r".into(), ChatMode::Default, None);
        let png = "iVBORw0KGgo=";
        let gif = "R0lGODlh";
        // Exactly the limit: a PNG signature padded to 3 MiB of base64.
        let mut big = vec![0u8; (crate::chat::MAX_IMAGE_DATA / 4) * 3];
        big[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let big = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, big);
        let over = format!("{big}AAAA");
        // Two halves fill a turn's 3 MiB exactly.
        let half = &big[..big.len() / 2];
        let block = |data: &str| json!({"type": "image", "source": {"data": data}});
        let records = lines(&[
            json!({"type": "user", "message": {"content": [
                {"type": "text", "text": "look"}, block(png), block("bm90IGFuIGltYWdl"),
                block(&over), block(gif),
            ]}}),
            json!({"type": "user", "message": {"content": [block(&big)]}}),
            // Within a turn's caps: 3 MiB of images together, at most 10.
            json!({"type": "user", "message": {"content": [block(&big), block(png)]}}),
            json!({"type": "user", "message": {"content": vec![block(png); 11]}}),
            json!({"type": "user", "message": {"content": [block(half), block(half)]}}),
        ]);
        let out = stream.history(&records, false);
        let shown: Vec<_> = entries(&out)
            .into_iter()
            .map(|e| {
                let images = e.images.into_iter().map(|i| (i.media_type, i.data.len()));
                (e.id, e.text, images.collect::<Vec<_>>())
            })
            .collect();
        let png = ("image/png".to_owned(), png.len());
        let gif = ("image/gif".to_owned(), gif.len());
        let largest = ("image/png".to_owned(), big.len());
        assert_eq!(
            shown,
            vec![
                (1, "look".into(), vec![png.clone(), gif]),
                (2, String::new(), vec![largest.clone()]),
                (3, String::new(), vec![largest]),
                (4, String::new(), vec![png; 10]),
                (
                    5,
                    String::new(),
                    vec![("image/png".to_owned(), half.len()); 2]
                ),
            ]
        );
        // The largest resumed user entry (control characters grow six times in JSON) fits
        // in a frame.
        let wide = "\u{1}".repeat(1 << 20);
        let records = lines(&[json!({"type": "user", "message": {"content": [
            {"type": "text", "text": wide}, block(&big),
        ]}})]);
        let out = Stream::new(3, "/r".into(), ChatMode::Default, None).history(&records, false);
        assert_eq!(entries(&out)[0].images.len(), 1);
        let json = serde_json::to_vec(&out.app[0]).unwrap();
        assert!(json.len() <= hive_protocol::MAX_PAYLOAD, "{}", json.len());
    }

    #[test]
    fn what_the_history_left_open_does_not_reach_the_live_turn() {
        let mut stream = Stream::new(3, "/r".into(), ChatMode::Default, None);
        let records = lines(&[
            json!({"type": "assistant", "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {}},
            ]}}),
            json!({"type": "assistant", "error": "rate_limit",
                   "message": {"content": [{"type": "text", "text": "limit"}]}}),
        ]);
        let out = stream.history(&records, false);
        let kinds: Vec<_> = entries(&out).into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [ChatEntryKind::Tool, ChatEntryKind::Error]);
        // A late result for the old call is not shown.
        let result = json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "x"},
        ]}});
        let late = stream.line(Some(result.to_string().as_bytes()));
        assert_eq!(entries(&late), vec![]);
        // The next failed turn shows its error.
        let failed = json!({"type": "result", "subtype": "error_during_execution",
                            "is_error": true, "errors": ["boom"]});
        let turn = stream.line(Some(failed.to_string().as_bytes()));
        assert_eq!(entries(&turn)[0], entry(3, ChatEntryKind::Error, "boom"));
    }
}
