//! Where the service keeps its socket, lockfile and installed files.

#[cfg(unix)]
use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use tokio::net::UnixStream;

/// A connection to the service ([`Paths::connect`]).
#[cfg(unix)]
pub type Stream = UnixStream;
#[cfg(windows)]
pub type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$XDG_RUNTIME_DIR/hive`; when it is unset, on macOS the user's own temporary folder's
    /// `hive` (`$TMPDIR/hive`, D17.5), elsewhere `/tmp/hive-<uid>`.
    pub runtime: PathBuf,
    /// `$XDG_DATA_HOME/hive`, or `~/.local/share/hive`.
    pub data: PathBuf,
    /// `$XDG_CONFIG_HOME/hive`, or `~/.config/hive`.
    pub config: PathBuf,
}

// Windows: `from_env`, `prepare_runtime` and `connect` are in `crate::windows`.
impl Paths {
    #[cfg(unix)]
    pub fn from_env() -> Self {
        #[cfg(target_os = "macos")]
        let user_temp = Some(crate::macos::user_temp_dir());
        #[cfg(not(target_os = "macos"))]
        let user_temp = None;
        Self::resolve(
            |key| std::env::var_os(key),
            nix::unistd::getuid().as_raw(),
            user_temp,
        )
    }

    /// `user_temp` is `None` off macOS, and on macOS the user's temporary folder as the system
    /// gives it ([`crate::macos::user_temp_dir`]), which `TMPDIR` replaces only when unknown,
    /// and only when it is the user's own folder ([`own_folder`]). Under `/tmp`, which every
    /// user can write to, another user could create the folder first and so keep Hive from
    /// starting (risk 9.7): there the name carries the uid, as on Linux.
    #[cfg(unix)]
    fn resolve(
        var: impl Fn(&str) -> Option<OsString>,
        uid: u32,
        user_temp: Option<Option<PathBuf>>,
    ) -> Self {
        let var = |key| {
            var(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let user_temp = user_temp.map(|dir| {
            dir.filter(|dir| dir.is_absolute())
                .or_else(|| var("TMPDIR").filter(|dir| own_folder(dir, uid)))
        });
        let runtime = var("XDG_RUNTIME_DIR").or(user_temp.flatten()).map_or_else(
            || PathBuf::from(format!("/tmp/hive-{uid}")),
            |dir| dir.join("hive"),
        );
        let home = |xdg, under| var(xdg).or_else(|| var("HOME").map(|home| home.join(under)));
        let data = home("XDG_DATA_HOME", ".local/share")
            .map_or_else(|| runtime.join("data"), |dir| dir.join("hive"));
        let config = home("XDG_CONFIG_HOME", ".config")
            .map_or_else(|| runtime.join("config"), |dir| dir.join("hive"));
        Self {
            runtime,
            data,
            config,
        }
    }

    /// The service's socket; on Windows, the file naming its pipe (`windows::Listener::bind`).
    pub fn socket(&self) -> PathBuf {
        self.runtime.join("hive.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.runtime.join("hive.lock")
    }

    /// stderr of a daemon started by `hive bridge`: where a failed start explains itself.
    pub fn daemon_log(&self) -> PathBuf {
        self.runtime.join("daemon.log")
    }

    /// Directory put first on `PATH` inside Hive terminals (the `claude` wrapper lives here).
    pub fn bin_dir(&self) -> PathBuf {
        self.data.join("bin")
    }

    /// Hooks settings the `claude` wrapper passes to Claude Code with `--settings`.
    pub fn hooks_settings(&self) -> PathBuf {
        self.data.join("hive-hooks.json")
    }

    /// The projects the app followed before spaces (6.14): read only while `spaces` is missing.
    pub fn projects(&self) -> PathBuf {
        self.data.join("projects.json")
    }

    /// The spaces and the projects the app follows (`hive::projects`).
    pub fn spaces(&self) -> PathBuf {
        self.data.join("spaces.json")
    }

    /// The block of ports of each worktree (`hive::scripts`).
    pub fn ports(&self) -> PathBuf {
        self.data.join("ports.json")
    }

    /// The pull requests already offered for removing their worktree (`hive::offers`).
    pub fn offers(&self) -> PathBuf {
        self.data.join("offered-pulls.json")
    }

    /// The Claude sessions that ran in Hive's terminals when the app last closed.
    pub fn open_sessions(&self) -> PathBuf {
        self.data.join("open-sessions.json")
    }

    /// The user's settings (`hive::settings`).
    pub fn settings(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// Creates the runtime directory with mode `0700` and refuses one that
    /// another user owns or that others can access (e.g. a planted `/tmp/hive-<uid>`), or
    /// whose socket path is too long to bind.
    #[cfg(unix)]
    pub fn prepare_runtime(&self) -> io::Result<()> {
        check_socket_len(&self.socket(), SUN_PATH)?;
        self.make_runtime()?;
        self.check_runtime()
    }

    /// Creates the runtime directory (and its parents) with mode `0700` if it is missing;
    /// [`Paths::check_runtime`] then tells whether it can be used.
    #[cfg(unix)]
    pub fn make_runtime(&self) -> io::Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.runtime)
    }

    /// Refuses a runtime directory another user owns or others can access: the socket in it
    /// could be someone else's. Checked before the bridge and `hive hook` connect (a planted
    /// `/tmp/hive-<uid>` with a listening socket would get every keystroke and saved file).
    #[cfg(unix)]
    pub fn check_runtime(&self) -> io::Result<()> {
        self.check_runtime_as(nix::unistd::getuid().as_raw())
    }

    #[cfg(unix)]
    fn check_runtime_as(&self, uid: u32) -> io::Result<()> {
        // lstat: a symlink reports mode 0777 and fails the mode check.
        let meta = std::fs::symlink_metadata(&self.runtime)?;
        if meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other(format!(
                "refusing insecure runtime directory {} (must be a directory owned by you with mode 0700)",
                self.runtime.display()
            )));
        }
        Ok(())
    }

    /// Connects to the service socket, only in a runtime directory of ours and only to a
    /// service run by us: the peer check also covers a directory swapped after the check
    /// (e.g. an `XDG_RUNTIME_DIR` in a shared folder).
    #[cfg(unix)]
    pub async fn connect(&self) -> io::Result<UnixStream> {
        self.check_runtime()?;
        let stream = UnixStream::connect(self.socket()).await?;
        check_peer(&stream, nix::unistd::getuid().as_raw())?;
        Ok(stream)
    }
}

/// Whether `dir` is an absolute path to a folder `uid` owns and others cannot write to (links
/// followed): never a shared one such as `/tmp`, where another user could create Hive's folder
/// first.
#[cfg(unix)]
fn own_folder(dir: &Path, uid: u32) -> bool {
    dir.is_absolute()
        && std::fs::metadata(dir)
            .is_ok_and(|meta| meta.is_dir() && meta.uid() == uid && meta.mode() & 0o022 == 0)
}

/// `sizeof(sockaddr_un.sun_path)`: a socket path takes one byte less (its closing NUL).
#[cfg(unix)]
const SUN_PATH: usize = if cfg!(target_os = "macos") { 104 } else { 108 };

/// Refuses a socket path `bind` would reject with an obscure error ("path must be shorter
/// than SUN_LEN"), e.g. under a long `XDG_RUNTIME_DIR`.
#[cfg(unix)]
fn check_socket_len(socket: &Path, sun_path: usize) -> io::Result<()> {
    let len = socket.as_os_str().len();
    if len >= sun_path {
        return Err(io::Error::other(format!(
            "the socket path {} is {len} bytes, over the {} a Unix socket path can have: \
             set XDG_RUNTIME_DIR to a shorter folder",
            socket.display(),
            sun_path - 1
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn check_peer(stream: &UnixStream, uid: u32) -> io::Result<()> {
    let peer = stream.peer_cred()?.uid();
    if peer != uid {
        return Err(io::Error::other(format!(
            "refusing a hive socket run by another user (uid {peer})"
        )));
    }
    Ok(())
}

/// `path` resolved (links followed, as `canonicalize`), in the form git and other programs
/// read: Windows puts `\\?\` before a resolved path, which git refuses and which never
/// equals the same path spelled plainly. On Windows a link to a network or device path is
/// refused before anything follows it ([`crate::windows::local_links`]).
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    crate::windows::local_links(path)?;
    path.canonicalize().map(plain)
}

/// `path` without the `\\?\` before a drive (`\\?\C:\`), with `\\?\UNC\` turned into `\\`;
/// any other path as it is (`\\?\Volume{…}` has no plain form).
pub fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_str().unwrap_or_default();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.get(1..2) == Some(":") => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn resolved_paths_lose_the_verbatim_prefix() {
        let plain = |path: &str| plain(path.into());
        assert_eq!(plain(r"\\?\C:\Users\me"), PathBuf::from(r"C:\Users\me"));
        assert_eq!(
            plain(r"\\?\UNC\host\share\x"),
            PathBuf::from(r"\\host\share\x")
        );
        for kept in [r"\\?\Volume{1}\x", r"\\?\", r"C:\x", "/tmp/x", ""] {
            assert_eq!(plain(kept), PathBuf::from(kept), "{kept}");
        }
        let tmp = tempfile::tempdir().unwrap();
        let real = canonical(tmp.path()).unwrap();
        assert!(real.is_absolute() && real.is_dir(), "{real:?}");
        assert!(!real.to_string_lossy().starts_with(r"\\?\"), "{real:?}");
        assert_eq!(canonical(&real).unwrap(), real);
        assert!(canonical(&tmp.path().join("missing")).is_err());
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn resolve(vars: &[(&str, &str)]) -> Paths {
        Paths::resolve(
            |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into()),
            1000,
            None,
        )
    }

    /// As this process's user, so that the `TMPDIR` folders a test makes are its own.
    fn resolve_as(vars: &[(&str, &str)], user_temp: Option<Option<&str>>) -> Paths {
        Paths::resolve(
            |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into()),
            nix::unistd::getuid().as_raw(),
            user_temp.map(|dir| dir.map(PathBuf::from)),
        )
    }

    #[test]
    fn on_macos_the_runtime_dir_is_in_the_users_temporary_folder() {
        let me = nix::unistd::getuid().as_raw();
        let tmp = tempfile::tempdir().unwrap();
        let folder = |name: &str, mode: u32| {
            let dir = tmp.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
            dir.to_str().unwrap().to_owned()
        };
        let own = folder("T", 0o700);
        let tmpdir = ("TMPDIR", own.as_str());
        let fallback = PathBuf::from(format!("/tmp/hive-{me}"));
        let system = Some(Some("/var/folders/sys/T/"));
        let runtime = |vars: &[(&str, &str)], user_temp| resolve_as(vars, user_temp).runtime;
        // The system's answer first: a `TMPDIR` changed in a shell does not move it.
        assert_eq!(
            runtime(&[tmpdir], system),
            PathBuf::from("/var/folders/sys/T/hive")
        );
        let own_hive = Path::new(&own).join("hive");
        assert_eq!(runtime(&[tmpdir], Some(None)), own_hive);
        assert_eq!(runtime(&[tmpdir], Some(Some("rel"))), own_hive);
        // `XDG_RUNTIME_DIR` still wins (the tests, `scripts/mac-dev.sh`).
        assert_eq!(
            runtime(&[("XDG_RUNTIME_DIR", "/x"), tmpdir], system),
            PathBuf::from("/x/hive")
        );
        // Neither known, or a `TMPDIR` that is not the user's own folder: `/tmp/hive-<uid>`,
        // as on Linux. Never `/tmp/hive`, which another user could create first.
        let shared = folder("shared", 0o1777);
        let grouped = folder("grouped", 0o770);
        let file = tmp.path().join("file");
        std::fs::write(&file, "").unwrap();
        let file = file.to_str().unwrap().to_owned();
        let missing = tmp.path().join("missing").to_str().unwrap().to_owned();
        for dir in ["", "rel/T", "/tmp", &missing, &file, &shared, &grouped] {
            assert_eq!(runtime(&[("TMPDIR", dir)], Some(None)), fallback, "{dir}");
        }
        assert_eq!(runtime(&[], Some(None)), fallback);
        assert_eq!(runtime(&[], Some(Some("rel"))), fallback);
        // Off macOS `TMPDIR` is not read.
        assert_eq!(runtime(&[tmpdir], None), fallback);
    }

    #[test]
    fn a_temporary_folder_of_another_user_is_not_used() {
        let tmp = tempfile::tempdir().unwrap();
        let me = nix::unistd::getuid().as_raw();
        assert!(own_folder(tmp.path(), me));
        assert!(!own_folder(tmp.path(), me + 1));
    }

    #[test]
    fn a_socket_path_too_long_to_bind_is_refused_clearly() {
        let path = |len: usize| PathBuf::from(format!("/{}", "s".repeat(len - 1)));
        for sun_path in [104, 108] {
            check_socket_len(&path(sun_path - 1), sun_path).unwrap();
            let err = check_socket_len(&path(sun_path), sun_path).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "the socket path {} is {sun_path} bytes, over the {} a Unix socket path \
                     can have: set XDG_RUNTIME_DIR to a shorter folder",
                    path(sun_path).display(),
                    sun_path - 1
                )
            );
        }
        // The service refuses it before creating anything; one byte less binds.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("run").as_os_str().len() + "/hive.sock".len();
        let runtime = |len: usize| {
            let mut dir = tmp.path().join("run").into_os_string();
            dir.push("x".repeat(len - base));
            PathBuf::from(dir)
        };
        let paths = |len| Paths {
            runtime: runtime(len),
            ..paths_in(tmp.path())
        };
        let long = paths(SUN_PATH);
        assert_eq!(long.socket().as_os_str().len(), SUN_PATH);
        let err = long.prepare_runtime().unwrap_err();
        assert!(err.to_string().contains("XDG_RUNTIME_DIR"), "{err}");
        assert!(!long.runtime.exists());
        let longest = paths(SUN_PATH - 1);
        longest.prepare_runtime().unwrap();
        std::os::unix::net::UnixListener::bind(longest.socket()).unwrap();
    }

    #[test]
    fn xdg_dirs_are_used_when_set() {
        let paths = resolve(&[
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_DATA_HOME", "/d"),
            ("XDG_CONFIG_HOME", "/c"),
        ]);
        assert_eq!(paths.runtime, PathBuf::from("/run/user/1000/hive"));
        assert_eq!(paths.data, PathBuf::from("/d/hive"));
        assert_eq!(paths.settings(), PathBuf::from("/c/hive/settings.json"));
        assert_eq!(
            paths.socket(),
            PathBuf::from("/run/user/1000/hive/hive.sock")
        );
        assert_eq!(paths.lock(), PathBuf::from("/run/user/1000/hive/hive.lock"));
        assert_eq!(
            paths.daemon_log(),
            PathBuf::from("/run/user/1000/hive/daemon.log")
        );
        assert_eq!(paths.bin_dir(), PathBuf::from("/d/hive/bin"));
        assert_eq!(
            paths.hooks_settings(),
            PathBuf::from("/d/hive/hive-hooks.json")
        );
        assert_eq!(paths.projects(), PathBuf::from("/d/hive/projects.json"));
        assert_eq!(paths.spaces(), PathBuf::from("/d/hive/spaces.json"));
        assert_eq!(paths.ports(), PathBuf::from("/d/hive/ports.json"));
        assert_eq!(paths.offers(), PathBuf::from("/d/hive/offered-pulls.json"));
        assert_eq!(
            paths.open_sessions(),
            PathBuf::from("/d/hive/open-sessions.json")
        );
    }

    #[test]
    fn fallbacks_use_uid_and_home() {
        let paths = resolve(&[("XDG_RUNTIME_DIR", ""), ("HOME", "/home/me")]);
        assert_eq!(paths.runtime, PathBuf::from("/tmp/hive-1000"));
        assert_eq!(paths.data, PathBuf::from("/home/me/.local/share/hive"));
        assert_eq!(paths.config, PathBuf::from("/home/me/.config/hive"));
    }

    #[test]
    fn without_home_data_goes_under_runtime() {
        let paths = resolve(&[]);
        assert_eq!(paths.data, PathBuf::from("/tmp/hive-1000/data"));
        assert_eq!(paths.config, PathBuf::from("/tmp/hive-1000/config"));
    }

    fn paths_in(dir: &std::path::Path) -> Paths {
        Paths {
            runtime: dir.join("run"),
            data: dir.join("data"),
            config: dir.join("config"),
        }
    }

    #[test]
    fn runtime_dir_is_created_private() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        paths.prepare_runtime().unwrap();
        let mode = std::fs::metadata(&paths.runtime)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn accessible_runtime_dir_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        std::fs::create_dir(&paths.runtime).unwrap();
        std::fs::set_permissions(&paths.runtime, std::fs::Permissions::from_mode(0o750)).unwrap();
        let err = paths.prepare_runtime().unwrap_err();
        assert!(
            err.to_string().contains("insecure runtime directory"),
            "{err}"
        );
    }

    #[test]
    fn runtime_dir_of_another_user_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        paths.prepare_runtime().unwrap();
        let me = nix::unistd::getuid().as_raw();
        paths.check_runtime_as(me).unwrap();
        let err = paths.check_runtime_as(me + 1).unwrap_err();
        assert!(
            err.to_string().contains("insecure runtime directory"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_peer_of_another_user_is_refused() {
        // The other end stays open: macOS has no peer credentials for a closed peer.
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let me = nix::unistd::getuid().as_raw();
        check_peer(&ours, me).unwrap();
        let err = check_peer(&ours, me + 1).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("refusing a hive socket run by another user (uid {me})")
        );
    }

    #[tokio::test]
    async fn connect_reaches_our_own_service() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        paths.prepare_runtime().unwrap();
        let listener = tokio::net::UnixListener::bind(paths.socket()).unwrap();
        paths.connect().await.unwrap();
        listener.accept().await.unwrap();
    }

    #[test]
    fn missing_runtime_dir_fails_the_check() {
        let tmp = tempfile::tempdir().unwrap();
        let err = paths_in(tmp.path()).check_runtime().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn symlinked_runtime_dir_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let target = tmp.path().join("elsewhere");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&target)
            .unwrap();
        std::os::unix::fs::symlink(&target, &paths.runtime).unwrap();
        assert!(paths.prepare_runtime().is_err());
    }
}
