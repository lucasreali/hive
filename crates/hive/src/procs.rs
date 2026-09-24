//! Minimal process listing: enough to find a terminal's processes. On Linux it reads
//! `/proc`; on macOS it asks the kernel through `libproc`.

use std::path::{Path, PathBuf};

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
            Source::System => macos::list(),
        }
    }

    fn cwd(self, pid: i32) -> Option<PathBuf> {
        match self {
            Source::Dir(root) => std::fs::read_link(root.join(pid.to_string()).join("cwd")).ok(),
            #[cfg(target_os = "linux")]
            Source::System => Source::Dir(Path::new("/proc")).cwd(pid),
            #[cfg(target_os = "macos")]
            Source::System => macos::cwd(pid),
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

/// The working directory of every `claude` process, once per terminal session (a wrapper
/// and the `claude` it starts share one).
pub fn claude_cwds(source: Source) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    list(source)
        .into_iter()
        .filter(|p| p.comm == "claude")
        .filter_map(|p| {
            let cwd = source.cwd(p.pid)?;
            seen.insert((p.session, cwd.clone())).then_some(cwd)
        })
        .collect()
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

/// The kernel's process table, through `libproc`.
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, c_char};
    use std::path::PathBuf;

    use libproc::bsd_info::BSDInfo;
    use libproc::proc_pid::{PIDInfo, PidInfoFlavor, pidinfo};
    use libproc::processes::{ProcFilter, pids_by_type};
    use nix::unistd::{Pid, getsid};

    use super::Proc;

    /// `SZOMB` in `<sys/proc.h>`.
    const ZOMBIE: u32 = 5;
    /// `MAXPATHLEN`.
    const PATH_MAX: usize = 1024;

    pub fn list() -> Vec<Proc> {
        let pids = pids_by_type(ProcFilter::All).unwrap_or_default();
        pids.into_iter()
            .filter_map(|pid| process(pid as i32))
            .collect()
    }

    fn process(pid: i32) -> Option<Proc> {
        let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
        if info.pbi_status == ZOMBIE {
            return None;
        }
        Some(Proc {
            pid,
            pgrp: info.pbi_pgid as i32,
            session: getsid(Some(Pid::from_raw(pid))).ok()?.as_raw(),
            comm: text(&info.pbi_comm)?,
        })
    }

    /// `struct vnode_info_path`: a `struct vnode_info` (152 bytes) and a path.
    #[repr(C)]
    struct VnodeInfoPath {
        info: [u8; 152],
        path: [c_char; PATH_MAX],
    }

    /// `struct proc_vnodepathinfo`, which `libproc` does not bind (its `pidcwd` is a stub
    /// on macOS).
    #[repr(C)]
    struct VnodePathInfo {
        cdir: VnodeInfoPath,
        rdir: VnodeInfoPath,
    }

    impl PIDInfo for VnodePathInfo {
        fn flavor() -> PidInfoFlavor {
            PidInfoFlavor::VNodePathInfo
        }
    }

    pub fn cwd(pid: i32) -> Option<PathBuf> {
        let info = pidinfo::<VnodePathInfo>(pid, 0).ok()?;
        text(&info.cdir.path).map(PathBuf::from)
    }

    /// A NUL-terminated C string from a fixed buffer.
    fn text(buf: &[c_char]) -> Option<String> {
        let bytes: Vec<u8> = buf.iter().map(|&c| c as u8).collect();
        let text = CStr::from_bytes_until_nul(&bytes).ok()?;
        Some(text.to_string_lossy().into_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn this_process_is_listed_with_its_group_session_and_folder() {
            let me = std::process::id() as i32;
            let found = list().into_iter().find(|p| p.pid == me).unwrap();
            assert_eq!(found.pgrp, nix::unistd::getpgrp().as_raw());
            assert_eq!(found.session, getsid(None).unwrap().as_raw());
            let exe = std::env::current_exe().unwrap();
            let name: String = exe
                .file_name()
                .unwrap()
                .to_string_lossy()
                .chars()
                .take(15)
                .collect();
            assert_eq!(found.comm, name);
            assert_eq!(cwd(me), Some(std::env::current_dir().unwrap()));
            assert_eq!(cwd(-1), None);
            assert_eq!(process(-1), None);
        }

        #[test]
        fn c_strings_end_at_the_first_nul() {
            let buf = [b'a' as c_char, b'b' as c_char, 0, b'c' as c_char];
            assert_eq!(text(&buf), Some("ab".to_owned()));
            assert_eq!(text(&[b'a' as c_char]), None);
        }
    }
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

    #[test]
    fn claude_processes_are_found_with_their_folder_once_per_session() {
        let root = fake_proc(&[
            ("20", "20 (claude) S 1 20 20 0 0"),
            ("21", "21 (claude) S 20 20 20 0 0"),
            ("22", "22 (claude) S 1 22 22 0 0"),
            ("23", "23 (fish) S 1 23 23 0 0"),
            ("24", "24 (claude) S 1 24 24 0 0"),
        ]);
        let link = |pid: &str, cwd: &str| {
            std::os::unix::fs::symlink(cwd, root.path().join(pid).join("cwd")).unwrap()
        };
        link("20", "/r");
        link("21", "/r");
        link("22", "/r");
        link("23", "/r");
        // 24 has no readable cwd.
        let mut cwds = claude_cwds(Source::Dir(root.path()));
        cwds.sort();
        assert_eq!(cwds, vec![PathBuf::from("/r"), PathBuf::from("/r")]);
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
}
