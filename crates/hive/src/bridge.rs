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
/// How much each side reads at a time: a burst of terminal output crosses in a few copies, not
/// tokio's default 8 KiB ones (D19.7).
const BUFFER: usize = 64 * 1024;

/// How long `--replace` waits for the running service to let go of its lock.
const REPLACE_WAIT: Duration = Duration::from_secs(15);

/// Connects stdio to the service; one it starts is a remote one (`hive daemon --remote`, 26.14)
/// when `remote`. With `replace` (26.17), the running service is ended first, so this bridge
/// starts a new one: the app chose to restart a remote service of another version.
pub async fn run(paths: &Paths, hive: &Path, remote: bool, replace: bool) -> io::Result<()> {
    if replace {
        end_running(paths, REPLACE_WAIT).await?;
    }
    // Only to our own service (`Paths::connect`). A missing or insecure runtime directory goes
    // to `start_daemon`, whose `prepare_runtime` creates it or refuses it with a clear error.
    let stream = match paths.connect().await {
        Ok(stream) => stream,
        Err(_) => {
            start_daemon(paths, hive, remote)?;
            connect_when_ready(paths).await?
        }
    };
    let (mut from_daemon, mut to_daemon) = tokio::io::split(stream);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    // The handshake and everything after it pass through unchanged.
    // Either side closing (app gone, or service refused the handshake) ends the bridge.
    tokio::select! {
        result = pipe(&mut stdin, &mut to_daemon) => result.map(drop),
        result = pipe(&mut from_daemon, &mut stdout) => result.map(drop),
    }
}

/// Ends the service holding the lock, if any (`--replace`): SIGTERM to the process its lockfile
/// names, then waits up to `wait` for the lock to be free. The service lets go of its socket and
/// lock first and then ends its terminals.
#[cfg(unix)]
async fn end_running(paths: &Paths, wait: Duration) -> io::Result<()> {
    use std::fs::TryLockError;
    use std::io::Read as _;
    let file = match File::open(paths.lock()) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        file => file?,
    };
    match file.try_lock() {
        Err(TryLockError::WouldBlock) => {}
        free => return free.map_err(io::Error::from),
    }
    let mut text = String::new();
    // A process id is at most 10 digits; more is not one.
    (&file).take(16).read_to_string(&mut text)?;
    let pid = text.trim().parse::<i32>().ok().filter(|pid| *pid > 1);
    let pid = pid.ok_or_else(|| io::Error::other("the running hive service cannot be found"))?;
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::SIGTERM)
        .map_err(io::Error::from)?;
    let free = async {
        loop {
            match file.try_lock() {
                Err(TryLockError::WouldBlock) => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                free => return free,
            }
        }
    };
    tokio::time::timeout(wait, free)
        .await
        .map_err(|_| io::Error::other("the running hive service did not end"))?
        .map_err(io::Error::from)
}

/// Remote services run only on Linux (Q5): there is nothing to replace here.
#[cfg(windows)]
async fn end_running(_: &Paths, _: Duration) -> io::Result<()> {
    Err(io::Error::other(
        "hive bridge --replace runs only on Linux and macOS",
    ))
}

/// Copies `from` into `to` until `from` ends, [`BUFFER`] bytes at a time at most.
async fn pipe<R, W>(from: R, to: &mut W) -> io::Result<u64>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + ?Sized,
{
    tokio::io::copy_buf(&mut tokio::io::BufReader::with_capacity(BUFFER, from), to).await
}

/// Starts `hive daemon` in its own session so it outlives nothing but the app connection.
/// Two bridges racing here is fine: the lockfile lets only one daemon run.
fn start_daemon(paths: &Paths, hive: &Path, remote: bool) -> io::Result<()> {
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
        .args(remote.then_some("--remote"))
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

    /// A reader of `data` that records the largest read it was offered.
    struct Recorder<'a> {
        data: &'a [u8],
        largest: usize,
    }

    impl tokio::io::AsyncRead for Recorder<'_> {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            self.largest = self.largest.max(buf.remaining());
            let n = buf.remaining().min(self.data.len());
            buf.put_slice(&self.data[..n]);
            self.data = &self.data[n..];
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn the_bridge_copies_everything_64_kib_at_a_time() {
        let data: Vec<u8> = (0..300_000_u32).map(|i| (i % 251) as u8).collect();
        let mut from = Recorder {
            data: &data,
            largest: 0,
        };
        let mut to = Vec::new();
        assert_eq!(pipe(&mut from, &mut to).await.unwrap(), 300_000);
        assert_eq!(to, data);
        assert_eq!(from.largest, 64 * 1024);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_ends_only_a_service_the_lockfile_names() {
        use std::os::unix::process::ExitStatusExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let paths = Paths {
            logs: dir.join("run"),
            runtime: dir.join("run"),
            data: dir.join("data"),
            config: dir.join("config"),
        };
        let wait = Duration::from_millis(300);
        // No lockfile, or a lockfile nobody holds: nothing runs, nothing to end.
        end_running(&paths, wait).await.unwrap();
        std::fs::create_dir_all(&paths.runtime).unwrap();
        std::fs::write(paths.lock(), "1").unwrap();
        end_running(&paths, wait).await.unwrap();
        // A holder whose lockfile names no process (a service before 26.17).
        let held = File::open(paths.lock()).unwrap();
        held.try_lock().unwrap();
        for text in ["", "1", "0", "-5", "12345678901234567890", "x"] {
            std::fs::write(paths.lock(), text).unwrap();
            let err = end_running(&paths, wait).await.unwrap_err();
            assert_eq!(
                err.to_string(),
                "the running hive service cannot be found",
                "{text}"
            );
        }
        // The named process gets SIGTERM; one that keeps the lock is waited for, then given up.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::fs::write(paths.lock(), format!(" {}\n", child.id())).unwrap();
        let err = end_running(&paths, wait).await.unwrap_err();
        assert_eq!(err.to_string(), "the running hive service did not end");
        assert_eq!(child.wait().unwrap().signal(), Some(15));
        // Once the lock is let go the wait ends.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::fs::write(paths.lock(), child.id().to_string()).unwrap();
        let release = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(held);
        };
        let (ended, ()) = tokio::join!(end_running(&paths, Duration::from_secs(5)), release);
        ended.unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(15));
    }

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
