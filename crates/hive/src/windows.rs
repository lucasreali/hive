//! What only native Windows needs, kept apart from the portable code (12.5): the service's
//! named pipe and its checks, how the bridge starts the service, the clock, files (a rename
//! that never replaces, a file another program holds, the drives), and what is not supported
//! yet ([`unsupported`]). Tested on the Windows CI runner (`windows.yml`).

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::ops::BitOr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use hive_protocol::{Dir, TerminalShell};
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_PIPE_BUSY, ERROR_SHARING_VIOLATION, HANDLE,
    LocalFree, WIN32_ERROR,
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
use crate::terminal::conpty;

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

/// Terminals on a pseudoconsole (ConPTY, 12.5.3). The shell starts suspended, joins a job
/// that kills what is left once the service lets go of it, and only then runs; its session
/// id is its process id. Ending a terminal closes its console (a console program's SIGHUP),
/// gives its processes [`GRACE`](crate::terminal::GRACE) and ends the job. Its shell, command line and environment
/// come from [`crate::terminal::conpty`].
pub mod terminal {
    use std::collections::{BTreeMap, HashMap};
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::hash::{BuildHasher, RandomState};
    use std::io;
    use std::ops::BitOr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, OwnedHandle, RawHandle};
    use std::os::windows::process::ExitStatusExt;
    use std::path::Path;
    use std::process::ExitStatus;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
    use std::time::Duration;

    use futures_util::FutureExt;
    use hive_protocol::TerminalShell;
    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::NamedPipeServer;
    use tokio::sync::{mpsc, watch};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
        TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
        InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
        PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject,
    };
    use windows_sys::core::HRESULT;

    use super::{check, instance, owned, process_user};
    use crate::terminal::{GRACE, Input, LastOutput, Terminal, conpty};
    use crate::watch::Watch;

    /// The exit code of the processes still running once their terminal's grace is over.
    pub const KILLED: u32 = 9;
    /// Most processes of one terminal listed (for the unhooked-`claude` watcher and ending).
    const JOB_LIMIT: usize = 1024;

    /// What a terminal's output is read from: its console's output pipe.
    pub type Pty = NamedPipeServer;

    /// The Windows calls a terminal's start makes, one per step: [`WINDOWS`], or a test's
    /// that fail one of them.
    #[derive(Clone, Copy)]
    struct Calls {
        /// A console pipe of this name and user ([`pipe`]).
        pipe: fn(&str, &str, bool) -> io::Result<(NamedPipeServer, File)>,
        /// A pseudoconsole of this size on the host's input and output ends.
        console: fn(COORD, RawHandle, RawHandle) -> io::Result<HPCON>,
        /// Closes a pseudoconsole (may wait for its output to be read).
        close: fn(HPCON),
        job: fn() -> io::Result<OwnedHandle>,
        /// [`create`]: the command line, environment block and folder, on the console.
        create: Create,
        /// Puts the process in the job.
        assign: fn(&OwnedHandle, RawHandle) -> io::Result<()>,
        /// Resumes the suspended thread.
        resume: fn(RawHandle) -> io::Result<()>,
        /// Runs this on a new thread.
        thread: fn(Box<dyn FnOnce() + Send>) -> io::Result<()>,
    }

    type Create = fn(&mut [u16], &[u16], &[u16], HPCON) -> io::Result<PROCESS_INFORMATION>;

    const WINDOWS: Calls = Calls {
        pipe,
        console: pseudo_console,
        close,
        job,
        create,
        assign,
        resume,
        thread,
    };

    /// A pseudoconsole, closed once its last owner lets go of it: its session ended, or its
    /// start failed.
    struct Console {
        hpc: HPCON,
        calls: Calls,
    }

    impl Drop for Console {
        fn drop(&mut self) {
            let (console, close) = (self.hpc, self.calls.close);
            // Closing waits for the console host to exit, which may wait for its last output
            // to be read: never on the runtime, unless no thread can start.
            if (self.calls.thread)(Box::new(move || close(console))).is_err() {
                close(console);
            }
        }
    }

    /// A running terminal: its console (held to keep it open), its processes' job, and its
    /// shell (held so that its process id, the session id, is not reused until it ended).
    struct Session {
        _console: Arc<Console>,
        job: OwnedHandle,
        _shell: Arc<OwnedHandle>,
    }

    /// The running terminals, by session id.
    static SESSIONS: Mutex<BTreeMap<i32, Session>> = Mutex::new(BTreeMap::new());

    fn sessions() -> MutexGuard<'static, BTreeMap<i32, Session>> {
        SESSIONS.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A terminal's shell, waited for by its pump.
    pub struct Child {
        exit: watch::Receiver<Option<u32>>,
    }

    impl Child {
        /// Waits for the shell to exit; again and again gives the same status.
        pub async fn wait(&mut self) -> io::Result<ExitStatus> {
            let exit = self.exit.wait_for(Option::is_some).await;
            let code = *exit.map_err(io::Error::other)?;
            Ok(ExitStatus::from_raw(code.unwrap_or_default()))
        }
    }

    /// A [`Child`] of `process`, told of its exit by a thread (of `calls`) that waits for it.
    fn waited(calls: Calls, process: Arc<OwnedHandle>) -> io::Result<Child> {
        let (exited, exit) = watch::channel(None);
        (calls.thread)(Box::new(move || {
            let mut code = 0;
            unsafe {
                WaitForSingleObject(process.as_raw_handle(), INFINITE);
                GetExitCodeProcess(process.as_raw_handle(), &mut code);
            }
            exited.send_replace(Some(code));
        }))?;
        Ok(Child { exit })
    }

    /// Starts the terminal's shell (the `shell` setting) on a pseudoconsole of `cols` ×
    /// `rows` in `cwd`, with the service's environment, `HIVE_TERMINAL_ID`, `TERM` and `env`
    /// over it and `bin_dir` first on `PATH`. As [`crate::terminal::spawn`] on Unix.
    pub fn spawn(
        id: u32,
        cwd: &str,
        (cols, rows): (u16, u16),
        bin_dir: &Path,
        env: &[(&'static str, String)],
        shell: TerminalShell,
    ) -> Result<(Terminal, mpsc::UnboundedSender<Input>, Pty, Child), String> {
        let own = [
            ("HIVE_TERMINAL_ID", id.to_string()),
            ("TERM", "xterm-256color".into()),
        ];
        let set = own.into_iter().chain(env.iter().cloned());
        let set = set
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        let started = start(WINDOWS, cwd, size(cols, rows), bin_dir, set, shell);
        let (session, console, output, pipe, child) =
            started.map_err(|err| format!("cannot start a terminal in {cwd}: {err}"))?;
        let (input, input_rx) = mpsc::unbounded_channel();
        tokio::spawn(feed(pipe, console, input_rx));
        let terminal = Terminal {
            session,
            watch: Watch::default(),
            last_output: LastOutput::new(),
            claude_dir: None,
        };
        Ok((terminal, input, output, child))
    }

    /// Starts the shell with `calls` and keeps its session; returns its session id, its
    /// console (for resizing), its output and input pipes and the shell. On a failure nothing
    /// is left: the shell killed, its console closed, its job ended. (Not generic: coverage
    /// counts each instance of a generic function apart, and only tests make `start` fail.)
    fn start(
        calls: Calls,
        cwd: &str,
        size: COORD,
        bin_dir: &Path,
        set: Vec<(OsString, OsString)>,
        shell: TerminalShell,
    ) -> io::Result<(i32, Weak<Console>, Pty, NamedPipeServer, Child)> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let root = std::env::var_os("SystemRoot");
        let args = conpty::shell(shell, &path, root.as_deref()).map_err(io::Error::other)?;
        let mut line = wide(&conpty::command_line(&args));
        let mut env = Vec::new();
        for (name, value) in conpty::environment(std::env::vars_os(), set, bin_dir) {
            env.extend(name.encode_wide().chain([u16::from(b'=')]));
            env.extend(value.encode_wide().chain([0]));
        }
        env.push(0);
        let user = process_user(std::process::id())?;
        let (output, host_output) = (calls.pipe)(&pipe_name(), &user, false)?;
        let (input, host_input) = (calls.pipe)(&pipe_name(), &user, true)?;
        let (host_in, host_out) = (host_input.as_raw_handle(), host_output.as_raw_handle());
        let hpc = (calls.console)(size, host_in, host_out)?;
        let console = Arc::new(Console { hpc, calls });
        // The console host has its own handles of its ends: the output ends when it does.
        drop((host_input, host_output));
        let job = (calls.job)()?;
        let started = (calls.create)(&mut line, &env, &wide(OsStr::new(cwd)), hpc)?;
        let (process, thread) = (owned(started.hProcess)?, owned(started.hThread)?);
        if let Err(err) = (calls.assign)(&job, process.as_raw_handle()) {
            // A shell left suspended outside its job would never end.
            kill(process.as_raw_handle());
            return Err(err);
        }
        // Left in the job, a shell that cannot resume, or be waited for, is killed with it.
        (calls.resume)(thread.as_raw_handle())?;
        let process = Arc::new(process);
        let child = waited(calls, process.clone())?;
        let session = started.dwProcessId as i32;
        let resize = Arc::downgrade(&console);
        let kept = Session {
            _console: console,
            job,
            _shell: process,
        };
        sessions().insert(session, kept);
        Ok((session, resize, output, input, child))
    }

    /// Kills `process`, with [`KILLED`] as its exit code.
    fn kill(process: RawHandle) {
        unsafe { TerminateProcess(process, KILLED) };
    }

    /// `text` for a Windows call: UTF-16 and nul-terminated.
    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain([0]).collect()
    }

    /// A console of `cols` × `rows` (at most `i16::MAX` each).
    fn size(cols: u16, rows: u16) -> COORD {
        let cells = |n: u16| i16::try_from(n).unwrap_or(i16::MAX);
        COORD {
            X: cells(cols),
            Y: cells(rows),
        }
    }

    /// The error of a failed `HRESULT` (negative).
    fn hresult(code: HRESULT) -> io::Result<()> {
        if code < 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        Ok(())
    }

    /// A pseudoconsole of `size` that reads its input from `host_in` and writes its output
    /// to `host_out`.
    fn pseudo_console(size: COORD, host_in: RawHandle, host_out: RawHandle) -> io::Result<HPCON> {
        let mut hpc = 0;
        hresult(unsafe { CreatePseudoConsole(size, host_in, host_out, 0, &mut hpc) })?;
        Ok(hpc)
    }

    fn close(console: HPCON) {
        unsafe { ClosePseudoConsole(console) };
    }

    fn assign(job: &OwnedHandle, process: RawHandle) -> io::Result<()> {
        check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) })
    }

    /// Resumes `thread`, suspended once.
    fn resume(thread: RawHandle) -> io::Result<()> {
        let resumed = unsafe { ResumeThread(thread) };
        check((resumed != u32::MAX).into())
    }

    /// Runs `run` on a new thread; an error, not a panic, when none can start.
    fn thread(run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new().spawn(run).map(drop)
    }

    /// A new name for a console pipe: unique in this process, and unguessable, so that
    /// another local user cannot create it first (and so keep terminals from starting).
    fn pipe_name() -> String {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // Keys from the system's random source (once per thread, then one more each time).
        let random = RandomState::new().hash_one(n);
        let pid = std::process::id();
        format!(r"\\.\pipe\hive-console-{pid}-{n}-{random:016x}")
    }

    /// A new pipe `name` for a console: the service's end, and the console host's (which
    /// reads from it when `host_reads`, else writes to it). Only `user` can open it; refused
    /// when a pipe of that name exists.
    fn pipe(name: &str, user: &str, host_reads: bool) -> io::Result<(NamedPipeServer, File)> {
        let ours = instance(name, user, true)?;
        let mut host = File::options();
        let host = host.read(host_reads).write(!host_reads).open(name)?;
        connected(&ours)?;
        Ok((ours, host))
    }

    /// Fails unless a client has opened `pipe` already; else only tells tokio, so that it
    /// reads and writes it.
    fn connected(pipe: &NamedPipeServer) -> io::Result<()> {
        match pipe.connect().now_or_never() {
            Some(connected) => connected,
            None => Err(io::ErrorKind::NotConnected.into()),
        }
    }

    /// A job that kills its processes once its last handle is closed.
    fn job() -> io::Result<OwnedHandle> {
        let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let (info, size) = ((&raw const limits).cast(), size_of_val(&limits) as u32);
        let class = JobObjectExtendedLimitInformation;
        let set = unsafe { SetInformationJobObject(job.as_raw_handle(), class, info, size) };
        check(set)?;
        Ok(job)
    }

    /// Starts the command `line` suspended on the console `hpc`, in `cwd` with the
    /// environment block `env`.
    fn create(
        line: &mut [u16],
        env: &[u16],
        cwd: &[u16],
        hpc: HPCON,
    ) -> io::Result<PROCESS_INFORMATION> {
        let mut size = 0;
        // Only asks for the size (and fails for want of room).
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
        let mut list = vec![0_u64; size.div_ceil(8)];
        let attributes = list.as_mut_ptr().cast();
        check(unsafe { InitializeProcThreadAttributeList(attributes, 1, 0, &mut size) })?;
        let (attribute, value) = (
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            hpc as *const _,
        );
        let (none, empty) = (std::ptr::null_mut(), std::ptr::null());
        let set = unsafe {
            UpdateProcThreadAttribute(
                attributes,
                0,
                attribute,
                value,
                size_of::<HPCON>(),
                none,
                empty,
            )
        };
        check(set)?;
        let mut info = STARTUPINFOEXW::default();
        info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        // Not the service's own standard handles (its log): the console's.
        info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        info.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        info.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        info.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        info.lpAttributeList = attributes;
        // `bitor`, not `|`: or-ing and xor-ing flags that share no bit are the same.
        let flags = EXTENDED_STARTUPINFO_PRESENT
            .bitor(CREATE_UNICODE_ENVIRONMENT)
            .bitor(CREATE_SUSPENDED);
        let mut started = PROCESS_INFORMATION::default();
        let created = unsafe {
            CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                env.as_ptr().cast(),
                cwd.as_ptr(),
                &info.StartupInfo,
                &mut started,
            )
        };
        unsafe { DeleteProcThreadAttributeList(attributes) };
        check(created)?;
        Ok(started)
    }

    /// Writes typing to the console and resizes it, until the terminal is dropped.
    async fn feed(
        mut pipe: NamedPipeServer,
        console: Weak<Console>,
        mut input: mpsc::UnboundedReceiver<Input>,
    ) {
        while let Some(input) = input.recv().await {
            match input {
                // A closed console fails every write.
                Input::Data(bytes) => _ = pipe.write_all(&bytes).await,
                // Only while the terminal runs: an ended one's console is closed.
                Input::Resize { cols, rows } => {
                    if let Some(console) = console.upgrade() {
                        unsafe { ResizePseudoConsole(console.hpc, size(cols, rows)) };
                    }
                }
            }
        }
    }

    /// The processes in `job`, at most [`JOB_LIMIT`].
    fn pids(job: &OwnedHandle) -> Vec<u32> {
        #[repr(C)]
        struct List {
            _assigned: u32,
            listed: u32,
            ids: [usize; JOB_LIMIT],
        }
        let mut list = List {
            _assigned: 0,
            listed: 0,
            ids: [0; JOB_LIMIT],
        };
        let (info, size) = ((&raw mut list).cast(), size_of::<List>() as u32);
        let class = JobObjectBasicProcessIdList;
        // With more processes (`ERROR_MORE_DATA`) the first ones are still listed.
        unsafe {
            QueryInformationJobObject(job.as_raw_handle(), class, info, size, std::ptr::null_mut())
        };
        let listed = &list.ids[..(list.listed as usize).min(JOB_LIMIT)];
        listed.iter().map(|&id| id as u32).collect()
    }

    /// The processes of Hive's terminals: each one's terminal (its session id), by process id.
    pub fn jobs() -> HashMap<u32, i32> {
        let sessions = sessions();
        let pids = sessions.iter().flat_map(|(&session, running)| {
            pids(&running.job)
                .into_iter()
                .map(move |pid| (pid, session))
        });
        pids.collect()
    }

    /// A job of a program run with a time limit (git, `gh`, a project script), as a process
    /// group on Unix: it holds the program and whatever it starts, killed by [`Job::kill`] or
    /// once the job is dropped.
    pub struct Job(OwnedHandle);

    /// Puts a process in a job ([`assign`], or a test's that fails).
    type Assign = fn(&OwnedHandle, RawHandle) -> io::Result<()>;

    impl Job {
        /// A new job holding `process`, which should have started nothing yet. A process that
        /// cannot join one is killed: it would run without a limit.
        // ponytail: joined right after its start, not started suspended in it (as a terminal's
        // shell): `std::process::Command` cannot resume a process. A program starts nothing in
        // the microseconds between.
        pub fn of(process: &impl AsRawHandle) -> io::Result<Self> {
            Self::joined(job(), process.as_raw_handle(), assign)
        }

        /// `process` in `job` (a new one, or why there is none), joined through `assign`; killed
        /// on a failure. Both are given by tests to fail.
        fn joined(
            job: io::Result<OwnedHandle>,
            process: RawHandle,
            assign: Assign,
        ) -> io::Result<Self> {
            let joined = job.and_then(|job| {
                assign(&job, process)?;
                Ok(Self(job))
            });
            if joined.is_err() {
                kill(process);
            }
            joined
        }

        /// Kills every process in the job, with [`KILLED`] as their exit code.
        pub fn kill(&self) {
            unsafe { TerminateJobObject(self.0.as_raw_handle(), KILLED) };
        }
    }

    /// Ends the terminals of `ended` (session ids): closes their consoles, waits up to
    /// [`GRACE`] for their processes to exit, then kills what is left.
    pub async fn end_sessions(ended: &[i32]) {
        let jobs: Vec<OwnedHandle> = {
            let mut sessions = sessions();
            let ended = ended.iter().filter_map(|session| sessions.remove(session));
            // Each console closes as its session is dropped (once no resize holds it).
            ended.map(|session| session.job).collect()
        };
        let _ = tokio::time::timeout(GRACE, async {
            while jobs.iter().any(|job| !pids(job).is_empty()) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        for job in &jobs {
            unsafe { TerminateJobObject(job.as_raw_handle(), KILLED) };
        }
    }

    #[cfg(test)]
    mod tests {
        use std::cell::RefCell;
        use std::time::Instant;

        use bytes::Bytes;
        use tokio::io::AsyncReadExt;
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{
            GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };

        use super::*;
        use crate::procs::Proc;
        use crate::windows::list;

        thread_local! {
            /// The console and the shell the last start on this thread made.
            static MADE: RefCell<(Option<HPCON>, Option<OwnedHandle>)> = RefCell::default();
        }

        /// The consoles closed, on any thread.
        static CLOSED: Mutex<Vec<HPCON>> = Mutex::new(Vec::new());

        /// The real calls, telling the test what they made and closed.
        const RECORDED: Calls = Calls {
            console: |size, host_in, host_out| {
                let hpc = pseudo_console(size, host_in, host_out)?;
                MADE.with_borrow_mut(|made| made.0 = Some(hpc));
                Ok(hpc)
            },
            close: |console| {
                CLOSED.lock().unwrap().push(console);
                close(console);
            },
            create: |line, env, cwd, hpc| {
                let started = create(line, env, cwd, hpc)?;
                // Opened while it waits, suspended: its handle outlives it.
                let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
                let shell = unsafe { OpenProcess(access, 0, started.dwProcessId) };
                MADE.with_borrow_mut(|made| made.1 = Some(owned(shell).unwrap()));
                Ok(started)
            },
            ..WINDOWS
        };

        /// The error of a refused Windows call.
        fn denied<T>() -> io::Result<T> {
            Err(io::Error::from_raw_os_error(5))
        }

        /// A Command Prompt started with `calls`, which fail: the error, and the console and
        /// shell made before it.
        fn failed(calls: Calls) -> (String, Option<HPCON>, Option<OwnedHandle>) {
            let dir = tempfile::tempdir().unwrap();
            let cwd = dir.path().to_str().unwrap();
            MADE.set(Default::default());
            let started = start(
                calls,
                cwd,
                size(80, 24),
                dir.path(),
                Vec::new(),
                TerminalShell::Cmd,
            );
            let err = started.map(drop).unwrap_err().to_string();
            let (console, shell) = MADE.take();
            (err, console, shell)
        }

        /// Whether `console` is closed within a few seconds (on its own thread).
        fn closed(console: HPCON) -> bool {
            (0..250).any(|_| {
                std::thread::sleep(Duration::from_millis(20));
                CLOSED.lock().unwrap().contains(&console)
            })
        }

        /// The exit code of `process`, once it ended within a few seconds.
        fn exit_code(process: &OwnedHandle) -> u32 {
            assert!(ends(process));
            let mut code = 0;
            unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) };
            code
        }

        /// Whether a session of `process` is kept.
        fn kept(process: &OwnedHandle) -> bool {
            let id = unsafe { GetProcessId(process.as_raw_handle()) };
            sessions().contains_key(&(id as i32))
        }

        #[tokio::test]
        async fn a_start_that_fails_before_its_shell_leaves_nothing() {
            let refused = denied::<()>().unwrap_err().to_string();
            let pipe = Calls {
                pipe: |_, _, _| denied(),
                ..RECORDED
            };
            let console = Calls {
                console: |_, _, _| denied(),
                ..RECORDED
            };
            for calls in [pipe, console] {
                let (err, console, shell) = failed(calls);
                assert_eq!(err, refused);
                assert!(console.is_none() && shell.is_none());
            }
            let job = Calls {
                job: denied,
                ..RECORDED
            };
            let create = Calls {
                create: |_, _, _, _| denied(),
                ..RECORDED
            };
            for calls in [job, create] {
                let (err, console, shell) = failed(calls);
                assert_eq!(err, refused);
                assert!(shell.is_none());
                assert!(closed(console.unwrap()));
            }
        }

        #[tokio::test]
        async fn a_shell_that_cannot_join_its_job_is_killed() {
            let (err, console, shell) = failed(Calls {
                assign: |_, _| denied(),
                ..RECORDED
            });
            assert_eq!(err, denied::<()>().unwrap_err().to_string());
            let shell = shell.unwrap();
            assert_eq!(exit_code(&shell), KILLED);
            assert!(!kept(&shell));
            assert!(closed(console.unwrap()));
        }

        #[tokio::test]
        async fn a_shell_that_cannot_resume_or_be_waited_for_ends_with_its_job() {
            let resume = Calls {
                resume: |_| denied(),
                ..RECORDED
            };
            let wait = Calls {
                thread: |_| denied(),
                ..RECORDED
            };
            for calls in [resume, wait] {
                let (err, console, shell) = failed(calls);
                assert_eq!(err, denied::<()>().unwrap_err().to_string());
                let shell = shell.unwrap();
                assert!(ends(&shell));
                assert!(!kept(&shell));
                assert!(closed(console.unwrap()));
            }
        }

        #[tokio::test]
        async fn without_a_thread_the_console_closes_on_this_one() {
            let (_, console, _) = failed(Calls {
                thread: |_| denied(),
                ..RECORDED
            });
            // Already closed: no wait.
            assert!(CLOSED.lock().unwrap().contains(&console.unwrap()));
        }

        #[tokio::test]
        async fn a_console_pipe_is_new_unguessable_and_connected() {
            let (a, b) = (pipe_name(), pipe_name());
            let prefix = format!(r"\\.\pipe\hive-console-{}-", std::process::id());
            for name in [&a, &b] {
                let random = name.rsplit('-').next().unwrap();
                assert!(name.starts_with(&prefix), "{name}");
                assert_eq!(random.len(), 16, "{name}");
                assert!(random.bytes().all(|c| c.is_ascii_hexdigit()), "{name}");
            }
            assert_ne!(a.rsplit('-').next(), b.rsplit('-').next());
            // A name someone made first is refused.
            let user = process_user(std::process::id()).unwrap();
            let _first = pipe(&a, &user, true).unwrap();
            let err = pipe(&a, &user, false).map(drop).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
            // Not opened by the console host yet: not connected.
            let alone = instance(&b, &user, true).unwrap();
            let err = connected(&alone).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        }

        #[tokio::test]
        async fn a_resize_after_the_end_is_dropped_and_typing_still_goes_through() {
            use std::io::Read;

            let user = process_user(std::process::id()).unwrap();
            let (ours, mut host) = pipe(&pipe_name(), &user, true).unwrap();
            let (input, typed) = mpsc::unbounded_channel();
            // No console: its terminal ended.
            let fed = tokio::spawn(feed(ours, Weak::new(), typed));
            input.send(Input::Resize { cols: 1, rows: 1 }).unwrap();
            input.send(Input::Data(Bytes::from_static(b"x"))).unwrap();
            let read = tokio::task::spawn_blocking(move || {
                let mut byte = [0];
                host.read_exact(&mut byte).map(|()| byte)
            });
            let read = tokio::time::timeout(SHOWN, read).await.unwrap();
            assert_eq!(read.unwrap().unwrap(), *b"x");
            // Until the terminal is dropped.
            drop(input);
            tokio::time::timeout(SHOWN, fed).await.unwrap().unwrap();
        }

        /// How long a shell gets to show something (a mutant that breaks it fails within 120 s).
        const SHOWN: Duration = Duration::from_secs(20);

        /// A terminal of a shell in a temporary folder, its output gathered as it comes (the
        /// console host waits for its output to be read).
        struct Shell {
            dir: tempfile::TempDir,
            session: i32,
            input: mpsc::UnboundedSender<Input>,
            child: Child,
            output: Arc<Mutex<String>>,
        }

        impl Shell {
            fn start(shell: TerminalShell) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let (cwd, bin) = (dir.path().to_str().unwrap(), dir.path().join("bin"));
                let env = [("HIVE_TEST", "x".to_owned())];
                let started = spawn(3, cwd, (200, 30), &bin, &env, shell).unwrap();
                let (terminal, input, mut pty, child) = started;
                let output = Arc::<Mutex<String>>::default();
                let gathered = output.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0; 4096];
                    while let Ok(n @ 1..) = pty.read(&mut buf).await {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        gathered.lock().unwrap().push_str(&text);
                    }
                });
                let session = terminal.session;
                Self {
                    dir,
                    session,
                    input,
                    child,
                    output,
                }
            }

            fn type_line(&self, line: &str) {
                let typed = Bytes::from(format!("{line}\r"));
                self.input.send(Input::Data(typed)).unwrap();
            }

            fn has(&self, text: &str) -> bool {
                self.output.lock().unwrap().contains(text)
            }

            /// Waits until the output shows `text`.
            async fn shows(&self, text: &str) {
                let shown = async {
                    while !self.has(text) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                };
                let waited = tokio::time::timeout(SHOWN, shown).await;
                let output = self.output.lock().unwrap().clone();
                assert!(waited.is_ok(), "no {text:?} in {output:?}");
            }

            /// The shell's exit code, once it exited.
            async fn exit_code(&mut self) -> Option<i32> {
                let exited = tokio::time::timeout(SHOWN, self.child.wait()).await;
                exited.unwrap().unwrap().code()
            }
        }

        /// Whether `process` ends within a few seconds.
        fn ends(process: &OwnedHandle) -> bool {
            let waited = unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) };
            waited == WAIT_OBJECT_0
        }

        #[tokio::test]
        async fn a_terminal_runs_its_shell_in_its_folder_with_its_environment() {
            let mut shell = Shell::start(TerminalShell::Cmd);
            // The prompt names the folder.
            shell
                .shows(&format!("{}>", shell.dir.path().display()))
                .await;
            shell.type_line("echo t=%HIVE_TERMINAL_ID%/%HIVE_TEST%/%TERM%");
            shell.shows("t=3/x/xterm-256color").await;
            shell.type_line("echo p=%PATH%");
            let bin = shell.dir.path().join("bin");
            shell.shows(&format!("p={};", bin.display())).await;
            // Its processes are listed, in its session.
            let me = Proc {
                pid: shell.session,
                pgrp: shell.session,
                session: shell.session,
                comm: "cmd".into(),
            };
            assert!(list().contains(&me), "{:?}", list());
            // It exits with its own code, told as often as asked.
            shell.type_line("exit 7");
            assert_eq!(shell.exit_code().await, Some(7));
            assert_eq!(shell.exit_code().await, Some(7));
            // Nothing is left to end: at once.
            let started = Instant::now();
            end_sessions(&[shell.session]).await;
            assert!(started.elapsed() < GRACE, "{:?}", started.elapsed());
            assert!(!list().contains(&me));
        }

        #[tokio::test]
        async fn powershell_by_default_and_the_console_follows_its_terminals_size() {
            // The runner has PowerShell 7 on its `PATH`.
            let shell = Shell::start(TerminalShell::Default);
            shell.type_line(r#""ed=" + $PSVersionTable.PSEdition"#);
            shell.shows("ed=Core").await;
            let resize = Input::Resize {
                cols: 132,
                rows: 40,
            };
            shell.input.send(resize).unwrap();
            let window = "$Host.UI.RawUI.WindowSize";
            shell.type_line(&format!(
                r#""size=" + {window}.Width + "x" + {window}.Height"#
            ));
            shell.shows("size=132x40").await;
            end_sessions(&[shell.session]).await;
        }

        #[tokio::test]
        async fn ending_closes_the_console_and_its_shell_exits_by_itself() {
            let mut shell = Shell::start(TerminalShell::Cmd);
            shell.shows(">").await;
            let started = Instant::now();
            end_sessions(&[shell.session]).await;
            assert!(started.elapsed() < GRACE, "{:?}", started.elapsed());
            assert_ne!(shell.exit_code().await, Some(KILLED as i32));
            // Ended once: again, nothing happens.
            end_sessions(&[shell.session]).await;
        }

        #[tokio::test]
        async fn what_outlives_its_console_is_killed_after_the_grace() {
            let shell = Shell::start(TerminalShell::Cmd);
            // `start` gives ping a console of its own, which closing the terminal's leaves.
            shell.type_line("start \"\" /min ping -n 60 127.0.0.1");
            let ping = async {
                loop {
                    let procs = list().into_iter();
                    let mut mine = procs.filter(|p| p.session == shell.session);
                    if let Some(ping) = mine.find(|p| p.comm.eq_ignore_ascii_case("ping")) {
                        return ping.pid;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            let ping = tokio::time::timeout(SHOWN, ping).await.unwrap();
            // Opened while it runs: its handle outlives it.
            let ping = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, ping as u32) });
            let ping = ping.unwrap();
            let started = Instant::now();
            end_sessions(&[shell.session]).await;
            assert!(started.elapsed() >= GRACE, "{:?}", started.elapsed());
            assert!(ends(&ping));
        }

        #[tokio::test]
        async fn a_terminal_in_a_missing_folder_does_not_start() {
            let dir = tempfile::tempdir().unwrap();
            let missing = dir.path().join("missing");
            let missing = missing.to_str().unwrap();
            let started = spawn(1, missing, (80, 24), dir.path(), &[], TerminalShell::Cmd);
            let err = started.map(drop).unwrap_err();
            let why = format!("cannot start a terminal in {missing}: ");
            assert!(err.starts_with(&why), "{err}");
        }

        #[test]
        fn a_killed_process_exits_with_the_killed_code() {
            // `ping` waits a second between tries: about 30 s unless killed.
            let mut ping = std::process::Command::new("ping")
                .args(["-n", "30", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            kill(ping.as_raw_handle());
            assert_eq!(ping.wait().unwrap().code(), Some(KILLED as i32));
        }

        #[tokio::test]
        async fn a_job_kills_what_joined_it_and_a_process_that_cannot_join_is_killed() {
            // `cmd` waits for the `ping` it starts (about 30 s): both are in the job.
            let mut cmd = std::process::Command::new("cmd")
                .args(["/c", "ping -n 30 127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let job = Job::of(&cmd).unwrap();
            let both = async {
                loop {
                    let listed = pids(&job.0);
                    if let Some(&ping) = listed.iter().find(|&&pid| pid != cmd.id()) {
                        return ping;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            let ping = tokio::time::timeout(SHOWN, both).await.unwrap();
            let ping = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, ping) }).unwrap();
            job.kill();
            assert_eq!(cmd.wait().unwrap().code(), Some(KILLED as i32));
            assert!(ends(&ping));
            // Left out of a job (none made, or not joined), it would run without a limit.
            let failures: [(_, Assign); 2] = [
                (Err(io::Error::other("no job")), assign),
                (super::job(), |_, _| denied()),
            ];
            for (job, assign) in failures {
                let mut lone = std::process::Command::new("ping")
                    .args(["-n", "30", "127.0.0.1"])
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                assert!(Job::joined(job, lone.as_raw_handle(), assign).is_err());
                assert_eq!(lone.wait().unwrap().code(), Some(KILLED as i32));
            }
        }

        #[tokio::test]
        async fn npms_claude_is_a_claude_in_its_terminal() {
            let shell = Shell::start(TerminalShell::Cmd);
            // A stand-in for the CLI npm installs, run by the runner's `node`.
            let package = shell.dir.path().join(r"npm\@anthropic-ai\claude-code");
            std::fs::create_dir_all(&package).unwrap();
            let cli = package.join("cli.js");
            std::fs::write(&cli, "setTimeout(() => {}, 30000);").unwrap();
            shell.type_line(&format!("node \"{}\"", cli.display()));
            let claude = async {
                loop {
                    let procs = list().into_iter();
                    let mut mine = procs.filter(|p| p.session == shell.session);
                    if mine.any(|p| p.comm == "claude") {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            tokio::time::timeout(SHOWN, claude).await.unwrap();
            end_sessions(&[shell.session]).await;
        }

        #[tokio::test]
        async fn powershells_working_folder_follows_its_location() {
            let shell = Shell::start(TerminalShell::Default);
            let elsewhere = tempfile::tempdir().unwrap();
            let holds = |dir: &Path| {
                let found = crate::windows::inside(dir);
                found.iter().any(|p| p.pid == shell.session)
            };
            // `cd` alone would leave it in the folder it started in.
            let at = elsewhere.path().display();
            shell.type_line(&format!("Set-Location -LiteralPath '{at}'"));
            let followed = async {
                while !holds(elsewhere.path()) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            tokio::time::timeout(SHOWN, followed).await.unwrap();
            assert!(!holds(shell.dir.path()));
            // A location outside the file system keeps the last folder, and the prompt.
            shell.type_line("Set-Location Env:");
            shell.shows(r"PS Env:\>").await;
            assert!(holds(elsewhere.path()));
            end_sessions(&[shell.session]).await;
        }

        #[tokio::test]
        async fn a_program_git_bash_starts_is_found_where_it_runs() {
            let shell = Shell::start(TerminalShell::GitBash);
            let sub = shell.dir.path().join("sub");
            std::fs::create_dir(&sub).unwrap();
            // Its `cd` moves no working folder of its own, but a program starts in it.
            let at = sub.display();
            shell.type_line(&format!("cd '{at}' && ping -n 30 127.0.0.1 > /dev/null"));
            let ping = async {
                loop {
                    let found = crate::windows::inside(&sub);
                    if found.iter().any(|p| p.comm.eq_ignore_ascii_case("ping")) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            tokio::time::timeout(SHOWN, ping).await.unwrap();
            end_sessions(&[shell.session]).await;
        }

        #[test]
        fn sizes_fit_a_console_and_failed_results_are_errors() {
            let fits = size(80, 24);
            assert_eq!((fits.X, fits.Y), (80, 24));
            let most = size(u16::MAX, 40_000);
            assert_eq!((most.X, most.Y), (i16::MAX, i16::MAX));
            hresult(0).unwrap();
            hresult(1).unwrap();
            // E_FAIL.
            assert!(hresult(0x8000_4005_u32 as i32).is_err());
        }
    }
}

/// Windows PowerShell asked for its `PATH`: the one the service inherited from the app,
/// which on Windows is the user's (a shell's config does not change it). Printed with a
/// bare `\n`, as [`crate::wrapper::user_path`] reads it.
pub fn path_shell() -> (OsString, Vec<OsString>) {
    let print = "[Console]::Out.Write($env:Path + [char]10)";
    let args = ["-NoProfile", "-NonInteractive", "-Command", print];
    ("powershell".into(), args.map(OsString::from).to_vec())
}

pub use terminal::Job;

/// The process table (`sysinfo`): every process, named by its file (see
/// [`conpty::comm`]), in its terminal's session (0 outside Hive's terminals).
pub fn list() -> Vec<Proc> {
    table(ProcessRefreshKind::nothing(), |_| true)
}

/// The processes whose working folder is `dir` or inside it: every process's is read at once.
/// Both are [`resolved`] first: a process may name its folder by an 8.3 short name, through a
/// junction, a link or a `subst` drive.
pub fn inside(dir: &Path) -> Vec<Proc> {
    let dir = resolved(dir);
    let kind = ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always);
    table(kind, |process| {
        process.cwd().is_some_and(|cwd| under(&resolved(cwd), &dir))
    })
}

/// `path` as [`crate::paths::canonical`] resolves it (long names, links followed, no `\\?\`),
/// or as it is when it cannot be (e.g. it is gone).
fn resolved(path: &Path) -> PathBuf {
    crate::paths::canonical(path).unwrap_or_else(|_| path.to_owned())
}

/// The processes `keep` keeps, read as `kind` says. Only the terminals' processes get their
/// command line read (one read of the process's memory each), which names npm's `claude`, a
/// `node`.
fn table(kind: ProcessRefreshKind, keep: impl Fn(&Process) -> bool) -> Vec<Proc> {
    let sessions = terminal::jobs();
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let terminals: Vec<Pid> = sessions.keys().map(|&pid| Pid::from_u32(pid)).collect();
    let args = ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always);
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&terminals), false, args);
    let kept = system.processes().values().filter(|process| keep(process));
    kept.map(|process| {
        let pid = process.pid().as_u32();
        let name = process.name().to_string_lossy();
        Proc {
            pid: pid as i32,
            pgrp: pid as i32,
            session: sessions.get(&pid).copied().unwrap_or_default(),
            comm: conpty::comm(&name, process.cmd()).to_owned(),
        }
    })
    .collect()
}

/// Whether `path` is `dir` or inside it, names compared ignoring (ASCII) case as Windows does
/// (a resolved path has its names' own case, one that could not be resolved may not).
fn under(path: &Path, dir: &Path) -> bool {
    let mut names = path.components();
    dir.components().all(|name| {
        let same = |of: Component| of.as_os_str().eq_ignore_ascii_case(name.as_os_str());
        names.next().is_some_and(same)
    })
}

/// How the user's archive `script` runs: in the terminals' shell (`terminal.shell`, found on
/// the service's `PATH` as for a terminal), PowerShell with `-NoProfile -Command`, Command
/// Prompt with `/D /S /C` (the script passed as it is: `cmd` has quoting rules of its own),
/// Git Bash with `-c`.
pub fn script(script: &str, shell: TerminalShell) -> io::Result<Command> {
    // `cmd /C` runs the first line alone: the others would be skipped without a word.
    if shell == TerminalShell::Cmd && script.contains(['\n', '\r']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Command Prompt runs only the first line of a script: put the archive script on one \
             line (commands joined with &&), or choose another shell",
        ));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let root = std::env::var_os("SystemRoot");
    let program = conpty::shell(shell, &path, root.as_deref()).map_err(io::Error::other)?;
    let mut command = Command::new(program.into_iter().next().unwrap_or_default());
    match shell {
        TerminalShell::Default => command.args(["-NoProfile", "-Command", script]),
        TerminalShell::Cmd => command
            .args(["/D", "/S", "/C"])
            .raw_arg(format!("\"{script}\"")),
        TerminalShell::GitBash => command.args(["-c", script]),
    };
    Ok(command)
}

/// Where the program `name` (`gh`) is on `path` alone: the first `<name>.exe` in its folders.
/// (Given `name`, Windows would also look beside Hive, in its system folders and on the
/// service's own `PATH`.)
pub fn on_path(name: &OsStr, path: &OsStr) -> Option<PathBuf> {
    let exe = Path::new(name).with_extension("exe");
    // A relative folder would be looked in from the service's own folder.
    let dirs = std::env::split_paths(path).filter(|dir| dir.is_absolute());
    let mut found = dirs.map(|dir| dir.join(&exe));
    found.find(|file| file.is_file())
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

/// `err`, told plainly when another program may hold the file `name` open without sharing it:
/// Windows then refuses to open it (a sharing or lock violation) or to replace it (access
/// denied, also what a file Hive may not write gets), and a save leaves it as it was.
pub fn in_use(err: io::Error, name: &str) -> io::Error {
    let held = [
        ERROR_SHARING_VIOLATION,
        ERROR_LOCK_VIOLATION,
        ERROR_ACCESS_DENIED,
    ];
    if !held
        .map(|code| Some(code as i32))
        .contains(&err.raw_os_error())
    {
        return err;
    }
    io::Error::other(format!(
        "{name} is open in another program, or cannot be replaced: close it there and save \
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
        for code in [
            ERROR_SHARING_VIOLATION,
            ERROR_LOCK_VIOLATION,
            ERROR_ACCESS_DENIED,
        ] {
            let err = in_use(io::Error::from_raw_os_error(code as i32), "a.rs").to_string();
            let said = "a.rs is open in another program, or cannot be replaced: close it there \
                        and save again (";
            assert!(err.starts_with(said), "{err}");
        }
        // Not found (2) is said as it is.
        let other = in_use(io::Error::from_raw_os_error(2), "a.rs");
        assert_eq!(other.raw_os_error(), Some(2));
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
    fn every_process_is_listed_and_found_by_its_folder_in_any_case() {
        let me = std::process::id() as i32;
        let listed = list();
        let found = listed.iter().find(|p| p.pid == me).unwrap();
        // Outside Hive's terminals, under its file's name.
        assert_eq!((found.pgrp, found.session), (me, 0));
        assert!(found.comm.starts_with("hive"), "{found:?}");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Work");
        std::fs::create_dir(&dir).unwrap();
        // `ping` waits a second between tries: about 30 s unless killed.
        let mut ping = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .current_dir(&dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = ping.id() as i32;
        let upper = PathBuf::from(dir.to_string_lossy().to_uppercase());
        for dir in [dir.as_path(), upper.as_path(), tmp.path()] {
            let found = inside(dir);
            let ping = found.iter().find(|p| p.pid == pid);
            assert!(
                ping.is_some_and(|p| p.comm.eq_ignore_ascii_case("ping")),
                "{dir:?}: {found:?}"
            );
        }
        // A folder beside it, whose name starts the same, holds nothing.
        let other = tmp.path().join("Workshop");
        std::fs::create_dir(&other).unwrap();
        assert_eq!(inside(&other), []);
        ping.kill().unwrap();
        ping.wait().unwrap();
    }

    #[test]
    fn a_process_is_found_by_its_folders_real_path() {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

        let tmp = tempfile::tempdir().unwrap();
        // The runner's temporary folder is itself an 8.3 short name (`RUNNER~1`).
        let real = resolved(tmp.path()).join("A long folder name");
        std::fs::create_dir(&real).unwrap();
        let long = wide(&real).unwrap();
        let mut buf = vec![0_u16; 1024];
        let n = unsafe { GetShortPathNameW(long.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        let short = PathBuf::from(OsString::from_wide(&buf[..n as usize]));
        assert_ne!(short, real);
        let link = tmp.path().join("link");
        let made = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .args([&link, &real])
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        // Started in the short name, or through the junction; asked by either name.
        for cwd in [&short, &link] {
            let mut ping = Command::new("ping")
                .args(["-n", "30", "127.0.0.1"])
                .current_dir(cwd)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            for dir in [&real, &short] {
                let found = inside(dir);
                let pid = ping.id() as i32;
                assert!(
                    found.iter().any(|p| p.pid == pid),
                    "{cwd:?} {dir:?}: {found:?}"
                );
            }
            ping.kill().unwrap();
            ping.wait().unwrap();
        }
        // What cannot be resolved is compared as it is.
        let gone = tmp.path().join("gone");
        assert_eq!(resolved(&gone), gone);
    }

    #[test]
    fn a_folder_holds_what_is_under_it_named_in_any_case() {
        let under = |path: &str, dir: &str| under(Path::new(path), Path::new(dir));
        assert!(under(r"C:\r\wt\", r"C:\r\wt"));
        assert!(under(r"c:\R\WT\src", r"C:\r\wt"));
        assert!(!under(r"C:\r\wt2", r"C:\r\wt"));
        assert!(!under(r"C:\r", r"C:\r\wt"));
        assert!(!under(r"D:\r\wt", r"C:\r\wt"));
    }

    #[test]
    fn archive_scripts_run_in_the_chosen_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let env = [("HIVE_PORT", "20000".to_owned())];
        let pwsh = r#"Set-Content -Path out -Value ("p=" + $env:HIVE_PORT + " " + $PSVersionTable.PSEdition)"#;
        let shells = [
            // The runner has PowerShell 7 on its `PATH`.
            (TerminalShell::Default, pwsh, "p=20000 Core"),
            // Its own quoting: the line as it is.
            (
                TerminalShell::Cmd,
                r#"echo "p=%HIVE_PORT%"> out"#,
                r#""p=20000""#,
            ),
            (
                TerminalShell::GitBash,
                r#"echo "p=$HIVE_PORT" > out"#,
                "p=20000",
            ),
        ];
        let time = Duration::from_secs(20);
        for (shell, script, written) in shells {
            crate::scripts::run(script, tmp.path(), &env, time, shell).unwrap();
            let out = std::fs::read_to_string(tmp.path().join("out")).unwrap();
            assert_eq!(out.trim_end(), written, "{shell:?}");
        }
    }

    #[test]
    fn a_program_is_found_on_the_path_it_is_given_only() {
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        let tmp = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([tmp.path(), &system]).unwrap();
        let cmd = OsStr::new("cmd");
        assert_eq!(on_path(cmd, &path), Some(system.join("cmd.exe")));
        assert_eq!(on_path(cmd, tmp.path().as_os_str()), None);
        // A relative folder is never looked in, though it holds the program.
        let here = tempfile::Builder::new()
            .prefix(".hive-test-")
            .tempdir_in(".")
            .unwrap();
        // Named from the current folder (the crate's).
        let relative = Path::new(here.path().file_name().unwrap());
        std::fs::write(relative.join("gh.exe"), "").unwrap();
        assert!(relative.join("gh.exe").is_file());
        let path = std::env::join_paths([relative, &system]).unwrap();
        assert_eq!(on_path(OsStr::new("gh"), &path), None);
    }

    #[test]
    fn command_prompt_refuses_a_script_of_more_than_one_line() {
        for script in ["echo a\necho b", "echo a\r\necho b", "echo a\r"] {
            let err = super::script(script, TerminalShell::Cmd).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert!(
                err.to_string().starts_with("Command Prompt runs only"),
                "{err}"
            );
        }
        // The other shells run every line.
        for shell in [TerminalShell::Default, TerminalShell::GitBash] {
            super::script("echo a\necho b", shell).unwrap();
        }
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
