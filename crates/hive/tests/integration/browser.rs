use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Settings};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::agents::hook;
use crate::common::{Conn, TIMEOUT, wait_until};
use crate::worktree::Repo;

fn exe(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The next browser message the app gets, others skipped.
async fn browser(app: &mut Conn) -> Control {
    loop {
        if let (0, m @ (Control::BrowserStatus { .. } | Control::BrowserFrame { .. })) =
            app.control().await
        {
            return m;
        }
    }
}

fn status(worktree: &str, running: bool, url: Option<&str>, error: Option<&str>) -> Control {
    Control::BrowserStatus {
        worktree: worktree.into(),
        running,
        url: url.map(Into::into),
        error: error.map(Into::into),
    }
}

/// `hive browser-mcp` run in `cwd` with only `bin` on `PATH`, as Claude Code runs it.
async fn mcp(repo: &Repo, cwd: &Path, bin: &Path) -> Output {
    let mut hive = repo.env.hive();
    hive.arg("browser-mcp").current_dir(cwd).env("PATH", bin);
    tokio::process::Command::from(hive).output().await.unwrap()
}

/// What a failed `hive browser-mcp` printed.
fn failed(out: &Output) -> String {
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The service's CDP connection to the fake browser: the test plays the browser.
async fn cdp(listener: &TcpListener) -> WebSocketStream<TcpStream> {
    let accepted = tokio::time::timeout(TIMEOUT, listener.accept()).await;
    let (stream, _) = accepted.expect("no browser connection").unwrap();
    tokio_tungstenite::accept_async(stream).await.unwrap()
}

async fn read(ws: &mut WebSocketStream<TcpStream>) -> Value {
    let next = tokio::time::timeout(TIMEOUT, ws.next()).await;
    let message = next.expect("no CDP message").unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

/// Waits until the service closes the connection (the browser is stopped).
async fn closed(ws: &mut WebSocketStream<TcpStream>) {
    let drained = async { while let Some(Ok(_)) = ws.next().await {} };
    tokio::time::timeout(TIMEOUT, drained)
        .await
        .expect("still connected");
}

async fn write(ws: &mut WebSocketStream<TcpStream>, message: Value) {
    ws.send(Message::text(message.to_string())).await.unwrap();
}

/// The browser settings on, with `path` and at most one browser.
fn on(path: &Path) -> Settings {
    let mut settings = Settings::default();
    settings.browser.enabled = true;
    settings.browser.path = Some(path.display().to_string());
    settings.browser.max = 1;
    settings
}

async fn set(app: &mut Conn, settings: Settings) {
    app.send(0, Control::SetSettings { settings }).await;
}

/// Waits until the process whose pid the fake browser last wrote is gone.
fn ended(pid: &Path) {
    let pid: i32 = std::fs::read_to_string(pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    wait_until(|| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err());
}

/// The profile the fake browser was last started with.
fn profile(args: &Path) -> PathBuf {
    let line = std::fs::read_to_string(args).unwrap();
    let dir = line
        .lines()
        .next()
        .unwrap()
        .strip_prefix("--user-data-dir=");
    PathBuf::from(dir.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_browser_starts_on_its_mcps_ask_streams_while_watched_and_stops_with_it() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "fix-a"]).status.success());
    let root = repo.root.display().to_string();
    let fix_a = repo.root.join(".claude/worktrees/fix-a");
    let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
    let port = listener.local_addr().unwrap().port();
    let pid = repo.env.path("browser.pid");
    let args = repo.env.path("browser.args");
    let fake = repo.env.path("fake-browser");
    let script = format!(
        "#!/bin/sh\nfor a; do case $a in --user-data-dir=*) d=${{a#--user-data-dir=}};; esac; done\n[ -n \"$d\" ] || exit 0\necho $$ > '{}'\nprintf '%s\\n' \"$@\" > '{}'\nprintf '{port}\\n/devtools/browser/x\\n' > \"$d/DevToolsActivePort\"\nexec sleep 30\n",
        pid.display(),
        args.display()
    );
    exe(&fake, &script);
    // Run once now: macOS checks a new program on its first run, seconds on a loaded machine,
    // which would count against the browser's start.
    let first = std::process::Command::new(&fake).status().unwrap();
    assert!(first.success());
    // The MCP's runner: records its arguments.
    let bin = repo.env.path("mcp-bin");
    std::fs::create_dir(&bin).unwrap();
    let ran = repo.env.path("bunx.args");
    exe(
        &bin.join("bunx"),
        &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", ran.display()),
    );
    let empty = repo.env.path("empty-bin");
    std::fs::create_dir(&empty).unwrap();

    let mut daemon = repo.env.daemon();
    let mut app = repo.env.app().await;
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    let config = repo.env.path("data/hive/bin/hive-browser.json");
    // Off by default: no MCP server, and its ask is refused.
    assert!(!config.exists());
    let off = failed(&mcp(&repo, &repo.root, &bin).await);
    assert!(off.contains("the agent's browser is off"), "{off}");

    let settings = on(&fake);
    set(&mut app, settings.clone()).await;
    assert_eq!(app.control().await, (0, Control::Settings { settings }));
    wait_until(|| config.exists());
    let written: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    let server = &written["mcpServers"]["hive-browser"];
    assert_eq!(server["command"], env!("CARGO_BIN_EXE_hive"));
    assert_eq!(server["args"], json!(["browser-mcp"]));
    let home = repo.env.path("home");
    let outside = failed(&mcp(&repo, &home, &bin).await);
    assert!(
        outside.contains("is in no worktree Hive follows"),
        "{outside}"
    );

    // The panel shows the main worktree before its browser runs.
    let watch = |worktree: Option<&str>| Control::WatchBrowser {
        worktree: worktree.map(Into::into),
    };
    app.send(0, watch(Some(&root))).await;
    assert_eq!(browser(&mut app).await, status(&root, false, None, None));

    // Two asks at once: one browser, started once (the browser answers only once both asked).
    let (go, wait) = tokio::sync::oneshot::channel::<()>();
    let (stopped, unwatched) = tokio::sync::oneshot::channel::<()>();
    let cast = tokio::spawn({
        let listener = listener.clone();
        async move {
            wait.await.unwrap();
            let mut ws = cdp(&listener).await;
            assert_eq!(read(&mut ws).await["method"], "Target.setDiscoverTargets");
            let page = json!({ "method": "Target.targetCreated", "params": { "targetInfo": {
                "targetId": "p1", "type": "page", "url": "http://localhost:5173/" } } });
            write(&mut ws, page).await;
            let attach = read(&mut ws).await;
            assert_eq!(attach["method"], "Target.attachToTarget");
            let attached = json!({ "id": attach["id"], "result": { "sessionId": "s" } });
            write(&mut ws, attached).await;
            assert_eq!(read(&mut ws).await["method"], "Page.startScreencast");
            let frame = json!({ "method": "Page.screencastFrame", "sessionId": "s",
                "params": { "data": "AAAA", "sessionId": 1, "metadata": {} } });
            write(&mut ws, frame).await;
            let ack = read(&mut ws).await;
            assert_eq!(ack["method"], "Page.screencastFrameAck");
            assert_eq!(ack["params"], json!({ "sessionId": 1 }));
            // The panel no longer shows it: its screencast stops.
            assert_eq!(read(&mut ws).await["method"], "Page.stopScreencast");
            assert_eq!(read(&mut ws).await["method"], "Target.detachFromTarget");
            stopped.send(()).unwrap();
            // Until the browser is stopped.
            closed(&mut ws).await;
        }
    });
    let dot_claude = repo.root.join(".claude");
    // Another worktree's ask meanwhile: the starting one counts against `browser.max`.
    let other = async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        mcp(&repo, &fix_a, &bin).await
    };
    let asks = async {
        tokio::join!(
            mcp(&repo, &dot_claude, &bin),
            mcp(&repo, &repo.root, &bin),
            other
        )
    };
    let release = async {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        go.send(()).unwrap();
    };
    let ((a, b, c), ()) = tokio::join!(asks, release);
    assert!(a.status.success(), "{a:?}");
    assert!(b.status.success(), "{b:?}");
    let starting = failed(&c);
    assert!(
        starting.contains("No more browsers: browser.max is 1"),
        "{starting}"
    );
    let endpoint = format!("http://127.0.0.1:{port}");
    let expected = format!("@playwright/mcp@latest\n--cdp-endpoint\n{endpoint}\n");
    assert_eq!(std::fs::read_to_string(&ran).unwrap(), expected);
    let line = std::fs::read_to_string(&args).unwrap();
    assert!(
        line.contains("\n--headless=new\n--remote-debugging-port=0\n"),
        "{line}"
    );
    let first_profile = profile(&args);
    // A new profile each time, in the runtime folder.
    assert!(
        first_profile.ends_with("run/hive/browsers/1"),
        "{first_profile:?}"
    );
    assert_eq!(browser(&mut app).await, status(&root, true, None, None));
    let shown = status(&root, true, Some("http://localhost:5173/"), None);
    assert_eq!(browser(&mut app).await, shown);
    let frame = Control::BrowserFrame {
        worktree: root.clone(),
        data: "AAAA".into(),
    };
    assert_eq!(browser(&mut app).await, frame);
    app.send(0, Control::BrowserDrawn).await;
    app.send(0, watch(None)).await;
    tokio::time::timeout(TIMEOUT, unwatched)
        .await
        .unwrap()
        .unwrap();
    app.send(0, watch(Some(&root))).await;
    assert_eq!(browser(&mut app).await, shown);

    // Without bunx or npx the MCP cannot run.
    let none = failed(&mcp(&repo, &repo.root, &empty).await);
    assert!(none.contains("neither bunx nor npx is on PATH"), "{none}");
    // Another worktree's ask: one browser at most.
    let most = failed(&mcp(&repo, &fix_a, &bin).await);
    assert!(
        most.contains("No more browsers: browser.max is 1"),
        "{most}"
    );

    // Its agent is detected after its MCP asked; the worktree's last agent ends: it stops.
    app.open_terminal(1, &repo.root).await;
    let start = json!({"session_id": "s1", "cwd": root, "source": "startup"});
    hook(&repo, &mut app, "1", "SessionStart", start).await;
    let end = json!({"session_id": "s1", "reason": "prompt_input_exit"});
    let seen = hook(&repo, &mut app, "1", "SessionEnd", end).await;
    assert!(
        seen.contains(&(0, status(&root, false, None, None))),
        "{seen:?}"
    );
    cast.await.unwrap();
    ended(&pid);
    wait_until(|| !first_profile.exists());

    // A browser that cannot start says why in the panel.
    let mut missing = on(Path::new("/nonexistent/browser"));
    set(&mut app, missing.clone()).await;
    let why = failed(&mcp(&repo, &repo.root, &bin).await);
    let not = "browser.path /nonexistent/browser is not a program Hive can run";
    assert!(why.contains(not), "{why}");
    assert_eq!(
        browser(&mut app).await,
        status(&root, false, None, Some(not))
    );

    // Unwatched, a browser whose connection closes is stopped.
    missing.browser.path = Some(fake.display().to_string());
    set(&mut app, missing).await;
    app.send(0, watch(None)).await;
    // Nothing is watched: a drawn frame is none's.
    app.send(0, Control::BrowserDrawn).await;
    let closes = tokio::spawn({
        let listener = listener.clone();
        async move {
            let mut ws = cdp(&listener).await;
            assert_eq!(read(&mut ws).await["method"], "Target.setDiscoverTargets");
            // Followed, though not shown.
            let page = json!({ "method": "Target.targetCreated", "params": { "targetInfo": {
                "targetId": "p1", "type": "page", "url": "about:blank" } } });
            write(&mut ws, page).await;
        }
    });
    assert!(mcp(&repo, &repo.root, &bin).await.status.success());
    closes.await.unwrap();
    ended(&pid);
    // The third browser asked for (the second failed to start).
    assert!(profile(&args).ends_with("browsers/3"));
    wait_until(|| !profile(&args).exists());

    // Turned off with one running and watched: it stops, and the wrapper passes no MCP server.
    let held = tokio::spawn({
        let listener = listener.clone();
        async move {
            let mut ws = cdp(&listener).await;
            closed(&mut ws).await;
        }
    });
    assert!(mcp(&repo, &repo.root, &bin).await.status.success());
    app.send(0, watch(Some(&root))).await;
    loop {
        if let Control::BrowserStatus { running: true, .. } = browser(&mut app).await {
            break;
        }
    }
    set(&mut app, Settings::default()).await;
    loop {
        if let Control::BrowserStatus { running: false, .. } = browser(&mut app).await {
            break;
        }
    }
    held.await.unwrap();
    ended(&pid);
    wait_until(|| !config.exists());
    drop(app);
    assert!(daemon.wait_exit().success());
}
