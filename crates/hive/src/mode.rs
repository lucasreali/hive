//! Unix permission bits, where the system has them. Windows has none: a file there gets its
//! folder's ACL (under the user's profile, the user's alone), so [`crate::windows::mode`]
//! leaves it as it is.

#[cfg(unix)]
use std::fs::{DirBuilder, File};
use std::fs::{Metadata, OpenOptions};
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

#[cfg(windows)]
pub use crate::windows::mode::{create, folder, of, set};

/// `options`, creating the file with `mode` (less the umask).
#[cfg(unix)]
pub fn create(options: &mut OpenOptions, mode: u32) -> &mut OpenOptions {
    options.mode(mode)
}

/// `builder`, creating the folder with `mode` (less the umask).
#[cfg(unix)]
pub fn folder(builder: &mut DirBuilder, mode: u32) -> &mut DirBuilder {
    builder.mode(mode)
}

/// `options`, creating a file only its owner can read and write (`0600`).
pub fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    create(options, 0o600)
}

/// Sets the permission bits of `file` to `mode`.
#[cfg(unix)]
pub fn set(file: &File, mode: u32) -> io::Result<()> {
    file.set_permissions(PermissionsExt::from_mode(mode))
}

/// The permission bits of the file `meta` describes.
#[cfg(unix)]
pub fn of(meta: &Metadata) -> u32 {
    meta.permissions().mode()
}

/// Whether anyone may run the file `meta` describes (never on Windows, where the extension
/// tells).
pub fn executable(meta: &Metadata) -> bool {
    of(meta) & 0o111 != 0
}
