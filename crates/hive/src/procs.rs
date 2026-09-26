//! Minimal process listing: enough to find a terminal's processes and the Claude session each
//! `claude` runs. On Linux it reads `/proc`; on macOS it asks the kernel through `libproc`.

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};

use crate::chat::is_session;

/// Most bytes read of a process's command line or of a Claude session record.
const READ_LIMIT: u64 = 64 * 1024;
/// Most open files looked at per process.
const FD_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub pgrp: i32,
    pub session: i32,
    pub comm: String,
}

/// Where processes are read from.
#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    /// This machine's processes.
    System,
    /// A `/proc`-shaped directory: `<pid>/stat` and a `<pid>/cwd` link (tests).
    Dir(&'a Path),
}

impl Source<'_> {
    fn list(self) -> Vec<Proc> {
        match self {
            Source::Dir(root) => read_dir(root),
            #[cfg(target_os = "linux")]
            Source::System => read_dir(Path::new("/proc")),
            #[cfg(target_os = "macos")]
            Source::System => crate::macos::list(),
        }
    }

    fn cwd(self, pid: i32) -> Option<PathBuf> {
        match self {
            Source::Dir(root) => std::fs::read_link(root.join(pid.to_string()).join("cwd")).ok(),
            #[cfg(target_os = "linux")]
            Source::System => Source::Dir(Path::new("/proc")).cwd(pid),
            #[cfg(target_os = "macos")]
            Source::System => crate::macos::cwd(pid),
        }
    }

    /// The process's arguments, `argv[0]` first; `None` when unreadable or too long.
    fn args(self, pid: i32) -> Option<Vec<String>> {
        match self {
            Source::Dir(root) => {
                let file = File::open(root.join(pid.to_string()).join("cmdline")).ok()?;
                let bytes = crate::git::read_limited(&mut &file, READ_LIMIT).ok()?;
                let words = bytes.split(|&b| b == 0).filter(|w| !w.is_empty());
                Some(
                    words
                        .map(|w| String::from_utf8_lossy(w).into_owned())
                        .collect(),
                )
            }
            #[cfg(target_os = "linux")]
            Source::System => Source::Dir(Path::new("/proc")).args(pid),
            // macOS: only Claude's record tells (arguments would need `KERN_PROCARGS2`).
            #[cfg(target_os = "macos")]
            Source::System => None,
        }
    }

    /// What the process's open files point at, the first [`FD_LIMIT`] of them.
    fn open_files(self, pid: i32) -> Vec<PathBuf> {
        match self {
            Source::Dir(root) => std::fs::read_dir(root.join(pid.to_string()).join("fd"))
                .into_iter()
                .flatten()
                .take(FD_LIMIT)
                .filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
                .collect(),
            #[cfg(target_os = "linux")]
            Source::System => Source::Dir(Path::new("/proc")).open_files(pid),
            #[cfg(target_os = "macos")]
            Source::System => Vec::new(),
        }
    }
}

/// Live processes. Zombies and unreadable entries are skipped.
pub fn list(source: Source) -> Vec<Proc> {
    source.list()
}

/// Live processes whose working directory is `dir` or inside it.
pub fn inside(source: Source, dir: &Path) -> Vec<Proc> {
    list(source)
        .into_iter()
        .filter(|p| source.cwd(p.pid).is_some_and(|cwd| cwd.starts_with(dir)))
        .collect()
}

/// The Claude session each `claude` process is known to run. A process tells it through, in
/// this order: Claude's own record `<records>/<pid>.json` (`sessionId`), its arguments
/// (`--session-id <id>`, or `--resume <id>` unless it forks), or an open `<id>.jsonl` log. A
/// process none of these name (a bare `claude`, `--continue`) marks nothing.
pub fn claude_sessions(source: Source, records: Option<&Path>) -> HashSet<String> {
    list(source)
        .into_iter()
        .filter(|p| p.comm == "claude")
        .filter_map(|p| {
            records
                .and_then(|dir| recorded(dir, p.pid))
                .or_else(|| source.args(p.pid).and_then(|args| named(&args)))
                .or_else(|| source.open_files(p.pid).iter().find_map(|f| logged(f)))
        })
        .collect()
}

/// The session in Claude's record of process `pid`, when the record is that process's.
fn recorded(dir: &Path, pid: i32) -> Option<String> {
    let path = dir.join(format!("{pid}.json"));
    // Checked before opening: opening a FIFO would block the service until a writer came.
    // A swap between the check and the open needs write access to Claude's folder (the
    // same user).
    if !path.symlink_metadata().ok()?.is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let bytes = crate::git::read_limited(&mut &file, READ_LIMIT).ok()?;
    let record: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if record["pid"].as_i64() != Some(pid.into()) {
        return None;
    }
    let id = record["sessionId"].as_str()?;
    is_session(id).then(|| id.to_owned())
}

/// The session `claude`'s arguments name: `--session-id`'s, else `--resume`'s (`-r`) unless
/// `--fork-session` starts a new one from it.
fn named(args: &[String]) -> Option<String> {
    let (mut id, mut resumed, mut fork) = (None, None, false);
    let mut words = args.iter().skip(1);
    while let Some(word) = words.next() {
        let (flag, value) = match word.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (word.as_str(), None),
        };
        let mut value = || value.or_else(|| words.next().map(String::as_str));
        match flag {
            "--session-id" => id = value(),
            "--resume" | "-r" => resumed = value(),
            "--fork-session" => fork = true,
            "--" => break,
            _ => {}
        }
    }
    let id = id.or(resumed.filter(|_| !fork))?;
    is_session(id).then(|| id.to_owned())
}

/// The session whose log `file` is (`<id>.jsonl`).
fn logged(file: &Path) -> Option<String> {
    let id = file.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    is_session(id).then(|| id.to_owned())
}

/// The processes of a `/proc`-shaped directory.
fn read_dir(root: &Path) -> Vec<Proc> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("stat")).ok())
        .filter_map(|stat| parse_stat(&stat))
        .collect()
}

/// Parses `/proc/<pid>/stat`: `pid (comm) state ppid pgrp session ...`.
/// `comm` may contain spaces and parentheses, so it ends at the last `)`.
fn parse_stat(stat: &str) -> Option<Proc> {
    let (head, rest) = stat.rsplit_once(')')?;
    let (pid, comm) = head.split_once(" (")?;
    let mut fields = rest.split_whitespace();
    if fields.next()? == "Z" {
        return None;
    }
    let _ppid = fields.next()?;
    let mut number = || fields.next()?.parse().ok();
    Some(Proc {
        pid: pid.parse().ok()?,
        pgrp: number()?,
        session: number()?,
        comm: comm.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_proc(entries: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (name, stat) in entries {
            std::fs::create_dir(root.path().join(name)).unwrap();
            std::fs::write(root.path().join(name).join("stat"), stat).unwrap();
        }
        root
    }

    #[test]
    fn parses_stat_fields_and_odd_command_names() {
        let root = fake_proc(&[
            ("10", "10 (fish) S 1 10 10 34816 0 0"),
            ("11", "11 (my (weird) cmd) R 10 11 10 34816"),
            ("12", "12 (dead) Z 10 12 10 0"),
            ("self", "garbage"),
            ("13", "13 (short) S 10"),
        ]);
        let mut procs = list(Source::Dir(root.path()));
        procs.sort_by_key(|p| p.pid);
        assert_eq!(
            procs,
            vec![
                Proc {
                    pid: 10,
                    pgrp: 10,
                    session: 10,
                    comm: "fish".into()
                },
                Proc {
                    pid: 11,
                    pgrp: 11,
                    session: 10,
                    comm: "my (weird) cmd".into()
                },
            ]
        );
    }

    #[test]
    fn processes_are_found_by_working_directory() {
        let root = fake_proc(&[
            ("10", "10 (fish) S 1 10 10 34816 0 0"),
            ("11", "11 (claude) S 10 11 10 34816 0 0"),
            ("12", "12 (vim) S 10 12 10 34816 0 0"),
            ("13", "13 (bash) S 10 13 10 34816 0 0"),
        ]);
        let link = |pid: &str, cwd: &str| {
            std::os::unix::fs::symlink(cwd, root.path().join(pid).join("cwd")).unwrap()
        };
        link("10", "/r/.claude/worktrees/a");
        link("11", "/r/.claude/worktrees/a/src");
        link("12", "/r/.claude/worktrees/ab");
        // 13 has no readable cwd.
        let mut found: Vec<i32> = inside(
            Source::Dir(root.path()),
            Path::new("/r/.claude/worktrees/a"),
        )
        .into_iter()
        .map(|p| p.pid)
        .collect();
        found.sort();
        assert_eq!(found, vec![10, 11]);
    }

    /// A session id ending in `n`.
    fn id(n: u32) -> String {
        format!("00000000-0000-0000-0000-{n:012x}")
    }

    #[test]
    fn claude_processes_tell_their_session_or_mark_nothing() {
        let pids = 20..=33;
        let stats: Vec<(String, String)> = pids
            .clone()
            .map(|pid| {
                let comm = if pid == 30 { "fish" } else { "claude" };
                (pid.to_string(), format!("{pid} ({comm}) S 1 {pid} {pid} 0"))
            })
            .collect();
        let stats: Vec<(&str, &str)> = stats.iter().map(|(p, s)| (&**p, &**s)).collect();
        let root = fake_proc(&stats);
        let records = tempfile::tempdir().unwrap();
        let record = |pid: u32, body: String| {
            std::fs::write(records.path().join(format!("{pid}.json")), body).unwrap()
        };
        let args = |pid: u32, words: &[&str]| {
            let line: String = words.iter().map(|w| format!("{w}\0")).collect();
            std::fs::write(root.path().join(pid.to_string()).join("cmdline"), line).unwrap()
        };
        let long = " ".repeat(READ_LIMIT as usize);
        // Claude's own record of the process wins over its arguments (a few KiB are fine).
        let padding = " ".repeat(4096);
        record(
            20,
            format!(r#"{{"pid": 20, "sessionId": "{}"}}{padding}"#, id(1)),
        );
        args(20, &["claude", "--resume", &id(99)]);
        // A record of another process, one without a session id and an unreadable one are
        // passed over.
        record(21, format!(r#"{{"pid": 7, "sessionId": "{}"}}"#, id(2)));
        args(21, &["claude", "--resume", &id(2)]);
        record(22, r#"{"pid": 22, "sessionId": "not-a-session"}"#.into());
        args(22, &["claude", "-r", &id(3)]);
        record(
            23,
            format!(r#"{{"pid": 23, "sessionId": "{}"}}{long}"#, id(98)),
        );
        args(
            23,
            &[
                "claude",
                &format!("--session-id={}", id(4)),
                "--resume",
                &id(97),
            ],
        );
        // Forking starts a new session: the resumed one is not running.
        args(24, &["claude", "--resume", &id(96), "--fork-session"]);
        // A record that is no regular file is passed over (a FIFO would block), a link too.
        let elsewhere = root.path().join("elsewhere.json");
        let linked = format!(r#"{{"pid": 25, "sessionId": "{}"}}"#, id(91));
        std::fs::write(&elsewhere, linked).unwrap();
        std::os::unix::fs::symlink(&elsewhere, records.path().join("25.json")).unwrap();
        // A named session is kept even when forking.
        args(
            25,
            &[
                "claude",
                "--fork-session",
                "--resume",
                &id(95),
                "--session-id",
                &id(5),
            ],
        );
        // `--continue` and a bare `claude` do not tell.
        args(26, &["claude", "--continue"]);
        args(27, &["claude"]);
        // Nor does a resume that is not an id, or anything after `--`.
        args(28, &["claude", "--resume", "search words"]);
        args(29, &["claude", "--", "--resume", &id(94)]);
        // Not a `claude`.
        args(30, &["fish", "--resume", &id(93)]);
        // A command line too long to read.
        args(31, &["claude", "--resume", &id(92), &long]);
        // An open log tells, when its name is a session id.
        let fds = |pid: u32, targets: &[&str]| {
            let dir = root.path().join(pid.to_string()).join("fd");
            std::fs::create_dir(&dir).unwrap();
            for (fd, target) in targets.iter().enumerate() {
                std::os::unix::fs::symlink(target, dir.join(fd.to_string())).unwrap();
            }
        };
        args(32, &["claude", "--continue"]);
        let log = format!("/h/.claude/projects/-r/{}.jsonl", id(6));
        fds(32, &["/dev/pts/1", "/h/.claude/history.jsonl", &log]);
        fds(33, &["/r/notes.jsonl", "/"]);

        let found = claude_sessions(Source::Dir(root.path()), Some(records.path()));
        let mut found: Vec<_> = found.into_iter().collect();
        found.sort();
        assert_eq!(found, [id(1), id(2), id(3), id(4), id(5), id(6)]);

        // Without records, the arguments tell.
        let found = claude_sessions(Source::Dir(root.path()), None);
        assert!(
            found.contains(&id(99)) && !found.contains(&id(1)),
            "{found:?}"
        );
    }

    #[test]
    fn missing_root_lists_nothing() {
        assert_eq!(
            list(Source::Dir(Path::new("/nonexistent/proc"))),
            Vec::new()
        );
    }

    #[test]
    fn the_system_lists_this_process_and_its_folder() {
        let me = std::process::id() as i32;
        let found = list(Source::System)
            .into_iter()
            .find(|p| p.pid == me)
            .unwrap();
        assert_eq!(found.pgrp, nix::unistd::getpgrp().as_raw());
        assert_eq!(found.session, nix::unistd::getsid(None).unwrap().as_raw());
        let here = std::env::current_dir().unwrap();
        let found = inside(Source::System, &here);
        assert!(found.iter().any(|p| p.pid == me), "{found:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_system_tells_this_process_arguments_and_open_files() {
        let me = std::process::id() as i32;
        let args = Source::System.args(me).unwrap();
        assert_eq!(args, std::env::args().collect::<Vec<_>>());
        let log = tempfile::NamedTempFile::new().unwrap();
        let files = Source::System.open_files(me);
        assert!(files.iter().any(|f| f == log.path()), "{files:?}");
    }
}
