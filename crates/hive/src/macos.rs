//! What only macOS needs, kept apart from the portable code: the kernel's process table
//! (through `libproc`), the terminal's login shell and native file paths. Tested on macOS.

use std::ffi::{CStr, CString, OsStr, c_char};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libproc::bsd_info::BSDInfo;
use libproc::proc_pid::{PIDInfo, PidInfoFlavor, pidinfo};
use libproc::processes::{ProcFilter, pids_by_type};
use nix::unistd::{Pid, getsid};

use crate::procs::Proc;
use crate::terminal::login;

/// `MAXPATHLEN`.
const PATH_MAX: usize = 1024;

pub fn list() -> Vec<Proc> {
    let pids = pids_by_type(ProcFilter::All).unwrap_or_default();
    pids.into_iter()
        .filter_map(|pid| process(pid as i32))
        .collect()
}

fn process(pid: i32) -> Option<Proc> {
    // Fails for a zombie too (ESRCH), so zombies are skipped.
    let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
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

/// The per-user temporary folder launchd gives every process of the user as `TMPDIR`
/// (`confstr(_CS_DARWIN_USER_TEMP_DIR)`), asked of the system so that a `TMPDIR` changed in a
/// shell (nix-shell sets its own) does not move the service's socket away from `hive hook`.
pub fn user_temp_dir() -> Option<PathBuf> {
    let mut buf: [c_char; PATH_MAX] = [0; PATH_MAX];
    // SAFETY: `confstr` writes at most `buf.len()` bytes, NUL-terminated.
    let len = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr(), buf.len()) };
    confstr_path(len, &buf)
}

/// The path `confstr` answered in `buf`, `len` bytes with its NUL: none when it failed (0),
/// when the answer did not fit (more than `buf`: truncated), or when it is empty.
fn confstr_path(len: usize, buf: &[c_char]) -> Option<PathBuf> {
    if len == 0 || len > buf.len() {
        return None;
    }
    let bytes: Vec<u8> = buf[..len].iter().map(|&c| c as u8).collect();
    let text = CStr::from_bytes_until_nul(&bytes).ok()?;
    let path = PathBuf::from(OsStr::from_bytes(text.to_bytes()));
    (!path.as_os_str().is_empty()).then_some(path)
}

/// The user's login shell (`$SHELL`), started as [`crate::terminal::login::launch`] says.
pub fn shell(bin_dir: &Path) -> pty_process::Command {
    let var = std::env::var_os;
    let launch = login::launch(var("SHELL"), var("ZDOTDIR"), var("PATH"), bin_dir);
    pty_process::Command::new(launch.program)
        .args(launch.args)
        .envs(launch.env)
}

/// The user's login shell (`$SHELL`), as terminals start it, asked for its `PATH`.
pub fn path_shell() -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
    let shell = login::launch(std::env::var_os("SHELL"), None, None, Path::new(""));
    let print = crate::wrapper::PRINT_PATH;
    (shell.program, vec!["-l".into(), "-c".into(), print.into()])
}

/// The app runs beside the service: it opens `path` itself.
pub fn native_path(path: &Path, _wslpath: &OsStr) -> io::Result<String> {
    Ok(path.to_string_lossy().into_owned())
}

/// Renames `from` to `to` in one step, failing (`EEXIST`) when `to` exists:
/// `renamex_np(RENAME_EXCL)`.
pub fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    let c = |path: &Path| CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other);
    let (from, to) = (c(from)?, c(to)?);
    // SAFETY: both are NUL-terminated strings that live until the call returns.
    match unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
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
        // Not waited for yet: once it exits it stays a zombie.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        // Every line runs however fast the child exits, so coverage does not depend on timing.
        let exited = (0..1000).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            process(pid).is_none()
        });
        assert!(exited, "the child never exited");
        // Still there, as a zombie, until it is waited for.
        assert_eq!(nix::sys::signal::kill(Pid::from_raw(pid), None), Ok(()));
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_rename_never_replaces_an_entry() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str| dir.path().join(name);
        std::fs::create_dir(at("a")).unwrap();
        std::fs::create_dir(at("empty")).unwrap();
        std::fs::write(at("f"), "f").unwrap();
        for taken in ["empty", "f"] {
            let err = rename_new(&at("a"), &at(taken)).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{taken}");
        }
        rename_new(&at("a"), &at("b")).unwrap();
        assert!(at("b").is_dir() && !at("a").exists());
        assert!(rename_new(&at("b\0"), &at("c")).is_err());
        assert!(rename_new(&at("b"), &at("c\0")).is_err());
        assert!(at("b").is_dir());
    }

    #[test]
    fn the_user_temp_dir_is_private_to_the_user() {
        use std::os::unix::fs::MetadataExt;
        let dir = user_temp_dir().unwrap();
        let meta = std::fs::metadata(&dir).unwrap();
        assert!(dir.is_absolute() && meta.is_dir(), "{dir:?}");
        assert_eq!(meta.uid(), nix::unistd::getuid().as_raw(), "{dir:?}");
        assert_eq!(meta.mode() & 0o077, 0, "{dir:?}");
    }

    #[test]
    fn a_failed_or_truncated_confstr_answer_is_no_path() {
        let c = |text: &[u8]| text.iter().map(|&b| b as c_char).collect::<Vec<_>>();
        let buf = c(b"/T/\0");
        assert_eq!(confstr_path(4, &buf), Some(PathBuf::from("/T/")));
        assert_eq!(confstr_path(0, &buf), None);
        assert_eq!(confstr_path(5, &buf), None);
        assert_eq!(confstr_path(1, &c(b"\0")), None);
        assert_eq!(confstr_path(3, &c(b"/T/")), None);
        // Not UTF-8: kept byte for byte.
        let odd = confstr_path(3, &c(b"/\xff\0")).unwrap();
        assert_eq!(odd.as_os_str().as_bytes(), b"/\xff");
    }

    #[test]
    fn c_strings_end_at_the_first_nul() {
        let buf = [b'a' as c_char, b'b' as c_char, 0, b'c' as c_char];
        assert_eq!(text(&buf), Some("ab".to_owned()));
        assert_eq!(text(&[b'a' as c_char]), None);
    }
}
