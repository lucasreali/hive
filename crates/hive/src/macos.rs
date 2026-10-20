//! What only macOS needs, kept apart from the portable code: the kernel's process table
//! (through `libproc`, `proc_pidinfo` and the `KERN_PROCARGS2` sysctl), the terminal's login
//! shell and native file paths. Tested on macOS.

use std::ffi::{CStr, CString, OsStr, OsString, c_char, c_int};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libproc::bsd_info::BSDInfo;
use libproc::proc_pid::{PIDInfo, PidInfoFlavor, pidinfo};
use libproc::processes::{ProcFilter, pids_by_type};
use nix::unistd::{Pid, getsid};

use crate::procs::Proc;
use crate::terminal::{conpty, login};

/// `MAXPATHLEN`.
const PATH_MAX: usize = 1024;
/// `kern.argmax`: the most a process's arguments and environment take together.
const ARGS_SIZE: usize = 1024 * 1024;
/// `PROC_PIDFDVNODEPATHINFO`, which `libc` does not bind.
const VNODE_PATH_INFO: c_int = 2;

pub fn list() -> Vec<Proc> {
    let pids = pids_by_type(ProcFilter::All).unwrap_or_default();
    // One buffer for every process's arguments.
    let mut buf = vec![0; ARGS_SIZE];
    pids.into_iter()
        .filter_map(|pid| process(pid as i32, &mut buf))
        .collect()
}

/// The process `pid`, named as [`name`] says; `buf` holds its arguments while they are read.
fn process(pid: i32, buf: &mut [u8]) -> Option<Proc> {
    // Fails for a zombie too (ESRCH), so zombies are skipped.
    let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
    Some(Proc {
        pid,
        pgrp: info.pbi_pgid as i32,
        session: getsid(Some(Pid::from_raw(pid))).ok()?.as_raw(),
        comm: comm(procargs(pid, buf), &info.pbi_comm)?,
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
        // Named after `argv[0]`, whole (the kernel's name stops at 15 characters).
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy();
        assert_eq!(found.comm, name);
        assert_eq!(cwd(me), Some(std::env::current_dir().unwrap()));
        assert_eq!(cwd(-1), None);
        assert_eq!(process(-1, &mut vec![0; ARGS_SIZE]), None);
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
            process(pid, &mut buf).is_none()
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
