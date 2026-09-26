//! The chat fixtures (`tests/fixtures/chat`, `docs/spike/chat.md` 14.2): one per scenario of
//! `scripts/spike/record-chat.py`, each line a stream-json message as `claude -p` prints it.

use std::path::Path;

const SCENARIOS: [&str; 13] = [
    "text",
    "thinking",
    "tools",
    "permission",
    "question",
    "plan",
    "subagent",
    "compaction",
    "error-model",
    "error-max-turns",
    "image",
    "interrupt",
    "resume",
];

#[test]
fn every_scenario_has_a_fixture_of_typed_json_lines() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    let mut expected: Vec<String> = SCENARIOS.iter().map(|s| format!("{s}.jsonl")).collect();
    expected.sort();
    assert_eq!(names, expected);
    for name in names {
        let text = std::fs::read_to_string(dir.join(&name)).unwrap();
        assert!(text.ends_with('\n'), "{name}");
        for line in text.lines() {
            let message: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(message["type"].is_string(), "{name}: {line}");
        }
    }
}
