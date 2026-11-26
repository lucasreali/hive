use super::*;
use crate::{openable, Host, Link};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn more_than_the_rate_in_a_second_is_a_flood() {
    let mut rate = Rate::default();
    let start = Instant::now();
    for _ in 0..RATE {
        assert_eq!(rate.count(start, 0), Ok(()));
    }
    let flood = Err("sent more than 1000 messages in a second".to_owned());
    assert_eq!(rate.count(start + Duration::from_millis(999), 0), flood);
    // A new second counts from one again: the last one's frames are gone.
    let mut rate = Rate::default();
    for _ in 0..RATE {
        rate.count(start, 0).unwrap();
    }
    assert_eq!(rate.count(start + RATE_WINDOW, 0), Ok(()));
    for _ in 1..RATE {
        rate.count(start + RATE_WINDOW, 0).unwrap();
    }
    assert!(rate.count(start + RATE_WINDOW, 0).is_err());
    // Bytes too: 16 MiB a second.
    let mut rate = Rate::default();
    let bytes = Err("sent more than 16 MiB of messages in a second".to_owned());
    assert_eq!(rate.count(start, RATE_BYTES - 1), Ok(()));
    assert_eq!(rate.count(start, 1), Ok(()));
    assert_eq!(rate.count(start, 1), bytes);
    assert_eq!(rate.count(start + RATE_WINDOW, RATE_BYTES), Ok(()));
    assert_eq!(RATE_BYTES, 16 * 1024 * 1024);
}

#[test]
fn more_than_20_notifications_in_10_seconds_is_a_flood() {
    let mut rate = Rate::default();
    let start = Instant::now();
    let finished = json!({"type": "command_finished", "exit": 1, "secs": 60});
    let badge = json!({"type": "badge", "text": "b"});
    for _ in 0..ALERTS {
        assert_eq!(rate.alert(start, &finished), Ok(()));
        assert_eq!(rate.alert(start, &badge), Ok(()));
    }
    let flood = Err("sent more than 20 notifications in 10 seconds".to_owned());
    let late = start + ALERT_WINDOW - Duration::from_millis(1);
    assert_eq!(rate.alert(late, &finished), flood);
    assert_eq!(rate.alert(start + ALERT_WINDOW, &finished), Ok(()));
    let alerting_ones = [
        finished,
        json!({"type": "terminal_notification", "title": "t", "body": "b", "notify": false}),
        json!({"type": "usage_alert"}),
        json!({"type": "agent_state", "notify": true}),
    ];
    assert!(alerting_ones.iter().all(alerting));
    let quiet = [badge, json!({"type": "agent_state", "notify": false})];
    assert!(!quiet.iter().any(alerting));
}

#[test]
fn what_the_ui_takes_of_a_remote_is_read_as_its_type() {
    let padded = json!({"type": "badge", "text": "ok", "pad": "x", "channel": 2, "host": "gpu"});
    let read = typed(padded);
    let want = json!({"type": "badge", "text": "ok", "channel": 2, "host": "gpu"});
    assert_eq!(read, Ok(want));
    let wrong = [
        (json!({"type": "badge", "text": {"x": 1}}), "badge"),
        (json!({"type": "error", "message": {}}), "error"),
        (
            json!({"type": "command_finished", "exit": "1) Hive:", "secs": 1}),
            "command_finished",
        ),
    ];
    for (message, kind) in wrong {
        assert_eq!(
            typed(message),
            Err(format!("sent a {kind} the app cannot read"))
        );
    }
}

#[test]
fn a_remote_s_queue_holds_so_many_frames_and_bytes_the_local_one_any() {
    let queue = Queue::new(true);
    for _ in 0..QUEUE_FRAMES {
        assert!(queue.push(1));
    }
    assert!(!queue.push(0));
    // Not counted: one written makes room for exactly one.
    queue.pop(1);
    assert!(queue.push(1));
    assert!(!queue.push(1));
    let queue = Queue::new(true);
    assert!(queue.push(QUEUE_BYTES - 1));
    assert!(queue.push(1));
    assert!(!queue.push(1));
    queue.pop(QUEUE_BYTES - 1);
    assert!(queue.push(QUEUE_BYTES - 2));
    assert!(!queue.push(2));
    // The local service's is counted, never capped.
    let local = Queue::new(false);
    for _ in 0..=QUEUE_FRAMES {
        assert!(local.push(QUEUE_BYTES));
    }
}

#[tokio::test]
async fn a_full_queue_wakes_its_link() {
    let queue = Queue::new(true);
    queue.push(QUEUE_BYTES);
    assert!(!queue.push(1));
    let woken = tokio::time::timeout(Duration::from_secs(10), queue.full.notified());
    assert!(woken.await.is_ok());
    // One that fits wakes nothing.
    let queue = Queue::new(true);
    assert!(queue.push(1));
    let woken = tokio::time::timeout(Duration::from_millis(50), queue.full.notified());
    assert!(woken.await.is_err());
}

#[test]
fn terminal_output_past_what_the_ui_acknowledged_is_a_limit() {
    let over = Some("sent more terminal output than the app could show".to_owned());
    assert_eq!(unacked(UNACKED, LINK_UNACKED), None);
    assert_eq!(unacked(UNACKED + 1, 0), over);
    assert_eq!(unacked(0, LINK_UNACKED + 1), over);
    assert_eq!((UNACKED, LINK_UNACKED), (16 << 20, 64 << 20));
}

/// A message of `kind` whose `field` is a list of `len` numbers.
fn list(kind: &str, field: &str, len: usize) -> Value {
    json!({"type": kind, field: vec![0; len]})
}

#[test]
fn projects_sessions_and_any_list_are_capped() {
    let lists = [
        ("projects", PROJECTS),
        ("sessions", SESSIONS),
        ("comments", COMMENTS),
    ];
    for (kind, most) in lists {
        assert_eq!(check(&mut list(kind, kind, most)), Ok(()));
        let over = format!("sent {} {kind}, over the {most} the app takes", most + 1);
        assert_eq!(check(&mut list(kind, kind, most + 1)), Err(over));
        // Its own cap only in its own message.
        assert_eq!(check(&mut list("files", kind, most + 1)), Ok(()));
    }
    let over = format!(
        "sent a list of {} items, over the {LIST} the app takes",
        LIST + 1
    );
    assert_eq!(check(&mut list("files", "files", LIST)), Ok(()));
    assert_eq!(
        check(&mut list("files", "files", LIST + 1)),
        Err(over.clone())
    );
    // At any depth, in a list too.
    let deep = json!({"type": "changes", "files": [{"lines": vec![0; LIST + 1]}]});
    assert_eq!(check(&mut deep.clone()), Err(over));
    let fine = json!({"type": "changes", "files": [{"lines": [1, 2]}], "n": 3});
    let mut checked = fine.clone();
    assert_eq!(check(&mut checked), Ok(()));
    assert_eq!(checked, fine);
    assert_eq!(
        (PROJECTS, SESSIONS, COMMENTS, LIST),
        (1000, 1000, 1000, 50_000)
    );
}

#[test]
fn a_remote_s_titles_badges_and_errors_are_clipped() {
    let long = "é".repeat(TEXT + 5);
    let clipped = format!("{}…", "é".repeat(TEXT));
    let exact = "a".repeat(TEXT);
    for key in CLIPPED {
        let mut message = json!({"type": "x", (*key): long, "nested": [{(*key): long}]});
        check(&mut message).unwrap();
        assert_eq!(message[*key], clipped, "{key}");
        assert_eq!(message["nested"][0][*key], clipped, "{key}");
        let mut message = json!({"type": "x", (*key): exact});
        check(&mut message).unwrap();
        assert_eq!(message[*key], exact, "{key}");
    }
    // A notification's body, not a pull request's; no other field, no other value.
    let mut notification = json!({"type": "terminal_notification", "body": long});
    check(&mut notification).unwrap();
    assert_eq!(notification["body"], clipped);
    // A review comment's lines and author (27.3), and its text at 4096 characters.
    let mut comments = json!({"type": "comments", "comments": [{
        "code": [long], "before": [long], "after": [long], "author": long, "file": long,
        "body": "é".repeat(LONG_TEXT + 1),
    }]});
    check(&mut comments).unwrap();
    let comment = &comments["comments"][0];
    for key in ["code", "before", "after"] {
        assert_eq!(comment[key][0], clipped, "{key}");
    }
    assert_eq!(comment["author"], clipped);
    assert_eq!(comment["file"], long);
    assert_eq!(comment["body"], format!("{}…", "é".repeat(LONG_TEXT)));
    let mut exact = json!({"type": "comments", "body": "a".repeat(LONG_TEXT)});
    check(&mut exact).unwrap();
    assert_eq!(exact["body"], "a".repeat(LONG_TEXT));
    assert_eq!(LONG_TEXT, 4096);
    let longer = "é".repeat(LONG_TEXT + 5);
    let untouched = json!({"type": "pull", "body": long, "path": longer, "title": 3, "text": null});
    let mut message = untouched.clone();
    check(&mut message).unwrap();
    assert_eq!(message, untouched);
    assert_eq!(TEXT, 1024);
}

#[test]
fn quitting_follows_a_bounded_set_of_agents() {
    let mut link = Link::default();
    let working = |id: &str| json!({"type": "agent_state", "id": id, "state": "working"});
    for n in 0..=AGENTS {
        link.follow(&working(&n.to_string()));
    }
    assert_eq!(link.at_risk.len(), AGENTS);
    // A full set still forgets an agent that is no longer at risk.
    let idle = json!({"type": "agent_state", "id": "0", "state": "idle"});
    link.follow(&idle);
    assert_eq!(link.at_risk.len(), AGENTS - 1);
    let mut link = Link::default();
    link.follow(&working(&"i".repeat(ID)));
    link.follow(&working(&"j".repeat(ID + 1)));
    let ids: Vec<usize> = link.at_risk.iter().map(String::len).collect();
    assert_eq!(ids, [ID]);
}

#[test]
fn a_remote_host_s_link_asks_for_nothing_the_app_would_open() {
    let (tx, mut frames) = tokio::sync::mpsc::unbounded_channel();
    let link = Link {
        host: Some("gpu".into()),
        frames: Some(tx),
        ..Link::default()
    };
    let host = Host {
        link: Arc::new(Mutex::new(link)),
    };
    let refused = Err("Hive opens nothing of the remote host gpu".to_owned());
    let target = hive_protocol::SessionTarget::Log;
    assert_eq!(host.open_in_editor("/w".into(), "a".into(), false), refused);
    assert_eq!(host.open_in_editor("/w".into(), "a".into(), true), refused);
    assert_eq!(host.locate_session("s".into(), target), refused);
    assert_eq!(host.open_settings_file(), refused);
    assert_eq!(host.open_shared(), refused);
    assert!(frames.try_recv().is_err());
    // Its other requests go.
    host.list_sessions().unwrap();
    assert!(frames.try_recv().is_ok());
}

#[test]
fn only_https_urls_open_and_in_markdown_also_http_and_mail_addresses() {
    for markdown in [false, true] {
        let https = openable(" https://github.com/o/r/pull/7 ", markdown);
        assert_eq!(https.as_deref(), Some("https://github.com/o/r/pull/7"));
        assert_eq!(
            openable("HTTPS://A.dev", markdown).as_deref(),
            Some("https://a.dev/")
        );
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vscode://file/etc/passwd",
            "x-apple.systempreferences:com.apple.preference.security",
            "tel:123",
            "https://",
            "/relative",
            "",
        ] {
            assert_eq!(openable(refused, markdown), None, "{refused} {markdown}");
        }
    }
    assert_eq!(openable("http://a.dev/x", false), None);
    assert_eq!(openable("mailto:a@b.dev", false), None);
    assert_eq!(
        openable("http://a.dev/x", true).as_deref(),
        Some("http://a.dev/x")
    );
    let mail = openable("mailto:a@b.dev?subject=s&attach=/etc/passwd", true);
    assert_eq!(mail.as_deref(), Some("mailto:a@b.dev"));
}
