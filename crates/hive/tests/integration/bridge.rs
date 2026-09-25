use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, PROTOCOL_VERSION, Role};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::common::{DISTRO, Env, TIMEOUT, wait_until};

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
    assert!(
        std::fs::read_to_string(log)
            .unwrap()
            .contains("is another hive daemon running?")
    );
}

#[tokio::test]
async fn bridge_refuses_an_insecure_runtime_directory() {
    let env = Env::new();
    std::fs::create_dir(env.path("run/hive")).unwrap();
    std::fs::set_permissions(env.path("run/hive"), std::fs::Permissions::from_mode(0o777)).unwrap();
    let out = env
        .hive()
        .arg("bridge")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("insecure runtime directory"));
}
