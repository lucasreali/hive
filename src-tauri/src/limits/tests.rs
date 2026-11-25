use super::*;
use crate::{openable, Host, Link};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn more_than_the_rate_in_a_second_is_a_flood() {
    let mut rate = Rate::default();
    let start = Instant::now();
    for _ in 0..RATE {
        assert_eq!(rate.count(start), Ok(()));
    }
    let flood = Err("sent more than 1000 messages in a second".to_owned());
    assert_eq!(rate.count(start + Duration::from_millis(999)), flood);
    // A new second counts from one again: the last one's frames are gone.
    let mut rate = Rate::default();
    for _ in 0..RATE {
        rate.count(start).unwrap();
    }
    assert_eq!(rate.count(start + RATE_WINDOW), Ok(()));
    for _ in 1..RATE {
        rate.count(start + RATE_WINDOW).unwrap();
    }
    assert!(rate.count(start + RATE_WINDOW).is_err());
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
    assert_eq!(unacked(UNACKED), None);
    assert_eq!(
        unacked(UNACKED + 1),
        Some("sent more terminal output than the app could show".into())
    );
    assert_eq!(UNACKED, 16 * 1024 * 1024);
}

/// A message of `kind` whose `field` is a list of `len` numbers.
fn list(kind: &str, field: &str, len: usize) -> Value {
    json!({"type": kind, field: vec![0; len]})
}

#[test]
fn projects_sessions_and_any_list_are_capped() {
    for (kind, most) in [("projects", PROJECTS), ("sessions", SESSIONS)] {
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
    assert_eq!((PROJECTS, SESSIONS, LIST), (1000, 1000, 50_000));
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
    let untouched = json!({"type": "pull", "body": long, "path": long, "title": 3, "text": null});
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
