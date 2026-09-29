//! What only native Windows needs, kept apart from the portable code (12.5). A skeleton for
//! now (12.5.1): the service does not run on Windows yet, so what needs it answers
//! [`unsupported`]. Tested on the Windows CI runner (`windows.yml`).

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tokio::net::windows::named_pipe::NamedPipeClient;

use crate::paths::Paths;
use crate::procs::Proc;

/// The error of what Hive cannot do on Windows yet.
pub fn unsupported(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not supported on Windows yet"),
    )
}

/// Data in `%LOCALAPPDATA%\hive`, settings in `%APPDATA%\hive` (under `%USERPROFILE%` when
/// unset, else the temporary folder). The lock and the service's log go with the data.
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
        runtime: data.clone(),
        data,
        config,
    }
}

impl Paths {
    /// [`paths`] from this process's environment.
    pub fn from_env() -> Self {
        paths(|key| std::env::var_os(key))
    }

    /// The service's pipe (12.5.2).
    pub async fn connect(&self) -> io::Result<NamedPipeClient> {
        Err(unsupported("the Hive service"))
    }
}

/// The machine-wide monotonic clock in ns; 0 (cannot be read) until 12.5.2 reads QPC.
pub fn monotonic_ns() -> u64 {
    0
}

/// No process table until 12.5.6a.
pub fn list() -> Vec<Proc> {
    vec![]
}

/// No process working folders until 12.5.6a.
pub fn cwd(_pid: i32) -> Option<PathBuf> {
    None
}

/// A folder rename that never replaces (12.5.6b).
pub fn rename_new(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(unsupported("moving a folder"))
}

/// The app runs beside the service: it opens `path` itself (12.5.6b).
pub fn native_path(_path: &Path, _wslpath: &std::ffi::OsStr) -> io::Result<String> {
    Err(unsupported("opening a file with its app"))
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

/// What a program printed (a path, a `PATH`): programs print UTF-8 on Windows.
pub fn os_string(bytes: &[u8]) -> OsString {
    String::from_utf8_lossy(bytes).into_owned().into()
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
    use std::time::{Duration, Instant};

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
        assert_eq!(paths.runtime, paths.data);
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

    #[tokio::test]
    async fn the_service_is_unsupported() {
        let err = Paths::from_env().connect().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn stubs_answer_nothing_or_unsupported() {
        assert_eq!(monotonic_ns(), 0);
        assert_eq!(list(), vec![]);
        assert_eq!(cwd(4), None);
        let here = Path::new(".");
        let unsupported = |result: io::Result<_>| result.unwrap_err().kind();
        assert_eq!(
            unsupported(rename_new(here, here)),
            io::ErrorKind::Unsupported
        );
        let open = native_path(here, "".as_ref()).map(drop);
        assert_eq!(unsupported(open), io::ErrorKind::Unsupported);
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
