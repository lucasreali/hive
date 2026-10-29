//! A downloaded update's installer, kept in the app's cache folder until it is installed
//! instead of in memory for the rest of the run (19.11).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The installer's file name in the cache folder.
pub const FILE: &str = "update";

/// The installer saved by [`save`]; dropping it removes the file.
#[derive(Debug)]
pub struct Saved {
    path: PathBuf,
    /// The length and SHA-256 of the bytes whose signature the download checked. The installer
    /// does not check it again, so the file must still match them when read back: another
    /// process of the user swapping it before the install is refused.
    len: usize,
    hash: [u8; 32],
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Removes an installer left by an earlier run (ended before installing it): never installed,
/// as nothing of this run vouches for it.
pub fn clear(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

/// Writes `bytes` to [`FILE`] in `dir` (created if missing), readable by the user only.
pub fn save(dir: &Path, bytes: &[u8]) -> Result<Saved, String> {
    let path = dir.join(FILE);
    let mut options = std::fs::File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    // A file already there (mode unknown) goes first: `create_new` then sets ours.
    clear(dir);
    let saved = Saved {
        path,
        len: bytes.len(),
        hash: hash(bytes),
    };
    std::fs::create_dir_all(dir)
        .and_then(|()| options.open(&saved.path))
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|error| format!("cannot save the update: {error}"))?;
    Ok(saved)
}

impl Saved {
    /// The installer as it was saved, read once: these very bytes are the ones checked and
    /// installed. A missing or changed file is an error.
    pub fn read(&self) -> Result<Vec<u8>, String> {
        let bytes = std::fs::read(&self.path)
            .map_err(|error| format!("cannot read the update: {error}"))?;
        if bytes.len() != self.len || hash(&bytes) != self.hash {
            return Err("the downloaded update changed; it is downloaded again next start".into());
        }
        Ok(bytes)
    }
}

impl Drop for Saved {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installer_is_saved_privately_read_back_and_removed() {
        let tmp = crate::tests::scratch("update-saved");
        let dir = tmp.join("cache");
        let path = dir.join(FILE);
        let saved = save(&dir, b"setup").unwrap();
        assert_eq!(saved.read(), Ok(b"setup".to_vec()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(saved);
        assert!(!path.exists());
    }

    #[test]
    fn a_file_already_there_is_replaced_with_a_private_one() {
        let tmp = crate::tests::scratch("update-replaced");
        let path = tmp.join(FILE);
        std::fs::write(&path, "older").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let open = std::fs::Permissions::from_mode(0o644);
            std::fs::set_permissions(&path, open).unwrap();
        }
        let saved = save(&tmp, b"setup").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"setup");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(saved);
    }

    #[test]
    fn a_missing_or_changed_installer_is_refused() {
        let tmp = crate::tests::scratch("update-changed");
        let path = tmp.join(FILE);
        let saved = save(&tmp, b"setup").unwrap();
        let changed =
            Err("the downloaded update changed; it is downloaded again next start".into());
        // Cut short, the same length changed, or longer.
        for other in [&b"set"[..], b"setUP", b"setup!"] {
            std::fs::write(&path, other).unwrap();
            assert_eq!(saved.read(), changed, "{other:?}");
        }
        std::fs::remove_file(&path).unwrap();
        let missing = saved.read().unwrap_err();
        assert!(missing.starts_with("cannot read the update: "), "{missing}");
    }

    #[test]
    fn an_earlier_runs_installer_is_cleared_and_a_folder_that_cannot_hold_it_fails() {
        let tmp = crate::tests::scratch("update-cleared");
        let path = tmp.join(FILE);
        std::fs::write(&path, "stale").unwrap();
        clear(&tmp);
        assert!(!path.exists());
        // The cache folder is a file.
        let blocked = tmp.join("file");
        std::fs::write(&blocked, "").unwrap();
        let failed = save(&blocked, b"setup").unwrap_err();
        assert!(failed.starts_with("cannot save the update: "), "{failed}");
    }
}
