//! Where the service keeps its socket, lockfile and installed files.

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$XDG_RUNTIME_DIR/hive`, or `/tmp/hive-<uid>` when it is unset.
    pub runtime: PathBuf,
    /// `$XDG_DATA_HOME/hive`, or `~/.local/share/hive`.
    pub data: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        Self::resolve(|key| std::env::var_os(key), nix::unistd::getuid().as_raw())
    }

    fn resolve(var: impl Fn(&str) -> Option<OsString>, uid: u32) -> Self {
        let var = |key| {
            var(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let runtime = var("XDG_RUNTIME_DIR").map_or_else(
            || PathBuf::from(format!("/tmp/hive-{uid}")),
            |dir| dir.join("hive"),
        );
        let data = var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".local/share")))
            .map_or_else(|| runtime.join("data"), |dir| dir.join("hive"));
        Self { runtime, data }
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime.join("hive.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.runtime.join("hive.lock")
    }

    /// Directory put first on `PATH` inside Hive terminals (the `claude` wrapper lives here).
    pub fn bin_dir(&self) -> PathBuf {
        self.data.join("bin")
    }

    /// Creates the runtime directory with mode `0700` and refuses one that
    /// another user owns or that others can access (e.g. a planted `/tmp/hive-<uid>`).
    pub fn prepare_runtime(&self) -> io::Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.runtime)?;
        // lstat: a symlink reports mode 0777 and fails the mode check.
        let meta = std::fs::symlink_metadata(&self.runtime)?;
        let uid = nix::unistd::getuid().as_raw();
        if meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other(format!(
                "refusing insecure runtime directory {} (must be a directory owned by you with mode 0700)",
                self.runtime.display()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn resolve(vars: &[(&str, &str)]) -> Paths {
        Paths::resolve(
            |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into()),
            1000,
        )
    }

    #[test]
    fn xdg_dirs_are_used_when_set() {
        let paths = resolve(&[
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_DATA_HOME", "/d"),
        ]);
        assert_eq!(paths.runtime, PathBuf::from("/run/user/1000/hive"));
        assert_eq!(paths.data, PathBuf::from("/d/hive"));
        assert_eq!(
            paths.socket(),
            PathBuf::from("/run/user/1000/hive/hive.sock")
        );
        assert_eq!(paths.lock(), PathBuf::from("/run/user/1000/hive/hive.lock"));
        assert_eq!(paths.bin_dir(), PathBuf::from("/d/hive/bin"));
    }

    #[test]
    fn fallbacks_use_uid_and_home() {
        let paths = resolve(&[("XDG_RUNTIME_DIR", ""), ("HOME", "/home/me")]);
        assert_eq!(paths.runtime, PathBuf::from("/tmp/hive-1000"));
        assert_eq!(paths.data, PathBuf::from("/home/me/.local/share/hive"));
    }

    #[test]
    fn without_home_data_goes_under_runtime() {
        assert_eq!(resolve(&[]).data, PathBuf::from("/tmp/hive-1000/data"));
    }

    fn paths_in(dir: &std::path::Path) -> Paths {
        Paths {
            runtime: dir.join("run"),
            data: dir.join("data"),
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
