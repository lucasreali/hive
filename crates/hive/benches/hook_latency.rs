//! Hook latency: time from starting `hive hook` until its event reaches the app
//! connection (the service forwards it right after receiving it). Target: p99 < 20 ms.
//! Run with `cargo bench -p hive --bench hook_latency`; exits non-zero above the target.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, Role};
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, FramedWrite};

const RUNS: usize = 300;
const TARGET_P99: Duration = Duration::from_millis(20);

fn hive(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hive"));
    for (var, sub) in [
        ("HOME", "home"),
        ("XDG_RUNTIME_DIR", "run"),
        ("XDG_DATA_HOME", "data"),
    ] {
        cmd.env(var, dir.join(sub));
    }
    cmd
}

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    for sub in ["home", "run", "data"] {
        std::fs::create_dir(dir.path().join(sub))?;
    }
    let mut daemon = hive(dir.path()).arg("daemon").spawn()?;
    let socket = dir.path().join("run/hive/hive.sock");
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(&socket).await {
            break stream;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let (read, write) = stream.into_split();
    let mut from_daemon = FramedRead::new(read, FrameCodec);
    let mut to_daemon = FramedWrite::new(write, FrameCodec);
    to_daemon
        .send(Frame::control(0, &Control::hello(Role::App, hive::VERSION)))
        .await?;
    from_daemon.next().await.ok_or("no welcome")??;

    let payload =
        br#"{"session_id":"bench","cwd":"/tmp","hook_event_name":"PreToolUse","tool_name":"Bash"}"#;
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let start = Instant::now();
        let mut hook = hive(dir.path())
            .args(["hook", "PreToolUse"])
            .stdin(Stdio::piped())
            .spawn()?;
        hook.stdin.take().ok_or("no stdin")?.write_all(payload)?;
        loop {
            let frame = from_daemon.next().await.ok_or("service closed")??;
            if matches!(frame.to_control(), Ok(Control::Agent(_))) {
                break;
            }
        }
        samples.push(start.elapsed());
        hook.wait()?;
    }
    drop((from_daemon, to_daemon));
    daemon.wait()?;

    samples.sort();
    let at = |percent: usize| samples[(samples.len() * percent / 100).min(samples.len() - 1)];
    let (p50, p99, max) = (at(50), at(99), at(100));
    println!(
        "hook latency over {RUNS} runs: p50 {p50:?}, p99 {p99:?}, max {max:?} (target p99 < {TARGET_P99:?})"
    );
    Ok(if p99 < TARGET_P99 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
