//! Copy-on-write copies of the files `.worktreeinclude` lists (28.9). A clone shares the
//! source's blocks until either side changes: macOS `fclonefileat` (APFS), Linux's `FICLONE`
//! ioctl (Btrfs, XFS). Where the filesystem has no clones, a plain copy of the bytes.
//!
//! The caller opens the source (`hive::worktree` through `file::open_plain`, which never
//! follows a link in any part of its path); the destination is always a new file, never an
//! existing path.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// How a file reached its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    /// A copy-on-write clone: no data was written.
    Cloned,
    /// A plain copy of the bytes.
    Copied,
}

/// The two ways of copying one file, behind a trait so each path and its errors are tested.
pub trait Copier {
    /// Clones the open regular file `src` into the new file `dst`, with `src`'s permissions.
    /// Fails, leaving no `dst` behind, where the filesystem has no clones.
    fn clone_file(&self, src: &File, dst: &Path) -> io::Result<()>;
    /// Copies `src`'s bytes, from its start, into the new file `dst`, with `src`'s
    /// permissions.
    fn copy_file(&self, src: &mut File, dst: &Path) -> io::Result<()>;
}

/// The system's clone and copy.
pub struct System;

impl Copier for System {
    fn clone_file(&self, src: &File, dst: &Path) -> io::Result<()> {
        clone_file(src, dst)
    }

    fn copy_file(&self, src: &mut File, dst: &Path) -> io::Result<()> {
        let mut out = create_new(dst)?;
        io::copy(src, &mut out)?;
        mirror_mode(src, &out)
    }
}

/// Copies the open regular file `src` to the new path `dst`: a clone where the filesystem
/// allows, else a plain copy. Refuses an existing `dst` (never overwritten, never written
/// through).
pub fn copy_new(copier: &dyn Copier, mut src: File, dst: &Path) -> io::Result<Copied> {
    match copier.clone_file(&src, dst) {
        Ok(()) => Ok(Copied::Cloned),
        // Something appeared at `dst`: a copy would refuse it too.
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Err(err),
        Err(_) => copier.copy_file(&mut src, dst).map(|()| Copied::Copied),
    }
}

/// A new file at `path`; an existing one (a symlink included) is refused.
fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Gives `out` the permissions of `src`, as `std::fs::copy` does.
fn mirror_mode(src: &File, out: &File) -> io::Result<()> {
    out.set_permissions(src.metadata()?.permissions())
}

/// `fclonefileat` makes `dst` itself, atomically, so a failure leaves nothing behind. Mode,
/// ACLs and extended attributes come with the clone; the owner is the caller.
#[cfg(target_os = "macos")]
fn clone_file(src: &File, dst: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    // `<sys/clonefile.h>`; the libc crate does not define it.
    const CLONE_NOOWNERCOPY: u32 = 0x0002;
    let dst = CString::new(dst.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // SAFETY: `src` is an open descriptor and `dst` a NUL-terminated string, both alive until
    // the call returns.
    let done = unsafe {
        libc::fclonefileat(
            src.as_raw_fd(),
            libc::AT_FDCWD,
            dst.as_ptr(),
            CLONE_NOOWNERCOPY,
        )
    };
    nix::errno::Errno::result(done)
        .map(drop)
        .map_err(io::Error::from)
}

/// `FICLONE` clones into a file already open, so a new `dst` is made first and removed again
/// when the clone fails.
#[cfg(target_os = "linux")]
fn clone_file(src: &File, dst: &Path) -> io::Result<()> {
    let out = create_new(dst)?;
    let done = mirror_mode(src, &out).and_then(|()| ficlone(src, &out));
    if done.is_err() {
        let _ = std::fs::remove_file(dst);
    }
    done
}

#[cfg(target_os = "linux")]
fn ficlone(src: &File, out: &File) -> io::Result<()> {
    use nix::libc;
    use std::os::fd::AsRawFd;
    // SAFETY: both descriptors are open until the call returns; `FICLONE` takes the source
    // descriptor as its argument.
    let done = unsafe { libc::ioctl(out.as_raw_fd(), libc::FICLONE, src.as_raw_fd()) };
    nix::errno::Errno::result(done)
        .map(drop)
        .map_err(io::Error::from)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clone_file(_src: &File, _dst: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use std::io::Read;

    /// A copier whose clone answers `clone` and whose copy answers `copy`, recording calls.
    struct Fake {
        clone: Option<io::ErrorKind>,
        copy: Option<io::ErrorKind>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Fake {
        fn new(clone: Option<io::ErrorKind>, copy: Option<io::ErrorKind>) -> Self {
            let calls = RefCell::new(Vec::new());
            Self { clone, copy, calls }
        }
    }

    impl Copier for Fake {
        fn clone_file(&self, _src: &File, _dst: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("clone");
            self.clone.map_or(Ok(()), |kind| Err(kind.into()))
        }

        fn copy_file(&self, src: &mut File, _dst: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("copy");
            // The copy reads the source from its start.
            let mut text = String::new();
            src.read_to_string(&mut text)?;
            assert_eq!(text, "data");
            self.copy.map_or(Ok(()), |kind| Err(kind.into()))
        }
    }

    fn source(dir: &Path) -> File {
        let src = dir.join("src");
        fs::write(&src, "data").unwrap();
        File::open(src).unwrap()
    }

    #[test]
    fn a_clone_is_tried_first_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new(None, None);
        let got = copy_new(&fake, source(dir.path()), &dir.path().join("dst")).unwrap();
        assert_eq!(got, Copied::Cloned);
        assert_eq!(*fake.calls.borrow(), ["clone"]);
    }

    #[test]
    fn a_refused_clone_falls_back_to_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("dst");
        for kind in [
            io::ErrorKind::Unsupported,
            io::ErrorKind::CrossesDevices,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::Other,
        ] {
            let fake = Fake::new(Some(kind), None);
            assert_eq!(
                copy_new(&fake, source(dir.path()), &dst).unwrap(),
                Copied::Copied
            );
            assert_eq!(*fake.calls.borrow(), ["clone", "copy"]);
        }
    }

    #[test]
    fn the_copys_error_is_the_answer() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new(
            Some(io::ErrorKind::Unsupported),
            Some(io::ErrorKind::StorageFull),
        );
        let err = copy_new(&fake, source(dir.path()), &dir.path().join("dst")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
    }

    #[test]
    fn an_existing_destination_is_never_copied_over() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new(Some(io::ErrorKind::AlreadyExists), None);
        let err = copy_new(&fake, source(dir.path()), &dir.path().join("dst")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(*fake.calls.borrow(), ["clone"]);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[cfg(unix)]
    #[test]
    fn the_system_copy_keeps_bytes_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(0o751)).unwrap();
        let dst = dir.path().join("dst");
        System
            .copy_file(&mut File::open(&src).unwrap(), &dst)
            .unwrap();
        assert_eq!(fs::read(&dst).unwrap(), data);
        assert_eq!(mode(&dst), 0o751);
        let again = System.copy_file(&mut File::open(&src).unwrap(), &dst);
        assert_eq!(again.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    }

    /// The system's clone on the temp folder's filesystem: APFS on macOS clones; elsewhere
    /// (ext4 on CI's Linux) it fails and leaves nothing. Either way `copy_new` gives the same
    /// file.
    #[cfg(unix)]
    #[test]
    fn the_system_clone_succeeds_whole_or_leaves_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 241) as u8).collect();
        fs::write(&src, &data).unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(0o640)).unwrap();
        let dst = dir.path().join("dst");
        let cloned = System.clone_file(&File::open(&src).unwrap(), &dst).is_ok();
        #[cfg(target_os = "macos")]
        assert!(cloned);
        // A failed clone leaves no file; a clone has the bytes and the mode.
        assert_eq!(fs::read(&dst).ok(), cloned.then(|| data.clone()));
        assert_eq!(dst.exists().then(|| mode(&dst)), cloned.then_some(0o640));
        let other = dir.path().join("other");
        let how = copy_new(&System, File::open(&src).unwrap(), &other).unwrap();
        assert_eq!(how == Copied::Cloned, cloned);
        assert_eq!(fs::read(&other).unwrap(), data);
        assert_eq!(mode(&other), 0o640);
    }

    /// A clone refuses an existing destination and leaves it as it was.
    #[cfg(unix)]
    #[test]
    fn the_system_clone_never_touches_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path());
        let dst = dir.path().join("dst");
        fs::write(&dst, "mine").unwrap();
        let err = System.clone_file(&src, &dst).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "mine");
    }
}
