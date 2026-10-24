//! The service's hold on its runtime folder: the lock that keeps a second service from
//! starting and the socket that clients find it by, both kept while it runs (17.9).
//!
//! macOS removes old files from temporary folders, and a user may delete the folder: with the
//! lock gone a second service would start, with the socket gone the app's next bridge would
//! start one. Every [`super::CLAIM_CHECK`] the service puts back what is missing; if another
//! service took the lock meanwhile, this one stops instead.

use std::fs::{File, TryLockError};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use tokio::net::UnixListener;

use crate::paths::Paths;

/// A file's identity, (device, inode): a file put back at the same path is another file.
type Id = (u64, u64);

/// Why the runtime folder could not be kept in place.
enum Problem {
    /// The service must stop: another one holds the lock, or the folder is not safe.
    Lost(io::Error),
    /// Tried again at the next check.
    Passing(io::Error),
}

pub struct Claim {
    paths: Paths,
    /// Locked for as long as it is held.
    lock: File,
    lock_id: Id,
    socket_id: Id,
}

impl Claim {
    /// `lock` locked by this service: binds its socket, replacing one left by a crashed service.
    pub fn take(paths: &Paths, lock: File) -> io::Result<(Self, UnixListener)> {
        let lock_id = id(&lock.metadata()?);
        let (listener, socket_id) = bind(&paths.socket())?;
        let claim = Self {
            paths: paths.clone(),
            lock,
            lock_id,
            socket_id,
        };
        Ok((claim, listener))
    }

    /// Puts back the runtime folder, lock and socket if any is gone or replaced: a new
    /// listener when the socket was. Fails only when the service must stop: another service
    /// holds the new lock, or the folder is no longer safe. Anything else that goes wrong
    /// (a folder that cannot be made, a socket that cannot be bound) is logged and tried
    /// again at the next check, so a passing failure does not end every terminal.
    pub fn keep(&mut self) -> io::Result<Option<UnixListener>> {
        match self.put_back() {
            Ok(listener) => Ok(listener),
            Err(Problem::Lost(err)) => Err(err),
            Err(Problem::Passing(err)) => {
                eprintln!("hive: cannot keep the runtime folder in place ({err}); trying again");
                Ok(None)
            }
        }
    }

    fn put_back(&mut self) -> Result<Option<UnixListener>, Problem> {
        self.paths.make_runtime().map_err(Problem::Passing)?;
        self.paths.check_runtime().map_err(Problem::Lost)?;
        if path_id(&self.paths.lock()) != Some(self.lock_id) {
            let lock = super::open_lock(&self.paths).map_err(Problem::Passing)?;
            let taken = lock.try_lock();
            if let Err(TryLockError::WouldBlock) = taken {
                let err = super::lock_error(&self.paths, TryLockError::WouldBlock);
                return Err(Problem::Lost(err));
            }
            taken.map_err(io::Error::from).map_err(Problem::Passing)?;
            self.lock_id = id(&lock.metadata().map_err(Problem::Passing)?);
            self.lock = lock;
        }
        if path_id(&self.paths.socket()) == Some(self.socket_id) {
            return Ok(None);
        }
        let (listener, socket_id) = bind(&self.paths.socket()).map_err(Problem::Passing)?;
        self.socket_id = socket_id;
        Ok(Some(listener))
    }

    /// Removes the socket, unless it is no longer this service's. The lock stays until the
    /// claim is dropped.
    pub fn release(&self) {
        let socket = self.paths.socket();
        if path_id(&socket) == Some(self.socket_id) {
            let _ = std::fs::remove_file(socket);
        }
    }
}

fn bind(socket: &Path) -> io::Result<(UnixListener, Id)> {
    // A socket left by a crashed daemon; the lock proves nobody is serving it.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, id(&std::fs::symlink_metadata(socket)?)))
}

fn id(meta: &std::fs::Metadata) -> Id {
    (meta.dev(), meta.ino())
}

fn path_id(path: &Path) -> Option<Id> {
    std::fs::symlink_metadata(path).ok().map(|meta| id(&meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(dir: &Path) -> Paths {
        Paths {
            logs: dir.join("run"),
            runtime: dir.join("run"),
            data: dir.join("data"),
            config: dir.join("config"),
        }
    }

    async fn claim(paths: &Paths) -> (Claim, UnixListener) {
        paths.prepare_runtime().unwrap();
        let lock = super::super::lock(paths).await.unwrap();
        Claim::take(paths, lock).unwrap()
    }

    /// The lock is held: another open of the file cannot take it.
    fn locked(paths: &Paths) -> bool {
        let other = File::options().write(true).open(paths.lock()).unwrap();
        matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock))
    }

    async fn reaches(listener: &UnixListener, paths: &Paths) {
        let _client = tokio::net::UnixStream::connect(paths.socket())
            .await
            .unwrap();
        listener.accept().await.unwrap();
    }

    #[tokio::test]
    async fn nothing_changes_while_the_lock_and_socket_are_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, listener) = claim(&paths).await;
        let socket = std::fs::symlink_metadata(paths.socket()).unwrap();
        assert_eq!(socket.permissions().mode() & 0o777, 0o600);
        assert!(claim.keep().unwrap().is_none());
        assert_eq!(path_id(&paths.socket()), Some(id(&socket)));
        assert!(locked(&paths));
        reaches(&listener, &paths).await;
    }

    #[tokio::test]
    async fn a_removed_socket_is_bound_again() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, _old) = claim(&paths).await;
        std::fs::remove_file(paths.socket()).unwrap();
        let listener = claim.keep().unwrap().unwrap();
        let mode = std::fs::metadata(paths.socket())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        reaches(&listener, &paths).await;
        assert!(claim.keep().unwrap().is_none());
        // Another file in its place is replaced too.
        std::fs::remove_file(paths.socket()).unwrap();
        std::fs::write(paths.socket(), "planted").unwrap();
        let listener = claim.keep().unwrap().unwrap();
        reaches(&listener, &paths).await;
    }

    #[tokio::test]
    async fn a_removed_lock_is_taken_again() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, _listener) = claim(&paths).await;
        std::fs::remove_file(paths.lock()).unwrap();
        assert!(claim.keep().unwrap().is_none());
        assert!(locked(&paths));
        let mode = std::fs::metadata(paths.lock())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(claim.keep().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_removed_runtime_folder_is_made_again() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, _old) = claim(&paths).await;
        std::fs::remove_dir_all(&paths.runtime).unwrap();
        let listener = claim.keep().unwrap().unwrap();
        assert!(locked(&paths));
        reaches(&listener, &paths).await;
    }

    #[tokio::test]
    async fn a_passing_failure_is_tried_again_at_the_next_check() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, _old) = claim(&paths).await;
        // The folder cannot be made: a file in its place.
        std::fs::remove_dir_all(&paths.runtime).unwrap();
        std::fs::write(&paths.runtime, "").unwrap();
        assert!(claim.keep().unwrap().is_none());
        std::fs::remove_file(&paths.runtime).unwrap();
        paths.make_runtime().unwrap();
        // The lock cannot be opened: a folder in its place.
        std::fs::create_dir(paths.lock()).unwrap();
        assert!(claim.keep().unwrap().is_none());
        assert!(!paths.socket().exists());
        std::fs::remove_dir(paths.lock()).unwrap();
        // The socket cannot be bound: a folder in its place.
        std::fs::create_dir(paths.socket()).unwrap();
        assert!(claim.keep().unwrap().is_none());
        assert!(locked(&paths));
        assert!(paths.socket().is_dir());
        std::fs::remove_dir(paths.socket()).unwrap();
        // Each one is put back once it can be.
        let listener = claim.keep().unwrap().unwrap();
        reaches(&listener, &paths).await;
        assert!(claim.keep().unwrap().is_none());
    }

    #[tokio::test]
    async fn the_service_stops_when_another_one_took_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, _listener) = claim(&paths).await;
        std::fs::remove_file(paths.lock()).unwrap();
        let (other, _theirs) = self::claim(&paths).await;
        let err = claim.keep().err().unwrap();
        assert!(
            err.to_string()
                .ends_with("; is another hive daemon running?"),
            "{err}"
        );
        // Its socket is the other service's now: left in place.
        claim.release();
        assert!(paths.socket().exists());
        other.release();
        assert!(!paths.socket().exists());
    }

    #[tokio::test]
    async fn an_insecure_runtime_folder_stops_the_service() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let (mut claim, _listener) = claim(&paths).await;
        std::fs::set_permissions(&paths.runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = claim.keep().err().unwrap();
        assert!(
            err.to_string().contains("insecure runtime directory"),
            "{err}"
        );
    }
}
