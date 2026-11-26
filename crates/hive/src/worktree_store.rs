//! What the service keeps per worktree in Hive's folder (22.1), never inside `.git` (#56): one
//! JSON file `{<worktree path>: <its data>}`, written atomically with mode `0600` and refused
//! past its size cap, so it never grows without bound. A worktree's data goes when it is removed
//! ([`WorktreeStore::forget`]), and, at the next change, once its folder is gone (removed
//! outside Hive). The seen marks of the diff (27.1, [`crate::seen`]) keep theirs here.

use std::collections::BTreeMap;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::git::read_limited;
use crate::wrapper::write_atomic;

/// Each worktree's `T` in one file of at most `limit` bytes.
pub struct WorktreeStore<T> {
    file: PathBuf,
    limit: u64,
    lock: Mutex<()>,
    data: PhantomData<fn() -> T>,
}

impl<T: Serialize + DeserializeOwned + Default + PartialEq> WorktreeStore<T> {
    pub fn new(file: PathBuf, limit: u64) -> Self {
        Self {
            file,
            limit,
            lock: Mutex::new(()),
            data: PhantomData,
        }
    }

    /// `worktree`'s data; the default when it has none.
    pub fn get(&self, worktree: &str) -> T {
        let _lock = self.lock();
        self.read().remove(worktree).unwrap_or_default()
    }

    /// Changes `worktree`'s data with `change` and saves it, with the worktrees whose folder is
    /// gone left out, and `worktree` too when it is left with the default. Nothing is saved, and
    /// the error says why, when the file would be over its cap.
    pub fn update(&self, worktree: &str, change: impl FnOnce(&mut T)) -> io::Result<()> {
        let _lock = self.lock();
        let mut all = self.read();
        all.retain(|path, _| Path::new(path).is_dir());
        let mut data = all.remove(worktree).unwrap_or_default();
        change(&mut data);
        if data != T::default() {
            all.insert(worktree.to_owned(), data);
        }
        self.write(&all)
    }

    /// Drops the data of `worktrees` (removed ones); nothing is written when none has any.
    pub fn forget(&self, worktrees: &[String]) -> io::Result<()> {
        let _lock = self.lock();
        let mut all = self.read();
        let before = all.len();
        all.retain(|path, _| !worktrees.contains(path));
        if all.len() == before {
            return Ok(());
        }
        self.write(&all)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The file's data; a missing, unreadable, invalid or oversized file counts as none.
    fn read(&self) -> BTreeMap<String, T> {
        let read = std::fs::File::open(&self.file)
            .and_then(|mut file| read_limited(&mut file, self.limit))
            .ok();
        read.and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write(&self, all: &BTreeMap<String, T>) -> io::Result<()> {
        let json = serde_json::to_vec(all)?;
        if json.len() as u64 > self.limit {
            let name = self.file.file_name().unwrap_or_default().to_string_lossy();
            return Err(io::Error::other(format!(
                "{name} would be over {} bytes",
                self.limit
            )));
        }
        self.file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
        write_atomic(&self.file, &json, 0o600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Store = WorktreeStore<BTreeMap<String, String>>;

    fn folder(tmp: &tempfile::TempDir, name: &str) -> String {
        let dir = tmp.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        dir.display().to_string()
    }

    fn data(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn each_worktree_keeps_its_data_across_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (folder(&tmp, "a"), folder(&tmp, "b"));
        let file = tmp.path().join("hive/store.json");
        let store = Store::new(file.clone(), 4096);
        assert_eq!(store.get(&a), data(&[]));
        store.update(&a, |d| d.extend(data(&[("x", "1")]))).unwrap();
        store.update(&b, |d| d.extend(data(&[("y", "2")]))).unwrap();
        store.update(&a, |d| d.extend(data(&[("z", "3")]))).unwrap();
        let again = Store::new(file.clone(), 4096);
        assert_eq!(again.get(&a), data(&[("x", "1"), ("z", "3")]));
        assert_eq!(again.get(&b), data(&[("y", "2")]));
        // Private, and out of `.git`: Hive's own folder.
        #[cfg(unix)]
        assert_eq!(
            crate::mode::of(&std::fs::metadata(&file).unwrap()) & 0o777,
            0o600
        );
    }

    #[test]
    fn a_worktree_left_with_nothing_or_gone_leaves_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (folder(&tmp, "a"), folder(&tmp, "b"));
        let file = tmp.path().join("store.json");
        let store = Store::new(file.clone(), 4096);
        store.update(&a, |d| d.extend(data(&[("x", "1")]))).unwrap();
        store.update(&b, |d| d.extend(data(&[("y", "2")]))).unwrap();
        store.update(&a, BTreeMap::clear).unwrap();
        let only = |path: &str, pair| {
            serde_json::to_string(
                &[(path, data(&[pair]))]
                    .into_iter()
                    .collect::<BTreeMap<_, _>>(),
            )
            .unwrap()
        };
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            only(&b, ("y", "2"))
        );
        // Removed outside Hive: gone at the next change.
        std::fs::remove_dir(&b).unwrap();
        store.update(&a, |d| d.extend(data(&[("x", "1")]))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            only(&a, ("x", "1"))
        );
    }

    #[test]
    fn a_removed_worktree_forgets_its_data() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("hive/store.json");
        let store = Store::new(file.clone(), 4096);
        store.forget(&["/x".into()]).unwrap();
        assert!(!file.exists());
        let [a, b] = ["a", "b"].map(|w| {
            let dir = folder(&tmp, w);
            store
                .update(&dir, |d| d.extend(data(&[("k", "v")])))
                .unwrap();
            dir
        });
        let saved = std::fs::read(&file).unwrap();
        // Nothing of its own: not written again.
        std::fs::write(&file, &saved[..saved.len() - 1]).unwrap();
        store.forget(&["/x".into()]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), &saved[..saved.len() - 1]);
        std::fs::write(&file, &saved).unwrap();
        store.forget(&[a.clone(), "/x".into()]).unwrap();
        assert_eq!(store.get(&a), data(&[]));
        assert_eq!(store.get(&b), data(&[("k", "v")]));
    }

    #[test]
    fn the_file_is_capped() {
        let tmp = tempfile::tempdir().unwrap();
        let a = folder(&tmp, "a");
        let file = tmp.path().join("store.json");
        Store::new(file.clone(), 4096)
            .update(&a, |d| d.extend(data(&[("x", "1")])))
            .unwrap();
        let saved = std::fs::read_to_string(&file).unwrap();
        // Room for `,"y":"vvvv"` exactly.
        let limit = saved.len() + 11;
        let store = Store::new(file.clone(), limit as u64);
        let err = store
            .update(&a, |d| d.extend(data(&[("y", "vvvvv")])))
            .unwrap_err();
        let message = format!("store.json would be over {limit} bytes");
        assert_eq!(err.to_string(), message);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
        // Exactly at the cap is kept; a file over the cap read back counts as none.
        store
            .update(&a, |d| d.extend(data(&[("y", "vvvv")])))
            .unwrap();
        assert_eq!(std::fs::read(&file).unwrap().len(), limit);
        assert_eq!(store.get(&a).len(), 2);
        let over = Store::new(file, limit as u64 - 1);
        assert_eq!(over.get(&a), data(&[]));
    }

    #[test]
    fn a_bad_file_counts_as_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let a = folder(&tmp, "a");
        let file = tmp.path().join("store.json");
        for bad in ["{", "[]", r#"{"/x":1}"#] {
            std::fs::write(&file, bad).unwrap();
            let store = Store::new(file.clone(), 4096);
            assert_eq!(store.get(&a), data(&[]));
            store.update(&a, |d| d.extend(data(&[("x", "1")]))).unwrap();
            assert_eq!(store.get(&a), data(&[("x", "1")]));
        }
    }
}
