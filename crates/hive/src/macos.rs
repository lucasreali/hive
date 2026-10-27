//! What only macOS needs, kept apart from the portable code: the kernel's process table
//! (through `libproc`, `proc_pidinfo` and the `KERN_PROCARGS2` sysctl), the terminal's login
//! shell and native file paths. Tested on macOS.

use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr, OsString, c_char, c_int};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libproc::bsd_info::BSDInfo;
use libproc::proc_pid::{PIDInfo, PidInfoFlavor, pidinfo};
use libproc::processes::{ProcFilter, pids_by_type};
use nix::unistd::{Pid, getsid};

use crate::git::Resolved;
use crate::procs::Proc;
use crate::terminal::{conpty, login};

/// `MAXPATHLEN`.
const PATH_MAX: usize = 1024;
/// `kern.argmax`: the most a process's arguments and environment take together.
const ARGS_SIZE: usize = 1024 * 1024;
/// `PROC_PIDFDVNODEPATHINFO`, which `libc` does not bind.
const VNODE_PATH_INFO: c_int = 2;

pub fn list() -> Vec<Proc> {
    scan(|_| true, &mut Vec::new(), read_args)
}

/// The processes in `sessions`: only theirs get their arguments read, into `buf`, which the
/// caller keeps from one scan to the next (sized on first use).
pub fn in_sessions(sessions: &HashSet<i32>, buf: &mut Vec<u8>) -> Vec<Proc> {
    scan(|session| sessions.contains(&session), buf, read_args)
}

/// The processes whose session `keep` keeps, their arguments read by `read` into `buf`.
fn scan(
    keep: impl Fn(i32) -> bool,
    buf: &mut Vec<u8>,
    mut read: impl FnMut(i32, &mut [u8]) -> Option<usize>,
) -> Vec<Proc> {
    let pids = pids_by_type(ProcFilter::All).unwrap_or_default();
    // One buffer for every process's arguments.
    buf.resize(ARGS_SIZE, 0);
    let kept = |pid: i32| {
        let session = getsid(Some(Pid::from_raw(pid))).ok()?.as_raw();
        keep(session).then_some((pid, session))
    };
    (pids.into_iter().filter_map(|pid| kept(pid as i32)))
        .filter_map(|(pid, session)| process(pid, session, buf, &mut read))
        .collect()
}

/// How many bytes of `buf` the `KERN_PROCARGS2` answer for `pid` takes ([`procargs`]).
fn read_args(pid: i32, buf: &mut [u8]) -> Option<usize> {
    procargs(pid, buf).map(<[u8]>::len)
}

/// The process `pid` of `session`, named as [`name`] says; `buf` holds its arguments while
/// `read` reads them.
fn process(
    pid: i32,
    session: i32,
    buf: &mut [u8],
    read: &mut impl FnMut(i32, &mut [u8]) -> Option<usize>,
) -> Option<Proc> {
    // Fails for a zombie too (ESRCH), so zombies are skipped.
    let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
    let area = read(pid, buf).map(|len| &buf[..len]);
    Some(Proc {
        pid,
        pgrp: info.pbi_pgid as i32,
        session,
        comm: comm(area, &info.pbi_comm)?,
    })
}

/// A process's name: from its `KERN_PROCARGS2` answer `area` ([`name`]), else (unreadable)
/// the kernel's (`pbi_comm`).
fn comm(area: Option<&[u8]>, kernel: &[c_char]) -> Option<String> {
    (area.and_then(|area| name(&parse_args(area)?))).or_else(|| text(kernel))
}

/// A process's name from its arguments (`argv[0]` first): the last name of `argv[0]` as the
/// shell that started it wrote it, not the kernel's name, which is the resolved file's (a
/// native `claude` is a link to `versions/<x.y.z>`); `claude` for a `node` running Claude
/// Code's npm CLI ([`conpty::comm`]). `None` without arguments. Any process of the user sets
/// its `argv[0]`, and the name reaches the app ("in use by …"): it keeps [`NAME_LIMIT`]
/// characters at most, without control or bidirectional-override characters.
fn name(args: &[String]) -> Option<String> {
    let program = args.first()?.rsplit('/').next()?;
    let program: String = (program.chars())
        .filter(|&c| !c.is_control() && !bidi(c))
        .take(NAME_LIMIT)
        .collect();
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    Some(conpty::comm(&program, &args).to_owned())
}

/// Most characters of a process's name from its `argv[0]`.
const NAME_LIMIT: usize = 64;

/// A character that changes the direction of the text around it.
fn bidi(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The process's arguments, `argv[0]` first; `None` when unreadable (another user's process).
pub fn args(pid: i32) -> Option<Vec<String>> {
    procargs(pid, &mut vec![0; ARGS_SIZE]).and_then(parse_args)
}

/// What the `KERN_PROCARGS2` sysctl answers for `pid`, read into `buf`; `None` when it fails
/// or fills `buf` (an answer cut short keeps its end, not its start).
fn procargs(pid: i32, buf: &mut [u8]) -> Option<&[u8]> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut len = buf.len();
    let (name, out) = (mib.as_mut_ptr(), buf.as_mut_ptr().cast());
    // SAFETY: `name` holds the 3 names given; `out` is `len` writable bytes.
    let done = unsafe { libc::sysctl(name, 3, out, &mut len, std::ptr::null_mut(), 0) } == 0;
    (done && len < buf.len()).then(|| &buf[..len])
}

/// The arguments in a `KERN_PROCARGS2` answer: `argc`, the executable's path, NUL padding,
/// then `argc` NUL-terminated arguments (the environment follows). Empty arguments are left
/// out, as on Linux (`node` blanks them when it sets its title).
fn parse_args(area: &[u8]) -> Option<Vec<String>> {
    let (argc, rest) = area.split_first_chunk::<4>()?;
    let argc = usize::try_from(i32::from_ne_bytes(*argc)).ok()?;
    let rest = &rest[rest.iter().position(|&b| b == 0)?..];
    let rest = &rest[rest.iter().position(|&b| b != 0)?..];
    let words = rest.split(|&b| b == 0).take(argc);
    let words = words.filter(|word| !word.is_empty());
    Some(
        words
            .map(|w| String::from_utf8_lossy(w).into_owned())
            .collect(),
    )
}

/// `struct vnode_fdinfowithpath`: a `struct proc_fileinfo` (24 bytes) and a
/// [`VnodeInfoPath`].
#[repr(C)]
struct VnodeFdPath {
    file: [u8; 24],
    vnode: VnodeInfoPath,
}

/// What the process's open files point at, of its first `limit` descriptors.
pub fn open_files(pid: i32, limit: usize) -> Vec<PathBuf> {
    // `struct proc_fdinfo`s: a descriptor and its type.
    let size = std::mem::size_of::<libc::proc_fdinfo>();
    let mut buf = vec![0u8; limit * size];
    let bytes = c_int::try_from(buf.len()).unwrap_or(c_int::MAX);
    let out = buf.as_mut_ptr().cast();
    // SAFETY: `out` is at least `bytes` writable bytes.
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, out, bytes) };
    let listed = buf
        .get(..usize::try_from(got).unwrap_or(0))
        .unwrap_or_default();
    let vnode = |entry: &[u8]| {
        let (fd, kind) = entry.split_first_chunk::<4>()?;
        let kind = u32::from_ne_bytes(kind.try_into().ok()?);
        (kind == libc::PROX_FDTYPE_VNODE as u32).then(|| i32::from_ne_bytes(*fd))
    };
    let fds = listed.chunks_exact(size).filter_map(vnode);
    fds.filter_map(|fd| vnode_path(pid, fd)).collect()
}

/// The path of the file that descriptor `fd` of `pid` points at.
fn vnode_path(pid: i32, fd: i32) -> Option<PathBuf> {
    let mut info = VnodeFdPath {
        file: [0; 24],
        vnode: VnodeInfoPath {
            info: [0; 152],
            path: [0; PATH_MAX],
        },
    };
    let size = std::mem::size_of::<VnodeFdPath>() as c_int;
    let out = (&raw mut info).cast();
    // SAFETY: `out` is a `struct vnode_fdinfowithpath`, `size` bytes.
    let got = unsafe { libc::proc_pidfdinfo(pid, fd, VNODE_PATH_INFO, out, size) };
    if got != size {
        return None;
    }
    text(&info.vnode.path).map(PathBuf::from)
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

/// The command that prints the system locale (D18.2), `AppleLocale` in the global
/// preferences (e.g. `en_BR`): `defaults read -g AppleLocale`.
pub fn apple_locale() -> std::process::Command {
    let mut defaults = std::process::Command::new("/usr/bin/defaults");
    defaults.args(["read", "-g", "AppleLocale"]);
    defaults
}

/// How long `defaults` may take to print the system locale; past that there is none.
pub const LOCALE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// The most `defaults` may print for it (a locale name is a few bytes); more is none.
pub const LOCALE_LIMIT: u64 = 256;

/// The system locale as `command` ([`apple_locale`]) prints it, trimmed, when it succeeds
/// within `time` with something printed, at most [`LOCALE_LIMIT`] bytes. A command that takes
/// longer is killed, with all it started.
pub fn system_locale(command: std::process::Command, time: std::time::Duration) -> Option<String> {
    let keep = crate::git::Stdout::Max(LOCALE_LIMIT);
    let out = crate::git::limited(command, "defaults", &[], &[], &[0], keep, Some(time)).ok()?;
    let text = String::from_utf8(out).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The command that prints where Apple's git is (D18.3): `xcrun -f git`, what the
/// `/usr/bin/git` stub asks at each call.
pub fn xcrun_git() -> std::process::Command {
    let mut xcrun = std::process::Command::new("/usr/bin/xcrun");
    xcrun.args(["-f", "git"]);
    xcrun
}

/// How long `xcrun` may take to find git; past that git is looked for on `PATH`.
pub const XCRUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// The most `xcrun` may print for it (a path).
pub const XCRUN_LIMIT: u64 = 4096;

/// What xcrun prints when the Command Line Tools (or Xcode) are missing.
const NO_TOOLS: [&str; 2] = [
    "invalid active developer path",
    "no developer tools were found",
];

/// The git `command` ([`xcrun_git`]) finds within `time`: the absolute path it prints;
/// [`Resolved::Missing`] when it fails saying Apple's tools are missing; otherwise (any other
/// failure, a relative path, more than [`XCRUN_LIMIT`] bytes) `git` on `PATH`, as before.
pub fn apple_git(command: std::process::Command, time: std::time::Duration) -> Resolved {
    let keep = crate::git::Stdout::Max(XCRUN_LIMIT);
    match crate::git::limited(command, "xcrun", &[], &[], &[0], keep, Some(time)) {
        Ok(out) => {
            let git = PathBuf::from(OsStr::from_bytes(out.trim_ascii()));
            if git.is_absolute() {
                Resolved::Apple(git)
            } else {
                Resolved::OnPath
            }
        }
        Err(err) => {
            let err = err.to_string().to_lowercase();
            if NO_TOOLS.iter().any(|no| err.contains(no)) {
                Resolved::Missing
            } else {
                Resolved::OnPath
            }
        }
    }
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

/// What `statfs` says of a folder's volume: its mount flags and its file system type.
pub type Volume = (u32, String);

/// The volume holding `path` (`statfs`).
pub fn volume(path: &Path) -> io::Result<Volume> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // SAFETY: an all-zero `statfs` is valid; it is only written by the call.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated and `stat` lives until the call returns.
    match unsafe { libc::statfs(path.as_ptr(), &mut stat) } {
        0 => Ok((stat.f_flags, text(&stat.f_fstypename).unwrap_or_default())),
        _ => Err(io::Error::last_os_error()),
    }
}

/// Why a project in `path` would be followed badly (D18.9), or none: a network volume
/// (`statfs` without `MNT_LOCAL`, or `smbfs`/`nfs`/`webdav`) or a cloud folder (iCloud Drive,
/// `~/Library/CloudStorage`), where FSEvents misses changes made elsewhere and git can hit its
/// time limit. Hive still adds it; the app shows this as a warning.
pub fn remote(path: &Path, home: Option<&Path>, volume: Option<&Volume>) -> Option<String> {
    const CLOUD: [&str; 2] = ["Library/Mobile Documents", "Library/CloudStorage"];
    const WHY: &str = "Hive may miss changes made elsewhere, and git may be slow";
    let cloud = home.is_some_and(|home| CLOUD.iter().any(|dir| path.starts_with(home.join(dir))));
    if cloud {
        return Some(format!("This folder is in cloud storage: {WHY}."));
    }
    let (flags, kind) = volume?;
    let network = ["smbfs", "nfs", "webdav"].contains(&kind.as_str());
    (network || flags & libc::MNT_LOCAL as u32 == 0)
        .then(|| format!("This folder is on a network volume ({kind}): {WHY}."))
}

/// [`remote`] for the folder at `path`, a link judged by where it leads (e.g. to iCloud Drive).
pub fn folder_warning(path: &Path, home: Option<&Path>) -> Option<String> {
    // The listed folder exists; one gone meanwhile has nothing to warn about.
    let path = std::fs::canonicalize(path).ok()?;
    remote(&path, home, volume(&path).ok().as_ref())
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

pub use trash::trash;

/// The Trash (D18.10e): `-[NSFileManager trashItemAtURL:resultingItemURL:error:]`, called
/// through the Objective-C runtime's C functions. The entry goes to the Trash of its volume:
/// `~/.Trash` on the home's, `<volume>/.Trashes/<uid>` on another.
mod trash {
    use std::ffi::{c_char, c_void};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    type Id = *mut c_void;
    type Sel = *const c_void;

    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    /// Most bytes of the error's description kept.
    const DESCRIPTION: usize = 1024;

    // Links Foundation, where `NSFileManager` lives.
    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            path: *const u8,
            len: isize,
            folder: u8,
        ) -> Id;
        fn CFErrorCopyDescription(error: Id) -> Id;
        fn CFStringGetCString(string: Id, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    /// Moves `at` (a file, a folder with what it holds, or a symlink itself) to the Trash; the
    /// error says why not, as Foundation words it.
    pub fn trash(at: &Path) -> io::Result<()> {
        let path = at.as_os_str().as_bytes();
        let failed = "cannot move it to the Trash";
        // SAFETY: `path` is `len` readable bytes; the default allocator (null) copies them.
        let url = unsafe {
            CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                path.as_ptr(),
                path.len() as isize,
                0,
            )
        };
        let url = (!url.is_null())
            .then_some(url)
            .ok_or(io::Error::other(failed))?;
        let mut error: Id = std::ptr::null_mut();
        // SAFETY: the pool holds the autoreleased manager and error until popped, once, after
        // the error was read; `url` is a live `CFURL` (toll-free bridged to `NSURL`), owned and
        // released once; `error` is a valid out pointer; each message is sent with the types
        // its method declares.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let manager: unsafe extern "C" fn(Id, Sel) -> Id =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let manager = manager(
                objc_getClass(c"NSFileManager".as_ptr()),
                sel_registerName(c"defaultManager".as_ptr()),
            );
            let trash: unsafe extern "C" fn(Id, Sel, Id, *mut Id, *mut Id) -> i8 =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let sel = sel_registerName(c"trashItemAtURL:resultingItemURL:error:".as_ptr());
            let done = trash(manager, sel, url, std::ptr::null_mut(), &mut error) != 0;
            let result = (done.then_some(()))
                .ok_or_else(|| io::Error::other(describe(error).unwrap_or(failed.to_owned())));
            CFRelease(url);
            objc_autoreleasePoolPop(pool);
            result
        }
    }

    /// The description of the `NSError` (toll-free bridged to `CFError`) `error`, if any.
    ///
    /// # Safety
    /// `error` is null or a live error.
    unsafe fn describe(error: Id) -> Option<String> {
        let mut buf = [0 as c_char; DESCRIPTION];
        // SAFETY: as the caller says; the description is owned and released once; `buf` is
        // `DESCRIPTION` writable bytes.
        unsafe {
            let text = (!error.is_null()).then(|| CFErrorCopyDescription(error));
            let text = text.filter(|text| !text.is_null())?;
            let done = CFStringGetCString(text, buf.as_mut_ptr(), DESCRIPTION as isize, UTF8);
            CFRelease(text);
            (done != 0).then(|| super::text(&buf)).flatten()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_and_cloud_folders_get_a_warning() {
        let home = Path::new("/Users/me");
        let local = (libc::MNT_LOCAL as u32 | 1, "apfs".to_owned());
        let at = |path: &str, volume: Option<&Volume>| remote(Path::new(path), Some(home), volume);
        assert_eq!(at("/Users/me/src/shop", Some(&local)), None);
        assert_eq!(at("/Users/me/src/shop", None), None);
        for kind in ["smbfs", "nfs", "webdav"] {
            // Named a network type even if it claimed to be local.
            let volume = (libc::MNT_LOCAL as u32, kind.to_owned());
            let warning = at("/Volumes/share/shop", Some(&volume)).unwrap();
            assert!(
                warning.contains(&format!("network volume ({kind})")),
                "{warning}"
            );
        }
        let fuse = (0, "macfuse".to_owned());
        assert!(at("/Volumes/x", Some(&fuse)).unwrap().contains("(macfuse)"));
        for cloud in [
            "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/shop",
            "/Users/me/Library/CloudStorage/Dropbox/shop",
        ] {
            let warning = at(cloud, Some(&local)).unwrap();
            assert!(warning.contains("cloud storage"), "{warning}");
            assert!(warning.contains("may miss changes"), "{warning}");
        }
        // Another user's, or with no home known: only the volume says.
        let theirs = "/Users/other/Library/CloudStorage/Dropbox";
        assert_eq!(at(theirs, Some(&local)), None);
        let cloud = "/Users/me/Library/CloudStorage/Dropbox";
        assert_eq!(remote(Path::new(cloud), None, Some(&local)), None);
    }

    /// Only a failure: a success moves to a Trash, which the integration test does on a disk
    /// image of its own, never the user's `~/.Trash`.
    #[test]
    fn what_cannot_go_to_the_trash_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let err = trash(&dir.path().join("hive-missing"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("hive-missing"), "{err}");
        assert_ne!(err, "cannot move it to the Trash");
        assert!(dir.path().read_dir().unwrap().next().is_none());
    }

    #[test]
    fn the_volume_is_what_statfs_says() {
        let (flags, kind) = volume(Path::new("/")).unwrap();
        assert_ne!(flags & libc::MNT_LOCAL as u32, 0);
        assert_eq!(kind, "apfs");
        assert!(volume(Path::new("/nonexistent/hive")).is_err());
        assert!(volume(Path::new("/a\0b")).is_err());
        assert_eq!(folder_warning(Path::new("/nonexistent/hive"), None), None);
    }

    #[test]
    fn the_system_locale_is_what_the_command_prints() {
        let sh = |script: &str| {
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-c", script]);
            system_locale(command, LOCALE_TIMEOUT)
        };
        assert_eq!(sh("echo ' pt_BR'").as_deref(), Some("pt_BR"));
        assert_eq!(sh("echo"), None);
        assert_eq!(sh("echo pt_BR; exit 1"), None);
        assert_eq!(sh(r"printf '\377'"), None);
        let missing = std::process::Command::new("/nonexistent/defaults");
        assert_eq!(system_locale(missing, LOCALE_TIMEOUT), None);
        // At most `LOCALE_LIMIT` bytes: more is none.
        let fits = format!("printf '%{LOCALE_LIMIT}s' x");
        assert_eq!(sh(&fits).map(|text| text.len()), Some(1));
        assert_eq!(sh(&format!("printf '%{}s' x", LOCALE_LIMIT + 1)), None);
    }

    #[test]
    fn a_locale_command_that_hangs_is_none_in_time() {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "echo pt_BR; sleep 30"]);
        let start = std::time::Instant::now();
        let time = std::time::Duration::from_millis(200);
        assert_eq!(system_locale(command, time), None);
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(LOCALE_TIMEOUT, std::time::Duration::from_secs(2));
    }

    /// [`apple_git`] of a shell `script` standing for `xcrun -f git`.
    fn xcrun(script: &str) -> Resolved {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", script]);
        apple_git(command, XCRUN_TIMEOUT)
    }

    #[test]
    fn xcrun_names_the_git_to_run() {
        let tools = "/Library/Developer/CommandLineTools/usr/bin/git";
        let found = Resolved::Apple(PathBuf::from(tools));
        assert_eq!(xcrun(&format!("echo ' {tools}'")), found);
        // Anything but an absolute path, or a failure other than missing tools: git on `PATH`.
        assert_eq!(xcrun("echo git"), Resolved::OnPath);
        assert_eq!(xcrun("echo /x/git; exit 1"), Resolved::OnPath);
        assert_eq!(
            xcrun("echo 'xcrun: error: unable to find utility' >&2; exit 72"),
            Resolved::OnPath
        );
        let fits = format!("printf '/%{}s' x", XCRUN_LIMIT - 1);
        assert!(matches!(xcrun(&fits), Resolved::Apple(_)));
        let over = format!("printf '/%{}s' x", XCRUN_LIMIT);
        assert_eq!(xcrun(&over), Resolved::OnPath);
        let missing = std::process::Command::new("/nonexistent/xcrun");
        assert_eq!(apple_git(missing, XCRUN_TIMEOUT), Resolved::OnPath);
    }

    #[test]
    fn xcrun_without_the_tools_means_no_git() {
        // What xcrun prints on a Mac without the Command Line Tools.
        let error = "xcrun: error: invalid active developer path \
            (/Library/Developer/CommandLineTools), missing xcrun at: \
            /Library/Developer/CommandLineTools/usr/bin/xcrun";
        assert_eq!(
            xcrun(&format!("echo '{error}' >&2; exit 1")),
            Resolved::Missing
        );
        let note = "xcode-select: note: No developer tools were found, requesting install.";
        assert_eq!(
            xcrun(&format!("echo '{note}' >&2; exit 1")),
            Resolved::Missing
        );
    }

    #[test]
    fn xcrun_that_hangs_is_given_up_in_time() {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "echo /x/git; sleep 30"]);
        let start = std::time::Instant::now();
        let time = std::time::Duration::from_millis(200);
        assert_eq!(apple_git(command, time), Resolved::OnPath);
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(XCRUN_TIMEOUT, std::time::Duration::from_secs(2));
        let command = xcrun_git();
        assert_eq!(command.get_program(), "/usr/bin/xcrun");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["-f", "git"]);
    }

    #[test]
    fn the_system_locale_is_apples_global_preference() {
        let command = apple_locale();
        assert_eq!(command.get_program(), "/usr/bin/defaults");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["read", "-g", "AppleLocale"]);
    }

    #[test]
    fn this_process_is_listed_with_its_group_session_and_folder() {
        let me = std::process::id() as i32;
        let found = list().into_iter().find(|p| p.pid == me).unwrap();
        assert_eq!(found.pgrp, nix::unistd::getpgrp().as_raw());
        assert_eq!(found.session, getsid(None).unwrap().as_raw());
        // Named after `argv[0]`, whole (the kernel's name stops at 15 characters).
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy();
        assert_eq!(found.comm, name);
        assert_eq!(cwd(me), Some(std::env::current_dir().unwrap()));
        assert_eq!(cwd(-1), None);
        assert_eq!(
            process(-1, 0, &mut vec![0; ARGS_SIZE], &mut read_args),
            None
        );
    }

    #[test]
    fn a_scan_reads_only_the_kept_sessions_arguments_into_one_buffer() {
        let me = std::process::id() as i32;
        let mine = getsid(None).unwrap().as_raw();
        let mut buf = Vec::new();
        let mut reads = Vec::new();
        let mut scans = Vec::new();
        for _ in 0..2 {
            let read = |pid, buf: &mut [u8]| {
                reads.push((pid, buf.as_ptr()));
                read_args(pid, buf)
            };
            scans.push(scan(|session| session == mine, &mut buf, read));
        }
        // Read once per process of the session, each scan, all into the buffer kept.
        let pids: Vec<i32> = scans.iter().flatten().map(|p| p.pid).collect();
        assert_eq!(reads.iter().map(|r| r.0).collect::<Vec<_>>(), pids);
        assert!(reads.iter().all(|r| r.1 == buf.as_ptr()));
        assert!(scans.iter().all(|s| s.iter().any(|p| p.pid == me)));
        assert!(scans.iter().flatten().all(|p| p.session == mine));
        // Other sessions' processes, which a whole listing has, are left out unread.
        let others: Vec<i32> = (list().into_iter())
            .filter(|p| p.session != mine)
            .map(|p| p.pid)
            .collect();
        assert!(!others.is_empty());
        assert!(!pids.iter().any(|pid| others.contains(pid)));
        let kept = in_sessions(&HashSet::from([mine]), &mut buf);
        assert!(kept.iter().any(|p| p.pid == me));
        assert_eq!(in_sessions(&HashSet::new(), &mut buf), Vec::new());
    }

    #[test]
    fn arguments_are_read_from_the_kernel() {
        let me = std::process::id() as i32;
        assert_eq!(args(me), Some(std::env::args().collect()));
        assert_eq!(args(-1), None);
        // An answer that fills the buffer may be cut short: refused.
        assert_eq!(procargs(me, &mut [0; 16]), None);
    }

    #[test]
    fn a_process_without_readable_arguments_has_the_kernels_name() {
        let kernel = [b'9' as c_char, b'.' as c_char, b'9' as c_char, 0];
        let read = area(1, b"/v/9.9\0\0claude\0E=1\0");
        assert_eq!(comm(Some(&read), &kernel).as_deref(), Some("claude"));
        assert_eq!(comm(None, &kernel).as_deref(), Some("9.9"));
        // No argument at all, or not even an answer.
        assert_eq!(
            comm(Some(&area(0, b"/v\0\0")), &kernel).as_deref(),
            Some("9.9")
        );
        assert_eq!(comm(Some(b"\0"), &kernel).as_deref(), Some("9.9"));
        assert_eq!(comm(None, &[]), None);
    }

    /// A `KERN_PROCARGS2` answer: `argc`, then `rest`.
    fn area(argc: i32, rest: &[u8]) -> Vec<u8> {
        [&argc.to_ne_bytes()[..], rest].concat()
    }

    #[test]
    fn arguments_follow_the_executables_path_and_precede_the_environment() {
        let parse = |argc, rest: &[u8]| parse_args(&area(argc, rest));
        let words = |words: &[&str]| Some(words.iter().map(|w| w.to_string()).collect());
        let found = parse(2, b"/bin/x\0\0\0x\0-a\0HOME=/h\0");
        assert_eq!(found, words(&["x", "-a"]));
        // `node` blanks its arguments when it sets its title.
        assert_eq!(parse(3, b"/n\0claude\0\0\0E=1\0"), words(&["claude"]));
        assert_eq!(parse(0, b"/n\0\0E=1\0"), words(&[]));
        // Not an answer: too short, a negative count, a path without its end, nothing after it.
        assert_eq!(parse_args(b"\x01\0\0"), None);
        assert_eq!(parse(-1, b"/n\0n\0"), None);
        assert_eq!(parse(1, b"/n"), None);
        assert_eq!(parse(1, b"/n\0\0\0"), None);
    }

    #[test]
    fn a_process_is_named_after_argv0_or_claude_codes_npm_cli() {
        let name = |args: &[&str]| name(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(
            name(&["/u/.local/bin/claude", "-r"]).as_deref(),
            Some("claude")
        );
        assert_eq!(name(&["-fish"]).as_deref(), Some("-fish"));
        let cli = "/u/lib/node_modules/@anthropic-ai/claude-code/cli.js";
        assert_eq!(name(&["node", cli]).as_deref(), Some("claude"));
        assert_eq!(name(&["node", "x.js"]).as_deref(), Some("node"));
        assert_eq!(name(&[]), None);
        // Another process's `argv[0]` is untrusted: cut to 64 characters (on a character),
        // control and direction characters dropped.
        let long = "é".repeat(100_000);
        assert_eq!(name(&[&long]), Some("é".repeat(64)));
        let odd = "/b/cl\u{1b}[2Ja\nu\u{202E}d\u{2066}e\u{200F}\u{061C}\u{200E}\u{2069}\u{7f}";
        assert_eq!(name(&[odd]).as_deref(), Some("cl[2Jaude"));
        let edges = "a\u{2029}\u{202A}\u{202D}\u{2067}\u{2068}\u{2065}\u{206A}\u{2029}";
        assert_eq!(
            name(&[edges]),
            Some("a\u{2029}\u{2065}\u{206A}\u{2029}".into())
        );
    }

    /// The tests' stand-in for Claude Code (`tests/fixtures/fake_claude.c`), built at `path`
    /// with `cc`: macOS 26 kills a copied system binary run from another folder.
    fn build_fake(path: &Path) {
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_claude.c");
        let cc = std::process::Command::new("cc")
            .arg("-o")
            .arg(path)
            .arg(source)
            .status();
        assert!(cc.unwrap().success());
    }

    #[test]
    fn every_claude_install_is_listed_as_claude_with_its_folder_and_session() {
        use crate::procs::{Source, claude_sessions, inside};
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        // The kernel names folders through `/private`.
        let at = |sub: &str| dir.path().canonicalize().unwrap().join(sub);
        // The native installer's `claude` links to `versions/<x.y.z>`; npm's runs `node`.
        std::fs::create_dir(at("versions")).unwrap();
        build_fake(&at("versions/9.9.9"));
        std::os::unix::fs::symlink(at("versions/9.9.9"), at("claude")).unwrap();
        build_fake(&at("node"));
        std::fs::create_dir(at("work")).unwrap();
        let (native_id, npm_id) = (
            "00000000-0000-0000-0000-000000000001",
            "00000000-0000-0000-0000-000000000002",
        );
        let log = at(&format!("work/{npm_id}.jsonl"));
        std::fs::write(&log, "").unwrap();
        let spawn = |program: PathBuf, args: &[&Path]| {
            (Command::new(program).args(args).current_dir(at("work")))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        };
        let native = ["--session-id", native_id].map(Path::new);
        let mut native = spawn(at("claude"), &native);
        let cli = at("lib/node_modules/@anthropic-ai/claude-code/cli.js");
        let mut npm = spawn(at("node"), &[&cli, &log]);
        let (native_pid, npm_pid) = (native.id() as i32, npm.id() as i32);
        // The kernel names them after their files.
        let kernel = |pid| text(&pidinfo::<BSDInfo>(pid, 0).unwrap().pbi_comm).unwrap();
        assert_eq!(
            (kernel(native_pid), kernel(npm_pid)),
            ("9.9.9".into(), "node".into())
        );
        // `npm` opens its log once started.
        let opened = (0..500).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            open_files(npm_pid, 64).contains(&log)
        });
        assert!(opened, "the log was never opened");

        let mut found: Vec<_> = inside(Source::System, &at("work"))
            .into_iter()
            .map(|p| (p.pid, p.comm))
            .collect();
        found.sort();
        let mut expected = vec![(native_pid, "claude".into()), (npm_pid, "claude".into())];
        expected.sort();
        assert_eq!(found, expected);
        // The native one names its session in its arguments, npm's has its log open.
        let sessions = claude_sessions(Source::System, &[]);
        assert!(sessions.contains(native_id), "{sessions:?}");
        assert!(sessions.contains(npm_id), "{sessions:?}");

        // Its first three descriptors are the null device's (more may be inherited from a test
        // runner, e.g. a jobserver's pipe, before the log).
        let null = PathBuf::from("/dev/null");
        let three = vec![null.clone(), null.clone(), null];
        assert_eq!(open_files(npm_pid, 3), three);
        assert_eq!(open_files(npm_pid, 0), Vec::<PathBuf>::new());
        assert_eq!(open_files(-1, 4), Vec::<PathBuf>::new());
        assert_eq!(vnode_path(npm_pid, 99), None);
        for child in [&mut native, &mut npm] {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }

    #[test]
    fn zombies_are_not_listed() {
        // Not waited for yet: once it exits it stays a zombie.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        let mut buf = vec![0; ARGS_SIZE];
        // Every line runs however fast the child exits, so coverage does not depend on timing.
        let exited = (0..1000).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            process(pid, 0, &mut buf, &mut read_args).is_none()
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
