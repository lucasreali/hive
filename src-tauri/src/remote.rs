//! The app's links to remote hosts over ssh (26.17; `docs/spike/ssh.md` §6.1–6.2): one link per
//! host of the local service's `remote.hosts`, each a `hive bridge --remote` on that host reached
//! through the user's `ssh` and `~/.ssh/config`.
//!
//! - **The command** is `ssh` with fixed options, `--`, the validated alias and a constant remote
//!   command: nothing from input is spliced into it. sshd joins that command into a string for
//!   the user's login shell (bash, zsh, fish, csh), so it is one line, single-quoted, with no `'`,
//!   `\` or `!`.
//! - **Check and install:** the remote command runs `~/.hive/bin/hive bridge --remote` when its
//!   `--version` is this app's, else prints `hive-needs <uname -s> <uname -m>` on stderr. A Linux
//!   x86_64 or aarch64 host then gets this app's static build (26.13) on the upload command's
//!   stdin, whose byte count and SHA-256 are checked on the host before the file runs at all,
//!   then its `--version`, before the rename. Any other OS or CPU is refused (Q5).
//! - **Failures** are told from `ssh`'s stderr: a host key not known or changed (Q6), a login
//!   that needs a password or passphrase, the platform, a remote service of another version
//!   (Q8); none of them is retried. Anything else retries after 1, 2, 5, 10 and 30 s, then every
//!   30 s while the host keeps terminals; waking from sleep retries at once (Q7).

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hive_protocol::{check_host, Control, RemoteSettings, Role, PROTOCOL_VERSION};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::{dropped, lock, pipe, speak, start, End, Link, STDERR_LIMIT, VERSION};

/// The `ssh` the app runs: the system's.
pub const SSH: &str = "/usr/bin/ssh";

/// `ssh`'s options, each its own argument (spike §6.1). Given on the command line, they win over
/// the user's `~/.ssh/config`; everything else (`HostName`, `User`, `ProxyJump`, the agent,
/// `ControlMaster`, Q12) comes from there.
const OPTIONS: [&str; 13] = [
    "-T",
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=yes",
    "-o",
    "ClearAllForwardings=yes",
    "-o",
    "ServerAliveInterval=10",
    "-o",
    "ServerAliveCountMax=3",
    "-o",
    "ConnectTimeout=15",
];

/// The start of each remote script: `$1` is this app's version, `d` Hive's bin folder there
/// (`$HIVE_HOME/bin` when that is absolute, else `~/.hive/bin`, as the local bridge's).
macro_rules! bin {
    () => {
        r#"v=$1; case "$HIVE_HOME" in /*) d=$HIVE_HOME/bin ;; *) d=$HOME/.hive/bin ;; esac; "#
    };
}

/// Runs the remote `hive bridge --remote` (`--replace` when `$2` says so) when its version is
/// this app's; else names the host's OS and CPU on stderr and exits 3.
const RUN: &str = concat!(
    bin!(),
    r#"h=$d/hive; if [ -x "$h" ] && [ "$("$h" --version 2>/dev/null)" = "hive $v" ]; then exec "$h" bridge --remote $2; fi; echo "hive-needs $(uname -s) $(uname -m)" >&2; exit 3"#
);

/// Installs the `hive` on stdin, after a first line `<byte count> <sha256>`: written private as
/// `hive.new` in a private folder, checked by byte count and checksum before it ever runs, then
/// by its `--version`, and only then renamed over `hive` (a running service keeps its file).
const UPLOAD: &str = concat!(
    bin!(),
    r#"umask 077; mkdir -p "$d" || exit 1; read -r n s || exit 1; f=$d/hive.new; rm -f "$f"; cat > "$f" || exit 1; [ $(wc -c < "$f") = "$n" ] || { rm -f "$f"; echo "hive-upload: the uploaded size does not match" >&2; exit 4; }; set -- $(sha256sum "$f"); [ "$1" = "$s" ] || { rm -f "$f"; echo "hive-upload: the uploaded checksum does not match" >&2; exit 4; }; chmod 755 "$f" && [ "$("$f" --version)" = "hive $v" ] || { rm -f "$f"; echo "hive-upload: the uploaded hive does not run" >&2; exit 4; }; mv -f "$f" "$d/hive""#
);

/// What a remote command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    /// Check the remote `hive` and run its bridge.
    Run,
    /// The same, ending the running service first (`hive bridge --replace`, Q8).
    Replace,
    /// Install the `hive` given on stdin.
    Upload,
}

/// The constant remote command of `script`: from compile-time text only.
pub fn command(script: Script) -> String {
    let (text, extra) = match script {
        Script::Run => (RUN, ""),
        Script::Replace => (RUN, " --replace"),
        Script::Upload => (UPLOAD, ""),
    };
    format!("exec /bin/sh -c '{text}' hive {VERSION}{extra}")
}

/// A host alias `ssh` may be given ([`check_host`]: no leading `-`, no whitespace or control
/// characters, at most 255 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias(String);

impl Alias {
    pub fn new(alias: String) -> Result<Self, String> {
        check_host(&alias)?;
        Ok(Self(alias))
    }
}

/// `ssh`'s arguments for `script` on the host `alias`: the alias is the only one that varies.
pub fn ssh_args(alias: &Alias, script: Script) -> Vec<OsString> {
    let mut args: Vec<OsString> = OPTIONS.iter().map(OsString::from).collect();
    args.extend(["--", &alias.0].map(OsString::from));
    args.push(command(script).into());
    args
}

/// A CPU Hive has a Linux build for (Q4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    fn cpu(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }

    /// Its build's file name in the app's resources (26.13).
    pub fn build(self) -> String {
        format!("hive-linux-{}", self.cpu())
    }
}

/// The most characters of a host's `uname` text shown.
const UNAME_LIMIT: usize = 64;

/// Why a link failed or ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The host key is not known or has changed (Q6).
    HostKey,
    /// The login needed a password or passphrase (`BatchMode`).
    Auth,
    /// The host needs this app's `hive`, for this CPU.
    Needs(Arch),
    /// A host Hive does not run on (Q5): its `uname -s` and `-m`, cut and printable.
    Platform(String),
    /// The upload failed, or did not take.
    Install(String),
    /// A service of another version runs there (Q8), with its terminals, if it says.
    Version(String, Option<u64>),
    /// Anything else: the network, or a link that dropped, with what `ssh` said.
    Network(String),
}

impl Failure {
    /// The `kind` the UI gets.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::HostKey => "host_key",
            Self::Auth => "auth",
            Self::Needs(_) | Self::Install(_) => "install",
            Self::Platform(_) => "platform",
            Self::Version(..) => "version",
            Self::Network(_) => "network",
        }
    }

    /// Whether trying again may help: not for a host key, a login, a platform, an install or a
    /// version.
    pub fn retries(&self) -> bool {
        matches!(self, Self::Network(_))
    }

    /// What the UI shows for host `alias`.
    pub fn message(&self, alias: &str) -> String {
        match self {
            Self::HostKey => format!(
                "{alias}'s host key is not known (or has changed). Run `ssh {alias}` once in a terminal to check and accept its key."
            ),
            Self::Auth => format!(
                "{alias} asked for a password or passphrase; add the key to your ssh agent (`ssh-add --apple-use-keychain`)."
            ),
            Self::Needs(arch) => format!(
                "Installing Hive on {alias} did not take: it still needs Hive {VERSION} for {}.",
                arch.cpu()
            ),
            Self::Platform(uname) if uname.starts_with("Darwin") => {
                format!("{alias} runs macOS; Hive connects only to Linux hosts.")
            }
            Self::Platform(uname) => format!(
                "{alias} runs {uname}; Hive connects only to Linux x86_64 and aarch64 hosts."
            ),
            Self::Install(why) => format!("Installing Hive on {alias} failed: {why}"),
            Self::Version(version, terminals) => {
                let terminals = terminals.map_or("its".to_owned(), |n| n.to_string());
                format!(
                    "{alias} runs Hive {version} with {terminals} terminals; this app is Hive {VERSION}."
                )
            }
            Self::Network(why) => format!("Cannot reach {alias}: {why}"),
        }
    }
}

/// What `ssh`'s stderr `text` says went wrong.
pub fn classify(text: &str) -> Failure {
    let line = |prefix: &str| {
        let found = text
            .lines()
            .find_map(|line| line.trim().strip_prefix(prefix));
        found.map(str::to_owned)
    };
    if let Some(uname) = line("hive-needs ") {
        let printable = |c: char| if c.is_control() { ' ' } else { c };
        let shown: String = uname.chars().take(UNAME_LIMIT).map(printable).collect();
        return match shown.split_once(' ') {
            Some(("Linux", "x86_64")) => Failure::Needs(Arch::X86_64),
            Some(("Linux", "aarch64")) => Failure::Needs(Arch::Aarch64),
            _ => Failure::Platform(shown),
        };
    }
    if let Some(why) = line("hive-upload: ") {
        return Failure::Install(why);
    }
    if text.contains("Host key verification failed")
        || text.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
    {
        return Failure::HostKey;
    }
    if text.contains("Permission denied") {
        return Failure::Auth;
    }
    let first = text.lines().map(str::trim).find(|line| !line.is_empty());
    Failure::Network(first.unwrap_or("the link closed").to_owned())
}

/// The waits before each reconnect (Q7, ledger 26.1 (1)), then every 30 s while the host keeps
/// terminals.
const LADDER: [u64; 5] = [1, 2, 5, 10, 30];

/// The wait before reconnect try `attempt` (0 first), or none: the ladder ran out and the host
/// keeps no terminals (`kept`).
pub fn backoff(attempt: usize, kept: bool) -> Option<Duration> {
    match LADDER.get(attempt) {
        Some(secs) => Some(Duration::from_secs(*secs)),
        None => kept.then_some(Duration::from_secs(30)),
    }
}

/// The upload command's stdin for the build `bytes`: its byte count and SHA-256, then the bytes.
pub fn upload_input(bytes: &[u8]) -> Vec<u8> {
    let sum = Sha256::digest(bytes);
    let hex: String = sum.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut input = format!("{} {hex}\n", bytes.len()).into_bytes();
    input.extend_from_slice(bytes);
    input
}

/// A random id for this app launch (26.15): a new link of the same app takes over a stale one.
/// Not a secret (spike §8.9).
pub fn app_id() -> String {
    let half = || RandomState::new().build_hasher().finish();
    format!("{:016x}{:016x}", half(), half())
}

/// How long an upload may take.
const UPLOAD_WAIT: Duration = Duration::from_secs(600);

/// The clock the reconnect ladder waits on: Tokio's, a fake one in tests.
type Sleep = dyn Fn(Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    + Send
    + Sync;

/// One remote host's link, and what drives it.
struct Remote {
    name: String,
    alias: Alias,
    link: Arc<Mutex<Link>>,
    /// A reconnect asked for (Reconnect, wake): ends a wait, and does nothing while the link
    /// is up or connecting.
    retry: Notify,
    /// The next try runs `hive bridge --replace` (Q8's "Restart the remote service").
    replace: AtomicBool,
    /// The link stopped on a failure that is not retried; only Reconnect tries again.
    stopped: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// The remote links (26.17), which the local service's `remote` settings start and end.
pub struct Remotes {
    ssh: OsString,
    /// The folder holding the Linux builds (`hive-linux-<cpu>`, the app's resources).
    builds: PathBuf,
    app_id: String,
    /// The terminal ids every link shares ([`Link::open`]).
    ids: Arc<AtomicU32>,
    /// `remote.keep_minutes`, in seconds, for the next `hello`.
    keep_secs: AtomicU64,
    hosts: Mutex<Vec<Arc<Remote>>>,
    sleep: Box<Sleep>,
    /// [`UPLOAD_WAIT`]; shorter in tests.
    upload_wait: Duration,
}

impl Remotes {
    pub fn new(ssh: OsString, builds: PathBuf, ids: Arc<AtomicU32>) -> Self {
        Self {
            ssh,
            builds,
            app_id: app_id(),
            ids,
            keep_secs: AtomicU64::new(u64::from(RemoteSettings::default().keep_minutes) * 60),
            hosts: Mutex::default(),
            sleep: Box::new(|wait| Box::pin(tokio::time::sleep(wait))),
            upload_wait: UPLOAD_WAIT,
        }
    }

    fn hosts(&self) -> std::sync::MutexGuard<'_, Vec<Arc<Remote>>> {
        self.hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn find(&self, host: &str) -> Option<Arc<Remote>> {
        self.hosts()
            .iter()
            .find(|remote| remote.name == host)
            .cloned()
    }

    /// Links to the hosts of `settings`, in order: a new host's link starts (with the UI `ui`),
    /// a host no longer there ends its link. Must run inside the Tokio runtime.
    pub fn sync(self: &Arc<Self>, settings: RemoteSettings, ui: Option<&Channel<Value>>) {
        let keep = u64::from(settings.keep_minutes) * 60;
        self.keep_secs.store(keep, SeqCst);
        let mut hosts = self.hosts();
        let (kept, gone) = std::mem::take(&mut *hosts)
            .into_iter()
            .partition::<Vec<_>, _>(|remote| settings.hosts.contains(&remote.name));
        for remote in gone {
            remote.end();
        }
        // The service checks each alias; one that is not valid is never linked to.
        let valid = settings
            .hosts
            .into_iter()
            .filter_map(|name| Alias::new(name).ok());
        for alias in valid {
            let known = kept.iter().find(|remote| remote.alias == alias);
            let remote = known.cloned().unwrap_or_else(|| self.start(alias, ui));
            hosts.push(remote);
        }
    }

    /// A new link to `name`, connecting.
    fn start(self: &Arc<Self>, alias: Alias, ui: Option<&Channel<Value>>) -> Arc<Remote> {
        let name = alias.0.clone();
        let link = Link {
            ui: ui.cloned(),
            ids: Arc::clone(&self.ids),
            host: Some(name.clone()),
            ..Link::default()
        };
        let remote = Arc::new(Remote {
            name,
            alias,
            link: Arc::new(Mutex::new(link)),
            retry: Notify::new(),
            replace: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            task: Mutex::default(),
        });
        let task = tokio::spawn(supervise(Arc::clone(self), Arc::clone(&remote)));
        *remote
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
        remote
    }

    /// The link to `host`.
    pub(crate) fn link(&self, host: &str) -> Option<Arc<Mutex<Link>>> {
        self.find(host).map(|remote| Arc::clone(&remote.link))
    }

    /// The link holding the terminal the UI knows as `ui`, and its frame channel there.
    pub(crate) fn terminal(&self, ui: u32) -> Option<(Arc<Mutex<Link>>, u32)> {
        self.hosts().iter().find_map(|remote| {
            let channel = lock(&remote.link).channel(ui)?;
            Some((Arc::clone(&remote.link), channel))
        })
    }

    /// A UI (re)connected: every link sends it there, after its state and what its service
    /// said (`Link::replay`); its old terminals close unless the UI reloaded after a crash.
    pub fn connect(&self, ui: &Option<Channel<Value>>, crashed: bool) {
        for remote in self.hosts().iter() {
            let mut link = lock(&remote.link);
            link.ui = ui.clone();
            if !crashed {
                for channel in std::mem::take(&mut link.terminals).into_keys() {
                    let _ = link.send(channel, &hive_protocol::Control::CloseTerminal);
                }
            }
            if let Some(status) = link.status.clone() {
                link.to_ui(status);
            }
            link.replay();
        }
    }

    /// Tries `host` again now, whatever stopped it.
    pub fn reconnect(&self, host: &str) -> Result<(), String> {
        let remote = self
            .find(host)
            .ok_or_else(|| format!("no link to the host {host}"))?;
        remote.stopped.store(false, SeqCst);
        remote.retry.notify_waiters();
        Ok(())
    }

    /// Restarts `host`'s service of another version (Q8): the next try ends it first.
    pub fn restart(&self, host: &str) -> Result<(), String> {
        let remote = self
            .find(host)
            .ok_or_else(|| format!("no link to the host {host}"))?;
        remote.replace.store(true, SeqCst);
        self.reconnect(host)
    }

    /// The computer woke (Q7): each link waiting to reconnect tries at once.
    pub fn wake(&self) {
        for remote in self.hosts().iter() {
            if !remote.stopped.load(SeqCst) {
                remote.retry.notify_waiters();
            }
        }
    }

    /// The app exits: every link ends, so no `ssh` outlives it. Its service keeps the terminals
    /// for the keep time, as for a dropped link.
    pub fn end(&self) {
        for remote in std::mem::take(&mut *self.hosts()) {
            remote.end();
        }
    }

    /// One try at `remote`'s link with `script`: why it failed or, once welcomed, why it ended,
    /// and whether it was welcomed. Its terminals stay for a reconnect.
    async fn connect_once(&self, remote: &Remote, script: Script) -> (Failure, bool) {
        let args = ssh_args(&remote.alias, script);
        let (stdout, stdin, exit) = match start(&self.ssh, &args) {
            Ok(started) => started,
            Err(error) => {
                return (
                    Failure::Network(format!("cannot start ssh: {error}")),
                    false,
                )
            }
        };
        let hello = Control::Hello {
            protocol: PROTOCOL_VERSION,
            version: VERSION.to_owned(),
            role: Role::App,
            keep_secs: Some(self.keep_secs.load(SeqCst)),
            app_id: Some(self.app_id.clone()),
        };
        let ended = speak(&remote.link, stdout, stdin, &hello).await;
        let failure = match ended {
            End::Refused => {
                let refused = lock(&remote.link).refused.take().unwrap_or_default();
                let version = refused["version"].as_str().unwrap_or_default();
                let version: String = version.chars().take(UNAME_LIMIT).collect();
                Failure::Version(version, refused["terminals"].as_u64())
            }
            End::Closed => classify(&exit.await),
            End::Broken(error) => Failure::Network(error),
        };
        let mut link = lock(&remote.link);
        let welcomed = link.welcome.is_some();
        dropped(&mut link);
        (failure, welcomed)
    }

    /// Installs this app's build for `arch` on `host` ([`Script::Upload`]).
    async fn upload(&self, host: &Alias, arch: Arch) -> Result<(), Failure> {
        let file = self.builds.join(arch.build());
        let bytes = std::fs::read(&file).map_err(|error| {
            Failure::Install(format!("this app has no {}: {error}", arch.build()))
        })?;
        let args = ssh_args(host, Script::Upload);
        let mut command = tokio::process::Command::new(&self.ssh);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let started = command.spawn().and_then(|mut child| {
            let stdin = pipe(child.stdin.take())?;
            let stderr = pipe(child.stderr.take())?;
            Ok((child, stdin, stderr))
        });
        let (mut child, mut stdin, stderr) =
            started.map_err(|error| Failure::Network(format!("cannot start ssh: {error}")))?;
        let input = upload_input(&bytes);
        let run = async {
            let mut text = Vec::new();
            let write = async move {
                // A failed write shows in the remote's checks.
                let _ = stdin.write_all(&input).await;
            };
            let mut stderr = stderr.take(STDERR_LIMIT);
            let read = stderr.read_to_end(&mut text);
            let _ = tokio::join!(write, read);
            (child.wait().await, text)
        };
        let Ok((status, text)) = tokio::time::timeout(self.upload_wait, run).await else {
            return Err(Failure::Install("the upload took too long".to_owned()));
        };
        if status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
        match classify(&String::from_utf8_lossy(&text)) {
            Failure::Network(why) => Err(Failure::Install(why)),
            failure => Err(failure),
        }
    }
}

impl Remote {
    fn link(&self) -> std::sync::MutexGuard<'_, Link> {
        lock(&self.link)
    }

    /// Ends the link: its `ssh` is killed, and the UI is told its terminals ended and the link
    /// is gone.
    fn end(&self) {
        let task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        task.iter().for_each(JoinHandle::abort);
        let mut link = self.link();
        dropped(&mut link);
        link.end_terminals();
        status(&mut link, json!({"state": "removed"}));
    }
}

/// Tells the UI the state of `link`'s host, `fields` with `type` and `host` added, and keeps it
/// for a reloaded UI.
fn status(link: &mut Link, mut fields: Value) {
    fields["type"] = "host_link".into();
    let message = link.tagged(fields);
    link.status = Some(message.clone());
    link.to_ui(message);
}

/// The remote's service welcomed the link: it is connected, and its kept terminals reattach.
pub(crate) fn welcomed(link: &mut Link) {
    status(link, json!({"state": "connected"}));
    reattach(link);
}

/// The hook for the terminals this link kept across a drop (26.16 attaches them to its service's
/// kept terminals). Until the service can attach one, each is told to the UI as ended.
fn reattach(link: &mut Link) {
    link.end_terminals();
}

/// Drives `remote`'s link until it is ended: connects, installs this app's `hive` when the host
/// needs it, and after a drop or a failure that may pass, waits on the ladder ([`backoff`]) or a
/// reconnect asked for; a failure that will not pass waits for Reconnect only.
async fn supervise(remotes: Arc<Remotes>, remote: Arc<Remote>) {
    let mut attempt = 0;
    let mut installed = false;
    loop {
        let script = if remote.replace.swap(false, SeqCst) {
            Script::Replace
        } else {
            Script::Run
        };
        status(&mut remote.link(), json!({"state": "connecting"}));
        let (mut failure, welcomed) = remotes.connect_once(&remote, script).await;
        if welcomed {
            (attempt, installed) = (0, false);
        }
        if let Failure::Needs(arch) = failure {
            if !installed {
                installed = true;
                let cpu = arch.cpu();
                status(
                    &mut remote.link(),
                    json!({"state": "installing", "cpu": cpu}),
                );
                match remotes.upload(&remote.alias, arch).await {
                    Ok(()) => continue,
                    Err(error) => failure = error,
                }
            }
        }
        let kept = !remote.link().terminals.is_empty();
        let wait = if failure.retries() {
            backoff(attempt, kept)
        } else {
            None
        };
        let reason = failure.message(&remote.name);
        // Waiting before the UI learns it, so a reconnect it asks for is never missed.
        let retry = remote.retry.notified();
        tokio::pin!(retry);
        retry.as_mut().enable();
        match wait {
            Some(wait) => {
                let secs = wait.as_secs();
                let state = json!({"state": "reconnecting", "reason": reason, "retry_secs": secs});
                status(&mut remote.link(), state);
                attempt += 1;
                tokio::select! {
                    () = (remotes.sleep)(wait) => {}
                    () = retry => {}
                }
            }
            None => {
                let mut state =
                    json!({"state": "disconnected", "reason": reason, "kind": failure.kind()});
                if let Failure::Version(version, terminals) = &failure {
                    state["version"] = version.as_str().into();
                    state["app_version"] = VERSION.into();
                    state["terminals"] = (*terminals).into();
                }
                remote.stopped.store(!failure.retries(), SeqCst);
                {
                    let mut link = remote.link();
                    link.end_terminals();
                    status(&mut link, state);
                }
                retry.await;
                (attempt, installed) = (0, false);
            }
        }
    }
}

#[cfg(test)]
mod tests;
