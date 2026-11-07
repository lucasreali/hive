use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Settings};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use crate::agents::hook;
use crate::common::{Conn, wait_until};
use crate::worktree::Repo;

fn exe(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The next browser message the app gets, others skipped.
async fn browser(app: &mut Conn) -> Control {
    loop {
        match app.control().await {
            (0, m @ (Control::BrowserStatus { .. } | Control::BrowserFrame { .. })) => return m,
            _ => {}
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

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_browser_starts_on_its_mcps_ask_streams_while_watched_and_stops_with_it() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "fix-a"]).status.success());
    let root = repo.root.display().to_string();
    let fix_a = repo.root.join(".claude/worktrees/fix-a");
    // The CDP side of a fake browser: the test plays the browser.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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
    assert!(
        std::process::Command::new(&fake)
            .status()
            .unwrap()
            .success()
    );
    // The MCP's runner: records its arguments.
    let bin = repo.env.path("mcp-bin");
    std::fs::create_dir(&bin).unwrap();
    let ran = repo.env.path("bunx.args");
    exe(
        &bin.join("bunx"),
        &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", ran.display()),
    );

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
    let out = mcp(&repo, &repo.root, &bin).await;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("the agent's browser is off"), "{stderr}");

    let mut settings = Settings::default();
    settings.browser.enabled = true;
    settings.browser.path = Some(fake.display().to_string());
    settings.browser.max = 1;
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    app.send(0, set).await;
    assert_eq!(app.control().await, (0, Control::Settings { settings }));
    wait_until(|| config.exists());
    let written: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    let server = &written["mcpServers"]["hive-browser"];
    assert_eq!(server["command"], env!("CARGO_BIN_EXE_hive"));
    assert_eq!(server["args"], json!(["browser-mcp"]));

    app.open_terminal(1, &repo.root).await;
    let start = json!({"session_id": "s1", "cwd": root, "source": "startup"});
    hook(&repo, &mut app, "1", "SessionStart", start).await;
    // The panel shows the main worktree before its browser runs.
    let watch = Control::WatchBrowser {
        worktree: Some(root.clone()),
    };
    app.send(0, watch).await;
    assert_eq!(browser(&mut app).await, status(&root, false, None, None));

    let cdp = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let read = async |ws: &mut tokio_tungstenite::WebSocketStream<_>| -> Value {
            let message = ws.next().await.unwrap().unwrap();
            serde_json::from_str(message.to_text().unwrap()).unwrap()
        };
        assert_eq!(read(&mut ws).await["method"], "Target.setDiscoverTargets");
        let page = json!({ "method": "Target.targetCreated", "params": { "targetInfo": {
            "targetId": "p1", "type": "page", "url": "http://localhost:5173/" } } });
        ws.send(Message::text(page.to_string())).await.unwrap();
        let attach = read(&mut ws).await;
        assert_eq!(attach["method"], "Target.attachToTarget");
        let attached = json!({ "id": attach["id"], "result": { "sessionId": "s" } });
        ws.send(Message::text(attached.to_string())).await.unwrap();
        assert_eq!(read(&mut ws).await["method"], "Page.startScreencast");
        let frame = json!({ "method": "Page.screencastFrame", "sessionId": "s",
            "params": { "data": "AAAA", "sessionId": 1, "metadata": {} } });
        ws.send(Message::text(frame.to_string())).await.unwrap();
        let ack = read(&mut ws).await;
        assert_eq!(ack["method"], "Page.screencastFrameAck");
        assert_eq!(ack["params"], json!({ "sessionId": 1 }));
        // Until the browser is stopped.
        while let Some(Ok(_)) = ws.next().await {}
    });

    let out = mcp(&repo, &repo.root.join(".claude"), &bin).await;
    assert!(out.status.success(), "{out:?}");
    let endpoint = format!("http://127.0.0.1:{port}");
    let expected = format!("@playwright/mcp@latest\n--cdp-endpoint\n{endpoint}\n");
    assert_eq!(std::fs::read_to_string(&ran).unwrap(), expected);
    let line = std::fs::read_to_string(&args).unwrap();
    let profile = line
        .lines()
        .next()
        .unwrap()
        .strip_prefix("--user-data-dir=");
    let profile = Path::new(profile.unwrap()).to_owned();
    assert!(
        line.contains("\n--headless=new\n--remote-debugging-port=0\n"),
        "{line}"
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

    // Another worktree's ask: one browser at most.
    let out = mcp(&repo, &fix_a, &bin).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No more browsers: browser.max is 1"),
        "{stderr}"
    );
    // The same worktree's again: the running one.
    assert!(mcp(&repo, &repo.root, &bin).await.status.success());

    // Its worktree's last agent ends: the browser stops, its profile goes.
    let end = json!({"session_id": "s1", "reason": "prompt_input_exit"});
    let seen = hook(&repo, &mut app, "1", "SessionEnd", end).await;
    assert!(
        seen.contains(&(0, status(&root, false, None, None))),
        "{seen:?}"
    );
    cdp.await.unwrap();
    let pid: i32 = std::fs::read_to_string(&pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let gone = || nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err();
    wait_until(gone);
    wait_until(|| !profile.exists());

    // Turned off: the wrapper passes no MCP server.
    app.send(
        0,
        Control::SetSettings {
            settings: Settings::default(),
        },
    )
    .await;
    wait_until(|| !config.exists());
    app.send(0, Control::WatchBrowser { worktree: None }).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}
