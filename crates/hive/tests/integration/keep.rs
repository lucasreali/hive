//! A remote service outlives a dropped link (26.15): an app whose `hello` gives `keep_secs`
//! leaves its terminals running for that long; the local app (no `keep_secs`) ends them, #18.

use std::time::Duration;

use crate::common::{self, Conn, Env, stop};
use hive_protocol::{Control, FrameType, KeptTerminal, PROTOCOL_VERSION, Role};

/// An app's `hello` with the remote fields.
fn hello(keep_secs: Option<u64>, app_id: Option<&str>) -> Control {
    Control::Hello {
        protocol: PROTOCOL_VERSION,
        version: hive::VERSION.into(),
        role: Role::App,
        keep_secs,
        app_id: app_id.map(str::to_owned),
    }
}

/// Connects an app with `hello(keep_secs, app_id)` and waits until the service serves it; no
/// terminal is kept.
async fn app(env: &Env, keep_secs: Option<u64>, app_id: Option<&str>) -> Conn {
    let (app, kept) = kept_app(env, keep_secs, app_id).await;
    assert_eq!(kept, []);
    app
}

/// [`app`], with the `terminals_kept` (26.16) the service sent first, if any.
async fn kept_app(
    env: &Env,
    keep_secs: Option<u64>,
    app_id: Option<&str>,
) -> (Conn, Vec<KeptTerminal>) {
    let mut app = welcomed(env, keep_secs, app_id).await;
    let settings = Control::Settings {
        settings: Default::default(),
    };
    let (kept, first) = match app.control().await {
        (0, Control::TerminalsKept { terminals }) => (terminals, app.control().await),
        first => (vec![], first),
    };
    assert_eq!(first, (0, settings));
    app.send(0, Control::ListProjects).await;
    let projects = Control::Projects { projects: vec![] };
    assert_eq!(app.control().await, (0, projects));
    (app, kept)
}

/// Connects with `hello(keep_secs, app_id)` and reads the `welcome`.
async fn welcomed(env: &Env, keep_secs: Option<u64>, app_id: Option<&str>) -> Conn {
    let mut conn = env.raw().await;
    conn.send(0, hello(keep_secs, app_id)).await;
    let welcome = Control::Welcome {
        version: hive::VERSION.into(),
        distro: Some(common::DISTRO.into()),
        warning: None,
    };
    assert_eq!(conn.control().await, (0, welcome));
    conn
}

/// Reads `conn` until the service closes it.
async fn closes(conn: &mut Conn) {
    while conn.next().await.is_some() {}
}

/// The processes of this environment other than the daemon: its terminals'.
fn terminals(env: &Env, daemon: &common::Daemon) -> Vec<hive::procs::Proc> {
    let service = i32::try_from(daemon.0.id()).unwrap();
    let all = env.processes().into_iter();
    all.filter(|proc| proc.pid != service).collect()
}

/// Whether the daemon is still running after `wait`.
fn alive_after(daemon: &mut common::Daemon, wait: Duration) -> bool {
    std::thread::sleep(wait);
    daemon.0.try_wait().unwrap().is_none()
}

#[tokio::test]
async fn a_kept_terminal_writes_on_past_the_window_once_the_link_drops() {
    let env = Env::new();
    let daemon = env.daemon();
    let mut app = app(&env, Some(60), Some("a")).await;
    app.open_terminal(1, &env.path("home")).await;
    // 4 MiB, far past the 512 KiB the app may leave unacknowledged, then a mark.
    let done = env.path("done");
    let flood = format!("yes | head -c 4194304; touch '{}'\r", done.display());
    app.input(1, &flood).await;
    // The flood has begun; the app acknowledges none of it, so the terminal soon waits for
    // the app (9.19), and then the app goes mid-flood.
    app.output_until(1, "y\r\ny\r\ny\r\n").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!done.exists());
    drop(app);
    common::wait_until(|| done.exists());
    stop(daemon);
}

#[tokio::test]
async fn kept_terminals_end_after_the_keep_time_unless_an_app_is_back() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut first = app(&env, Some(2), Some("a")).await;
    first.open_terminal(1, &env.path("home")).await;
    drop(first);
    // Back within the keep time: the service stays past it, while the app is there.
    std::thread::sleep(Duration::from_millis(500));
    let (back, kept) = kept_app(&env, Some(1), Some("a")).await;
    assert_eq!(kept.len(), 1);
    assert!(alive_after(&mut daemon, Duration::from_secs(3)));
    // Gone again: its own keep time later, everything ends as when the local app goes.
    drop(back);
    assert!(alive_after(&mut daemon, Duration::from_millis(300)));
    assert!(daemon.wait_exit().success());
    assert!(!env.socket().exists());
    common::wait_until(|| env.processes().is_empty());
}

#[tokio::test]
async fn bye_with_end_ends_at_once_and_bye_without_keeps() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = app(&env, Some(3600), Some("a")).await;
    app.open_terminal(1, &env.path("home")).await;
    app.send(0, Control::Bye { end: false }).await;
    // The service closes the connection, and keeps the terminal: its shell still runs.
    closes(&mut app).await;
    assert!(alive_after(&mut daemon, Duration::from_millis(500)));
    assert!(!terminals(&env, &daemon).is_empty());
    let (mut app, kept) = kept_app(&env, Some(3600), Some("b")).await;
    assert_eq!(kept.len(), 1);
    app.send(0, Control::Bye { end: true }).await;
    closes(&mut app).await;
    // Not after the hour: now.
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn the_local_app_ends_the_terminals_when_it_goes_as_before() {
    let env = Env::new();
    let mut daemon = env.daemon();
    // An `app_id` without `keep_secs` keeps nothing.
    let mut app = app(&env, None, Some("a")).await;
    app.open_terminal(1, &env.path("home")).await;
    drop(app);
    assert!(daemon.wait_exit().success());
    // Nor does `bye {end: false}` without it.
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    app.send(0, Control::Bye { end: false }).await;
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn the_same_app_takes_over_a_stale_connection_and_another_is_refused() {
    let env = Env::new();
    let daemon = env.daemon();
    let mut stale = app(&env, Some(60), Some("a")).await;
    let refused = Control::Error {
        message: "another app is already connected".into(),
    };
    // Another app, or one without an id, while it is attached: refused as before.
    for id in [Some("b"), None] {
        let mut other = welcomed(&env, Some(60), id).await;
        assert_eq!(other.control().await, (0, refused.clone()));
        assert_eq!(other.next().await, None);
    }
    // The same app launch takes over, and the stale connection is closed.
    let mut taken = app(&env, Some(60), Some("a")).await;
    closes(&mut stale).await;
    taken.send(0, Control::GetSettings).await;
    let settings = Control::Settings {
        settings: Default::default(),
    };
    assert_eq!(taken.control().await, (0, settings));
    // Once it dropped, another app may come.
    drop(taken);
    let other = app(&env, Some(60), Some("b")).await;
    drop(other);
    stop(daemon);
}

#[tokio::test]
async fn only_a_remote_connection_takes_over_or_is_taken_over() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let refused = Control::Error {
        message: "another app is already connected".into(),
    };
    // An `app_id` without `keep_secs` is the local app: never taken over, even by itself...
    let mut local = app(&env, None, Some("a")).await;
    local.open_terminal(1, &env.path("home")).await;
    for keep in [None, Some(60)] {
        let mut again = welcomed(&env, keep, Some("a")).await;
        assert_eq!(again.control().await, (0, refused.clone()));
        closes(&mut again).await;
    }
    // ...so its terminals end with it (#18).
    drop(local);
    assert!(daemon.wait_exit().success());
    common::wait_until(|| env.processes().is_empty());
    // A remote connection is not taken over by one without `keep_secs`.
    let env = Env::new();
    let daemon = env.daemon();
    let _remote = app(&env, Some(60), Some("a")).await;
    let mut local = welcomed(&env, None, Some("a")).await;
    assert_eq!(local.control().await, (0, refused));
    closes(&mut local).await;
    stop(daemon);
}

#[tokio::test]
async fn an_app_id_past_its_limit_is_refused() {
    let env = Env::new();
    let daemon = env.daemon();
    let long = "x".repeat(129);
    let mut conn = env.raw().await;
    conn.send(0, hello(Some(60), Some(&long))).await;
    let refused = Control::Error {
        message: "an app_id is at most 128 bytes".into(),
    };
    assert_eq!(conn.control().await, (0, refused));
    assert_eq!(conn.next().await, None);
    // 128 bytes is fine.
    drop(app(&env, Some(60), Some(&long[1..])).await);
    stop(daemon);
}

/// Terminal `channel`'s output on `conn` after its `attach_terminal`: whether a
/// `terminal_reset` came first (with its offset), then `len` bytes.
async fn replay(conn: &mut Conn, channel: u32, len: usize) -> (Option<u64>, Vec<u8>) {
    let (mut reset, mut bytes) = (None, Vec::new());
    while bytes.len() < len {
        let frame = conn.next().await.expect("connection closed");
        if frame.channel != channel {
            continue;
        }
        match frame.kind {
            FrameType::Terminal => bytes.extend_from_slice(&frame.payload),
            _ => match frame.to_control().unwrap() {
                Control::TerminalReset { offset } if bytes.is_empty() => reset = Some(offset),
                other => panic!("unexpected {other:?}"),
            },
        }
    }
    (reset, bytes)
}

#[tokio::test]
async fn a_reattached_terminal_gets_every_byte_its_ring_still_holds_in_order() {
    let env = Env::new();
    let daemon = env.daemon();
    let mut app = app(&env, Some(60), Some("a")).await;
    let home = env.path("home");
    app.open_terminal(1, &home).await;
    // The shell reads its input (fish's queries answered at its prompt); then 2 MiB of
    // numbers and a mark: once they begin, the app drops and the rest is written with no app.
    app.input(1, "echo rea''dy\r").await;
    app.output_until(1, "ready\r\n").await;
    let done = env.path("done");
    let flood = format!("seq 1 300000; touch '{}'\r", done.display());
    app.input(1, &flood).await;
    app.output_until(1, "\r\n2\r\n3\r\n4\r\n").await;
    drop(app);
    common::wait_until(|| done.exists());
    // Its next prompt written too, so the offsets below stay put.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Back: the terminal is kept, with every byte it wrote counted.
    let (mut app, kept) = kept_app(&env, Some(60), Some("a")).await;
    let [kept] = kept.as_slice() else {
        panic!("one kept terminal: {kept:?}");
    };
    let numbers: usize = (1..=300_000).map(|n: u32| n.to_string().len() + 2).sum();
    assert!(kept.offset > numbers as u64, "{kept:?}");
    let place = (kept.channel, kept.cwd.as_str(), kept.worktree.as_deref());
    assert_eq!(place, (1, home.to_str().unwrap(), None));
    // From its first byte, long gone: a reset, then the whole ring, the last 1 MiB.
    let ring = hive::ring::RING;
    app.send(1, Control::AttachTerminal { from: 0 }).await;
    let (reset, bytes) = replay(&mut app, 1, ring).await;
    // The ring's start: at least where `terminals_kept`'s offset puts it (a slow prompt may
    // have come since).
    let start = reset.unwrap();
    assert!(start >= kept.offset - ring as u64, "{start} {kept:?}");
    assert_eq!(bytes.len(), ring);
    // In order: whole lines of consecutive numbers, up to the last one.
    let text = String::from_utf8(bytes.clone()).unwrap();
    let mut lines = text.split("\r\n").skip(1);
    let first: u32 = lines.next().unwrap().parse().unwrap();
    let last = (lines.map_while(|line| line.parse::<u32>().ok()))
        .enumerate()
        .inspect(|(i, n)| assert_eq!(*n, first + 1 + *i as u32))
        .last();
    assert_eq!(last.map(|(_, n)| n), Some(300_000));
    // From inside the ring: only the bytes after it.
    let tail = 4096;
    let from = start + (ring - tail) as u64;
    app.send(1, Control::AttachTerminal { from }).await;
    let (reset, after) = replay(&mut app, 1, tail).await;
    assert_eq!((reset, after.as_slice()), (None, &bytes[ring - tail..]));
    // A terminal no longer open answers its exit.
    app.send(9, Control::AttachTerminal { from: 0 }).await;
    let exited = Control::TerminalExited { code: None };
    assert_eq!(app.control().await, (9, exited));
    stop(daemon);
}

#[tokio::test]
async fn the_local_apps_terminals_are_not_kept_nor_attached() {
    let env = Env::new();
    let daemon = env.daemon();
    let mut app = env.app().await;
    app.open_terminal(1, &env.path("home")).await;
    app.send(1, Control::AttachTerminal { from: 0 }).await;
    let refused = Control::Error {
        message: "terminal 1 is not kept".into(),
    };
    assert_eq!(app.control().await, (1, refused));
    stop(daemon);
}
