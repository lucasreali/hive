//! Git always runs as the `git` executable with separate arguments, output size-limited.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

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
    let mut child = command(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run git: {err}")))?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    // Feed stdin and drain stderr from other threads, so no pipe can deadlock another.
    let (out, err) = std::thread::scope(|scope| {
        scope.spawn(move || stdin.map(|mut stdin| stdin.write_all(input)));
        let err = scope.spawn(move || {
            let mut buf = Vec::new();
            stderr.map(|stderr| stderr.take(STDERR_LIMIT).read_to_end(&mut buf));
            buf
        });
        let out = stdout.map_or(Ok(Vec::new()), |mut out| read_limited(&mut out, limit));
        (out, err.join().unwrap_or_default())
    });
    let status = child.wait()?;
    let command: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
    let command = command.join(" ");
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
