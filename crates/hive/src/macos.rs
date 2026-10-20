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

/// The power assertion that keeps the Mac awake while agents work (17.17), as `caffeinate -i`:
/// `IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep)`. The display
/// may still sleep, and closing the lid on battery still sleeps the Mac.
pub mod power {
    use std::ffi::{OsString, c_void};

    use crate::power::Power;

    type CfStringRef = *const c_void;
    type AssertionId = u32;
    type IoReturn = i32;

    /// `kIOPMAssertionTypePreventUserIdleSystemSleep` (a `CFSTR` macro in `IOPMLib.h`).
    const PREVENT_IDLE_SLEEP: &str = "PreventUserIdleSystemSleep";
    /// Shown by `pmset -g assertions` next to the service.
    const NAME: &str = "Hive: agents are working";
    /// `kIOPMAssertionLevelOn`.
    const LEVEL_ON: u32 = 255;
    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    /// `kIOReturnSuccess`.
    const SUCCESS: IoReturn = 0;
    /// `kIOReturnNoMemory`.
    const NO_MEMORY: IoReturn = 0xe000_02bd_u32 as IoReturn;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CfStringRef,
            level: u32,
            name: CfStringRef,
            id: *mut AssertionId,
        ) -> IoReturn;
        fn IOPMAssertionRelease(id: AssertionId) -> IoReturn;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            allocator: *const c_void,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CfStringRef;
        fn CFRelease(cf: *const c_void);
    }

    /// An owned `CFString` (never null), released on drop.
    struct CfString(CfStringRef);

    impl CfString {
        /// `None` when Core Foundation could not make it (out of memory).
        fn new(text: &str) -> Option<Self> {
            let len = text.len() as isize;
            // SAFETY: `text` is valid UTF-8 of `len` bytes; the default allocator (null)
            // copies it.
            let string =
                unsafe { CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), len, UTF8, 0) };
            (!string.is_null()).then_some(Self(string))
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            // SAFETY: an owned, non-null reference from `CFStringCreateWithBytes`, released
            // once.
            unsafe { CFRelease(self.0) }
        }
    }

    /// Takes an assertion of `kind`: its id, or IOKit's error code.
    fn create(kind: &str) -> Result<AssertionId, IoReturn> {
        let kind = CfString::new(kind).ok_or(NO_MEMORY)?;
        let name = CfString::new(NAME).ok_or(NO_MEMORY)?;
        let mut id: AssertionId = 0;
        // SAFETY: both strings live until the call returns; `id` is a valid out pointer.
        match unsafe { IOPMAssertionCreateWithName(kind.0, LEVEL_ON, name.0, &mut id) } {
            SUCCESS => Ok(id),
            code => Err(code),
        }
    }

    /// Gives back the assertion `id`.
    fn release(id: AssertionId) -> Result<(), IoReturn> {
        // SAFETY: a plain integer argument; an id this process does not hold is refused.
        match unsafe { IOPMAssertionRelease(id) } {
            SUCCESS => Ok(()),
            code => Err(code),
        }
    }

    /// The service's assertion, released on drop.
    pub struct Assertion {
        kind: &'static str,
        id: Option<AssertionId>,
    }

    impl Power for Assertion {
        fn hold(&mut self) {
            match create(self.kind) {
                Ok(id) => self.id = Some(id),
                Err(code) => {
                    eprintln!("hive: warning: cannot keep the Mac awake (IOKit {code:#x})")
                }
            }
        }

        fn release(&mut self) {
            if let Some(id) = self.id.take()
                && let Err(code) = release(id)
            {
                eprintln!("hive: warning: cannot let the Mac sleep again (IOKit {code:#x})");
            }
        }
    }

    impl Drop for Assertion {
        fn drop(&mut self) {
            self.release();
        }
    }

    /// The service's power assertion, not yet taken; none when `off` (the variable
    /// [`crate::power::OFF`]) is set.
    pub fn system(off: Option<OsString>) -> Option<Box<dyn Power>> {
        let assertion = Assertion {
            kind: PREVENT_IDLE_SLEEP,
            id: None,
        };
        off.is_none().then(|| Box::new(assertion) as Box<dyn Power>)
    }

    /// The only tests that take a real assertion, for a moment. One test, so no other test's
    /// assertion can take an id it released.
    #[cfg(test)]
    mod tests {
        use super::*;

        /// IOKit's answer for an id it does not hold (`kIOReturnBadArgument`).
        const UNKNOWN: IoReturn = 0xe000_02c2_u32 as IoReturn;

        unsafe extern "C" {
            fn CFRetain(cf: *const c_void) -> *const c_void;
            fn CFGetRetainCount(cf: *const c_void) -> isize;
        }

        #[test]
        fn assertions_are_taken_and_released_once() {
            let id = create(PREVENT_IDLE_SLEEP).unwrap();
            assert_ne!(id, 0);
            assert_eq!(release(id), Ok(()));
            assert_eq!(release(id), Err(UNKNOWN));
            assert!(create("NotAnAssertionType").is_err());
            // The service's.
            assert!(system(Some("1".into())).is_none());
            let mut power = system(None).unwrap();
            power.hold();
            power.release();
            power.release();
            let mut assertion = Assertion {
                kind: PREVENT_IDLE_SLEEP,
                id: None,
            };
            assertion.hold();
            let id = assertion.id.unwrap();
            assertion.release();
            assert_eq!((assertion.id, release(id)), (None, Err(UNKNOWN)));
            assertion.hold();
            let id = assertion.id.unwrap();
            drop(assertion);
            assert_eq!(release(id), Err(UNKNOWN));
        }

        #[test]
        fn a_string_is_released_once() {
            // Longer than a tagged pointer holds, so it is a counted object.
            let string = CfString::new(NAME).unwrap();
            // SAFETY: a live CFString, retained once more and released below.
            let raw = unsafe { CFRetain(string.0) };
            drop(string);
            // SAFETY: still alive through the extra reference.
            assert_eq!(unsafe { CFGetRetainCount(raw) }, 1);
            // SAFETY: the extra reference, released once.
            unsafe { CFRelease(raw) }
        }

        #[test]
        fn a_refused_assertion_or_release_is_only_reported() {
            let mut refused = Assertion {
                kind: "NotAnAssertionType",
                id: None,
            };
            refused.hold();
            assert_eq!(refused.id, None);
            // 0 is `kIOPMNullAssertionID`, never a real one.
            let mut unknown = Assertion {
                kind: PREVENT_IDLE_SLEEP,
                id: Some(0),
            };
            unknown.release();
            assert_eq!(unknown.id, None);
        }
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
    fn c_strings_end_at_the_first_nul() {
        let buf = [b'a' as c_char, b'b' as c_char, 0, b'c' as c_char];
        assert_eq!(text(&buf), Some("ab".to_owned()));
        assert_eq!(text(&[b'a' as c_char]), None);
    }
}
