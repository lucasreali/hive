//! PTY-backed terminals running fish. Pass-through only: no scrollback is kept here.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use hive_protocol::Frame;
#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;
/// What a terminal's output is read from.
#[cfg(unix)]
pub use pty_process::OwnedReadPty as Pty;
#[cfg(unix)]
use pty_process::{OwnedWritePty, Size};
#[cfg(unix)]
use tokio::io::AsyncWriteExt;
/// A terminal's shell, waited for by its pump.
#[cfg(unix)]
pub use tokio::process::Child;
use tokio::sync::{mpsc, watch};

#[cfg(unix)]
use crate::procs;
use crate::watch::Watch;
#[cfg(windows)]
pub use crate::windows::terminal::{Child, Pty, end_sessions, spawn};

/// Time a terminal's processes get to exit after SIGHUP (Windows: its console closed) before
/// they are killed.
pub(crate) const GRACE: Duration = Duration::from_secs(2);

/// A running terminal, as kept in the service registry.
pub struct Terminal {
    /// Session id of the shell (it is the session leader, so this is also its pid).
    pub session: i32,
    /// Unhooked-`claude` detector for this terminal.
    pub watch: Watch,
    /// When the PTY last printed something (the silence rule of agent states).
    pub last_output: LastOutput,
    /// The Claude config folder its space gave it (6.14), where its agents' sessions are;
    /// `None` for the service's own.
    pub claude_dir: Option<String>,
}

pub enum Input {
    Data(Bytes),
    Resize { cols: u16, rows: u16 },
}

/// When a terminal last printed something, shared by its output pump and the registry
/// without a lock, so output never waits for one (9.13).
#[derive(Clone)]
pub struct LastOutput {
    start: Instant,
    /// Nanoseconds after `start`.
    since: Arc<AtomicU64>,
}

impl LastOutput {
    pub(crate) fn new() -> Self {
        Self {
            start: Instant::now(),
            since: Arc::default(),
        }
    }

    /// Records output now.
    pub fn touch(&self) {
        let since = u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.since.store(since, Ordering::Relaxed);
    }

    pub fn get(&self) -> Instant {
        self.start + Duration::from_nanos(self.since.load(Ordering::Relaxed))
    }
}

/// Output the app has not acknowledged above which a terminal's PTY is no longer read (9.19).
pub const HIGH_WATER: usize = 512 * 1024;
/// Output the app has not acknowledged under which a paused terminal's PTY is read again.
pub const LOW_WATER: usize = 128 * 1024;

/// A terminal's output on its way to the app (9.19): its queue holds at most what the app
/// has not acknowledged as written to its screen, [`HIGH_WATER`] plus one read, so a flooding
/// terminal slows its own program down, never another terminal nor the service's memory.
/// Clones share the count: the pump sends, the app's `ack` lowers it, without a lock.
#[derive(Clone)]
pub struct Output {
    channel: u32,
    frames: mpsc::UnboundedSender<Frame>,
    unacked: watch::Sender<usize>,
}

impl Output {
    pub fn new(channel: u32, frames: mpsc::UnboundedSender<Frame>) -> Self {
        Self {
            channel,
            frames,
            unacked: watch::Sender::new(0),
        }
    }

    /// Queues `bytes` for the app; above [`HIGH_WATER`] unacknowledged, returns only once
    /// under [`LOW_WATER`]. False when the app is gone.
    pub async fn send(&self, bytes: &[u8]) -> bool {
        // Counted first: the app may acknowledge it as soon as it is queued.
        self.unacked.send_modify(|n| *n += bytes.len());
        let frame = Frame::terminal(self.channel, Bytes::copy_from_slice(bytes));
        if self.frames.send(frame).is_err() {
            return false;
        }
        if *self.unacked.borrow() > HIGH_WATER {
            // Never fails: `self` is a sender.
            let _ = self.unacked.subscribe().wait_for(|&n| n < LOW_WATER).await;
        }
        true
    }

    /// The app wrote `bytes` of this terminal's output to its screen.
    pub fn ack(&self, bytes: u32) {
        let bytes = bytes as usize;
        self.unacked.send_modify(|n| *n = n.saturating_sub(bytes));
    }
}

/// Starts the shell (see [`shell`]) on a new PTY in `cwd`, with `bin_dir` first on `PATH`
/// and `HIVE_TERMINAL_ID` set, plus `env` (its space's, 6.14, and its worktree's `HIVE_*`,
/// 6.8). Returns the registry entry, its input queue (typing and resizes; it ends once
/// every sender is dropped), the output side and the child. `_shell` is the native Windows
/// setting (12.5.3): Unix terminals always run the shell above.
#[cfg(unix)]
pub fn spawn(
    id: u32,
    cwd: &str,
    (cols, rows): (u16, u16),
    bin_dir: &Path,
    env: &[(&'static str, String)],
    _shell: hive_protocol::TerminalShell,
) -> Result<(Terminal, mpsc::UnboundedSender<Input>, Pty, Child), String> {
    let start = || -> pty_process::Result<_> {
        let (pty, pts) = pty_process::open()?;
        pty.resize(Size::new(rows, cols))?;
        let child = shell(bin_dir)
            .env("HIVE_TERMINAL_ID", id.to_string())
            .env("TERM", "xterm-256color")
            .envs(env.iter().cloned())
            .current_dir(cwd)
            .spawn(pts)?;
        // The shell leads its own session, so its pid is the session id. A child always has
        // one right after spawning; without it there is no session to end, so no terminal.
        let pid = child
            .id()
            .ok_or(std::io::Error::other("the shell has no pid"))?;
        Ok((pty, child, pid))
    };
    let (pty, child, pid) =
        start().map_err(|err| format!("cannot start a terminal in {cwd}: {err}"))?;
    let (output, writer) = pty.into_split();
    let (input, input_rx) = mpsc::unbounded_channel();
    tokio::spawn(feed(writer, input_rx));
    Ok((
        Terminal {
            session: pid as i32,
            watch: Watch::default(),
            last_output: LastOutput::new(),
            claude_dir: None,
        },
        input,
        output,
        child,
    ))
}

/// fish, with `bin_dir` put first on `PATH` after the user's config (WSL).
#[cfg(target_os = "linux")]
fn shell(bin_dir: &Path) -> pty_process::Command {
    pty_process::Command::new("fish")
        .arg("-C")
        .arg(path_command(bin_dir))
}

#[cfg(target_os = "macos")]
use crate::macos::shell;

/// fish command run after the user's config: puts `bin_dir` first on `PATH` for this shell only.
/// Never `fish_add_path` without flags: it would persist through a universal variable.
fn path_command(bin_dir: &Path) -> String {
    let dir = bin_dir
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    format!("set -gx PATH '{dir}' $PATH")
}

/// How a login shell starts on macOS so that Hive's bin dir stays first on `PATH`: a GUI
/// app gets only `/usr/bin:/bin`, and the login files (`path_helper` among them) reorder
/// `PATH`, so the bin dir goes first only after they ran. Portable, so tested everywhere.
pub mod login {
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};

    /// Used when `$SHELL` is unset: macOS's default shell.
    const DEFAULT_SHELL: &str = "/bin/zsh";

    /// A shell's program, arguments and extra environment.
    #[derive(Debug, PartialEq, Eq)]
    pub struct Launch {
        pub program: OsString,
        pub args: Vec<OsString>,
        pub env: Vec<(&'static str, OsString)>,
    }

    /// Hive's shell startup files, beside the bin dir: `<data>/hive/shell`.
    pub fn startup_dir(bin_dir: &Path) -> PathBuf {
        bin_dir.with_file_name("shell")
    }

    /// How to start `shell` (`$SHELL`) as a login shell. zsh reads Hive's `ZDOTDIR`, whose
    /// files run the user's (from `zdotdir`, else `HOME`) and then put the bin dir first;
    /// bash reads Hive's rc file, which does the same with the login profile; fish runs a
    /// command after its config. Any other shell only gets the bin dir first on `path`.
    pub fn launch(
        shell: Option<OsString>,
        zdotdir: Option<OsString>,
        path: Option<OsString>,
        bin_dir: &Path,
    ) -> Launch {
        // The service's environment: empty counts as unset.
        let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
        let (shell, zdotdir, path) = (set(shell), set(zdotdir), set(path));
        let program = shell.unwrap_or_else(|| DEFAULT_SHELL.into());
        let name = Path::new(&program).file_name().unwrap_or_default();
        let startup = startup_dir(bin_dir);
        let bin = OsString::from(bin_dir);
        let (args, env): (Vec<OsString>, _) = match name.to_str() {
            Some("fish") => {
                let command = super::path_command(bin_dir);
                (vec!["-l".into(), "-C".into(), command.into()], vec![])
            }
            Some("zsh") => {
                let mut env = vec![("HIVE_BIN_DIR", bin), ("ZDOTDIR", startup.into())];
                env.extend(zdotdir.map(|dir| ("HIVE_USER_ZDOTDIR", dir)));
                (vec!["-l".into()], env)
            }
            Some("bash") => {
                let rc = startup.join("bashrc").into();
                (vec!["--rcfile".into(), rc], vec![("HIVE_BIN_DIR", bin)])
            }
            _ => {
                let mut first = bin;
                if let Some(path) = path {
                    first.push(":");
                    first.push(path);
                }
                (vec!["-l".into()], vec![("PATH", first)])
            }
        };
        Launch { program, args, env }
    }

    /// Runs one of the user's zsh startup files from their `ZDOTDIR`, keeping Hive's in
    /// between (the user's `.zshenv` may move their `ZDOTDIR`).
    fn zsh_file(name: &str) -> String {
        format!(
            "ZDOTDIR=$HIVE_USER_ZDOTDIR\n\
             [[ -f $ZDOTDIR/{name} ]] && builtin source $ZDOTDIR/{name}\n\
             HIVE_USER_ZDOTDIR=$ZDOTDIR\n\
             ZDOTDIR=$HIVE_ZDOTDIR\n"
        )
    }

    /// The startup files, by name: zsh's four (`.zlogin` runs last in a login shell) and
    /// bash's rc file, which reads what a login bash reads.
    pub fn startup_files() -> Vec<(&'static str, String)> {
        let header =
            "# Written by Hive: runs your startup files, then puts Hive's bin dir first on PATH.\n";
        let zshenv = "HIVE_ZDOTDIR=$ZDOTDIR\nHIVE_USER_ZDOTDIR=${HIVE_USER_ZDOTDIR:-$HOME}\n";
        let zlogin = "ZDOTDIR=$HIVE_USER_ZDOTDIR\n\
                      export PATH=\"$HIVE_BIN_DIR:$PATH\"\n\
                      unset HIVE_BIN_DIR HIVE_ZDOTDIR HIVE_USER_ZDOTDIR\n";
        let bashrc = "[ -f /etc/profile ] && . /etc/profile\n\
                      if [ -f ~/.bash_profile ]; then . ~/.bash_profile\n\
                      elif [ -f ~/.bash_login ]; then . ~/.bash_login\n\
                      elif [ -f ~/.profile ]; then . ~/.profile\n\
                      fi\n\
                      export PATH=\"$HIVE_BIN_DIR:$PATH\"\n\
                      unset HIVE_BIN_DIR\n";
        vec![
            (
                ".zshenv",
                format!("{header}{zshenv}{}", zsh_file(".zshenv")),
            ),
            (".zprofile", format!("{header}{}", zsh_file(".zprofile"))),
            (".zshrc", format!("{header}{}", zsh_file(".zshrc"))),
            (
                ".zlogin",
                format!("{header}{}{zlogin}", zsh_file(".zlogin")),
            ),
            ("bashrc", format!("{header}{bashrc}")),
        ]
    }

    /// Writes the startup files next to the bin dir, replacing earlier versions.
    pub fn install(bin_dir: &Path) -> io::Result<()> {
        let dir = startup_dir(bin_dir);
        std::fs::create_dir_all(&dir)?;
        for (name, contents) in startup_files() {
            crate::wrapper::write_atomic(&dir.join(name), contents.as_bytes(), 0o644)?;
        }
        Ok(())
    }

    // Unix shells and paths: macOS's login shells.
    #[cfg(unix)]
    #[cfg(test)]
    mod tests {
        use super::*;

        fn os(values: &[&str]) -> Vec<OsString> {
            values.iter().map(OsString::from).collect()
        }

        #[test]
        fn each_shell_starts_as_a_login_shell_with_the_bin_dir_last_on_top() {
            let bin = Path::new("/d/hive/bin");
            let at = |shell: Option<&str>, zdotdir: Option<&str>| {
                launch(
                    shell.map(OsString::from),
                    zdotdir.map(OsString::from),
                    Some("/usr/bin:/bin".into()),
                    bin,
                )
            };
            let zsh = Launch {
                program: "/bin/zsh".into(),
                args: os(&["-l"]),
                env: vec![
                    ("HIVE_BIN_DIR", "/d/hive/bin".into()),
                    ("ZDOTDIR", "/d/hive/shell".into()),
                ],
            };
            assert_eq!(at(None, None), zsh);
            assert_eq!(at(Some("/bin/zsh"), None), zsh);
            let mut own = at(Some("/opt/zsh"), Some("/u/z"));
            assert_eq!(own.env.pop(), Some(("HIVE_USER_ZDOTDIR", "/u/z".into())));
            assert_eq!(own.env, zsh.env);
            let bash = at(Some("/bin/bash"), None);
            assert_eq!(bash.args, os(&["--rcfile", "/d/hive/shell/bashrc"]));
            assert_eq!(bash.env, vec![("HIVE_BIN_DIR", "/d/hive/bin".into())]);
            let fish = at(Some("/opt/homebrew/bin/fish"), None);
            let set = "set -gx PATH '/d/hive/bin' $PATH";
            assert_eq!(fish.args, os(&["-l", "-C", set]));
            assert!(fish.env.is_empty());
            let other = at(Some("/bin/ksh"), None);
            assert_eq!(other.args, os(&["-l"]));
            assert_eq!(
                other.env,
                vec![("PATH", "/d/hive/bin:/usr/bin:/bin".into())]
            );
            let bare = launch(Some("sh".into()), None, Some("".into()), bin);
            assert_eq!(bare.env, vec![("PATH", "/d/hive/bin".into())]);
            let empty = launch(Some("".into()), Some("".into()), None, bin);
            assert_eq!(empty, zsh);
        }

        #[test]
        fn startup_files_are_written_beside_the_bin_dir() {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            install(&bin).unwrap();
            for (name, contents) in startup_files() {
                let written = std::fs::read_to_string(dir.path().join("shell").join(name));
                assert_eq!(written.unwrap(), contents, "{name}");
            }
            let zshrc = std::fs::read_to_string(dir.path().join("shell/.zshrc")).unwrap();
            assert!(zshrc.contains("builtin source $ZDOTDIR/.zshrc"), "{zshrc}");
        }

        #[test]
        fn bash_runs_the_login_profile_then_puts_the_bin_dir_first() {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            install(&bin).unwrap();
            let home = dir.path().join("home");
            std::fs::create_dir(&home).unwrap();
            let profile = "export PATH=/user/first:$PATH\nexport HIVE_TEST_RC=read\n";
            std::fs::write(home.join(".bash_profile"), profile).unwrap();
            let bash = launch(Some("bash".into()), None, None, &bin);
            let echo = "echo \"rc=$HIVE_TEST_RC first=${PATH%%:*} left=$HIVE_BIN_DIR.\"";
            let out = std::process::Command::new(&bash.program)
                .args(&bash.args)
                .args(["-i", "-c", echo])
                .env_clear()
                .env("HOME", &home)
                .env("PATH", "/usr/bin:/bin")
                .envs(bash.env)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let want = format!("rc=read first={} left=.", bin.display());
            assert!(stdout.contains(&want), "{out:?}");
        }
    }
}

/// How a terminal starts on native Windows (12.5.3): its shell, command line and environment.
/// Portable, so tested everywhere; `crate::windows::terminal` starts it on a ConPTY.
pub mod conpty {
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};

    use hive_protocol::TerminalShell;

    /// The program and arguments of `shell`, programs found on `path` (the service's `PATH`,
    /// else left to `CreateProcessW`'s search): `pwsh` when there, else Windows PowerShell;
    /// `cmd`; Git Bash, the `bin\bash.exe` two folders above the first `git.exe` that has one
    /// (`<Git>\cmd\git.exe`).
    pub fn shell(shell: TerminalShell, path: &OsStr) -> Result<Vec<OsString>, String> {
        let on_path = |name: &'static str| {
            let dirs = std::env::split_paths(path);
            dirs.map(move |dir| dir.join(name))
                .filter(|file| file.is_file())
        };
        let find = |name| on_path(name).next().map(PathBuf::into_os_string);
        match shell {
            TerminalShell::Default => {
                let found = find("pwsh.exe").or_else(|| find("powershell.exe"));
                Ok(vec![found.unwrap_or_else(|| "powershell.exe".into())])
            }
            TerminalShell::Cmd => Ok(vec![find("cmd.exe").unwrap_or_else(|| "cmd.exe".into())]),
            TerminalShell::GitBash => {
                let above = |mut git: PathBuf| {
                    git.pop();
                    git.pop();
                    git.extend(["bin", "bash.exe"]);
                    git
                };
                let bash = on_path("git.exe").map(above).find(|bash| bash.is_file());
                let bash = bash.ok_or("Git Bash was not found: no git.exe on PATH")?;
                let args = [bash.into_os_string(), "--login".into(), "-i".into()];
                Ok(args.to_vec())
            }
        }
    }

    /// The command line of `args` as `CreateProcessW` takes it: an argument holding a space
    /// or a tab is quoted.
    // ponytail: no escaping of `"` or a trailing `\`: the arguments are a program's path (no
    // `"` in a Windows path, and it ends in `.exe`) and fixed words.
    pub fn command_line(args: &[OsString]) -> OsString {
        let mut line = OsString::new();
        for (n, arg) in args.iter().enumerate() {
            let bytes = arg.as_encoded_bytes();
            let quote = if bytes.contains(&b' ') || bytes.contains(&b'\t') {
                "\""
            } else {
                ""
            };
            line.push(if n == 0 { "" } else { " " });
            line.push(quote);
            line.push(arg);
            line.push(quote);
        }
        line
    }

    /// A terminal's environment: `base` (the service's) with `set` over it, names compared
    /// ignoring case as Windows does, then `bin_dir` first on `PATH`; sorted by name, as
    /// `CreateProcessW` wants it.
    pub fn environment(
        base: impl IntoIterator<Item = (OsString, OsString)>,
        set: impl IntoIterator<Item = (OsString, OsString)>,
        bin_dir: &Path,
    ) -> Vec<(OsString, OsString)> {
        let mut vars = BTreeMap::new();
        for (name, value) in base.into_iter().chain(set) {
            vars.insert(name.to_string_lossy().to_uppercase(), (name, value));
        }
        let (name, old) = vars
            .remove("PATH")
            .unwrap_or_else(|| ("Path".into(), OsString::new()));
        let mut path = OsString::from(bin_dir);
        if !old.is_empty() {
            path.push(";");
            path.push(old);
        }
        vars.insert("PATH".into(), (name, path));
        vars.into_values().collect()
    }

    /// A process's name as the unhooked-`claude` watcher compares it: its file's, without
    /// `.exe`.
    pub fn comm(exe: &str) -> &str {
        match exe.split_at_checked(exe.len().saturating_sub(4)) {
            Some((name, ext)) if ext.eq_ignore_ascii_case(".exe") => name,
            _ => exe,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn os(values: &[&str]) -> Vec<OsString> {
            values.iter().map(OsString::from).collect()
        }

        /// Empty files at `names` under `root`.
        fn files(root: &Path, names: &[&str]) {
            for name in names {
                let file = root.join(name);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, "").unwrap();
            }
        }

        #[test]
        fn powershell_is_pwsh_when_on_path_else_windows_powershell() {
            let dir = tempfile::tempdir().unwrap();
            let (pwsh, windows) = (dir.path().join("7"), dir.path().join("v1.0"));
            files(
                dir.path(),
                &["7/pwsh.exe", "v1.0/powershell.exe", "cmd/cmd.exe"],
            );
            let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
            let run = |dirs: &[&Path]| shell(TerminalShell::Default, &path(dirs)).unwrap();
            let empty = dir.path().join("cmd");
            assert_eq!(
                run(&[&empty, &windows, &pwsh]),
                [pwsh.join("pwsh.exe").into_os_string()]
            );
            assert_eq!(
                run(&[&empty, &windows]),
                [windows.join("powershell.exe").into_os_string()]
            );
            assert_eq!(run(&[&empty]), os(&["powershell.exe"]));
            let cmd = shell(TerminalShell::Cmd, &path(&[&windows, &empty])).unwrap();
            assert_eq!(cmd, [empty.join("cmd.exe").into_os_string()]);
            assert_eq!(
                shell(TerminalShell::Cmd, OsStr::new("")).unwrap(),
                os(&["cmd.exe"])
            );
        }

        #[test]
        fn git_bash_is_the_bash_two_folders_above_a_git_on_path() {
            let dir = tempfile::tempdir().unwrap();
            files(
                dir.path(),
                &["Other/cmd/git.exe", "Git/cmd/git.exe", "Git/bin/bash.exe"],
            );
            let (other, git) = (dir.path().join("Other/cmd"), dir.path().join("Git/cmd"));
            let path = std::env::join_paths([&other, &git]).unwrap();
            let bash = dir.path().join("Git/bin/bash.exe").into_os_string();
            assert_eq!(
                shell(TerminalShell::GitBash, &path).unwrap(),
                [bash, "--login".into(), "-i".into()]
            );
            // Only a git without its bash: none.
            let none = std::env::join_paths([&other]).unwrap();
            assert_eq!(
                shell(TerminalShell::GitBash, &none).unwrap_err(),
                "Git Bash was not found: no git.exe on PATH"
            );
        }

        #[test]
        fn arguments_with_blanks_are_quoted() {
            let args = os(&[
                r"C:\Program Files\Git\bin\bash.exe",
                "--login",
                "a\tb",
                "-i",
            ]);
            assert_eq!(
                command_line(&args),
                "\"C:\\Program Files\\Git\\bin\\bash.exe\" --login \"a\tb\" -i"
            );
            assert_eq!(command_line(&os(&["cmd.exe"])), "cmd.exe");
            assert_eq!(command_line(&[]), "");
        }

        #[test]
        fn the_environment_is_the_services_with_the_terminals_over_it_and_the_bin_dir_first() {
            let pairs = |vars: &[(&str, &str)]| -> Vec<(OsString, OsString)> {
                vars.iter().map(|(k, v)| (k.into(), v.into())).collect()
            };
            let base = pairs(&[("Path", r"C:\W"), ("b", "1"), ("A", "0"), ("TERM", "x")]);
            let set = pairs(&[("term", "xterm-256color"), ("HIVE_TERMINAL_ID", "3")]);
            let env = environment(base, set, Path::new(r"C:\hive\bin"));
            let want = pairs(&[
                ("A", "0"),
                ("b", "1"),
                ("HIVE_TERMINAL_ID", "3"),
                ("Path", r"C:\hive\bin;C:\W"),
                ("term", "xterm-256color"),
            ]);
            assert_eq!(env, want);
            // No `PATH`, or an empty one: only the bin dir.
            let bin = pairs(&[("Path", r"C:\hive\bin")]);
            assert_eq!(environment([], [], Path::new(r"C:\hive\bin")), bin);
            let empty = pairs(&[("PATH", "")]);
            let env = environment(empty, [], Path::new(r"C:\hive\bin"));
            assert_eq!(env, pairs(&[("PATH", r"C:\hive\bin")]));
        }

        #[test]
        fn a_process_name_drops_its_exe() {
            assert_eq!(comm("claude.exe"), "claude");
            assert_eq!(comm("PING.EXE"), "PING");
            assert_eq!(comm("claude"), "claude");
            assert_eq!(comm("exe"), "exe");
            assert_eq!(comm(".exe"), "");
            // Not cut inside a character.
            assert_eq!(comm("aé.ex"), "aé.ex");
        }
    }
}

#[cfg(unix)]
async fn feed(mut pty: OwnedWritePty, mut input: mpsc::UnboundedReceiver<Input>) {
    while let Some(input) = input.recv().await {
        // A dead PTY fails every write; the loop ends when the terminal is dropped.
        let _ = match input {
            Input::Data(bytes) => pty.write_all(&bytes).await.map_err(|_| ()),
            Input::Resize { cols, rows } => pty.resize(Size::new(rows, cols)).map_err(|_| ()),
        };
    }
}

/// Ends every process group in the given sessions: SIGHUP, then SIGKILL for
/// whatever is still alive after [`GRACE`].
#[cfg(unix)]
pub async fn end_sessions(sessions: &[i32]) {
    signal(&in_sessions(sessions).await, Signal::SIGHUP);
    let _ = tokio::time::timeout(GRACE, async {
        while !in_sessions(sessions).await.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    signal(&in_sessions(sessions).await, Signal::SIGKILL);
}

/// The processes in `sessions`, read from `/proc` on a blocking thread (9.13).
#[cfg(unix)]
async fn in_sessions(sessions: &[i32]) -> Vec<procs::Proc> {
    let all = tokio::task::spawn_blocking(|| procs::list(procs::Source::System)).await;
    let all = all.unwrap_or_default().into_iter();
    all.filter(|p| sessions.contains(&p.session)).collect()
}

#[cfg(unix)]
fn signal(procs: &[procs::Proc], signal: Signal) {
    for proc in procs {
        let _ = killpg(Pid::from_raw(proc.pgrp), signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only a terminal's spawn makes one (12.5.3 on Windows).
    #[cfg(unix)]
    #[test]
    fn last_output_is_shared_by_its_clones() {
        let last = LastOutput::new();
        let pump = last.clone();
        let started = last.get();
        std::thread::sleep(Duration::from_millis(5));
        pump.touch();
        assert!(last.get() >= started + Duration::from_millis(5));
        assert!(last.get() <= Instant::now());
    }

    /// Whether `send` is still waiting for acknowledgements.
    async fn waits(send: &mut std::pin::Pin<&mut impl Future<Output = bool>>) -> bool {
        tokio::time::timeout(Duration::from_millis(50), send.as_mut())
            .await
            .is_err()
    }

    /// Whether `send` returns true without waiting for acknowledgements (bounded, so a
    /// mutant that always waits fails instead of hanging).
    async fn flows(send: impl Future<Output = bool>) -> bool {
        tokio::time::timeout(Duration::from_secs(5), send)
            .await
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn output_pauses_above_the_high_water_and_resumes_under_the_low_water() {
        let (frames, mut queued) = mpsc::unbounded_channel();
        let output = Output::new(7, frames);
        let app = output.clone();
        // Up to the high water mark, output flows.
        assert!(flows(output.send(&vec![b'y'; HIGH_WATER - 1])).await);
        assert!(flows(output.send(b"y")).await);
        // One byte more and the terminal waits, with that byte already queued.
        let send = output.send(b"!");
        tokio::pin!(send);
        assert!(waits(&mut send).await);
        let sizes: Vec<usize> = std::iter::from_fn(|| queued.try_recv().ok())
            .map(|frame| {
                assert_eq!(frame.channel, 7);
                frame.payload.len()
            })
            .collect();
        assert_eq!(sizes, [HIGH_WATER - 1, 1, 1]);
        // Down to the low water mark it still waits (no flapping at the high one).
        app.ack(u32::try_from(HIGH_WATER + 1 - LOW_WATER).unwrap());
        assert!(waits(&mut send).await);
        app.ack(1);
        assert!(flows(send).await);
        // Acknowledging more than was sent counts as everything.
        app.ack(u32::MAX);
        assert!(flows(output.send(&vec![b'y'; HIGH_WATER])).await);
    }

    #[test]
    fn the_water_marks_are_the_task_defaults() {
        // 9.19's defaults (pending human review): the app's 64 KiB batches stay under the low one.
        assert_eq!((HIGH_WATER, LOW_WATER), (524_288, 131_072));
    }

    #[tokio::test]
    async fn output_to_a_gone_app_fails() {
        let (frames, queued) = mpsc::unbounded_channel();
        drop(queued);
        assert!(!Output::new(1, frames).send(b"x").await);
    }

    #[test]
    fn path_command_quotes_the_directory_for_fish() {
        assert_eq!(
            path_command(Path::new("/d/hive/bin")),
            "set -gx PATH '/d/hive/bin' $PATH"
        );
        assert_eq!(
            path_command(Path::new("/it's a\\dir")),
            r"set -gx PATH '/it\'s a\\dir' $PATH"
        );
    }
}
