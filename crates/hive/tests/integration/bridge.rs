use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, PROTOCOL_VERSION, Role};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::common::{DISTRO, Env, TIMEOUT, wait_until};
use crate::terminal::printed_pid;

struct Bridge {
    child: Child,
    to_bridge: FramedWrite<ChildStdin, FrameCodec>,
    from_bridge: FramedRead<ChildStdout, FrameCodec>,
}

fn bridge(env: &Env) -> Bridge {
    let mut cmd = tokio::process::Command::from(env.hive());
    let mut child = cmd
        .arg("bridge")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let to_bridge = FramedWrite::new(child.stdin.take().unwrap(), FrameCodec);
    let from_bridge = FramedRead::new(child.stdout.take().unwrap(), FrameCodec);
    Bridge {
        child,
        to_bridge,
        from_bridge,
    }
}

impl Bridge {
    async fn send(&mut self, message: Control) {
        self.to_bridge
            .send(Frame::control(0, &message))
            .await
            .unwrap();
    }

    async fn next(&mut self) -> Option<Control> {
        let frame = tokio::time::timeout(TIMEOUT, self.from_bridge.next())
            .await
            .unwrap();
        frame.map(|frame| frame.unwrap().to_control().unwrap())
    }

    /// Closes stdin, like the app going away, and waits for the bridge to exit.
    async fn close(mut self) -> std::process::ExitStatus {
        drop(self.to_bridge);
        tokio::time::timeout(TIMEOUT, self.child.wait())
            .await
            .unwrap()
            .unwrap()
    }
}

fn welcome() -> Control {
    Control::Welcome {
        version: hive::VERSION.into(),
        distro: Some(DISTRO.into()),
        windows: false,
    }
}

#[tokio::test]
async fn bridge_starts_the_service_and_forwards_frames() {
    let env = Env::new();
    let mut bridge = bridge(&env);
    bridge.send(Control::hello(Role::App, hive::VERSION)).await;
    assert_eq!(bridge.next().await, Some(welcome()));
    // The service leads its own session, apart from the bridge's.
    let bridge_pid = bridge.child.id().unwrap() as i32;
    let processes = env.processes();
    let daemon = processes.iter().find(|p| p.pid != bridge_pid).unwrap();
    assert_eq!(daemon.session, daemon.pid, "{processes:?}");
    let log = env.path("run/hive/daemon.log");
    assert_eq!(
        std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(bridge.close().await.success());
    // The app connection ended, so the service it started ends too.
    wait_until(|| !env.socket().exists());
    assert_eq!(std::fs::read_to_string(log).unwrap(), "");
}

#[tokio::test]
async fn bridge_reuses_a_running_service() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut bridge = bridge(&env);
    bridge.send(Control::hello(Role::App, hive::VERSION)).await;
    assert_eq!(bridge.next().await, Some(welcome()));
    assert!(!env.path("run/hive/daemon.log").exists());
    assert!(bridge.close().await.success());
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn bridge_forwards_a_version_mismatch_and_exits() {
    let env = Env::new();
    let mut bridge = bridge(&env);
    bridge.send(Control::hello(Role::App, "0.0.0-other")).await;
    let refused = Control::VersionMismatch {
        protocol: PROTOCOL_VERSION,
        version: hive::VERSION.into(),
    };
    assert_eq!(bridge.next().await, Some(refused));
    // The service closed the connection: the bridge ends on its own.
    assert_eq!(bridge.next().await, None);
    let status = tokio::time::timeout(TIMEOUT, bridge.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    // The refused client was not the app; end the service it started.
    drop(env.connect(Role::App).await);
    wait_until(|| !env.socket().exists());
}

#[tokio::test]
async fn bridge_reports_a_service_that_does_not_start() {
    let env = Env::new();
    std::fs::DirBuilder::new()
        .recursive(true)
        .create(env.path("run/hive"))
        .unwrap();
    std::fs::set_permissions(env.path("run/hive"), std::fs::Permissions::from_mode(0o700)).unwrap();
    // Someone holds the lock but serves no socket: the new service cannot start.
    let lock = std::fs::File::create(env.path("run/hive/hive.lock")).unwrap();
    lock.try_lock().unwrap();
    let mut cmd = tokio::process::Command::from(env.hive());
    let run = cmd.arg("bridge").stdin(Stdio::null()).output();
    let out = tokio::time::timeout(TIMEOUT, run).await.unwrap().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let log = env.path("run/hive/daemon.log");
    assert!(!out.status.success());
    assert_eq!(
        stderr,
        format!(
            "hive: the hive service did not start; see {}\n",
            log.display()
        )
    );
    // The service waits for the lock as long as the bridge waits for it, so it may give up
    // just after the bridge.
    wait_until(|| {
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("is another hive daemon running?")
    });
}

#[tokio::test]
async fn a_bridge_started_while_the_service_ends_gets_a_new_service() {
    let env = Env::new();
    let mut old = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    // Ignores SIGHUP: the old service waits out its whole grace period to end it.
    app.input(1, "sh -c 'trap \"\" HUP; echo pid=$$; exec sleep 30'\r")
        .await;
    printed_pid(&mut app, 1).await;
    drop(app);
    // The socket goes as soon as the app does, while the terminal is still being ended.
    wait_until(|| !env.socket().exists());
    assert!(old.0.try_wait().unwrap().is_none());
    let mut bridge = bridge(&env);
    bridge.send(Control::hello(Role::App, hive::VERSION)).await;
    assert_eq!(bridge.next().await, Some(welcome()));
    assert!(old.wait_exit().success());
    assert!(bridge.close().await.success());
    wait_until(|| !env.socket().exists());
}

#[tokio::test]
async fn bridge_refuses_an_insecure_runtime_directory() {
    let env = Env::new();
    std::fs::create_dir(env.path("run/hive")).unwrap();
    std::fs::set_permissions(env.path("run/hive"), std::fs::Permissions::from_mode(0o777)).unwrap();
    // Another user's socket, planted before Hive starts: the bridge never connects to it.
    let planted = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
    planted.set_nonblocking(true).unwrap();
    let out = env
        .hive()
        .arg("bridge")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("insecure runtime directory"));
    let accepted = planted.accept().map(drop).map_err(|err| err.kind());
    assert_eq!(accepted, Err(std::io::ErrorKind::WouldBlock));
}
