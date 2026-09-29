//! Project scripts (6.8): each worktree's block of ports, the `HIVE_*` environment of its
//! terminals and scripts, and the archive script the service runs before removing it. The
//! scripts are the user's own, from the settings, never read from a repository.

use std::collections::BTreeMap;
use std::io::{self, Read};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use hive_protocol::TerminalShell;
#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::git::read_limited;
use crate::wrapper::write_atomic;

/// Ports each worktree gets, from `HIVE_PORT` on.
pub const BLOCK: u16 = 10;
/// The first block starts here; below Linux's ephemeral range (32768 and up).
const FIRST: u16 = 20_000;
const _: () = assert!(FIRST.is_multiple_of(BLOCK));
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
        self.write(&blocks)?;
        Ok(port)
    }

    /// Takes back the blocks of `worktrees` (a removed project's, 9.28); nothing is written
    /// when none of them has one.
    pub fn forget(&self, worktrees: &[String]) -> io::Result<()> {
        let _lock = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut blocks = self.read();
        let before = blocks.len();
        blocks.retain(|path, _| !worktrees.contains(path));
        if blocks.len() == before {
            return Ok(());
        }
        self.write(&blocks)
    }

    fn write(&self, blocks: &BTreeMap<String, u16>) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(blocks)?;
        self.file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        write_atomic(&self.file, &json, 0o600)
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
        // FIRST is a multiple of BLOCK, so every block starts at one.
        let valid = |port: &u16| (FIRST..=last).contains(port) && port.is_multiple_of(BLOCK);
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

/// How the user's `script` runs: `sh -c` (`shell`, the terminals' shell, only matters on
/// native Windows).
#[cfg(unix)]
fn command(script: &str, _shell: TerminalShell) -> io::Result<std::process::Command> {
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(script);
    Ok(command)
}

#[cfg(windows)]
use crate::windows::script as command;

/// Runs the user's `script` ([`command`]: `sh -c`, on native Windows the terminals' `shell`)
/// in `dir` with `env` added, within `time`. Once it ended or its time is up, everything left
/// in its process group (its job on Windows) is killed. A failure (an exit code other than 0,
/// a signal, the time limit) is an error ending with the end of its output.
pub fn run(
    script: &str,
    dir: &Path,
    env: &[(&str, String)],
    time: Duration,
    shell: TerminalShell,
) -> io::Result<()> {
    let (reader, writer) = io::pipe()?;
    let mut command = command(script, shell)?;
    command
        .current_dir(dir)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    // Its own process group, so the limit ends what it started too.
    #[cfg(unix)]
    command.process_group(0);
    let program = command.get_program().display().to_string();
    let mut child = command
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run {program}: {err}")))?;
    // Its copy of the output's write end: the output ends only once none is left open.
    drop(command);
    #[cfg(unix)]
    let group = Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX));
    // Windows has no process groups: a job holds the script and what it starts.
    #[cfg(windows)]
    let group = crate::windows::Job::of(&child)?;
    let (output, tail) = mpsc::channel();
    // Not waited for: a process left in the background may hold the output open. Without a
    // thread (not a panic: it would end the service) the output is closed and not shown.
    let _ = std::thread::Builder::new().spawn(move || output.send(last_bytes(reader)));
    // Checked once per poll until `time` is spent: bounded by a count, not a clock.
    let polls = time.as_millis().div_ceil(POLL.as_millis());
    let mut waited = Ok(None);
    for _ in 0..polls {
        waited = child.try_wait();
        if !matches!(waited, Ok(None)) {
            break;
        }
        std::thread::sleep(POLL);
    }
    // Whatever the script left running in its group ends with it: its worktree is about to
    // go. A group with members keeps its id, so this cannot reach another process's group;
    // an empty one (everything already ended) has no one to reach but for a pid reused in the
    // moment since, as a group leader.
    #[cfg(unix)]
    let _ = killpg(group, Signal::SIGKILL);
    #[cfg(windows)]
    group.kill();
    // Reaps it when it was killed (an exited script keeps its status).
    let _ = child.wait();
    let status = waited?;
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
    use std::time::Instant;

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
        #[cfg(unix)]
        assert_eq!(
            crate::mode::of(&std::fs::metadata(&file).unwrap()) & 0o777,
            0o600
        );
        // A removed worktree's block goes to the next one that needs a block.
        std::fs::remove_dir(a).unwrap();
        assert_eq!(ports.port(b).unwrap(), 20_010);
        assert_eq!(ports.port("/new").unwrap(), 20_000);
    }

    #[test]
    fn a_removed_project_gives_its_blocks_back() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("hive/ports.json");
        let ports = Ports::new(file.clone());
        ports.forget(&["/x".into()]).unwrap();
        assert!(!file.exists());
        let [a, b, c] = ["a", "b", "c"].map(|w| {
            let dir = tmp.path().join(w);
            std::fs::create_dir(&dir).unwrap();
            let dir = dir.display().to_string();
            ports.port(&dir).unwrap();
            dir
        });
        let saved = std::fs::read(&file).unwrap();
        ports.forget(&["/x".into()]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), saved);
        ports.forget(&[a, c, "/x".into()]).unwrap();
        let kept = BTreeMap::from([(b, 20_010)]);
        assert_eq!(Ports::new(file).read(), kept);
        // Their folders may still exist: the blocks are free again all the same.
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

    // The same `sh` scripts run on Windows in the Bash of Git for Windows (on the runner).
    const SH: TerminalShell = TerminalShell::GitBash;
    /// How an exit code reads.
    const EXIT: &str = if cfg!(windows) {
        "exit code"
    } else {
        "exit status"
    };
    /// Prints the process id of the last command started in the background (the Bash of Git
    /// for Windows has ids of its own).
    const LAST_PID: &str = if cfg!(windows) {
        "cat /proc/$!/winpid"
    } else {
        "echo $!"
    };
    /// How much longer than `sh` the Bash of Git for Windows may take to start.
    const SLACK: Duration = Duration::from_secs(if cfg!(windows) { 5 } else { 0 });

    #[test]
    fn a_script_runs_in_its_folder_with_the_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let env = [("HIVE_PORT", "20000".to_owned())];
        let script = "echo \"$HIVE_PORT\" > \"$(pwd)/out\"";
        run(script, tmp.path(), &env, ARCHIVE_TIME, SH).unwrap();
        let out = std::fs::read_to_string(tmp.path().join("out")).unwrap();
        assert_eq!(out, "20000\n");
    }

    #[test]
    fn a_failed_script_reports_the_end_of_its_output() {
        let tmp = tempfile::tempdir().unwrap();
        let run = |script: &str| run(script, tmp.path(), &[], ARCHIVE_TIME, SH);
        let err = run("echo out; echo err >&2; exit 3").unwrap_err();
        let failed = format!("the archive script failed ({EXIT}: 3):\nout\nerr");
        assert_eq!(err.to_string(), failed);
        let err = run("exit 1").unwrap_err();
        let failed = format!("the archive script failed ({EXIT}: 1)");
        assert_eq!(err.to_string(), failed);
        let long = format!("head -c {} /dev/zero | tr '\\0' a; exit 1", OUTPUT_TAIL * 3);
        let err = run(&long).unwrap_err();
        let prefix = format!("the archive script failed ({EXIT}: 1):\n");
        assert_eq!(
            err.to_string(),
            format!("{prefix}{}", "a".repeat(OUTPUT_TAIL))
        );
    }

    #[test]
    fn a_script_past_its_time_is_killed_with_what_it_started() {
        let tmp = tempfile::tempdir().unwrap();
        let limit = Duration::from_millis(300) + SLACK;
        let script = format!("echo started; sleep 30 & {LAST_PID} > pid; wait");
        let start = Instant::now();
        let err = run(&script, tmp.path(), &[], limit, SH).unwrap_err();
        assert!(start.elapsed() >= limit);
        assert!(start.elapsed() < Duration::from_secs(10) + SLACK);
        let took = format!(
            "the archive script took longer than {} s:\nstarted",
            limit.as_secs_f32()
        );
        assert_eq!(err.to_string(), took);
        assert!(gone(&tmp.path().join("pid")));
    }

    /// Whether the process whose pid is in `file` ended (killed, then reaped by init).
    #[cfg(unix)]
    fn gone(file: &Path) -> bool {
        let pid: i32 = std::fs::read_to_string(file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        (0..250).any(|_| {
            std::thread::sleep(POLL);
            nix::sys::signal::kill(Pid::from_raw(pid), None).is_err()
        })
    }

    /// Whether the process whose pid is in `file` ended (or ends within 5 s).
    #[cfg(windows)]
    fn gone(file: &Path) -> bool {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        let pid: u32 = std::fs::read_to_string(file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        // No such process any more.
        if process.is_null() {
            return true;
        }
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) == WAIT_OBJECT_0 }
    }

    #[test]
    fn a_script_that_cannot_start_is_an_error() {
        let missing = Path::new("/nonexistent/dir");
        let err = run("true", missing, &[], ARCHIVE_TIME, SH).unwrap_err();
        let err = err.to_string();
        #[cfg(unix)]
        assert!(err.starts_with("cannot run sh: "), "{err}");
        #[cfg(windows)]
        assert!(
            err.starts_with("cannot run ") && err.contains(r"\bin\bash.exe: "),
            "{err}"
        );
    }

    #[test]
    fn what_a_script_leaves_running_is_killed_when_it_ends() {
        let tmp = tempfile::tempdir().unwrap();
        let start = Instant::now();
        // The sleep holds the output open: it ends with the script, not 30 s later.
        let script = format!("sleep 30 & {LAST_PID} > pid");
        run(&script, tmp.path(), &[], ARCHIVE_TIME, SH).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2) + SLACK);
        assert!(gone(&tmp.path().join("pid")));
        let err = run("sleep 30 & exit 2", tmp.path(), &[], ARCHIVE_TIME, SH).unwrap_err();
        let failed = format!("the archive script failed ({EXIT}: 2)");
        assert_eq!(err.to_string(), failed);
        assert!(start.elapsed() < Duration::from_secs(4) + SLACK * 2);
    }
}
