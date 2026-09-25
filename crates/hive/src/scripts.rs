//! Project scripts (6.8): each worktree's block of ports, the `HIVE_*` environment of its
//! terminals and scripts, and the archive script the service runs before removing it. The
//! scripts are the user's own, from the settings, never read from a repository.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::git::read_limited;
use crate::wrapper::write_atomic;

/// Ports each worktree gets, from `HIVE_PORT` on.
pub const BLOCK: u16 = 10;
/// The first block starts here; below Linux's ephemeral range (32768 and up).
const FIRST: u16 = 20_000;
/// How many blocks there are (20000 to 29999).
const BLOCKS: u16 = 1000;
/// Largest ports file read.
const FILE_LIMIT: u64 = 256 * 1024;
/// How long the archive script may run.
pub const ARCHIVE_TIME: Duration = Duration::from_secs(60);
/// How often a running script is checked for its exit.
const POLL: Duration = Duration::from_millis(20);
/// The end of a script's output kept for its error message.
const OUTPUT_TAIL: usize = 4096;
/// How long the output may take to close after the script exited (a process it left in the
/// background may keep it open).
const OUTPUT_GRACE: Duration = Duration::from_millis(500);

/// The port blocks, by worktree path, kept in `<data>/hive/ports.json` so they stay stable.
pub struct Ports {
    file: PathBuf,
    lock: Mutex<()>,
}

impl Ports {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            lock: Mutex::new(()),
        }
    }

    /// The first port of `worktree`'s block, given one when it has none. Blocks of worktrees
    /// that no longer exist are taken back first.
    pub fn port(&self, worktree: &str) -> io::Result<u16> {
        let _lock = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut blocks = self.read();
        if let Some(port) = blocks.get(worktree) {
            return Ok(*port);
        }
        blocks.retain(|path, _| Path::new(path).is_dir());
        let port = (0..BLOCKS)
            .map(|i| FIRST + i * BLOCK)
            .find(|port| !blocks.values().any(|p| p == port))
            .ok_or_else(|| io::Error::other("every block of ports is taken"))?;
        blocks.insert(worktree.to_owned(), port);
        let json = serde_json::to_vec_pretty(&blocks)?;
        self.file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        write_atomic(&self.file, &json, 0o600)?;
        Ok(port)
    }

    /// The blocks in the file; an unreadable file or a block outside the range counts as none.
    fn read(&self) -> BTreeMap<String, u16> {
        let read = std::fs::File::open(&self.file)
            .and_then(|mut file| read_limited(&mut file, FILE_LIMIT))
            .ok();
        let blocks: BTreeMap<String, u16> = read
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let last = FIRST + (BLOCKS - 1) * BLOCK;
        let valid =
            |port: &u16| (FIRST..=last).contains(port) && (port - FIRST).is_multiple_of(BLOCK);
        blocks.into_iter().filter(|(_, p)| valid(p)).collect()
    }
}

/// The environment of a process in `worktree` of the project at `root`: `HIVE_PORT` (the
/// first of its block, when it has one), `HIVE_WORKTREE_PATH` and `HIVE_ROOT_PATH`.
pub fn env(root: &str, worktree: &str, port: Option<u16>) -> Vec<(&'static str, String)> {
    let mut env = vec![
        ("HIVE_WORKTREE_PATH", worktree.to_owned()),
        ("HIVE_ROOT_PATH", root.to_owned()),
    ];
    env.extend(port.map(|port| ("HIVE_PORT", port.to_string())));
    env
}

/// Runs the user's `script` with `sh -c` in `dir` with `env` added, within `time`: then the
/// script and everything it started in its process group are killed. A failure (an exit code
/// other than 0, a signal, the time limit) is an error ending with the end of its output.
pub fn run(script: &str, dir: &Path, env: &[(&str, String)], time: Duration) -> io::Result<()> {
    let (reader, writer) = io::pipe()?;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        // Its own process group, so the limit ends what it started too.
        .process_group(0)
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run sh: {err}")))?;
    let group = Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX));
    let (output, tail) = mpsc::channel();
    // Not waited for: a process left in the background may hold the output open.
    std::thread::spawn(move || output.send(last_bytes(reader)));
    let deadline = Instant::now() + time;
    // Only this thread reaps the script, so its pid still names its group when killed.
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = killpg(group, Signal::SIGKILL);
            child.wait()?;
            break None;
        }
        std::thread::sleep(POLL);
    };
    let output = tail.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
    let output = String::from_utf8_lossy(&output);
    let output = output.trim();
    let why = match status {
        Some(status) if status.success() => return Ok(()),
        Some(status) => format!("failed ({status})"),
        None => format!("took longer than {} s", time.as_secs_f32()),
    };
    let message = if output.is_empty() {
        format!("the archive script {why}")
    } else {
        format!("the archive script {why}:\n{output}")
    };
    Err(io::Error::other(message))
}

/// Everything read from `input` until it ends, keeping only the last [`OUTPUT_TAIL`] bytes.
fn last_bytes(mut input: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = [0; 8192];
    while let Ok(n @ 1..) = input.read(&mut buf) {
        kept.extend_from_slice(&buf[..n]);
        let extra = kept.len().saturating_sub(OUTPUT_TAIL);
        kept.drain(..extra);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn each_worktree_keeps_its_own_block() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());
        let file = tmp.path().join("hive/ports.json");
        let ports = Ports::new(file.clone());
        assert_eq!(ports.port(a).unwrap(), 20_000);
        assert_eq!(ports.port(b).unwrap(), 20_010);
        assert_eq!(ports.port(a).unwrap(), 20_000);
        // Kept across restarts, private.
        assert_eq!(Ports::new(file.clone()).port(b).unwrap(), 20_010);
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // A removed worktree's block goes to the next one that needs a block.
        std::fs::remove_dir(a).unwrap();
        assert_eq!(ports.port(b).unwrap(), 20_010);
        assert_eq!(ports.port("/new").unwrap(), 20_000);
    }

    #[test]
    fn a_bad_file_or_block_counts_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("ports.json");
        let ports = Ports::new(file.clone());
        for bad in [
            "{",
            "[]",
            r#"{"/x":20005}"#,
            r#"{"/x":19990}"#,
            r#"{"/x":30000}"#,
        ] {
            std::fs::write(&file, bad).unwrap();
            assert_eq!(ports.read(), BTreeMap::new(), "{bad}");
        }
        // The last block counts, and a real one is kept even for a missing folder while
        // asked for by that path.
        std::fs::write(&file, r#"{"/gone":29990}"#).unwrap();
        assert_eq!(ports.port("/gone").unwrap(), 29_990);
        std::fs::write(&file, " ".repeat(FILE_LIMIT as usize + 1)).unwrap();
        assert_eq!(ports.read(), BTreeMap::new());
    }

    #[test]
    fn blocks_run_out() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("ports.json");
        let dir = tmp.path().to_str().unwrap();
        // Every block taken by an existing folder (the same one: only its path is a key).
        let taken: BTreeMap<String, u16> = (0..BLOCKS)
            .map(|i| (format!("{dir}/{i}/.."), FIRST + i * BLOCK))
            .collect();
        for i in 0..BLOCKS {
            std::fs::create_dir(tmp.path().join(i.to_string())).unwrap();
        }
        std::fs::write(&file, serde_json::to_vec(&taken).unwrap()).unwrap();
        let err = Ports::new(file).port("/one-more").unwrap_err();
        assert_eq!(err.to_string(), "every block of ports is taken");
    }

    #[test]
    fn a_failed_save_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("hive"), "").unwrap();
        let ports = Ports::new(tmp.path().join("hive/ports.json"));
        assert!(ports.port("/w").is_err());
    }

    #[test]
    fn the_environment_names_the_worktree_its_root_and_port() {
        assert_eq!(
            env("/r", "/r/w", Some(20_010)),
            vec![
                ("HIVE_WORKTREE_PATH", "/r/w".to_owned()),
                ("HIVE_ROOT_PATH", "/r".to_owned()),
                ("HIVE_PORT", "20010".to_owned()),
            ]
        );
        assert_eq!(env("/r", "/r", None).len(), 2);
    }

    #[test]
    fn a_script_runs_in_its_folder_with_the_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let env = [("HIVE_PORT", "20000".to_owned())];
        let script = "echo \"$HIVE_PORT\" > \"$(pwd)/out\"";
        run(script, tmp.path(), &env, ARCHIVE_TIME).unwrap();
        let out = std::fs::read_to_string(tmp.path().join("out")).unwrap();
        assert_eq!(out, "20000\n");
    }

    #[test]
    fn a_failed_script_reports_the_end_of_its_output() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run(
            "echo out; echo err >&2; exit 3",
            tmp.path(),
            &[],
            ARCHIVE_TIME,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, "the archive script failed (exit status: 3):\nout\nerr");
        let err = run("exit 1", tmp.path(), &[], ARCHIVE_TIME).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the archive script failed (exit status: 1)"
        );
        let long = format!("head -c {} /dev/zero | tr '\\0' a; exit 1", OUTPUT_TAIL * 3);
        let err = run(&long, tmp.path(), &[], ARCHIVE_TIME).unwrap_err();
        let prefix = "the archive script failed (exit status: 1):\n";
        assert_eq!(
            err.to_string(),
            format!("{prefix}{}", "a".repeat(OUTPUT_TAIL))
        );
    }

    #[test]
    fn a_script_past_its_time_is_killed_with_what_it_started() {
        let tmp = tempfile::tempdir().unwrap();
        let pid_file = tmp.path().join("pid");
        let script = format!(
            "echo started; sleep 30 & echo $! > {}; wait",
            pid_file.display()
        );
        let start = Instant::now();
        let err = run(&script, tmp.path(), &[], Duration::from_millis(300)).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(
            err.to_string(),
            "the archive script took longer than 0.3 s:\nstarted"
        );
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The background sleep was in the group: killed, then reaped by init.
        let gone = (0..250).any(|_| {
            std::thread::sleep(POLL);
            nix::sys::signal::kill(Pid::from_raw(pid), None).is_err()
        });
        assert!(gone);
    }

    #[test]
    fn a_script_that_cannot_start_is_an_error() {
        let err = run("true", Path::new("/nonexistent/dir"), &[], ARCHIVE_TIME).unwrap_err();
        assert!(err.to_string().starts_with("cannot run sh: "), "{err}");
    }

    #[test]
    fn a_background_process_holding_the_output_does_not_hold_the_script() {
        let tmp = tempfile::tempdir().unwrap();
        let start = Instant::now();
        // Only the grace period is waited for the output the sleep keeps open.
        let err = run("sleep 3 & exit 2", tmp.path(), &[], ARCHIVE_TIME).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(err.to_string().starts_with("the archive script failed"));
    }
}
