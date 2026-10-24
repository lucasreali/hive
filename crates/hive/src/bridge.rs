//! `hive bridge`: connects the app (on stdio, through `wsl.exe` on Windows) to the service socket
//! (its named pipe on native Windows), starting the service first when it is not running.

use std::fs::File;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::paths::Paths;

/// How long to wait for a freshly started service to accept connections.
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run(paths: &Paths, hive: &Path) -> io::Result<()> {
    // Only to our own service (`Paths::connect`). A missing or insecure runtime directory goes
    // to `start_daemon`, whose `prepare_runtime` creates it or refuses it with a clear error.
    let stream = match paths.connect().await {
        Ok(stream) => stream,
        Err(_) => {
            start_daemon(paths, hive)?;
            connect_when_ready(paths).await?
        }
    };
    let (mut from_daemon, mut to_daemon) = tokio::io::split(stream);
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
    crate::mode::folder(std::fs::DirBuilder::new().recursive(true), 0o700).create(&paths.logs)?;
    let log = open_log(&paths.daemon_log())?;
    // The service runs a copy in the bin folder, so that it never keeps the app's installer
    // from replacing the installed `hive.exe`.
    #[cfg(windows)]
    let hive = &crate::windows::install_hive(hive, &paths.bin_dir())?;
    let mut daemon = Command::new(hive);
    daemon
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    // On Unix the daemon leaves this session itself. Not waited for: a daemon that fails to
    // start is caught by the connection timeout, with its error in the log.
    #[cfg(unix)]
    daemon.spawn()?;
    #[cfg(windows)]
    crate::windows::spawn_detached(&mut daemon)?;
    Ok(())
}

/// Opens `path` for a new run, the previous run's log kept as `<path>.1` (D18.6) and any
/// older one dropped. Appends: should the rename fail, the previous log is kept in place.
fn open_log(path: &Path) -> io::Result<File> {
    let mut previous = path.as_os_str().to_owned();
    previous.push(".1");
    // A missing log (first start) or a failed rename only means nothing is rotated.
    let _ = std::fs::rename(path, previous);
    crate::mode::private(File::options().create(true).append(true)).open(path)
}

async fn connect_when_ready(paths: &Paths) -> io::Result<crate::paths::Stream> {
    let connect = async {
        loop {
            if let Ok(stream) = paths.connect().await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn each_start_keeps_the_previous_log() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("daemon.log");
        let previous = tmp.path().join("daemon.log.1");
        let read = |path: &Path| std::fs::read_to_string(path).unwrap();
        for run in ["first", "second", "third"] {
            write!(open_log(&log).unwrap(), "{run}").unwrap();
        }
        assert_eq!(read(&log), "third");
        assert_eq!(read(&previous), "second");
        #[cfg(unix)]
        assert_eq!(
            crate::mode::of(&std::fs::metadata(&log).unwrap()) & 0o777,
            0o600
        );
        // A rename that fails (`daemon.log.1` is a folder with something in it) loses nothing.
        std::fs::remove_file(&previous).unwrap();
        std::fs::create_dir_all(previous.join("x")).unwrap();
        write!(open_log(&log).unwrap(), "+fourth").unwrap();
        assert_eq!(read(&log), "third+fourth");
    }
}
