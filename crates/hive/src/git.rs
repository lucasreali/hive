//! Git always runs as the `git` executable with separate arguments, output size-limited.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;

/// Most bytes [`output`] reads, e.g. from `git status` or `git diff`.
const OUTPUT_LIMIT: u64 = 16_777_216; // 16 MiB
/// Most bytes read from one git command's stderr.
const STDERR_LIMIT: u64 = 67_108_864; // 64 MiB
/// How much [`read_tail`] reads at a time.
const TAIL_BLOCK: u64 = 65_536;
/// The longest a git command that only reads may take (e.g. on a hung network drive).
pub const TIME_LIMIT: Duration = Duration::from_secs(10);

/// `git -C <dir>`, ready for its arguments.
pub fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        // A repository's own `.git/config` (from an archive or a shared folder) must not make
        // Hive's `git status` run its fsmonitor, nor `git log` its `gpg.program`. (Its filter
        // drivers still run: their names are arbitrary, so no `-c` turns them off.)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
        ])
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
    command.args(args);
    limited(command, "git", args, input, ok, Stdout::Max(limit), time)
}

/// How much of a command's stdout [`limited`] keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdout {
    /// All of it, at most this many bytes: more is an error.
    Max(u64),
    /// Its last bytes only, however much it prints (a log's tail).
    Tail(u64),
}

/// Runs `command`, already given its `args` (`program` names it in errors), as
/// [`run_within`] runs git: `input` on stdin, its stdout kept as `keep` says, errors carrying
/// its stderr only (never its stdout), and everything it started killed after `time`. A
/// program that cannot start is an error of that kind (`NotFound` when it is not installed).
pub fn limited(
    mut command: Command,
    program: &str,
    args: &[&OsStr],
    input: &[u8],
    ok: &[i32],
    keep: Stdout,
    time: Option<Duration>,
) -> io::Result<Vec<u8>> {
    if time.is_some() {
        // Its own process group, so the limit ends its children too.
        #[cfg(unix)]
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run {program}: {err}")))?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let group = child.id();
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
                    kill_group(group);
                }
                expired
            })
        });
        let out = stdout.map_or(Ok(Vec::new()), |mut out| match keep {
            Stdout::Max(limit) => read_limited(&mut out, limit),
            Stdout::Tail(limit) => read_tail(&mut out, limit),
        });
        let err = err.join().unwrap_or_default();
        drop(finished);
        let expired = watchdog.is_some_and(|w| w.join().unwrap_or_default());
        (out, err, expired)
    });
    let status = child.wait()?;
    let command: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
    let command = command.join(" ");
    if let (true, Some(time)) = (expired, time) {
        let message = format!(
            "{program} {command} took longer than {} s",
            time.as_secs_f32()
        );
        return Err(io::Error::new(io::ErrorKind::TimedOut, message));
    }
    let (Stdout::Max(limit) | Stdout::Tail(limit)) = keep;
    let out = out.map_err(|_| {
        io::Error::other(format!(
            "{program} {command} printed more than {limit} bytes"
        ))
    })?;
    if status.code().is_some_and(|code| ok.contains(&code)) {
        return Ok(out);
    }
    Err(io::Error::other(format!(
        "{program} {command} failed: {}",
        String::from_utf8_lossy(&err).trim()
    )))
}

/// Kills the process group `leader` leads, everything in it.
#[cfg(unix)]
pub fn kill_group(leader: u32) {
    let group = Pid::from_raw(i32::try_from(leader).unwrap_or(i32::MAX));
    let _ = killpg(group, Signal::SIGKILL);
}

/// Windows has no process groups: `leader` and what it started.
#[cfg(windows)]
pub use crate::windows::kill_tree as kill_group;

/// What a program printed (a path, a `PATH`) as an OS string: its bytes on Unix; on Windows,
/// where programs print UTF-8, its text.
#[cfg(unix)]
pub fn os_string(bytes: &[u8]) -> OsString {
    std::os::unix::ffi::OsStringExt::from_vec(bytes.to_vec())
}

#[cfg(windows)]
pub fn os_string(bytes: &[u8]) -> OsString {
    String::from_utf8_lossy(bytes).into_owned().into()
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

/// The last `keep` bytes of `input`, however long it is: read [`TAIL_BLOCK`] bytes at a time.
pub fn read_tail(input: &mut dyn Read, keep: u64) -> io::Result<Vec<u8>> {
    let mut tail = Vec::new();
    loop {
        let read = input.take(TAIL_BLOCK).read_to_end(&mut tail)?;
        let over = tail.len().saturating_sub(keep as usize);
        tail.drain(..over);
        if read == 0 {
            return Ok(tail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_size_limited() {
        assert_eq!(read_limited(&mut &b"abcd"[..], 4).unwrap(), b"abcd");
        // A tail: the last bytes of an input over several blocks.
        let long: Vec<u8> = (0..3 * TAIL_BLOCK).map(|i| i as u8).collect();
        let tail = read_tail(&mut &long[..], 5).unwrap();
        assert_eq!(tail, long[long.len() - 5..]);
        assert_eq!(read_tail(&mut &b"abc"[..], 5).unwrap(), b"abc");
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
    fn a_repository_fsmonitor_never_runs() {
        let tmp = tempfile::tempdir().unwrap();
        output(tmp.path(), &["init", "-q"], &[0]).unwrap();
        let hook = "touch ran; false";
        output(tmp.path(), &["config", "core.fsmonitor", hook], &[0]).unwrap();
        output(tmp.path(), &["status"], &[0]).unwrap();
        assert!(!tmp.path().join("ran").exists());
        // Every git command carries the flags, `git::command` used directly too.
        let args: Vec<_> = command(tmp.path())
            .get_args()
            .map(OsStr::to_owned)
            .collect();
        let flags = [
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
        ];
        assert_eq!(args[2..], flags.map(std::ffi::OsString::from));
    }

    #[cfg(unix)]
    #[test]
    fn a_repository_gpg_program_never_runs_for_a_signed_head() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let git = |args: &[&str]| output(dir, args, &[0]).unwrap();
        git(&["init", "-q"]);
        let who = ["-c", "user.name=a", "-c", "user.email=a@b"];
        git(&[&who[..], &["commit", "-q", "--allow-empty", "-m", "m"]].concat());
        // HEAD, signed: git checks a signature without running anything but `gpg.program`.
        let commit = String::from_utf8(git(&["cat-file", "commit", "HEAD"])).unwrap();
        let signature = "gpgsig -----BEGIN PGP SIGNATURE-----\n x\n -----END PGP SIGNATURE-----\n";
        let signed = commit.replacen("\n\n", &format!("\n{signature}\n"), 1);
        let args = ["hash-object", "-t", "commit", "-w", "--stdin"].map(OsStr::new);
        let sha = run(dir, &args, signed.as_bytes(), &[0], OUTPUT_LIMIT).unwrap();
        git(&["update-ref", "HEAD", String::from_utf8(sha).unwrap().trim()]);
        let gpg = dir.join("gpg");
        std::fs::write(&gpg, "#!/bin/sh\ntouch \"$0.ran\"\n").unwrap();
        std::fs::set_permissions(&gpg, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        git(&["config", "gpg.program", gpg.to_str().unwrap()]);
        git(&["config", "log.showSignature", "true"]);
        git(&["log", "-1", "--format=%ct", "HEAD", "--"]);
        assert!(!dir.join("gpg.ran").exists());
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
