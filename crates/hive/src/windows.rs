//! What only native Windows needs, kept apart from the portable code (12.5): the service's
//! named pipe and its checks, how the bridge starts the service, the clock, files (a rename
//! that never replaces, a file another program holds, the drives), and what is not supported
//! yet ([`unsupported`]). Tested on the Windows CI runner (`windows.yml`).

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::ops::BitOr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use hive_protocol::Dir;
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{
    ERROR_LOCK_VIOLATION, ERROR_PIPE_BUSY, ERROR_SHARING_VIOLATION, HANDLE, LocalFree, WIN32_ERROR,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{GetLogicalDrives, MoveFileExW};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, OpenProcess,
    OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::core::BOOL;

use crate::paths::Paths;
use crate::procs::Proc;

/// How long a client waits for a free instance of the service's pipe (an instance serves one
/// connection, and the service makes the next one right after).
const BUSY_TIME: Duration = Duration::from_secs(2);
/// How often a client tries a busy pipe again.
const BUSY_RETRY: Duration = Duration::from_millis(10);

/// The error of what Hive cannot do on Windows yet.
pub fn unsupported(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not supported on Windows yet"),
    )
}

/// Data in `%LOCALAPPDATA%\hive`, settings in `%APPDATA%\hive` (under `%USERPROFILE%` when
/// unset, else the temporary folder). The lock and the service's log go in the data's `run`.
pub fn paths(var: impl Fn(&str) -> Option<OsString>) -> Paths {
    let var = |key| {
        var(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let profile = |under| var("USERPROFILE").map(|home| home.join(under));
    let local = var("LOCALAPPDATA").or_else(|| profile(r"AppData\Local"));
    let data = local.unwrap_or_else(std::env::temp_dir).join("hive");
    let config = var("APPDATA")
        .or_else(|| profile(r"AppData\Roaming"))
        .map_or_else(|| data.join("config"), |dir| dir.join("hive"));
    Paths {
        runtime: data.join("run"),
        data,
        config,
    }
}

impl Paths {
    /// [`paths`] from this process's environment.
    pub fn from_env() -> Self {
        paths(|key| std::env::var_os(key))
    }

    /// Creates the folder of the lock and the log. It gets the ACL of the user's local app
    /// data, which only the user (and administrators) can open.
    pub fn prepare_runtime(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.runtime)
    }

    /// Connects to the service's pipe, only to a service run by this user: anyone may create
    /// a pipe of this name first, so the serving process's user is checked (as the socket's
    /// peer on Unix).
    pub async fn connect(&self) -> io::Result<NamedPipeClient> {
        let user = process_user(std::process::id())?;
        let name = pipe_name(&user, &self.runtime);
        let open = async {
            loop {
                match ClientOptions::new().open(&name) {
                    Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                        tokio::time::sleep(BUSY_RETRY).await;
                    }
                    opened => return opened,
                }
            }
        };
        let busy = |_| io::Error::new(io::ErrorKind::TimedOut, "the hive pipe stayed busy");
        let pipe = tokio::time::timeout(BUSY_TIME, open)
            .await
            .map_err(busy)??;
        check_owner(&pipe, &user)?;
        Ok(pipe)
    }
}

/// The service's pipe for `user` (a SID) and its data's `runtime` folder: one service per
/// user and data folder, as one socket per runtime folder on Unix (a test's service never
/// meets the real one).
fn pipe_name(user: &str, runtime: &Path) -> String {
    // FNV-1a: short, and the same in every build.
    let bytes = runtime.as_os_str().as_encoded_bytes().iter();
    let hash = bytes.fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!(r"\\.\pipe\hive-{user}-{hash:016x}")
}

/// Refuses a pipe `user` (a SID) does not own. The service gives its pipe the user as owner,
/// which another user cannot give an object without a privilege. (The serving process's id
/// is not enough: a pipe handed to another process keeps its creator's id, which a process of
/// the user could get again.)
fn check_owner(pipe: &impl AsRawHandle, user: &str) -> io::Result<()> {
    let (mut owner, mut descriptor) = (std::ptr::null_mut(), std::ptr::null_mut());
    let found = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    win32(found)?;
    // The owner lives in the descriptor.
    let owner = sid_text(owner);
    unsafe { LocalFree(descriptor) };
    let owner = owner?;
    if owner != user {
        return Err(io::Error::other(format!(
            "refusing a hive pipe of another user ({owner})"
        )));
    }
    Ok(())
}

/// The error of a failed Windows call (its result is 0).
fn check(ok: BOOL) -> io::Result<()> {
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The error of a Windows call that returns its error code (0 for none).
fn win32(code: WIN32_ERROR) -> io::Result<()> {
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    Ok(())
}

/// A handle a Windows call opened, closed when dropped; null when the call failed.
fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// The SID (`S-1-5-21-…`) of the user process `pid` runs as.
fn process_user(pid: u32) -> io::Result<String> {
    let process = owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })?;
    let mut token = std::ptr::null_mut();
    check(unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut token) })?;
    let token = owned(token)?;
    // A `TOKEN_USER` and the SID it points to (a SID takes at most 68 bytes), aligned.
    let mut buffer = [0_u64; 32];
    let (size, mut used) = (size_of_val(&buffer) as u32, 0);
    let info = buffer.as_mut_ptr().cast();
    check(unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, info, size, &mut used) })?;
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid_text(user.User.Sid)
}

/// The text of `sid` (`S-1-5-21-…`).
fn sid_text(sid: PSID) -> io::Result<String> {
    let mut text = std::ptr::null_mut();
    check(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    // A SID's text is short: the bound only keeps a bad string from running on.
    let len = (0..256)
        .take_while(|&i| unsafe { *text.add(i) } != 0)
        .count();
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    unsafe { LocalFree(text.cast()) };
    Ok(sid)
}

/// The service's end of its pipe: an instance waiting for the next client.
pub struct Listener {
    name: String,
    user: String,
    next: NamedPipeServer,
}

impl Listener {
    /// Creates the pipe of this user and `paths`. Refused when a pipe of that name exists
    /// (another service, or someone squatting the name).
    pub fn bind(paths: &Paths) -> io::Result<Self> {
        let user = process_user(std::process::id())?;
        let name = pipe_name(&user, &paths.runtime);
        let next = instance(&name, &user, true)?;
        Ok(Self { name, user, next })
    }
}

/// The next client of `listener`; a new instance then waits for the one after.
pub async fn accept(listener: &mut Listener) -> io::Result<NamedPipeServer> {
    listener.next.connect().await?;
    let next = instance(&listener.name, &listener.user, false)?;
    Ok(std::mem::replace(&mut listener.next, next))
}

/// A new instance of the pipe `name`, owned by `user` (what clients check), that only `user`
/// can open, and only from this machine.
fn instance(name: &str, user: &str, first: bool) -> io::Result<NamedPipeServer> {
    // Owned by the user (an elevated process would make it the administrators'); protected
    // (nothing inherited): all access for the user alone.
    let sddl = format!("O:{user}D:P(A;;GA;;;{user})\0");
    let sddl: Vec<u16> = sddl.encode_utf16().collect();
    let (mut descriptor, size) = (std::ptr::null_mut(), std::ptr::null_mut());
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            size,
        )
    };
    check(converted)?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let created = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, (&raw mut attributes).cast())
    };
    unsafe { LocalFree(descriptor) };
    created
}

/// Starts the service out of this console and process group, so it outlives the bridge and
/// ends with the app connection only; also out of this process's job (the app's), unless the
/// job forbids it (e.g. cargo's).
pub fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    // `bitor`, not `|`: or-ing and xor-ing flags that share no bit are the same.
    let detached = DETACHED_PROCESS.bitor(CREATE_NEW_PROCESS_GROUP);
    let breakaway = detached.bitor(CREATE_BREAKAWAY_FROM_JOB);
    command
        .creation_flags(breakaway)
        .spawn()
        .or_else(|_| command.creation_flags(detached).spawn())
}

/// The machine-wide monotonic clock in ns, from the performance counter: one clock for every
/// process, so it orders hook calls made by different `hive hook` processes.
pub fn monotonic_ns() -> u64 {
    let (mut count, mut frequency) = (0, 0);
    // Neither fails since Windows XP.
    unsafe {
        QueryPerformanceCounter(&mut count);
        QueryPerformanceFrequency(&mut frequency);
    }
    ns(count, frequency)
}

/// `count` ticks of `frequency` a second, in ns (0 for a negative count).
fn ns(count: i64, frequency: i64) -> u64 {
    let ns = i128::from(count) * 1_000_000_000 / i128::from(frequency.max(1));
    u64::try_from(ns).unwrap_or(0)
}

/// Terminals, until ConPTY (12.5.3): none starts, so none has processes to end.
pub mod terminal {
    use std::path::Path;

    use tokio::process::Child;
    use tokio::sync::mpsc::UnboundedSender;

    use crate::terminal::{Input, Terminal};

    /// What a terminal's output is read from.
    pub type Pty = tokio::io::Empty;

    pub fn spawn(
        _id: u32,
        cwd: &str,
        _cols: u16,
        _rows: u16,
        _bin_dir: &Path,
        _env: &[(&'static str, String)],
    ) -> Result<(Terminal, UnboundedSender<Input>, Pty, Child), String> {
        let why = super::unsupported("a terminal");
        Err(format!("cannot start a terminal in {cwd}: {why}"))
    }

    pub async fn end_sessions(_sessions: &[i32]) {}
}

/// Windows PowerShell asked for its `PATH`: the one the service inherited from the app,
/// which on Windows is the user's (a shell's config does not change it). Printed with a
/// bare `\n`, as [`crate::wrapper::user_path`] reads it.
pub fn path_shell() -> (OsString, Vec<OsString>) {
    let print = "[Console]::Out.Write($env:Path + [char]10)";
    let args = ["-NoProfile", "-NonInteractive", "-Command", print];
    ("powershell".into(), args.map(OsString::from).to_vec())
}

/// No process table until 12.5.6a.
pub fn list() -> Vec<Proc> {
    vec![]
}

/// No process working folders until 12.5.6a.
pub fn cwd(_pid: i32) -> Option<PathBuf> {
    None
}

/// Renames `from` to `to` in one step, failing (`AlreadyExists`) when `to` exists, as
/// `renameat2(RENAME_NOREPLACE)`: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`.
// ponytail: plain paths, so one over `MAX_PATH` (260) fails unless long paths are enabled;
// `\\?\` before both if that matters.
pub fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (wide(from)?, wide(to)?);
    check(unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) })
}

/// `path` as a NUL-terminated wide string for a Windows call; refused when it holds a NUL,
/// which would cut it short.
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a path holds a NUL",
        ));
    }
    Ok(wide.into_iter().chain([0]).collect())
}

/// The app runs beside the service: it opens `path` itself.
pub fn native_path(path: &Path, _wslpath: &std::ffi::OsStr) -> io::Result<String> {
    Ok(path.to_string_lossy().into_owned())
}

/// `err`, told plainly when another program holds the file `name` open without sharing it:
/// Windows then refuses to open or replace it, and a save leaves it as it was.
pub fn in_use(err: io::Error, name: &str) -> io::Error {
    let held = [ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION].map(|code| Some(code as i32));
    if !held.contains(&err.raw_os_error()) {
        return err;
    }
    io::Error::other(format!(
        "{name} is open in another program that keeps it locked: close it there and save \
         again ({err})"
    ))
}

/// The drives (`C:`, `D:`…), as folders: the folder browser lists them for a typed name
/// without a separator.
pub fn drives() -> Vec<Dir> {
    drive_dirs(unsafe { GetLogicalDrives() })
}

/// The drives of `mask` (bit 0 `A:`, bit 25 `Z:`), as `GetLogicalDrives` gives them.
fn drive_dirs(mask: u32) -> Vec<Dir> {
    (b'A'..=b'Z')
        .zip(0..)
        .filter(|&(_, bit)| mask >> bit & 1 == 1)
        .map(|(letter, _)| Dir {
            name: format!("{}:", char::from(letter)),
            git: false,
        })
        .collect()
}

/// Kills process `pid` and every process it started.
// ponytail: `taskkill` by pid, which a pid reused meanwhile could misdirect; a job object
// (12.5.6a) ends exactly the tree.
pub fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// What a program printed (a path, a `PATH`): programs print UTF-8 on Windows, and git
/// separates a path's names with `/`, here `\` as every other path (no name holds a `/`).
pub fn os_string(bytes: &[u8]) -> OsString {
    String::from_utf8_lossy(bytes).replace('/', r"\").into()
}

/// A plain open: Windows has no FIFO a measured file could turn into.
pub fn open_nonblocking(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// No Unix permission bits: a file gets its folder's ACL (see [`crate::mode`]).
pub mod mode {
    use std::fs::{DirBuilder, File, Metadata, OpenOptions};
    use std::io;

    pub fn create(options: &mut OpenOptions, _mode: u32) -> &mut OpenOptions {
        options
    }

    pub fn folder(builder: &mut DirBuilder, _mode: u32) -> &mut DirBuilder {
        builder
    }

    pub fn set(_file: &File, _mode: u32) -> io::Result<()> {
        Ok(())
    }

    /// None: nothing is executable by its bits.
    pub fn of(_meta: &Metadata) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn unsupported_names_what() {
        let err = unsupported("the Hive service");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(
            err.to_string(),
            "the Hive service is not supported on Windows yet"
        );
    }

    fn resolve(vars: &[(&str, &str)]) -> Paths {
        paths(|key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into()))
    }

    #[test]
    fn data_and_settings_go_in_the_app_data_folders() {
        let paths = resolve(&[
            ("LOCALAPPDATA", r"C:\L"),
            ("APPDATA", r"C:\R"),
            ("USERPROFILE", r"C:\U"),
        ]);
        assert_eq!(paths.data, PathBuf::from(r"C:\L\hive"));
        assert_eq!(paths.runtime, PathBuf::from(r"C:\L\hive\run"));
        assert_eq!(paths.lock(), PathBuf::from(r"C:\L\hive\run\hive.lock"));
        assert_eq!(paths.settings(), PathBuf::from(r"C:\R\hive\settings.json"));
    }

    #[test]
    fn the_profile_stands_in_for_unset_folders() {
        let paths = resolve(&[("LOCALAPPDATA", ""), ("USERPROFILE", r"C:\U")]);
        assert_eq!(paths.data, PathBuf::from(r"C:\U\AppData\Local\hive"));
        assert_eq!(paths.config, PathBuf::from(r"C:\U\AppData\Roaming\hive"));
    }

    #[test]
    fn without_a_profile_data_goes_in_the_temporary_folder() {
        let paths = resolve(&[]);
        assert_eq!(paths.data, std::env::temp_dir().join("hive"));
        assert_eq!(paths.config, paths.data.join("config"));
    }

    /// Paths in a temporary folder: a pipe of their own.
    fn paths_in(dir: &Path) -> Paths {
        Paths {
            runtime: dir.join("run"),
            data: dir.join("data"),
            config: dir.join("config"),
        }
    }

    #[test]
    fn the_runtime_folder_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(&tmp.path().join("a"));
        paths.prepare_runtime().unwrap();
        assert!(paths.runtime.is_dir());
    }

    #[test]
    fn the_pipe_is_the_users_and_the_data_folders() {
        assert_eq!(
            pipe_name("S-1-5-21-1", Path::new(r"C:\L\hive\run")),
            r"\\.\pipe\hive-S-1-5-21-1-bb7225ed63f8b90b"
        );
    }

    #[test]
    fn this_process_runs_as_a_user_sid() {
        let me = process_user(std::process::id()).unwrap();
        assert!(me.starts_with("S-1-5-"), "{me}");
        // No such process.
        assert!(process_user(u32::MAX).is_err());
    }

    #[test]
    fn failed_windows_calls_are_errors() {
        check(1).unwrap();
        assert!(check(0).is_err());
        win32(0).unwrap();
        let denied = win32(5).unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
        assert!(owned(std::ptr::null_mut()).is_err());
    }

    #[tokio::test]
    async fn clients_reach_the_service_one_instance_each() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let mut listener = Listener::bind(&paths).unwrap();
        // One service per pipe.
        assert!(Listener::bind(&paths).is_err());
        for n in [1_u8, 2] {
            let (client, server) = tokio::join!(paths.connect(), accept(&mut listener));
            let (mut client, mut server) = (client.unwrap(), server.unwrap());
            client.write_all(&[n]).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), n);
            // The pipe is this user's: another user's is refused.
            let me = process_user(std::process::id()).unwrap();
            check_owner(&client, &me).unwrap();
            let err = check_owner(&client, "S-1-5-18").unwrap_err();
            let owner = format!("refusing a hive pipe of another user ({me})");
            assert_eq!(err.to_string(), owner);
        }
    }

    #[tokio::test]
    async fn a_client_waits_for_a_free_instance_for_a_while() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let mut listener = Listener::bind(&paths).unwrap();
        // The only instance, taken and not accepted yet: the next client waits.
        let _first = paths.connect().await.unwrap();
        let accept = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            accept(&mut listener).await
        };
        let (second, _) = tokio::join!(paths.connect(), accept);
        second.unwrap();
        // Now nothing accepts: it gives up.
        let started = Instant::now();
        let err = paths.connect().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= BUSY_TIME);
    }

    #[tokio::test]
    async fn without_a_service_connecting_fails_at_once() {
        let tmp = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let err = paths_in(tmp.path()).connect().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(started.elapsed() < BUSY_TIME);
    }

    #[test]
    fn a_detached_program_runs() {
        let mut exit = Command::new("cmd");
        exit.args(["/c", "exit 7"]);
        let status = spawn_detached(&mut exit).unwrap().wait().unwrap();
        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn the_monotonic_clock_moves_forward_in_ns() {
        let before = monotonic_ns();
        std::thread::sleep(Duration::from_millis(20));
        let after = monotonic_ns();
        assert!(before > 0);
        assert!(after >= before + 20_000_000, "{before} {after}");
        assert!(after < before + 10_000_000_000, "{before} {after}");
        assert_eq!(ns(30_000_000, 10_000_000), 3_000_000_000);
        assert_eq!(ns(7, 0), 7_000_000_000);
        assert_eq!(ns(-1, 1), 0);
    }

    #[tokio::test]
    async fn no_terminal_starts_yet() {
        let started = terminal::spawn(1, r"C:\w", 80, 24, Path::new("."), &[]).map(drop);
        assert_eq!(
            started.unwrap_err(),
            r"cannot start a terminal in C:\w: a terminal is not supported on Windows yet"
        );
        terminal::end_sessions(&[1]).await;
    }

    #[tokio::test]
    async fn the_users_path_is_the_one_powershell_prints() {
        // Neither the service's `PATH` nor a home to fall back to: only what it printed.
        let path = crate::wrapper::user_path(path_shell(), None, None, BUSY_TIME * 15).await;
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        let mut dirs = std::env::split_paths(&path);
        assert!(
            dirs.any(|dir| dir.as_os_str().eq_ignore_ascii_case(&system)),
            "{path:?}"
        );
    }

    #[test]
    fn stubs_answer_nothing() {
        assert_eq!(list(), vec![]);
        assert_eq!(cwd(4), None);
    }

    #[test]
    fn a_rename_never_replaces_a_file_or_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let at = |name: &str| tmp.path().join(name);
        std::fs::write(at("a"), "a").unwrap();
        std::fs::write(at("f"), "f").unwrap();
        std::fs::create_dir(at("d")).unwrap();
        std::fs::create_dir(at("e")).unwrap();
        for (from, taken) in [("a", "f"), ("a", "d"), ("e", "d"), ("e", "f")] {
            let err = rename_new(&at(from), &at(taken)).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{from} {taken}");
        }
        assert_eq!(std::fs::read_to_string(at("f")).unwrap(), "f");
        rename_new(&at("a"), &at("b")).unwrap();
        assert_eq!(std::fs::read_to_string(at("b")).unwrap(), "a");
        rename_new(&at("e"), &at("g")).unwrap();
        assert!(!at("a").exists() && !at("e").exists() && at("g").is_dir());
        let err = rename_new(&at("missing"), &at("c")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        // A NUL would cut the path short.
        let err = rename_new(&at("b\0x"), &at("c")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(rename_new(&at("b"), &at("c\0")).is_err());
        assert!(at("b").exists() && !at("c").exists());
    }

    #[test]
    fn a_path_opens_as_it_is() {
        let path = Path::new(r"C:\Users\me\a b.rs");
        assert_eq!(
            native_path(path, "wslpath".as_ref()).unwrap(),
            r"C:\Users\me\a b.rs"
        );
    }

    #[test]
    fn a_locked_file_is_said_to_be_in_use() {
        for code in [ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION] {
            let err = in_use(io::Error::from_raw_os_error(code as i32), "a.rs").to_string();
            let said = "a.rs is open in another program that keeps it locked: close it there \
                        and save again (";
            assert!(err.starts_with(said), "{err}");
        }
        let denied = in_use(io::Error::from_raw_os_error(5), "a.rs");
        assert_eq!(denied.raw_os_error(), Some(5));
    }

    #[test]
    fn a_save_leaves_a_file_another_program_holds_as_it_was() {
        use hive_protocol::SaveError;
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.rs");
        std::fs::write(&path, "old").unwrap();
        let old = crate::file::version(b"old");
        // Not shared at all, then shared but not for deleting (as some editors do).
        for share in [0, FILE_SHARE_READ | FILE_SHARE_WRITE] {
            let held = File::options()
                .read(true)
                .share_mode(share)
                .open(&path)
                .unwrap();
            let saved = crate::file::save(tmp.path(), "a.rs", "new", Some(&old));
            drop(held);
            let (error, message) = saved.unwrap_err();
            assert_eq!(error, SaveError::Io, "{share}: {message}");
            assert!(
                message.starts_with("a.rs is open in another program"),
                "{share}: {message}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
            let names: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
            assert_eq!(names.len(), 1, "{names:?}");
        }
        // Released: saved.
        let saved = crate::file::save(tmp.path(), "a.rs", "new", Some(&old));
        assert_eq!(saved, Ok(crate::file::version(b"new")));
    }

    #[test]
    fn the_drives_are_listed_as_folders() {
        let drives = drives();
        assert!(drives.iter().any(|d| d.name == "C:"), "{drives:?}");
        for drive in &drives {
            assert!(
                Path::new(&format!(r"{}\", drive.name)).is_dir(),
                "{drive:?}"
            );
            assert!(!drive.git);
        }
        let names = |mask| {
            drive_dirs(mask)
                .into_iter()
                .map(|d| d.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(0b101), ["A:", "C:"]);
        assert_eq!(names(1 << 25), ["Z:"]);
        assert_eq!(names(0), Vec::<String>::new());
    }

    #[test]
    fn a_killed_tree_ends() {
        // `ping` waits a second between tries: about 30 s unless killed.
        let mut child = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        kill_tree(child.id());
        let started = Instant::now();
        let ended = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(100));
            child.try_wait().unwrap().is_some()
        });
        assert!(ended, "not killed after {:?}", started.elapsed());
        assert!(!child.wait().unwrap().success());
    }

    #[test]
    fn printed_bytes_are_utf8_text() {
        assert_eq!(os_string("C:\\é".as_bytes()), "C:\\é");
        assert_eq!(os_string(b"C:/Users/me/r"), r"C:\Users\me\r");
        assert_eq!(os_string(b"a\xffb"), "a\u{fffd}b");
    }

    #[test]
    fn files_keep_their_folders_acl_and_never_run_by_their_bits() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f");
        let file = crate::mode::private(File::options().write(true).create_new(true))
            .open(&path)
            .unwrap();
        crate::mode::set(&file, 0o755).unwrap();
        let meta = file.metadata().unwrap();
        assert!(!meta.permissions().readonly());
        assert!(!crate::mode::executable(&meta));
        assert_eq!(crate::mode::of(&meta), 0);
        let folder = tmp.path().join("d");
        crate::mode::folder(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&folder)
            .unwrap();
        assert!(folder.is_dir());
        assert_eq!(
            std::io::read_to_string(open_nonblocking(&path).unwrap()).unwrap(),
            ""
        );
        assert!(open_nonblocking(&folder.join("missing")).is_err());
    }
}
