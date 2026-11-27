//! A fake remote service that misbehaves (26.19, spike §8): the app stays up, drops the link
//! past a cap with the reason, and opens nothing a remote names.

use super::*;
use tokio_util::bytes::{BufMut, BytesMut};
use tokio_util::codec::Encoder as _;

/// `frames` as the bytes the codec writes.
fn encoded(frames: impl IntoIterator<Item = Frame>) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    for frame in frames {
        FrameCodec.encode(frame, &mut bytes).unwrap();
    }
    bytes.to_vec()
}

/// A control frame of `message` on `channel`, any JSON.
fn on(channel: u32, message: &Value) -> Frame {
    Frame {
        channel,
        ..raw(message)
    }
}

#[tokio::test]
async fn a_remote_past_a_cap_is_dropped_with_why_a_broken_stream_retried() {
    let lab = Lab::new("limits");
    let listener = lab.listen();
    lab.plan(&[lab.pipe()]);
    let header = |kind: u8, len: usize| {
        let mut bytes = BytesMut::new();
        bytes.put_u8(kind);
        bytes.put_u32(0);
        bytes.put_u32(u32::try_from(len).unwrap());
        bytes.to_vec()
    };
    let nonsense = json!({"type": "nonsense"});
    let flood = encoded((0..3 * crate::limits::RATE).map(|_| raw(&nonsense)));
    let projects = json!({"type": "projects", "projects": vec![0; 1001]});
    let files = json!({"type": "files", "files": vec![0; 50_001]});
    let pad = "p".repeat(hive_protocol::MAX_PAYLOAD - 64);
    let heavy = encoded((0..5).map(|_| raw(&json!({"type": "nonsense", "pad": pad}))));
    let cases = [
        (
            header(0, hive_protocol::MAX_PAYLOAD + 1),
            "reconnecting",
            "Cannot reach gpu: frame payload of 4194305 bytes exceeds the limit of 4194304 bytes",
        ),
        (
            header(7, 0),
            "reconnecting",
            "Cannot reach gpu: unknown frame type 7",
        ),
        (
            flood,
            "disconnected",
            "Hive dropped the link to gpu: it sent more than 1000 messages in a second.",
        ),
        (
            encoded([raw(&projects)]),
            "disconnected",
            "Hive dropped the link to gpu: it sent 1001 projects, over the 1000 the app takes.",
        ),
        (
            encoded([raw(&files)]),
            "disconnected",
            "Hive dropped the link to gpu: it sent a list of 50001 items, over the 50000 the app takes.",
        ),
        (
            heavy,
            "disconnected",
            "Hive dropped the link to gpu: it sent more than 16 MiB of messages in a second.",
        ),
    ];
    for (bad, state, reason) in cases {
        let mut remotes = lab.remotes(Arc::default());
        // A retry waits for ever: no later connection of this link meets the next case's accept.
        remotes.sleep = Box::new(|_| Box::pin(std::future::pending()));
        let remotes = Arc::new(remotes);
        let (channel, mut rx) = ui();
        remotes.sync(settings(&["gpu"], 1), Some(&channel));
        let mut service = Service::accept(&listener).await;
        service.welcome(60).await;
        until(&mut rx, "connected").await;
        // Sent again and again until the link closes: a flood stays a flood however slowly a
        // loaded machine reads it.
        let Service {
            mut reader,
            mut writer,
        } = service;
        tokio::spawn(async move {
            for _ in 0..1000 {
                if writer.get_mut().write_all(&bad).await.is_err() {
                    break;
                }
            }
        });
        let seen = until(&mut rx, state).await;
        assert_eq!(states(&seen), [state], "{reason}");
        let ended = seen.last().unwrap();
        assert_eq!(ended["reason"], reason);
        // A frame the codec refuses breaks the stream, retried as before (26.17: a shell's
        // startup file printing on stdout); past a cap the link stops at once, and only
        // Reconnect tries again.
        if state == "disconnected" {
            assert_eq!(ended["kind"], "limit");
        }
        // Its `ssh` is gone.
        let closed = async { while reader.next().await.is_some() {} };
        tokio::time::timeout(WAIT, closed).await.unwrap();
        remotes.end();
    }
}

/// A `Hive` whose local service links it to the host `gpu`, connected: the app, the local and
/// the remote test services, and the UI's messages.
async fn to_gpu(
    lab: &Lab,
) -> (
    Hive,
    crate::tests::Service,
    Service,
    mpsc::UnboundedReceiver<Value>,
) {
    let listener = lab.listen();
    lab.plan(&[lab.pipe()]);
    let (hive, mut local, mut rx) = linked(lab);
    local.control().await;
    let mut settings = hive_protocol::Settings::default();
    settings.remote = super::settings(&["gpu"], 5);
    local
        .send(
            0,
            Control::Settings {
                settings,
                problems: Vec::new(),
            },
        )
        .await;
    let mut remote = Service::accept(&listener).await;
    remote.welcome(300).await;
    until(&mut rx, "connected").await;
    (hive, local, remote, rx)
}

#[tokio::test]
async fn a_remote_names_no_path_or_url_the_app_opens_and_its_text_is_clipped() {
    let lab = Lab::new("names");
    let (hive, _local, mut remote, mut rx) = to_gpu(&lab).await;
    let gpu = hive.on(Some("gpu")).unwrap();
    let id = gpu.open_terminal("/w".into(), 80, 24, None, output().0);
    assert_eq!(id, Ok(1));
    assert_eq!(remote.control().await.0, 1);
    // Unknown types and paths to open, on channel 0 and on its terminal's: none reaches the UI,
    // none is approved for `open_path`.
    let messages = [
        json!({"type": "open_url", "url": "file:///etc/passwd"}),
        json!({"type": "editor_target", "worktree": "", "path": "", "windows_path": "/Applications/Calculator.app", "reveal": false, "error": null}),
        json!({"type": "editor_target", "worktree": "", "path": "shared", "windows_path": "/etc", "reveal": true, "error": null}),
        json!({"type": "session_located", "id": "s", "target": "folder", "windows_path": "/etc/hosts", "error": null}),
    ];
    for message in &messages {
        remote.writer.send(on(0, message)).await.unwrap();
        remote.writer.send(on(1, message)).await.unwrap();
    }
    // A badge longer than the app shows: the first message the UI gets, clipped.
    let long = "b".repeat(crate::limits::TEXT + 1);
    let badge = json!({"type": "badge", "text": long});
    remote.writer.send(on(1, &badge)).await.unwrap();
    let got = tokio::time::timeout(WAIT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    let clipped = format!("{}…", &long[..crate::limits::TEXT]);
    assert_eq!(
        got,
        json!({"type": "badge", "text": clipped, "channel": 1, "host": "gpu"})
    );
    for path in ["/Applications/Calculator.app", "/etc", "/etc/hosts"] {
        let unasked = Err(format!("Hive opens only a path the service sent: {path}"));
        assert_eq!(hive.open_path(path.into(), false), unasked);
        assert_eq!(hive.open_path(path.into(), true), unasked);
    }
    assert!(lock(&gpu.link).approved.is_empty());
    // A pull request's and a run's `javascript:` / `file:` URLs reach the UI with the host's
    // lists (26.18), and nothing opens them: links open only through `open_url`, https only.
    let pull = json!({"number": 1, "title": "t", "url": "javascript:alert(1)", "state": "open", "branch": "b", "base": "main", "author": "a", "review": null, "checks": null, "updated_at": "", "worktree": null});
    let pulls = json!({"type": "pulls", "project": "/r", "repo": null, "mine": [pull], "review": [], "fetched_ms": 0, "error": null});
    let run = json!({"id": 1, "number": 1, "workflow": "w", "title": "t", "branch": "b", "event": "push", "state": "passing", "status": "completed", "url": "file:///etc/passwd", "created_at": "", "started_at": "", "updated_at": "", "worktree": null});
    let runs = json!({"type": "runs", "project": "/r", "branch": null, "runs": [run], "fetched_ms": 0, "error": null});
    for (message, url) in [(pulls, "javascript:alert(1)"), (runs, "file:///etc/passwd")] {
        remote.writer.send(on(0, &message)).await.unwrap();
        let got = tokio::time::timeout(WAIT, rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (&got["type"], &got["host"]),
            (&message["type"], &json!("gpu"))
        );
        assert!(got.to_string().contains(url));
    }
    for url in ["file:///etc/passwd", "javascript:alert(1)"] {
        for markdown in [false, true] {
            let refused = Err(format!("Hive opens only https links: {url}"));
            assert_eq!(hive.open_url(url, markdown), refused);
        }
    }
    // Nothing the app would open or reveal is asked of a remote; the link stays up.
    let refused = Err("Hive opens nothing of the remote host gpu".to_owned());
    let target = hive_protocol::SessionTarget::Folder;
    assert_eq!(
        gpu.open_in_editor("/w".into(), String::new(), true),
        refused
    );
    assert_eq!(gpu.locate_session("s".into(), target), refused);
    assert_eq!(gpu.open_settings_file(), refused);
    assert_eq!(gpu.open_shared(), refused);
    gpu.list_sessions().unwrap();
    assert_eq!(remote.control().await, (0, Control::ListSessions));
    hive.remotes().unwrap().end();
}

/// The bytes of terminal output `bytes` gets until it has `want`.
async fn shown(bytes: &mut mpsc::UnboundedReceiver<Vec<u8>>, want: usize) -> usize {
    let mut got = 0;
    while got < want {
        let chunk = tokio::time::timeout(WAIT, bytes.recv()).await.unwrap();
        got += chunk.unwrap().len();
    }
    got
}

#[tokio::test]
async fn a_remote_terminal_s_output_flows_at_the_ui_s_pace_and_no_faster() {
    let lab = Lab::new("pace");
    let (hive, _local, mut remote, mut rx) = to_gpu(&lab).await;
    let gpu = hive.on(Some("gpu")).unwrap();
    let (out, mut bytes) = output();
    assert_eq!(gpu.open_terminal("/w".into(), 80, 24, None, out), Ok(1));
    assert_eq!(remote.control().await.0, 1);
    // Many small frames are terminal output, not a flood of messages.
    let many = 2 * crate::limits::RATE;
    let small = encoded((0..many).map(|_| Frame::terminal(1, b"x".to_vec())));
    remote.writer.get_mut().write_all(&small).await.unwrap();
    assert_eq!(shown(&mut bytes, many).await, many);
    // What the UI acknowledged no longer counts; up to the cap it flows.
    hive.ack_terminal(1, u32::try_from(many).unwrap()).unwrap();
    let ack = Control::Ack {
        bytes: u32::try_from(many).unwrap(),
    };
    assert_eq!(remote.control().await, (1, ack));
    let chunk = vec![b'y'; hive_protocol::MAX_PAYLOAD];
    let cap = usize::try_from(crate::limits::UNACKED).unwrap();
    let full = encoded((0..cap / chunk.len()).map(|_| Frame::terminal(1, chunk.clone())));
    remote.writer.get_mut().write_all(&full).await.unwrap();
    assert_eq!(shown(&mut bytes, cap).await, cap);
    // One byte more than the UI could show ends the link.
    let more = encoded([Frame::terminal(1, b"z".to_vec())]);
    remote.writer.get_mut().write_all(&more).await.unwrap();
    let seen = until(&mut rx, "disconnected").await;
    let stopped = seen.last().unwrap();
    let reason =
        "Hive dropped the link to gpu: it sent more terminal output than the app could show.";
    assert_eq!(
        (&stopped["kind"], &stopped["reason"]),
        (&json!("limit"), &json!(reason))
    );
    // Its terminal ended with it; the byte never reached the screen.
    let exited = json!({"type": "terminal_exited", "channel": 1, "code": null, "host": "gpu"});
    assert!(seen.contains(&exited), "{seen:?}");
    assert!(bytes.try_recv().is_err());
    hive.remotes().unwrap().end();
}

#[tokio::test]
async fn a_remote_that_stops_reading_is_dropped_before_the_queue_grows() {
    let lab = Lab::new("queue");
    let (hive, _local, mut remote, mut rx) = to_gpu(&lab).await;
    let gpu = hive.on(Some("gpu")).unwrap();
    assert_eq!(
        gpu.open_terminal("/w".into(), 80, 24, None, output().0),
        Ok(1)
    );
    assert_eq!(remote.control().await.0, 1);
    // The remote reads nothing more: pastes fill the pipes, then the queue.
    let paste = "p".repeat(hive_protocol::MAX_PAYLOAD);
    let most = crate::limits::QUEUE_BYTES / paste.len() + 8;
    let refused = (0..most).find_map(|_| hive.write_terminal(1, &paste).err());
    assert_eq!(
        refused,
        Some("gpu stopped reading what the app sends".into())
    );
    let seen = until(&mut rx, "disconnected").await;
    let stopped = seen.last().unwrap();
    let reason = "Hive dropped the link to gpu: it stopped reading what the app sends.";
    assert_eq!(
        (&stopped["kind"], &stopped["reason"]),
        (&json!("limit"), &json!(reason))
    );
    assert!(lock(&gpu.link).frames.is_none());
    hive.remotes().unwrap().end();
}

#[tokio::test]
async fn what_the_ui_takes_of_a_remote_is_read_as_its_type_and_its_notifications_counted() {
    let lab = Lab::new("typed");
    let (hive, _local, mut remote, mut rx) = to_gpu(&lab).await;
    let gpu = hive.on(Some("gpu")).unwrap();
    assert_eq!(
        gpu.open_terminal("/w".into(), 80, 24, None, output().0),
        Ok(1)
    );
    assert_eq!(remote.control().await.0, 1);
    // A field the type has not is dropped.
    let padded = json!({"type": "badge", "text": "ok", "pad": "p".repeat(1000)});
    remote.writer.send(on(1, &padded)).await.unwrap();
    let got = next_of(&mut rx, "badge").await;
    assert_eq!(
        got,
        json!({"type": "badge", "text": "ok", "channel": 1, "host": "gpu"})
    );
    // Up to 20 notifications in 10 seconds reach the UI; the next ends the link.
    let finished = json!({"type": "command_finished", "exit": 1, "secs": 60});
    for _ in 0..crate::limits::ALERTS {
        remote.writer.send(on(1, &finished)).await.unwrap();
        next_of(&mut rx, "command_finished").await;
    }
    remote.writer.send(on(1, &finished)).await.unwrap();
    let seen = until(&mut rx, "disconnected").await;
    assert!(
        seen.iter().all(|m| m["type"] != "command_finished"),
        "{seen:?}"
    );
    let reason = "Hive dropped the link to gpu: it sent more than 20 notifications in 10 seconds.";
    assert_eq!(seen.last().unwrap()["reason"], reason);
    hive.remotes().unwrap().end();
    // A field of another type (an object React cannot show) ends the link.
    let lab = Lab::new("typed-wrong");
    let (hive, _local, mut remote, mut rx) = to_gpu(&lab).await;
    let gpu = hive.on(Some("gpu")).unwrap();
    assert_eq!(
        gpu.open_terminal("/w".into(), 80, 24, None, output().0),
        Ok(1)
    );
    assert_eq!(remote.control().await.0, 1);
    let wrong = json!({"type": "badge", "text": {"x": 1}});
    remote.writer.send(on(1, &wrong)).await.unwrap();
    let seen = until(&mut rx, "disconnected").await;
    assert!(seen.iter().all(|m| m["type"] != "badge"), "{seen:?}");
    let stopped = seen.last().unwrap();
    let reason = "Hive dropped the link to gpu: it sent a badge the app cannot read.";
    assert_eq!(
        (&stopped["kind"], &stopped["reason"]),
        (&json!("limit"), &json!(reason))
    );
    hive.remotes().unwrap().end();
}

#[tokio::test]
async fn all_of_a_remote_s_terminals_together_flow_no_faster_than_the_ui() {
    let lab = Lab::new("pace-all");
    let (hive, _local, mut remote, mut rx) = to_gpu(&lab).await;
    let gpu = hive.on(Some("gpu")).unwrap();
    // Six terminals of 12 MiB each, under each one's cap: past 64 MiB of them all, the link ends.
    for id in 1..=6 {
        let opened = gpu.open_terminal("/w".into(), 80, 24, None, output().0);
        assert_eq!(opened, Ok(id));
        assert_eq!(remote.control().await.0, id);
    }
    let chunk = vec![b'y'; hive_protocol::MAX_PAYLOAD];
    let frames = (1..=6).flat_map(|id| (0..3).map(move |_| id));
    let all = encoded(frames.map(|id| Frame::terminal(id, chunk.clone())));
    // The link may close before all of it is written.
    let _ = remote.writer.get_mut().write_all(&all).await;
    let seen = until(&mut rx, "disconnected").await;
    let reason =
        "Hive dropped the link to gpu: it sent more terminal output than the app could show.";
    assert_eq!(seen.last().unwrap()["reason"], reason);
    hive.remotes().unwrap().end();
}

#[tokio::test]
async fn terminal_output_no_terminal_takes_is_counted_as_messages_are() {
    let lab = Lab::new("unknown-channel");
    let (hive, _local, remote, mut rx) = to_gpu(&lab).await;
    // Output for a channel the link does not know is dropped, but a flood of it ends the link,
    // however slowly a loaded machine reads it.
    let chunk = vec![b'y'; hive_protocol::MAX_PAYLOAD];
    let flood = encoded((0..4).map(|_| Frame::terminal(99, chunk.clone())));
    let Service { mut writer, .. } = remote;
    tokio::spawn(async move {
        for _ in 0..1000 {
            if writer.get_mut().write_all(&flood).await.is_err() {
                break;
            }
        }
    });
    let seen = until(&mut rx, "disconnected").await;
    let stopped = seen.last().unwrap();
    let reason = "Hive dropped the link to gpu: it sent more than 16 MiB of messages in a second.";
    assert_eq!(
        (&stopped["kind"], &stopped["reason"]),
        (&json!("limit"), &json!(reason))
    );
    hive.remotes().unwrap().end();
}
