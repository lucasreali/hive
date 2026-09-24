//! `hive bridge`: connects the app (on stdio, through `wsl.exe` on Windows) to the service socket,
//! starting the service first when it is not running.

use std::fs::File;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::net::UnixStream;

use crate::paths::Paths;

/// How long to wait for a freshly started service to accept connections.
const START_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run(paths: &Paths, hive: &Path) -> io::Result<()> {
    let stream = match UnixStream::connect(paths.socket()).await {
        Ok(stream) => stream,
        Err(_) => {
            start_daemon(paths, hive)?;
            connect_when_ready(paths).await?
        }
    };
    let (mut from_daemon, mut to_daemon) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    // The handshake and everything after it pass through unchanged.
    // Either side closing (app gone, or service refused the handshake) ends the bridge.
    tokio::select! {
        result = tokio::io::copy(&mut stdin, &mut to_daemon) => result.map(drop),
        result = tokio::io::copy(&mut from_daemon, &mut stdout) => result.map(drop),
    }
}

/// Starts `hive daemon` in its own session so it outlives nothing but the app connection.
/// Two bridges racing here is fine: the lockfile lets only one daemon run.
fn start_daemon(paths: &Paths, hive: &Path) -> io::Result<()> {
    paths.prepare_runtime()?;
    let log = File::options()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(paths.daemon_log())?;
    let mut daemon = Command::new(hive);
    daemon
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    // SAFETY: between fork and exec the child only calls setsid(2), which is
    // async-signal-safe and touches no memory of the parent.
    unsafe {
        daemon.pre_exec(|| Ok(nix::unistd::setsid().map(drop)?));
    }
    // Not waited for: a daemon that fails to start is caught by the connection timeout,
    // with its error in the log.
    daemon.spawn()?;
    Ok(())
}

async fn connect_when_ready(paths: &Paths) -> io::Result<UnixStream> {
    let socket = paths.socket();
    let connect = async {
        loop {
            if let Ok(stream) = UnixStream::connect(&socket).await {
                return stream;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(START_TIMEOUT, connect)
        .await
        .map_err(|_| {
            io::Error::other(format!(
                "the hive service did not start; see {}",
                paths.daemon_log().display()
            ))
        })
}
