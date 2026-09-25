//! What only macOS needs, kept apart from the portable code: the kernel's process table
//! (through `libproc`), the terminal's login shell and native file paths. Tested on macOS.

use std::ffi::{CStr, OsStr, c_char};
use std::io;
use std::path::{Path, PathBuf};

use libproc::bsd_info::BSDInfo;
use libproc::proc_pid::{PIDInfo, PidInfoFlavor, pidinfo};
use libproc::processes::{ProcFilter, pids_by_type};
use nix::unistd::{Pid, getsid};

use crate::procs::Proc;
use crate::terminal::login;

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

/// The user's login shell (`$SHELL`), started as [`crate::terminal::login::launch`] says.
pub fn shell(bin_dir: &Path) -> pty_process::Command {
    let var = std::env::var_os;
    let launch = login::launch(var("SHELL"), var("ZDOTDIR"), var("PATH"), bin_dir);
    pty_process::Command::new(launch.program)
        .args(launch.args)
        .envs(launch.env)
}

/// The app runs beside the service: it opens `path` itself.
pub fn native_path(path: &Path, _wslpath: &OsStr) -> io::Result<String> {
    Ok(path.to_string_lossy().into_owned())
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
    fn zombies_are_not_listed() {
        // Never waited for: once it exits it stays a zombie.
        let child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        let start = std::time::Instant::now();
        while pidinfo::<BSDInfo>(pid, 0).unwrap().pbi_status != ZOMBIE {
            assert!(start.elapsed().as_secs() < 10, "the child never exited");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(process(pid), None);
    }

    #[test]
    fn c_strings_end_at_the_first_nul() {
        let buf = [b'a' as c_char, b'b' as c_char, 0, b'c' as c_char];
        assert_eq!(text(&buf), Some("ab".to_owned()));
        assert_eq!(text(&[b'a' as c_char]), None);
    }
}
