use super::*;
use ChatEntryKind::*;

const SESSION: &str = "9f1c2b7e-5d3a-4c1e-8b2f-0a6d4e8c1f00";
const CWD: &str = "/home/u/proj";

fn stream() -> Stream {
    let mut stream = Stream::new(7, CWD.into(), ChatMode::Default, None);
    let initialize = stream.initialize();
    assert_eq!(
        initialize,
        json!({"type": "control_request", "request_id": "hive-1",
               "request": {"subtype": "initialize", "hooks": null}})
    );
    stream
}

/// Feeds a fixture (`tests/fixtures/chat`) line by line.
fn replay(stream: &mut Stream, name: &str) -> Out {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat");
    let text = std::fs::read_to_string(dir.join(format!("{name}.jsonl"))).unwrap();
    let mut all = Out::default();
    for line in text.lines() {
        let out = stream.line(Some(line.as_bytes()));
        all.app.extend(out.app);
        all.write.extend(out.write);
        all.turn = out.turn.or(all.turn);
    }
    all
}

fn entries(out: &Out) -> Vec<ChatEntry> {
    let batches = out.app.iter().filter_map(|message| match message {
        Control::ChatEntries {
            chat: 7,
            entries,
            replace_last: false,
        } => Some(entries.clone()),
        _ => None,
    });
    batches.flatten().collect()
}

fn kinds(out: &Out) -> Vec<(ChatEntryKind, String)> {
    let entries = entries(out).into_iter();
    entries.map(|e| (e.kind, e.text)).collect()
}

fn statuses(out: &Out) -> Vec<Control> {
    let statuses = out
        .app
        .iter()
        .filter(|m| matches!(m, Control::ChatStatus { .. }));
    statuses.cloned().collect()
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
        image: None,
    }
}

fn status(busy: bool, session: bool) -> Control {
    Control::ChatStatus {
        chat: 7,
        busy,
        mode: ChatMode::Default,
        model: session.then(|| "claude-haiku-4-5".into()),
        retry: None,
        compacting: false,
        session: session.then(|| SESSION.into()),
    }
}

fn turn(kind: EventKind) -> Option<AgentEvent> {
    Some(agent_event(7, SESSION.into(), kind))
}

#[test]
fn modes_are_the_clis_permission_modes() {
    let modes = [ChatMode::Default, ChatMode::AcceptEdits, ChatMode::Plan];
    let args: Vec<&str> = modes.iter().map(|&m| mode_arg(m)).collect();
    assert_eq!(args, ["default", "acceptEdits", "plan"]);
    for mode in modes {
        assert_eq!(mode_of(mode_arg(mode)), Some(mode));
    }
    assert_eq!(mode_of("bypassPermissions"), None);
}

#[test]
fn claude_runs_headless_with_hives_hooks() {
    let settings = Path::new("/d/hive-hooks.json");
    let fixed = "-p --input-format stream-json --output-format stream-json --verbose \
                 --replay-user-messages --permission-prompt-tool stdio --permission-mode";
    let expected = |mode: &str, rest: &[&str]| {
        let mut args: Vec<OsString> = fixed.split(' ').map(OsString::from).collect();
        args.push(mode.into());
        args.extend(["--settings", "/d/hive-hooks.json"].map(OsString::from));
        args.extend(rest.iter().map(OsString::from));
        args
    };
    assert_eq!(args(settings, ChatMode::Plan, None), expected("plan", &[]));
    assert_eq!(
        args(settings, ChatMode::AcceptEdits, Some(SESSION)),
        expected("acceptEdits", &["--resume", SESSION])
    );
}

#[test]
fn only_uuids_are_resumed() {
    assert!(is_session(SESSION));
    assert!(is_session("9F1C2B7E-5D3A-4C1E-8B2F-0A6D4E8C1F00"));
    for bad in [
        "",
        "9f1c2b7e-5d3a-4c1e-8b2f-0a6d4e8c1f0",
        "9f1c2b7e-5d3a-4c1e-8b2f-0a6d4e8c1f000",
        "9f1c2b7e05d3a-4c1e-8b2f-0a6d4e8c1f00",
        "9f1c2b7e-5d3a04c1e-8b2f-0a6d4e8c1f00",
        "9f1c2b7e-5d3a-4c1e08b2f-0a6d4e8c1f00",
        "9f1c2b7e-5d3a-4c1e-8b2f00a6d4e8c1f00",
        "9f1c2b7e-5d3a-4c1e-8b2f-0a6d4e8c1f0g",
        "--resume-5d3a-4c1e-8b2f-0a6d4e8c1f00",
    ] {
        assert!(!is_session(bad), "{bad}");
    }
}

#[test]
fn ids_are_short_printable_ascii() {
    let long = "a".repeat(MAX_ID);
    assert_eq!(id_of(&json!(long)), Some(long.as_str()));
    assert_eq!(id_of(&json!("a".repeat(MAX_ID + 1))), None);
    for bad in [json!(""), json!("a b"), json!("é"), json!("a\n"), json!(3)] {
        assert_eq!(id_of(&bad), None, "{bad}");
    }
}

#[test]
fn long_text_is_cut_on_a_character_boundary() {
    assert_eq!(cut("abcd", 4), "abcd");
    assert_eq!(cut("abcde", 4), "abcd… (1 KiB more)");
    // "é" is two bytes: the cut moves back before it.
    assert_eq!(cut("abcé", 4), "abc… (1 KiB more)");
    let long = "x".repeat(3000);
    assert_eq!(cut(&long, 900), format!("{}… (3 KiB more)", &long[..900]));
}

#[test]
fn tool_calls_are_summed_up_in_one_line() {
    let cases = [
        (
            "Bash",
            json!({"command": "cargo test", "description": "d"}),
            "cargo test",
        ),
        ("Read", json!({"file_path": "/r/a"}), "/r/a"),
        ("Edit", json!({"file_path": "/r/b"}), "/r/b"),
        ("MultiEdit", json!({"file_path": "/r/c"}), "/r/c"),
        ("Write", json!({"file_path": "/r/d"}), "/r/d"),
        ("NotebookEdit", json!({"file_path": "/r/e"}), "/r/e"),
        ("Grep", json!({"pattern": "fn main"}), "fn main"),
        ("Glob", json!({"pattern": "*.rs"}), "*.rs"),
        ("WebFetch", json!({"url": "https://x"}), "https://x"),
        ("WebSearch", json!({"query": "rust"}), "rust"),
        ("Agent", json!({"description": "Count"}), "Count"),
        ("Task", json!({"description": "Old"}), "Old"),
        ("Mcp", json!({"a": 1}), r#"{"a":1}"#),
        ("Bash", json!({"cmd": "x"}), r#"{"cmd":"x"}"#),
    ];
    for (tool, input, expected) in cases {
        assert_eq!(summary(tool, &input), expected, "{tool}");
    }
    let long = summary("Bash", &json!({"command": "y".repeat(600)}));
    assert_eq!(long.chars().count(), MAX_SUMMARY);
    assert!(long.ends_with('…'));
}

#[test]
fn tool_output_is_its_text() {
    assert_eq!(output(&json!("plain")), "plain");
    let blocks = json!([
        {"type": "text", "text": "a"},
        {"type": "image", "source": {}},
        {"type": "other"},
        {"type": "text", "text": "b"},
    ]);
    assert_eq!(output(&blocks), "a\n(image not shown)\nb");
    assert_eq!(output(&json!(null)), "");
}

#[test]
fn the_usage_footer_has_time_tokens_and_context() {
    let result = json!({"duration_ms": 2345, "usage": {"output_tokens": 40},
        "modelUsage": {"a": {"contextWindow": 1000}, "b": {"contextWindow": 2000}, "c": {}}});
    assert_eq!(
        usage(&result, 500),
        "2.3 s · 40 output tokens · 25% context"
    );
    let no_window = json!({"duration_ms": 50, "modelUsage": {"a": {"contextWindow": 0}}});
    assert_eq!(usage(&no_window, 5), "0.1 s · 0 output tokens");
    assert_eq!(usage(&json!({}), 5), "0.0 s · 0 output tokens");
    let huge = json!({"modelUsage": {"a": {"contextWindow": 1}}});
    assert_eq!(
        usage(&huge, u64::MAX),
        format!("0.0 s · 0 output tokens · {}% context", u64::MAX)
    );
    let tokens = json!({"input_tokens": 1, "cache_read_input_tokens": 20,
        "cache_creation_input_tokens": 300, "output_tokens": 4000});
    assert_eq!(context(&tokens), 321);
    assert_eq!(
        context(&json!({"input_tokens": u64::MAX, "cache_read_input_tokens": 1})),
        u64::MAX
    );
}

#[test]
fn reset_times_are_utc_times_of_day() {
    assert_eq!(time_of_day(0), "00:00 UTC");
    assert_eq!(time_of_day(86_400 + 13 * 3600 + 5 * 60 + 59), "13:05 UTC");
}

#[test]
fn entries_are_sent_in_bounded_batches() {
    let list = |n: u32| (1..=n).map(|id| entry(id, Note, "x")).collect::<Vec<_>>();
    let sizes = |batches: Vec<Control>| -> Vec<usize> {
        let sizes = batches.into_iter().map(|b| match b {
            Control::ChatEntries {
                chat: 7, entries, ..
            } => entries.len(),
            other => panic!("{other:?}"),
        });
        sizes.collect()
    };
    let len = serde_json::to_vec(&entry(1, Note, "x")).unwrap().len();
    assert_eq!(sizes(batches(7, list(5), 2, 1000)), [2, 2, 1]);
    assert_eq!(sizes(batches(7, list(5), 10, 2 * len)), [2, 2, 1]);
    assert_eq!(sizes(batches(7, list(5), 10, 2 * len - 1)), [1, 1, 1, 1, 1]);
    assert_eq!(sizes(batches(7, list(5), 10, 3 * len)), [3, 2]);
    assert_eq!(sizes(batches(7, list(2), 10, 1)), [1, 1]);
    assert!(batches(7, Vec::new(), 1, 1).is_empty());
}

#[test]
fn a_text_turn_opens_the_chat_then_shows_the_reply_and_its_usage() {
    let mut stream = stream();
    let text = "Reply with exactly two short sentences about git worktrees. Do not use any tools.";
    let sent = stream.send(text, &[]);
    let user = json!({"type": "user", "message": {"role": "user", "content": text},
        "parent_tool_use_id": null, "session_id": "default"});
    assert_eq!(
        sent,
        Out {
            app: vec![
                Control::ChatEntries {
                    chat: 7,
                    entries: vec![entry(1, User, text)],
                    replace_last: false,
                },
                status(true, false),
            ],
            write: vec![user],
            turn: None,
        }
    );
    let out = replay(&mut stream, "text");
    let opened = Control::ChatOpened {
        chat: 7,
        cwd: CWD.into(),
        session: None,
        model: None,
        mode: ChatMode::Default,
        commands: vec!["compact".into()],
        api_key_source: None,
    };
    let reply = "Git worktrees let one repository have several checkouts at once. \
                 Each one has its own branch and working files.";
    let entries = |entries| Control::ChatEntries {
        chat: 7,
        entries,
        replace_last: false,
    };
    assert_eq!(
        out.app,
        vec![
            opened,
            status(true, true),
            entries(vec![entry(2, Assistant, reply)]),
            entries(vec![entry(
                3,
                Usage,
                "2.3 s · 40 output tokens · 0% context"
            )]),
            status(false, true),
        ]
    );
    assert!(out.write.is_empty());
    assert_eq!(out.turn, turn(EventKind::TurnFinished));
}

#[test]
fn a_resumed_chat_keeps_its_session_and_writes_as_default() {
    let mut stream = Stream::new(7, CWD.into(), ChatMode::Plan, Some(SESSION.into()));
    stream.initialize();
    let out = replay(&mut stream, "resume");
    let Some(Control::ChatOpened { session, mode, .. }) = out.app.first() else {
        panic!("{out:?}");
    };
    assert_eq!((session.as_deref(), *mode), (Some(SESSION), ChatMode::Plan));
    // `system/init` says the mode is `default` now.
    assert!(statuses(&out).iter().any(|s| s == &status(false, true)));
}

#[test]
fn thinking_is_an_entry_and_redacted_thinking_is_not() {
    let mut stream = stream();
    let out = replay(&mut stream, "thinking");
    let kinds = kinds(&out);
    assert_eq!(
        kinds[0],
        (Thinking, "17 * 23 is 391; 391 - 19 is 372.".into())
    );
    assert_eq!(kinds[1].0, Assistant);
    assert_eq!(kinds[2].0, Usage);
    assert_eq!(kinds.len(), 3);
}

#[test]
fn a_tool_result_updates_its_calls_entry() {
    let mut stream = stream();
    let out = replay(&mut stream, "tools");
    let tools: Vec<ChatEntry> = entries(&out)
        .into_iter()
        .filter(|e| e.kind == Tool)
        .collect();
    let call = |id: u32, tool: &str, text: &str, status, output: Option<&str>| ChatEntry {
        tool: Some(tool.into()),
        status: Some(status),
        output: output.map(str::to_owned),
        ..entry(id, Tool, text)
    };
    let notes = "     1\talpha\n     2\tbeta\n";
    let listing = "total 16\ndrwxr-xr-x 3 u u 4096 .\n-rw-r--r-- 1 u u 11 notes.txt\n";
    assert_eq!(
        tools,
        vec![
            call(
                1,
                "Read",
                "/home/u/proj/notes.txt",
                ToolStatus::Running,
                None
            ),
            call(
                1,
                "Read",
                "/home/u/proj/notes.txt",
                ToolStatus::Ok,
                Some(notes)
            ),
            call(2, "Bash", "ls -la", ToolStatus::Running, None),
            call(2, "Bash", "ls -la", ToolStatus::Ok, Some(listing)),
        ]
    );
    assert_eq!(kinds(&out).len(), 6);
}

/// The requests a replay asked for.
fn requests(out: &Out) -> Vec<ChatRequest> {
    let asked = out.app.iter().filter_map(|message| match message {
        Control::ChatRequest { chat: 7, request } => Some(request.clone()),
        _ => None,
    });
    asked.collect()
}

fn gone(id: &str) -> Control {
    Control::ChatRequestGone {
        chat: 7,
        request: id.into(),
    }
}

/// A request answered: the note of what was decided, then its card goes.
fn noted(id: u32, note: &str, request: &str) -> Vec<Control> {
    let entries = vec![entry(id, Note, note)];
    vec![
        Control::ChatEntries {
            chat: 7,
            entries,
            replace_last: false,
        },
        gone(request),
    ]
}

fn refused(message: &str) -> Out {
    Out {
        app: vec![Control::Error {
            message: message.into(),
        }],
        ..Out::default()
    }
}

fn allowed(id: &str, input: Value) -> Value {
    json!({"type": "control_response", "response": {"subtype": "success", "request_id": id,
        "response": {"behavior": "allow", "updatedInput": input}}})
}

fn denied(id: &str, message: &str) -> Value {
    json!({"type": "control_response", "response": {"subtype": "success", "request_id": id,
        "response": {"behavior": "deny", "message": message, "interrupt": false}}})
}

fn waiting(tool: &str) -> Option<AgentEvent> {
    turn(EventKind::PermissionRequested {
        tool: Some(tool.into()),
    })
}

fn working() -> Option<AgentEvent> {
    turn(EventKind::ToolFinished { tool: None })
}

fn ask(stream: &mut Stream, id: &str, tool: &str, input: Value) -> Out {
    let line = json!({"type": "control_request", "request_id": id,
        "request": {"subtype": "can_use_tool", "tool_name": tool, "input": input}});
    stream.line(Some(line.to_string().as_bytes()))
}

fn deny(message: Option<&str>) -> ChatAnswer {
    ChatAnswer::Deny {
        message: message.map(str::to_owned),
    }
}

#[test]
fn permission_requests_wait_for_the_humans_answer_given_once() {
    let mut stream = stream();
    let out = replay(&mut stream, "permission");
    assert!(out.write.is_empty());
    let request = |id: &str, tool: &str, detail: &str, reason: Option<&str>| ChatRequest {
        id: id.into(),
        kind: ChatRequestKind::Permission,
        tool: tool.into(),
        detail: detail.into(),
        reason: reason.map(str::to_owned),
        questions: vec![],
        plan: None,
    };
    assert_eq!(
        requests(&out),
        [
            request(
                "req_1",
                "Write",
                "/home/u/proj/hello.txt",
                Some("Write requires approval in default mode")
            ),
            request("req_2", "Bash", "touch made-by-bash.txt", None),
        ]
    );

    // Allowed with claude's own input; the other one still waits.
    let out = stream.answer("req_1", &ChatAnswer::Allow);
    let input = json!({"file_path": "/home/u/proj/hello.txt", "content": "hi"});
    assert_eq!(
        out,
        Out {
            app: noted(5, "Allowed Write: /home/u/proj/hello.txt", "req_1"),
            write: vec![allowed("req_1", input)],
            turn: waiting("Bash"),
        }
    );
    // Once only; never an id of no pending request.
    assert_eq!(
        stream.answer("req_1", &ChatAnswer::Allow),
        refused("no such pending request")
    );
    assert_eq!(
        stream.answer("hive-1", &deny(None)),
        refused("no such pending request")
    );
    // Only answers that fit a permission.
    for answer in [
        ChatAnswer::Answers {
            answers: vec![vec!["a".into()]],
        },
        ChatAnswer::ApprovePlan {
            accept_edits: false,
        },
        ChatAnswer::KeepPlanning {
            feedback: "x".into(),
        },
    ] {
        let out = stream.answer("req_2", &answer);
        assert_eq!(out, refused("This answer does not fit the request."));
    }
    let long = "m".repeat(MAX_ANSWER + 1);
    assert_eq!(
        stream.answer("req_2", &deny(Some(&long))),
        refused("The message is longer than 4 KiB.")
    );
    let out = stream.answer("req_2", &deny(None));
    assert_eq!(
        out,
        Out {
            app: noted(6, "Denied Bash: touch made-by-bash.txt", "req_2"),
            write: vec![denied("req_2", "The user denied this.")],
            turn: working(),
        }
    );
}

#[test]
fn a_deny_message_is_passed_on_up_to_4_kib() {
    let mut stream = stream();
    ask(&mut stream, "r1", "Bash", json!({"command": "ls"}));
    ask(&mut stream, "r2", "Bash", json!({"command": "ls"}));
    let most = "m".repeat(MAX_ANSWER);
    let out = stream.answer("r1", &deny(Some(&most)));
    assert_eq!(out.write, [denied("r1", &most)]);
    let out = stream.answer("r2", &deny(Some("")));
    assert_eq!(out.write, [denied("r2", "The user denied this.")]);
}

#[test]
fn questions_are_answered_with_their_labels_or_a_free_text() {
    let mut stream = stream();
    let out = replay(&mut stream, "question");
    let question = "Which language should I greet you in?";
    let option = |label: &str, description: &str| ChatOption {
        label: label.into(),
        description: description.into(),
    };
    let asked = requests(&out);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].kind, ChatRequestKind::Question);
    assert_eq!(asked[0].tool, "AskUserQuestion");
    assert_eq!(
        asked[0].questions,
        [ChatQuestion {
            question: question.into(),
            header: "Language".into(),
            multi: false,
            options: vec![
                option("English", "Greet in English"),
                option("Portuguese", "Greet in Portuguese"),
            ],
        }]
    );
    let input: Value = json!({"questions": [{"question": question, "header": "Language",
        "multiSelect": false, "options": [
            {"label": "English", "description": "Greet in English"},
            {"label": "Portuguese", "description": "Greet in Portuguese"}]}]});
    assert_eq!(asked[0].detail, input.to_string());
    let answers = |answers: &[&[&str]]| ChatAnswer::Answers {
        answers: answers
            .iter()
            .map(|a| a.iter().map(|&s| s.to_owned()).collect())
            .collect(),
    };
    let long = "t".repeat(MAX_ANSWER + 1);
    for (answer, why) in [
        (answers(&[]), "Answer every question."),
        (
            answers(&[&["English"], &["English"]]),
            "Answer every question.",
        ),
        (answers(&[&["English", "Portuguese"]]), "Choose one option."),
        (answers(&[&[]]), "Choose an option or write an answer."),
        (answers(&[&[""]]), "Choose an option or write an answer."),
        (
            answers(&[&["English", "Klingon"]]),
            "Choose an option or write an answer.",
        ),
        (answers(&[&[&long]]), "The answer is longer than 4 KiB."),
        (
            answers(&[&["English", "English"]]),
            "Choose each option once.",
        ),
        (
            answers(&[&["English", "Portuguese", "x"]]),
            "Choose each option once.",
        ),
        (ChatAnswer::Allow, "This answer does not fit the request."),
    ] {
        assert_eq!(stream.answer("req_3", &answer), refused(why), "{answer:?}");
    }
    let chosen = |value: Value| {
        let mut input = input.clone();
        input["answers"] = json!({question: value});
        allowed("req_3", input)
    };
    let out = stream.answer("req_3", &answers(&[&["Portuguese"]]));
    assert_eq!(
        out,
        Out {
            app: noted(4, "Answered: Portuguese", "req_3"),
            write: vec![chosen(json!("Portuguese"))],
            turn: working(),
        }
    );
    // A free text, up to 4 KiB.
    let most = "t".repeat(MAX_ANSWER);
    let out = replay(&mut stream, "question");
    assert_eq!(requests(&out).len(), 1);
    let out = stream.answer("req_3", &answers(&[&[&most]]));
    assert_eq!(out.write, [chosen(json!(most))]);
}

#[test]
fn multiple_choices_are_answered_with_every_label_as_claude_wrote_it() {
    let mut stream = stream();
    let label = "L".repeat(MAX_ANSWER + 1);
    let input = json!({"questions": [
        {"question": "Which?", "header": "Pick", "multiSelect": true,
         "options": [{"label": "a", "description": ""}, {"label": label}]},
        {"question": "One?", "options": [{"label": "x"}, {"label": "y"}]}]});
    let out = ask(&mut stream, "q", "AskUserQuestion", input.clone());
    let asked = &requests(&out)[0];
    let shown = cut(&label, MAX_ANSWER);
    assert_eq!(asked.questions[0].options[1].label, shown);
    assert!(asked.questions[0].multi);
    assert!(!asked.questions[1].multi);
    assert_eq!(asked.questions[1].header, "");
    let answer = ChatAnswer::Answers {
        answers: vec![vec!["a".into(), shown], vec!["y".into()]],
    };
    let out = stream.answer("q", &answer);
    let mut expected = input;
    expected["answers"] = json!({"Which?": ["a", label], "One?": "y"});
    assert_eq!(out.write, [allowed("q", expected)]);
    // One label of a multiple choice is a list too.
    let input = json!({"questions": [{"question": "Q", "multiSelect": true,
        "options": [{"label": "a"}]}]});
    ask(&mut stream, "q2", "AskUserQuestion", input.clone());
    let answer = ChatAnswer::Answers {
        answers: vec![vec!["a".into()]],
    };
    let mut expected = input;
    expected["answers"] = json!({"Q": ["a"]});
    assert_eq!(
        stream.answer("q2", &answer).write,
        [allowed("q2", expected)]
    );
}

#[test]
fn questions_are_bounded_and_a_question_without_any_is_a_permission() {
    let mut stream = stream();
    let options: Vec<Value> = (0..=MAX_QUESTIONS)
        .map(|i| json!({"label": i.to_string()}))
        .collect();
    let long = "?".repeat(MAX_ANSWER + 1);
    let question = json!({"question": long, "options": options});
    let input = json!({"questions": vec![question; MAX_QUESTIONS + 1]});
    let asked = &requests(&ask(&mut stream, "q", "AskUserQuestion", input))[0];
    assert_eq!(asked.questions.len(), MAX_QUESTIONS);
    assert_eq!(asked.questions[0].options.len(), MAX_QUESTIONS);
    assert_eq!(asked.questions[0].question, cut(&long, MAX_ANSWER));
    let out = ask(
        &mut stream,
        "p",
        "AskUserQuestion",
        json!({"questions": []}),
    );
    let asked = &requests(&out)[0];
    assert_eq!(asked.kind, ChatRequestKind::Permission);
    assert_eq!(asked.detail, r#"{"questions":[]}"#);
    // Only `AskUserQuestion` asks questions.
    let input = json!({"questions": [{"question": "Q", "options": [{"label": "a"}]}]});
    let asked = &requests(&ask(&mut stream, "o", "Other", input))[0];
    assert_eq!(
        (asked.kind, asked.questions.len()),
        (ChatRequestKind::Permission, 0)
    );
}

#[test]
fn plans_are_approved_or_sent_back_with_feedback() {
    let mut stream = stream();
    let out = replay(&mut stream, "plan");
    let plan = "## Add CONTRIBUTING.md\n1. Create CONTRIBUTING.md.\n\
                2. Add a \"Commit messages\" section with two lines.";
    let asked = requests(&out);
    assert_eq!(asked[0].kind, ChatRequestKind::Plan);
    assert_eq!(asked[0].plan.as_deref(), Some(plan));
    assert_eq!(asked[1].kind, ChatRequestKind::Permission);
    // Its approval left plan mode (`system/status`).
    let modes: Vec<ChatMode> = (out.app.iter())
        .filter_map(|s| match s {
            Control::ChatStatus { mode, .. } => Some(*mode),
            _ => None,
        })
        .collect();
    assert_eq!(modes, [ChatMode::Plan, ChatMode::Default]);
    let input = json!({"plan": plan, "planFilePath": "/home/u/.claude/plans/contributing.md"});
    for answer in [ChatAnswer::Allow, ChatAnswer::Answers { answers: vec![] }] {
        let out = stream.answer("req_4", &answer);
        assert_eq!(out, refused("This answer does not fit the request."));
    }
    let long = "f".repeat(MAX_ANSWER + 1);
    let keep = |feedback: &str| ChatAnswer::KeepPlanning {
        feedback: feedback.into(),
    };
    assert_eq!(
        stream.answer("req_4", &keep(&long)),
        refused("The message is longer than 4 KiB.")
    );
    let out = stream.answer("req_4", &keep("Shorter, please."));
    assert_eq!(out.write, [denied("req_4", "Shorter, please.")]);
    assert_eq!(
        out.app,
        noted(5, "Kept planning: Shorter, please.", "req_4")
    );
    assert_eq!(out.turn, waiting("Write"));

    // Approving keeps the mode, or also accepts edits.
    ask(&mut stream, "p1", "ExitPlanMode", input.clone());
    let approve = |accept_edits| ChatAnswer::ApprovePlan { accept_edits };
    let out = stream.answer("p1", &approve(false));
    assert_eq!(out.app, noted(6, "Plan approved", "p1"));
    assert_eq!(out.write, [allowed("p1", input.clone())]);
    ask(&mut stream, "p2", "ExitPlanMode", input.clone());
    let out = stream.answer("p2", &approve(true));
    let accept = json!({"type": "control_request", "request_id": "hive-2",
        "request": {"subtype": "set_permission_mode", "mode": "acceptEdits"}});
    assert_eq!(out.write, [allowed("p2", input.clone()), accept]);
    assert!(matches!(
        out.app[0],
        Control::ChatStatus {
            mode: ChatMode::AcceptEdits,
            ..
        }
    ));
    let accepted = noted(7, "Plan approved, accepting edits", "p2");
    assert_eq!(out.app[1..], accepted);
    ask(&mut stream, "p3", "ExitPlanMode", input);
    let out = stream.answer("p3", &keep(""));
    assert_eq!(
        out.write,
        [denied("p3", "The user wants to keep planning.")]
    );
}

#[test]
fn a_dismissed_question_or_plan_is_noted_as_such() {
    let mut stream = stream();
    let input = json!({"questions": [{"question": "Q", "options": [{"label": "a"}]}]});
    ask(&mut stream, "q", "AskUserQuestion", input);
    ask(&mut stream, "p", "ExitPlanMode", json!({"plan": "x"}));
    let out = stream.answer("q", &deny(None));
    assert_eq!(out.write, [denied("q", "The user denied this.")]);
    assert_eq!(out.app, noted(1, "Question dismissed", "q"));
    let out = stream.answer("p", &deny(None));
    assert_eq!(out.write, [denied("p", "The user denied this.")]);
    assert_eq!(out.app, noted(2, "Plan rejected", "p"));
}

#[test]
fn a_plan_detail_and_reason_are_bounded() {
    let mut stream = stream();
    let big = "p".repeat(MAX_PLAN + 1);
    let line = json!({"type": "control_request", "request_id": "r",
        "request": {"subtype": "can_use_tool", "tool_name": "ExitPlanMode",
                    "input": {"plan": big}, "decision_reason": big}});
    let out = stream.line(Some(line.to_string().as_bytes()));
    let asked = &requests(&out)[0];
    assert_eq!(asked.plan.as_deref(), Some(cut(&big, MAX_PLAN).as_str()));
    assert_eq!(asked.reason.as_deref(), Some(cut(&big, MAX_TEXT).as_str()));
    let detail = json!({"plan": big}).to_string();
    assert_eq!(asked.detail, cut(&detail, MAX_TEXT));
}

#[test]
fn a_cancelled_request_goes_and_a_closed_chat_denies_every_one() {
    let mut stream = stream();
    replay(&mut stream, "permission");
    let cancel = |id: &str| {
        json!({"type": "control_cancel_request", "request_id": id})
            .to_string()
            .into_bytes()
    };
    assert_eq!(stream.line(Some(&cancel("nope"))), Out::default());
    let out = stream.line(Some(&cancel("req_1")));
    assert_eq!(
        out,
        Out {
            app: vec![
                gone("req_1"),
                Control::ChatEntries {
                    chat: 7,
                    entries: vec![entry(5, Note, "Request cancelled: Write")],
                    replace_last: false,
                },
            ],
            turn: waiting("Bash"),
            ..Out::default()
        }
    );
    assert_eq!(
        stream.answer("req_1", &ChatAnswer::Allow),
        refused("no such pending request")
    );
    ask(&mut stream, "r3", "Edit", json!({}));
    let closed = "The chat was closed.";
    assert_eq!(
        stream.deny_all(),
        Out {
            write: vec![denied("req_2", closed), denied("r3", closed)],
            ..Out::default()
        }
    );
    assert_eq!(stream.deny_all(), Out::default());
    // A request after the close is denied, never shown.
    let out = ask(&mut stream, "late", "Bash", json!({}));
    assert_eq!(
        out,
        Out {
            write: vec![denied("late", closed)],
            ..Out::default()
        }
    );
}

#[test]
fn pending_requests_are_bounded_and_asked_once() {
    let mut stream = stream();
    for i in 0..MAX_PENDING {
        let out = ask(&mut stream, &format!("r{i}"), "Bash", json!({}));
        assert_eq!(requests(&out).len(), 1);
    }
    let out = ask(&mut stream, "r0", "Bash", json!({"command": "again"}));
    assert_eq!(out, Out::default());
    let out = ask(&mut stream, "over", "Bash", json!({}));
    assert_eq!(
        out,
        Out {
            write: vec![denied("over", "Too many requests are waiting.")],
            ..Out::default()
        }
    );
    assert_eq!(stream.deny_all().write.len(), MAX_PENDING);
}

#[test]
fn requests_before_the_session_is_known_change_no_state() {
    let mut stream = stream();
    let out = ask(&mut stream, "r", "Bash", json!({"command": "ls"}));
    assert_eq!(requests(&out).len(), 1);
    assert_eq!(out.turn, None);
}

#[test]
fn other_requests_to_the_service_are_refused() {
    let mut stream = stream();
    let line = |value: Value| value.to_string().into_bytes();
    let hook = line(json!({"type": "control_request", "request_id": "r1",
        "request": {"subtype": "hook_callback"}}));
    let out = stream.line(Some(&hook));
    let refused = json!({"type": "control_response", "response": {"subtype": "error",
        "request_id": "r1", "error": "not supported"}});
    assert_eq!(out.write, vec![refused]);
    assert!(out.app.is_empty());
    // Without a usable id there is nothing to answer.
    let bad = line(json!({"type": "control_request", "request_id": "a b",
        "request": {"subtype": "can_use_tool"}}));
    assert_eq!(stream.line(Some(&bad)), Out::default());
}

#[test]
fn a_subagents_entries_carry_their_agent_call() {
    let mut stream = stream();
    let out = replay(&mut stream, "subagent");
    let all = entries(&out);
    let under: Vec<(ChatEntryKind, String)> = all
        .iter()
        .filter(|e| e.parent.as_deref() == Some("toolu_08"))
        .map(|e| (e.kind, e.text.clone()))
        .collect();
    assert!(
        under.contains(&(User, "Count the lines of notes.txt.".into())),
        "{under:?}"
    );
    assert!(under.iter().any(|(kind, _)| *kind == Tool), "{under:?}");
    let agent = all
        .iter()
        .rfind(|e| e.tool.as_deref() == Some("Agent"))
        .unwrap();
    assert_eq!(
        (agent.status, agent.output.as_deref()),
        (Some(ToolStatus::Ok), Some("notes.txt has 2 lines."))
    );
    assert_eq!(agent.parent, None);
}

#[test]
fn compaction_shows_in_the_status_and_as_a_divider() {
    let mut stream = stream();
    let out = replay(&mut stream, "compaction");
    let compacting = |c| match c {
        Control::ChatStatus { compacting, .. } => compacting,
        _ => unreachable!(),
    };
    let flags: Vec<bool> = statuses(&out).into_iter().map(compacting).collect();
    assert_eq!(flags, [false, true, false]);
    let dividers: Vec<String> = kinds(&out)
        .into_iter()
        .filter(|(kind, _)| *kind == Divider)
        .map(|(_, text)| text)
        .collect();
    assert_eq!(dividers, ["Conversation compacted (150k tokens)"]);
    let bare = json!({"type": "system", "subtype": "compact_boundary"}).to_string();
    let out = stream.line(Some(bare.as_bytes()));
    assert_eq!(kinds(&out), [(Divider, "Conversation compacted".into())]);
}

#[test]
fn a_cleared_conversation_gets_its_new_session() {
    let mut stream = stream();
    replay(&mut stream, "text");
    let new = "11111111-2222-4333-8444-555555555555";
    for reset in [
        json!({"type": "system", "subtype": "conversation_reset", "new_conversation_id": new}),
        json!({"type": "conversation_reset"}),
    ] {
        let out = stream.line(Some(reset.to_string().as_bytes()));
        assert_eq!(kinds(&out), [(Divider, "Conversation cleared".into())]);
    }
    let Control::ChatStatus { session, .. } = stream.status() else {
        unreachable!()
    };
    assert_eq!(session.as_deref(), Some(new));
}

#[test]
fn a_model_error_shows_once_and_fails_the_turn() {
    let mut stream = stream();
    let out = replay(&mut stream, "error-model");
    let error = "There's an issue with the selected model (no-such-model-hive-spike). \
                 It may not exist or you may not have access to it.";
    let kinds = kinds(&out);
    assert_eq!(kinds[0], (Error, error.into()));
    assert_eq!(kinds[1].0, Usage);
    assert_eq!(kinds.len(), 2);
    assert_eq!(out.turn, turn(EventKind::TurnFailed { error: None }));
    // The next turn starts without the error.
    let result = json!({"type": "result", "subtype": "success", "is_error": true});
    let out = stream.line(Some(result.to_string().as_bytes()));
    assert_eq!(
        kinds_of_first(&out),
        (Error, "The turn failed (success).".into())
    );
}

fn kinds_of_first(out: &Out) -> (ChatEntryKind, String) {
    kinds(out).into_iter().next().unwrap()
}

#[test]
fn retries_informational_notes_and_failed_turns_are_shown() {
    let mut stream = stream();
    let out = replay(&mut stream, "error-max-turns");
    let retries: Vec<Option<String>> = statuses(&out)
        .into_iter()
        .map(|s| match s {
            Control::ChatStatus { retry, .. } => retry,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(retries, [None, Some("Retrying 1/10…".into()), None]);
    let kinds = kinds(&out);
    assert!(
        kinds.contains(&(Note, "Max turns is set to 1".into())),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(Error, "Reached maximum number of turns (1)".into())),
        "{kinds:?}"
    );
    assert_eq!(out.turn, turn(EventKind::TurnFailed { error: None }));
    // Other failed results: their text, else their subtype; a bare retry counts 0/0.
    let mut line = |value: Value| stream.line(Some(value.to_string().as_bytes()));
    let out = line(
        json!({"type": "result", "subtype": "error_during_execution",
        "errors": [], "result": "Boom"}),
    );
    assert_eq!(kinds_of_first(&out), (Error, "Boom".into()));
    let out = line(json!({"type": "result", "subtype": "error_max_budget_usd", "errors": [7]}));
    assert_eq!(
        kinds_of_first(&out),
        (Error, "The turn failed (error_max_budget_usd).".into())
    );
    let out = line(json!({"type": "system", "subtype": "api_retry"}));
    let Some(Control::ChatStatus { retry, .. }) = out.app.last() else {
        panic!("{out:?}")
    };
    assert_eq!(retry.as_deref(), Some("Retrying 0/0…"));
    // An assistant message clears it.
    let out = line(json!({"type": "assistant", "message": {"content": []}}));
    assert!(matches!(
        out.app.last(),
        Some(Control::ChatStatus { retry: None, .. })
    ));
}

#[test]
fn the_end_of_a_turn_clears_its_transient_status() {
    let mut stream = stream();
    let mut line = |value: Value| stream.line(Some(value.to_string().as_bytes()));
    line(json!({"type": "system", "subtype": "status", "status": "compacting"}));
    let out =
        line(json!({"type": "system", "subtype": "api_retry", "attempt": 2, "max_retries": 9}));
    let retrying = Control::ChatStatus {
        chat: 7,
        busy: false,
        mode: ChatMode::Default,
        model: None,
        retry: Some("Retrying 2/9…".into()),
        compacting: true,
        session: None,
    };
    assert_eq!(out.app, [retrying]);
    let out = line(json!({"type": "result", "subtype": "success"}));
    let idle = Control::ChatStatus {
        chat: 7,
        busy: false,
        mode: ChatMode::Default,
        model: None,
        retry: None,
        compacting: false,
        session: None,
    };
    assert_eq!(out.app.last(), Some(&idle));
}

#[test]
fn an_interrupted_turn_is_noted_not_failed() {
    let mut stream = stream();
    let out = replay(&mut stream, "interrupt");
    let kinds = kinds(&out);
    let at = kinds
        .iter()
        .position(|k| *k == (Note, "Interrupted".into()));
    assert_eq!(kinds[at.unwrap() + 1].0, Usage);
    assert!(!kinds.iter().any(|(kind, _)| *kind == Error), "{kinds:?}");
    // The cut call got its error result.
    let bash = entries(&out).into_iter().rfind(|e| e.kind == Tool).unwrap();
    assert_eq!(bash.status, Some(ToolStatus::Error));
    assert_eq!(out.turn, turn(EventKind::TurnFinished));
}

#[test]
fn an_image_turns_reply_is_shown() {
    let mut stream = stream();
    let out = replay(&mut stream, "image");
    let expected = (
        Assistant,
        "A tiny square, red on the left and blue on the right.".into(),
    );
    assert_eq!(kinds(&out)[0], expected);
}

#[test]
fn rejected_rate_limits_are_errors() {
    let mut stream = stream();
    let mut line = |value: Value| kinds(&stream.line(Some(value.to_string().as_bytes())));
    let rejected = json!({"type": "rate_limit_event",
        "rate_limit_info": {"status": "rejected", "resetsAt": 90_000}});
    assert_eq!(
        line(rejected),
        [(Error, "Usage limit reached; it resets at 01:00 UTC.".into())]
    );
    let bare = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected"}});
    assert_eq!(line(bare), [(Error, "Usage limit reached.".into())]);
    let allowed = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed"}});
    assert!(line(allowed).is_empty());
}

#[test]
fn garbage_unknown_and_oversized_lines_are_harmless() {
    let mut stream = stream();
    for line in [
        &b"not json"[..],
        b"",
        b"[1, 2]",
        br#"{"type": "future_thing", "subtype": "x"}"#,
        br#"{"type": "system", "subtype": "hook_started"}"#,
        br#"{"type": "user", "isReplay": true, "message": {"content": "mine"}}"#,
        br#"{"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "nope"}]}}"#,
        br#"{"type": "user", "message": {"content": [{"type": "other"}]}}"#,
        br#"{"type": "control_response", "response": {"request_id": "hive-9"}}"#,
        br#"{"type": "assistant", "message": {"content": [{"type": "redacted_thinking"}]}}"#,
        br#"{"type": "stream_event", "event": {}}"#,
    ] {
        assert_eq!(stream.line(Some(line)), Out::default(), "{}", String::from_utf8_lossy(line));
    }
    let out = stream.line(None);
    assert_eq!(
        kinds(&out),
        [(Note, "A message too large to show was left out.".into())]
    );
    // Long text is cut; a subagent's plain prompt is a user entry.
    let long = json!({"type": "user", "parent_tool_use_id": "toolu_1",
        "message": {"content": "z".repeat(MAX_TEXT + 1)}});
    let out = stream.line(Some(long.to_string().as_bytes()));
    let [user] = &entries(&out)[..] else {
        panic!("{out:?}")
    };
    assert_eq!((user.kind, user.parent.as_deref()), (User, Some("toolu_1")));
    assert!(
        user.text.ends_with("… (1 KiB more)"),
        "{}",
        &user.text[MAX_TEXT..]
    );
}

#[test]
fn the_initialize_answer_opens_once_even_when_it_failed() {
    let mut stream = stream();
    let failed = json!({"type": "control_response", "response": {"subtype": "error",
        "request_id": "hive-1", "error": "bad flag"}});
    let out = stream.line(Some(failed.to_string().as_bytes()));
    assert_eq!(
        kinds(&out),
        [(Error, "claude did not start: bad flag".into())]
    );
    assert!(
        matches!(out.app.first(), Some(Control::ChatOpened { commands, .. }) if commands.is_empty())
    );
    assert_eq!(
        stream.line(Some(failed.to_string().as_bytes())),
        Out::default()
    );
    // Before `initialize` is sent, nothing is ours.
    let mut fresh = Stream::new(7, CWD.into(), ChatMode::Default, None);
    let answer = json!({"type": "control_response", "response": {}});
    assert_eq!(
        fresh.line(Some(answer.to_string().as_bytes())),
        Out::default()
    );
}

#[test]
fn slash_commands_are_bounded() {
    let mut stream = stream();
    let mut commands: Vec<Value> = (0..MAX_COMMANDS + 1)
        .map(|i| json!({"name": format!("c{i}")}))
        .collect();
    commands.insert(0, json!({"name": "has space"}));
    let answer = json!({"type": "control_response", "response": {"subtype": "success",
        "request_id": "hive-1", "response": {"commands": commands}}});
    let out = stream.line(Some(answer.to_string().as_bytes()));
    let [Control::ChatOpened { commands, .. }] = &out.app[..] else {
        panic!("{out:?}")
    };
    assert_eq!(commands.len(), MAX_COMMANDS);
    assert_eq!(
        (&commands[0][..], &commands[MAX_COMMANDS - 1][..]),
        ("c0", "c499")
    );
}

#[test]
fn tool_calls_waiting_for_results_are_bounded_and_forgotten_after_the_turn() {
    let mut stream = stream();
    let mut line = |value: Value| stream.line(Some(value.to_string().as_bytes()));
    let call = |i: usize| {
        json!({"type": "tool_use", "id": format!("t{i}"), "name": "Bash",
        "input": {"command": "x"}})
    };
    let calls: Vec<Value> = (0..=MAX_TOOLS).map(call).collect();
    line(json!({"type": "assistant", "message": {"content": calls}}));
    let result = |i: usize| {
        json!({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": format!("t{i}"), "content": "ok"}]}})
    };
    assert_eq!(entries(&line(result(MAX_TOOLS - 1))).len(), 1);
    assert!(entries(&line(result(MAX_TOOLS))).is_empty());
    line(json!({"type": "result", "subtype": "success"}));
    assert!(entries(&line(result(0))).is_empty());
}

#[test]
fn the_context_comes_from_the_main_threads_calls() {
    let mut stream = stream();
    let mut line = |value: Value| stream.line(Some(value.to_string().as_bytes()));
    let call = |parent: Value, tokens: u64| {
        json!({"type": "assistant", "parent_tool_use_id": parent,
        "message": {"usage": {"input_tokens": tokens}, "content": []}})
    };
    line(call(json!(null), 500));
    line(call(json!("toolu_1"), 900));
    let result = json!({"type": "result", "subtype": "success",
        "modelUsage": {"m": {"contextWindow": 1000}}});
    let out = line(result);
    assert_eq!(
        kinds(&out),
        [(Usage, "0.0 s · 0 output tokens · 50% context".into())]
    );
    // No session known: no event for the agent states.
    assert_eq!(out.turn, None);
}

#[test]
fn turns_modes_and_interrupts_are_written_to_claude() {
    let mut stream = stream();
    let out = stream.send(
        "x",
        &[ChatImage {
            media_type: "image/png".into(),
            data: "AA".into(),
        }],
    );
    assert_eq!(kinds(&out), [(Error, "Images cannot be sent yet.".into())]);
    assert!(out.write.is_empty());
    let big = "y".repeat(MAX_TURN + 1);
    let out = stream.send(&big, &[]);
    assert_eq!(
        kinds(&out),
        [(Error, "The message is longer than 1 MiB.".into())]
    );
    assert!(stream.send(&big[1..], &[]).write.len() == 1);
    let out = stream.set_mode(ChatMode::AcceptEdits);
    assert_eq!(
        out.write,
        [json!({"type": "control_request", "request_id": "hive-2",
            "request": {"subtype": "set_permission_mode", "mode": "acceptEdits"}})]
    );
    assert!(matches!(
        out.app[..],
        [Control::ChatStatus {
            mode: ChatMode::AcceptEdits,
            ..
        }]
    ));
    let out = stream.interrupt();
    assert_eq!(
        out,
        Out {
            write: vec![json!({"type": "control_request", "request_id": "hive-3",
                "request": {"subtype": "interrupt"}})],
            ..Out::default()
        }
    );
    // The same mode again changes no status.
    assert!(stream.set_mode(ChatMode::AcceptEdits).app.is_empty());
}

/// Every line of `input`; bounded by its length, so a mutant cannot loop forever.
async fn lines(input: &[u8], max: usize) -> Vec<Option<String>> {
    let mut reader = input;
    let (mut buf, mut read) = (Vec::new(), Vec::new());
    for _ in 0..input.len() + 2 {
        match next_line(&mut reader, &mut buf, max).await.unwrap() {
            Some(whole) => read.push(whole.then(|| String::from_utf8_lossy(&buf).into_owned())),
            None => return read,
        }
    }
    panic!("more lines than bytes: {read:?}")
}

#[tokio::test]
async fn lines_are_read_whole_or_skipped_whole() {
    let some = |text: &str| Some(text.to_owned());
    assert_eq!(lines(b"", 4).await, []);
    assert_eq!(lines(b"ab\ncd", 4).await, [some("ab"), some("cd")]);
    assert_eq!(lines(b"abcd\nabcd", 4).await, [some("abcd"), some("abcd")]);
    assert_eq!(lines(b"abcde\nx\n", 4).await, [None, some("x")]);
    assert_eq!(lines(b"abcdefghijklm\nx\n", 4).await, [None, some("x")]);
    assert_eq!(lines(b"abcdefghij", 4).await, [None]);
    assert_eq!(lines(b"\n\n", 4).await, [some(""), some("")]);
}

#[tokio::test]
async fn only_the_end_of_stderr_is_kept() {
    let mut input = vec![b'a'; 10_000];
    input.extend_from_slice(b"the end");
    let tail = tail(&input[..]).await;
    assert_eq!(tail.len(), STDERR_TAIL);
    assert!(tail.ends_with(b"aaathe end"));
    assert_eq!(super::tail(&b"short"[..]).await, b"short");
}

fn exit(code: i32) -> ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw(code << 8)
}

#[test]
fn a_chat_that_failed_says_why() {
    assert_eq!(ended(Some(exit(0)), false, b"noise"), None);
    assert_eq!(ended(Some(exit(1)), true, b"noise"), None);
    assert_eq!(ended(None, true, b""), None);
    assert_eq!(ended(Some(exit(1)), false, b" oops\n"), Some("oops".into()));
    assert_eq!(
        ended(Some(exit(2)), false, b"\n"),
        Some("claude ended (exit status: 2)".into())
    );
    assert_eq!(ended(None, false, b""), Some("claude ended".into()));
}

/// Starts `sh -c script` as a chat's `claude`.
fn start(script: &str, dir: &Path, env: &[(&'static str, String)]) -> (Chat, Pipes) {
    let stream = Stream::new(7, dir.display().to_string(), ChatMode::Default, None);
    let args = ["-c", script].map(OsString::from);
    let cwd = dir.to_str().unwrap();
    Chat::start(stream, Path::new("/bin/sh"), &args, cwd, env).unwrap()
}

/// Reads claude's stdout to its end and reaps it; its group is killed after 10 s, so a
/// test (or a mutant) never leaves it running or waits forever.
async fn read_all(pipes: Pipes) -> (String, Option<ExitStatus>) {
    let Pipes {
        mut child,
        mut stdout,
        ..
    } = pipes;
    let group = Pid::from_raw(child.id().unwrap() as i32);
    let mut out = String::new();
    let limit = Duration::from_secs(10);
    let read = tokio::time::timeout(limit, stdout.read_to_string(&mut out)).await;
    if read.is_err() {
        let _ = killpg(group, Signal::SIGKILL);
    }
    let status = child.wait().await.ok();
    assert!(read.is_ok(), "claude did not end: {out:?}");
    (out, status)
}

#[tokio::test]
async fn claude_runs_in_its_worktree_and_group_with_the_chats_environment() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let script = r#"printf '%s|%s|%s|%s|%s\n' "$HIVE_TERMINAL_ID" "$HIVE_WRAPPED" "${CLAUDECODE-unset}" "$SPACE" "$(pwd)"
[ "$(ps -o pgid= -p $$ | tr -d ' ')" = "$$" ] && echo leader
while read -r line; do printf 'got %s\n' "$line"; done"#;
    let env = [("CLAUDECODE", "1".to_owned()), ("SPACE", "work".to_owned())];
    let (mut chat, pipes) = start(script, &dir, &env);
    assert!(chat.group > 1);
    let out = chat.stream.send("hi", &[]);
    let out = chat.run(out);
    assert!(out.write.is_empty());
    assert_eq!(out.app.len(), 2);
    // Closing denies what waits for the human first.
    let asked = json!({"type": "control_request", "request_id": "r",
        "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {}}});
    chat.stream.line(Some(asked.to_string().as_bytes()));
    assert!(chat.close());
    assert!(!chat.close());
    assert!(chat.closing);
    // Lines written after closing go nowhere.
    let late = chat.stream.interrupt();
    chat.run(late);
    let (out, status) = read_all(pipes).await;
    let initialize = r#"{"request":{"hooks":null,"subtype":"initialize"},"request_id":"hive-1","type":"control_request"}"#;
    let user = r#"{"message":{"content":"hi","role":"user"},"parent_tool_use_id":null,"session_id":"default","type":"user"}"#;
    let denied = r#"{"response":{"request_id":"r","response":{"behavior":"deny","interrupt":false,"message":"The chat was closed."},"subtype":"success"},"type":"control_response"}"#;
    assert_eq!(
        out,
        format!(
            "7|1|unset|work|{}\nleader\ngot {initialize}\ngot {user}\ngot {denied}\n",
            dir.display()
        )
    );
    assert!(status.unwrap().success());
}

#[tokio::test]
async fn a_missing_program_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let stream = Stream::new(7, "/".into(), ChatMode::Default, None);
    let missing = dir.path().join("claude");
    let started = Chat::start(stream, &missing, &[], "/", &[]);
    assert_eq!(started.err().unwrap().kind(), io::ErrorKind::NotFound);
}

/// Stops a chat whose `sh` handles signals as `traps` says, and returns how it ended.
async fn stopped(traps: &str) -> (String, Option<ExitStatus>, Duration) {
    let dir = tempfile::tempdir().unwrap();
    // Reads stdin until it closes, then waits: only signals end it.
    let script = format!("{traps}\necho ready\ncat >/dev/null\nwhile :; do sleep 0.05; done");
    let (mut chat, mut pipes) = start(&script, dir.path(), &[]);
    let mut ready = [0; 6];
    pipes.stdout.read_exact(&mut ready).await.unwrap();
    chat.close();
    let started = std::time::Instant::now();
    let reaping = tokio::spawn(read_all(pipes));
    stop(chat.group, Duration::from_millis(300)).await;
    let (out, status) = reaping.await.unwrap();
    (out, status, started.elapsed())
}

#[tokio::test]
async fn closing_escalates_from_sigint_to_sigkill() {
    use std::os::unix::process::ExitStatusExt;
    let (out, status, _) = stopped("trap 'echo int; exit 3' INT").await;
    assert_eq!((out.as_str(), status.unwrap().code()), ("int\n", Some(3)));
    let (out, status, _) = stopped("trap 'echo int' INT\ntrap 'echo term; exit 4' TERM").await;
    assert_eq!(out, "int\nterm\n");
    assert_eq!(status.unwrap().code(), Some(4));
    let (out, status, took) = stopped("trap 'echo int' INT\ntrap 'echo term' TERM").await;
    assert_eq!(out, "int\nterm\n");
    assert_eq!(status.unwrap().signal(), Some(9));
    assert!(took >= Duration::from_millis(600), "{took:?}");
}

#[test]
fn a_message_with_several_blocks_is_one_batch() {
    let mut stream = stream();
    let two = json!({"type": "assistant", "message": {"content": [
        {"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}});
    let out = stream.line(Some(two.to_string().as_bytes()));
    let batch = vec![entry(1, Assistant, "a"), entry(2, Assistant, "b")];
    assert_eq!(
        out.app,
        [Control::ChatEntries {
            chat: 7,
            entries: batch,
            replace_last: false,
        }]
    );
}

#[tokio::test]
async fn a_group_already_gone_is_not_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    let (mut chat, pipes) = start("exit 0", dir.path(), &[]);
    let group = chat.group;
    chat.close();
    read_all(pipes).await;
    let started = std::time::Instant::now();
    end(vec![group]).await;
    stop(group, Duration::from_secs(5)).await;
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn the_service_ending_kills_every_chat() {
    let dir = tempfile::tempdir().unwrap();
    let script = "trap '' INT TERM\necho ready\nwhile :; do sleep 0.05; done";
    let (mut chat, mut pipes) = start(script, dir.path(), &[]);
    let mut ready = [0; 6];
    pipes.stdout.read_exact(&mut ready).await.unwrap();
    chat.close();
    let reaping = tokio::spawn(read_all(pipes));
    let started = std::time::Instant::now();
    end(vec![chat.group]).await;
    let took = started.elapsed();
    let (_, status) = reaping.await.unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.unwrap().signal(), Some(9));
    assert!(took >= 3 * SHUTDOWN_GRACE, "{took:?}");
    assert!(
        took < 3 * SHUTDOWN_GRACE + Duration::from_secs(2),
        "{took:?}"
    );
}
