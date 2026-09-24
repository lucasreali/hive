//! Minimal `/proc` reader: enough to find a terminal's processes.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub pgrp: i32,
    pub session: i32,
    pub comm: String,
}

/// Live processes under `root` (normally `/proc`). Zombies and unreadable entries are skipped.
pub fn list(root: &Path) -> Vec<Proc> {
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
        let mut procs = list(root.path());
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
    fn missing_root_lists_nothing() {
        assert_eq!(list(Path::new("/nonexistent/proc")), Vec::new());
    }

    #[test]
    fn real_proc_contains_this_process() {
        let me = std::process::id() as i32;
        let found = list(Path::new("/proc"))
            .into_iter()
            .find(|p| p.pid == me)
            .unwrap();
        assert_eq!(found.pgrp, nix::unistd::getpgrp().as_raw());
    }
}
