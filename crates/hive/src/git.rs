//! Git always runs as the `git` executable with separate arguments, output size-limited.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

/// Most bytes [`output`] reads, e.g. from `git status` or `git diff`.
const OUTPUT_LIMIT: u64 = 16_777_216; // 16 MiB
/// Most bytes read from one git command's stderr.
const STDERR_LIMIT: u64 = 67_108_864; // 64 MiB

/// `git -C <dir>`, ready for its arguments.
pub fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        // Hive always names the repository with `-C`.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        // Reads must not rewrite the index: the files watcher would see it as a change.
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

/// Runs `git -C <dir> <args>` with `input` on stdin; any exit code outside `ok` is an error
/// carrying git's stderr, and so is more than `limit` bytes on stdout (git then stops on a
/// broken pipe).
pub fn run(
    dir: &Path,
    args: &[&OsStr],
    input: &[u8],
    ok: &[i32],
    limit: u64,
) -> io::Result<Vec<u8>> {
    run_within(dir, args, input, ok, limit, None)
}

/// [`run`]; with a `time` limit, git and everything it started (hooks, filters) are killed
/// once it has passed, which is an error of kind `TimedOut`.
fn run_within(
    dir: &Path,
    args: &[&OsStr],
    input: &[u8],
    ok: &[i32],
    limit: u64,
    time: Option<Duration>,
) -> io::Result<Vec<u8>> {
    let mut command = command(dir);
    if time.is_some() {
        // Its own process group, so the limit ends git's children too.
        command.process_group(0);
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run git: {err}")))?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let group = Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX));
    let (finished, done) = mpsc::channel::<()>();
    // Feed stdin and drain stderr from other threads, so no pipe can deadlock another.
    let (out, err, expired) = std::thread::scope(|scope| {
        scope.spawn(move || stdin.map(|mut stdin| stdin.write_all(input)));
        let err = scope.spawn(move || {
            let mut buf = Vec::new();
            stderr.map(|stderr| stderr.take(STDERR_LIMIT).read_to_end(&mut buf));
            buf
        });
        // Git is reaped only after this thread ends, so its pid still names its group.
        let watchdog = time.map(|time| {
            scope.spawn(move || {
                let expired = done.recv_timeout(time) == Err(RecvTimeoutError::Timeout);
                if expired {
                    let _ = killpg(group, Signal::SIGKILL);
                }
                expired
            })
        });
        let out = stdout.map_or(Ok(Vec::new()), |mut out| read_limited(&mut out, limit));
        let err = err.join().unwrap_or_default();
        drop(finished);
        let expired = watchdog.is_some_and(|w| w.join().unwrap_or_default());
        (out, err, expired)
    });
    let status = child.wait()?;
    let command: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
    let command = command.join(" ");
    if let (true, Some(time)) = (expired, time) {
        let message = format!("git {command} took longer than {} s", time.as_secs_f32());
        return Err(io::Error::new(io::ErrorKind::TimedOut, message));
    }
    let out = out
        .map_err(|_| io::Error::other(format!("git {command} printed more than {limit} bytes")))?;
    if status.code().is_some_and(|code| ok.contains(&code)) {
        return Ok(out);
    }
    Err(io::Error::other(format!(
        "git {command} failed: {}",
        String::from_utf8_lossy(&err).trim()
    )))
}

/// `git <args>` in `dir` with `ok` exit codes and no input; at most [`OUTPUT_LIMIT`] bytes.
pub fn output(dir: &Path, args: &[&str], ok: &[i32]) -> io::Result<Vec<u8>> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run(dir, &args, &[], ok, OUTPUT_LIMIT)
}

/// [`output`] within a `time` limit (see [`run_within`]).
pub fn output_within(dir: &Path, args: &[&str], ok: &[i32], time: Duration) -> io::Result<Vec<u8>> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run_within(dir, &args, &[], ok, OUTPUT_LIMIT, Some(time))
}

/// Reads at most `limit` bytes; more is an error.
pub fn read_limited(input: &mut dyn Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    input.take(limit + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(io::Error::other(format!("input larger than {limit} bytes")));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_size_limited() {
        assert_eq!(read_limited(&mut &b"abcd"[..], 4).unwrap(), b"abcd");
        let err = read_limited(&mut &b"abcde"[..], 4).unwrap_err();
        assert_eq!(err.to_string(), "input larger than 4 bytes");
    }

    #[test]
    fn git_and_its_children_are_killed_after_the_time_limit() {
        let tmp = tempfile::tempdir().unwrap();
        // An alias runs through the shell, a child of git that holds its output open.
        let nap = ["-c", "alias.nap=!sleep 30", "nap"];
        let started = std::time::Instant::now();
        let err = output_within(tmp.path(), &nap, &[0], Duration::from_millis(300)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(20), "not killed");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            err.to_string(),
            "git -c alias.nap=!sleep 30 nap took longer than 0.3 s"
        );
        // Within the limit it is an ordinary run.
        let version = output_within(tmp.path(), &["version"], &[0], Duration::from_secs(30));
        assert_eq!(
            version.unwrap(),
            output(tmp.path(), &["version"], &[0]).unwrap()
        );
        let failed = output_within(tmp.path(), &["nope"], &[0], Duration::from_secs(30));
        assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::Other);
    }

    #[test]
    fn git_output_is_size_limited() {
        let tmp = tempfile::tempdir().unwrap();
        let args = [OsStr::new("version")];
        let out = run(tmp.path(), &args, &[], &[0], STDERR_LIMIT).unwrap();
        let len = out.len() as u64;
        assert_eq!(run(tmp.path(), &args, &[], &[0], len).unwrap(), out);
        let err = run(tmp.path(), &args, &[], &[0], len - 1).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("git version printed more than {} bytes", len - 1)
        );
    }
}
