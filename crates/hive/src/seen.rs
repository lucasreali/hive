//! Files marked "seen" in a worktree's diff (27.1). A mark is kept per worktree and diff base
//! (`head` or `branch`, the base the list was made against) with the file's diff identity when it
//! was marked: git's blob id of its base side (from `git diff --raw`) and of its side on disk
//! (`git hash-object --no-filters`). A file whose identity changed since is "changed since seen",
//! unseen again. Marks live in `<data>/seen.json` ([`WorktreeStore`]), never inside `.git`
//! (#56), and go with the worktree.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use hive_protocol::{ChangedFile, SeenMark};

use crate::changes::{self, Against};
use crate::git;
use crate::worktree_store::WorktreeStore;

/// Largest marks file: about ten thousand marks.
pub const FILE_LIMIT: u64 = 1_048_576; // 1 MiB
/// A larger file is not hashed: its size and modification time stand for its content.
const HASH_LIMIT: u64 = 33_554_432; // 32 MiB
/// The identity of a side that does not exist (an added file's base, a deleted file on disk).
const NONE: &str = "-";

/// A worktree's marks: by base, each marked file's diff identity.
type Marks = BTreeMap<String, BTreeMap<String, String>>;

/// The seen marks of every worktree.
pub struct Seen {
    store: WorktreeStore<Marks>,
}

impl Seen {
    pub fn new(file: PathBuf) -> Self {
        Self::with_limit(file, FILE_LIMIT)
    }

    pub fn with_limit(file: PathBuf, limit: u64) -> Self {
        Self {
            store: WorktreeStore::new(file, limit),
        }
    }

    /// Drops the marks of `worktrees` (removed ones).
    pub fn forget(&self, worktrees: &[String]) -> io::Result<()> {
        self.store.forget(worktrees)
    }

    /// Sets the mark of each of `files` (the worktree's changes `against` its base) that has one:
    /// seen while its diff is the one it was marked on, else changed (also when git cannot
    /// tell: a seen file is never shown seen unchecked).
    pub fn apply(&self, worktree: &str, against: &Against, files: &mut [ChangedFile]) {
        let Some(marks) = self.store.get(worktree).remove(key(against)) else {
            return;
        };
        let marked: Vec<&ChangedFile> = files
            .iter()
            .filter(|f| marks.contains_key(&f.path))
            .collect();
        if marked.is_empty() {
            return;
        }
        let now = identities(against, &marked).unwrap_or_default();
        for file in files.iter_mut() {
            if let Some(mark) = marks.get(&file.path) {
                let same = now.get(&file.path) == Some(mark);
                file.seen = Some(if same {
                    SeenMark::Seen
                } else {
                    SeenMark::Changed
                });
            }
        }
    }

    /// Marks the `paths` among `files` (the worktree's changes `against` its base) as seen as
    /// they are now, or not (`seen` false); other paths are ignored. The marks of files no longer
    /// changed go too.
    pub fn mark(
        &self,
        worktree: &str,
        against: &Against,
        files: &[ChangedFile],
        paths: &[String],
        seen: bool,
    ) -> io::Result<()> {
        let paths: HashSet<&str> = paths.iter().map(String::as_str).collect();
        let chosen: Vec<&ChangedFile> = files
            .iter()
            .filter(|f| paths.contains(f.path.as_str()))
            .collect();
        if chosen.is_empty() {
            return Ok(());
        }
        let now = if seen {
            identities(against, &chosen)?
        } else {
            HashMap::new()
        };
        let listed: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
        self.store.update(worktree, |all| {
            let marks = all.entry(key(against).to_owned()).or_default();
            marks.retain(|path, _| listed.contains(path.as_str()));
            for file in chosen {
                match now.get(&file.path) {
                    Some(id) => marks.insert(file.path.clone(), id.clone()),
                    None => marks.remove(&file.path),
                };
            }
            if marks.is_empty() {
                all.remove(key(against));
            }
        })
    }
}

/// The base the marks of a listing `against` are kept under.
fn key(against: &Against) -> &'static str {
    match against.commit {
        Some(_) => "branch",
        None => "head",
    }
}

/// The diff identity of each of `files`: its base side's blob id and its side on disk.
fn identities(against: &Against, files: &[&ChangedFile]) -> io::Result<HashMap<String, String>> {
    let dir = &against.dir;
    let base = match &against.commit {
        Some(commit) => commit.clone(),
        None => changes::head(dir)?,
    };
    let args = [
        "diff",
        "--raw",
        "-z",
        "--no-abbrev",
        "--find-renames",
        &base,
        "--",
    ];
    let before = parse_ids(&git::output(dir, &args, &[0])?);
    let mut after = HashMap::new();
    let mut hashed = Vec::new();
    for file in files {
        match on_disk(dir, &file.path) {
            Some(id) => after.insert(file.path.as_str(), id),
            None => {
                hashed.push(file.path.as_str());
                None
            }
        };
    }
    if !hashed.is_empty() {
        let input: String = hashed.iter().map(|p| format!("{p}\n")).collect();
        let args = ["hash-object", "--no-filters", "--stdin-paths"];
        let out = git::output_with(dir, &args, input.as_bytes())?;
        // One id a line, in order; git fails as a whole on a file it cannot read.
        let out = String::from_utf8_lossy(&out);
        after.extend(hashed.into_iter().zip(out.lines().map(str::to_owned)));
    }
    let side = |id: Option<&String>| id.map_or(NONE, String::as_str).to_owned();
    Ok(files
        .iter()
        .map(|f| {
            let path = f.path.as_str();
            let id = format!("{} {}", side(before.get(path)), side(after.get(path)));
            (f.path.clone(), id)
        })
        .collect())
}

/// What stands for the file `path` of the worktree at `dir` on disk, or `None` when git is to
/// hash it: a regular file of at most [`HASH_LIMIT`] bytes whose name `--stdin-paths` can take.
/// A link is its target (never followed: a link to a FIFO would block git), a larger file (or
/// one whose name `--stdin-paths` would read otherwise: a line break, or a leading `"`, which
/// git C-unquotes, so `"..\057x"` would name another file) its size and time, anything else
/// none.
fn on_disk(dir: &Path, path: &str) -> Option<String> {
    let full = dir.join(path);
    let Ok(meta) = full.symlink_metadata() else {
        return Some(NONE.to_owned());
    };
    if meta.is_symlink() {
        let target = std::fs::read_link(&full).unwrap_or_default();
        return Some(format!("link:{}", target.display()));
    }
    if !meta.is_file() {
        return Some(NONE.to_owned());
    }
    let plain = !path.contains(['\n', '\r']) && !path.starts_with('"');
    let hashable = meta.len() <= HASH_LIMIT && plain;
    if hashable {
        return None;
    }
    let time = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok());
    let nanos = time.unwrap_or_default().as_nanos();
    Some(format!("stat:{}:{nanos}", meta.len()))
}

/// The base side's blob id by path (a rename's new path) in `git diff --raw -z --no-abbrev`.
fn parse_ids(out: &[u8]) -> HashMap<String, String> {
    let mut ids = HashMap::new();
    let mut records = out.split(|&b| b == 0);
    while let Some(meta) = records.next() {
        let fields: Vec<&[u8]> = meta.split(|&b| b == b' ').collect();
        let (Some(id), Some(code)) = (fields.get(2), fields.get(4)) else {
            continue;
        };
        let mut path = records.next().unwrap_or_default();
        if code.first() == Some(&b'R') {
            path = records.next().unwrap_or_default();
        }
        let path = String::from_utf8_lossy(path).into_owned();
        ids.insert(path, String::from_utf8_lossy(id).into_owned());
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::tests::{commit, run};

    struct Repo {
        _tmp: tempfile::TempDir,
        dir: PathBuf,
        file: PathBuf,
        seen: Seen,
    }

    impl Repo {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let top = crate::paths::canonical(tmp.path()).unwrap();
            let dir = top.join("r");
            std::fs::create_dir(&dir).unwrap();
            run(&dir, &["init", "-q", "-b", "main"]);
            commit(&dir, "a");
            commit(&dir, "b");
            let file = top.join("hive/seen.json");
            Self {
                seen: Seen::new(file.clone()),
                _tmp: tmp,
                dir,
                file,
            }
        }

        fn path(&self) -> String {
            self.dir.display().to_string()
        }

        fn head(&self) -> Against {
            Against {
                dir: self.dir.clone(),
                ..Against::default()
            }
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.dir.join(name), text).unwrap();
        }

        /// The changes against `against`, with their marks.
        fn listed(&self, against: &Against) -> Vec<ChangedFile> {
            let mut files = changes::list(&self.dir, against.commit.as_deref())
                .unwrap()
                .files;
            self.seen.apply(&self.path(), against, &mut files);
            files
        }

        fn marks(&self, against: &Against) -> Vec<(String, Option<SeenMark>)> {
            let files = self.listed(against);
            files.into_iter().map(|f| (f.path, f.seen)).collect()
        }

        fn mark(&self, against: &Against, paths: &[&str], seen: bool) {
            let files = self.listed(against);
            let paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
            let path = self.path();
            self.seen
                .mark(&path, against, &files, &paths, seen)
                .unwrap();
        }
    }

    use SeenMark::{Changed, Seen as Marked};

    fn of(pairs: &[(&str, Option<SeenMark>)]) -> Vec<(String, Option<SeenMark>)> {
        pairs.iter().map(|(p, m)| ((*p).to_owned(), *m)).collect()
    }

    #[test]
    fn marks_are_kept_until_the_diff_changes() {
        let repo = Repo::new();
        let head = repo.head();
        repo.write("a", "a2");
        repo.write("b", "b2");
        repo.write("u", "new");
        assert_eq!(
            repo.marks(&head),
            of(&[("a", None), ("b", None), ("u", None)])
        );
        // Nothing marked: nothing is hashed, nor written.
        repo.mark(&head, &["nope"], true);
        assert!(!repo.file.exists());
        repo.mark(&head, &["a", "u", "nope"], true);
        assert_eq!(
            repo.marks(&head),
            of(&[("a", Some(Marked)), ("b", None), ("u", Some(Marked))])
        );
        // Kept across restarts, in Hive's folder: nothing inside `.git`.
        let again = Seen::new(repo.file.clone());
        let mut files = changes::list(&repo.dir, None).unwrap().files;
        again.apply(&repo.path(), &head, &mut files);
        assert_eq!(files[0].seen, Some(Marked));
        // A new change to a seen file makes it changed since seen; the same text again, seen.
        repo.write("a", "a3");
        repo.write("u", "new");
        assert_eq!(
            repo.marks(&head),
            of(&[("a", Some(Changed)), ("b", None), ("u", Some(Marked))])
        );
        repo.write("a", "a2");
        assert_eq!(repo.marks(&head)[0].1, Some(Marked));
        // The base side counts too: committing part of the work changes the diff.
        repo.write("a", "a3");
        run(&repo.dir, &["add", "a"]);
        run(&repo.dir, &["commit", "-q", "-m", "a3"]);
        repo.write("a", "a2");
        assert_eq!(repo.marks(&head)[0].1, Some(Changed));
        // Unmarked.
        repo.mark(&head, &["u"], false);
        assert_eq!(
            repo.marks(&head),
            of(&[("a", Some(Changed)), ("b", None), ("u", None)])
        );
    }

    #[test]
    fn each_base_has_its_own_marks_and_unchanged_files_lose_theirs() {
        let repo = Repo::new();
        let head = repo.head();
        let fork = changes::head(&repo.dir).unwrap();
        commit(&repo.dir, "c");
        repo.write("a", "a2");
        let branch = Against {
            commit: Some(fork),
            ..repo.head()
        };
        repo.mark(&branch, &["c"], true);
        assert_eq!(repo.marks(&head), of(&[("a", None)]));
        assert_eq!(repo.marks(&branch), of(&[("a", None), ("c", Some(Marked))]));
        repo.mark(&head, &["a"], true);
        // Back to its committed text: no longer changed, so its mark goes with the next marking.
        repo.write("a", "a");
        repo.write("b", "b2");
        repo.mark(&head, &["b"], true);
        repo.write("a", "a2");
        assert_eq!(repo.marks(&head), of(&[("a", None), ("b", Some(Marked))]));
        // The last mark of a base goes, and the worktree with it.
        repo.mark(&head, &["b"], false);
        repo.mark(&branch, &["c"], false);
        assert_eq!(std::fs::read_to_string(&repo.file).unwrap(), "{}");
    }

    #[test]
    fn deleted_renamed_linked_and_large_files_have_identities() {
        let repo = Repo::new();
        let head = repo.head();
        std::fs::remove_file(repo.dir.join("a")).unwrap();
        run(&repo.dir, &["mv", "b", "moved"]);
        #[cfg(unix)]
        std::os::unix::fs::symlink("a", repo.dir.join("link")).unwrap();
        repo.write("big", "");
        std::fs::File::options()
            .write(true)
            .open(repo.dir.join("big"))
            .unwrap()
            .set_len(HASH_LIMIT + 1)
            .unwrap();
        // A deleted file alone: nothing to hash.
        repo.mark(&head, &["a"], true);
        assert_eq!(repo.marks(&head)[0], ("a".to_owned(), Some(Marked)));
        let all = ["a", "moved", "link", "big"];
        repo.mark(&head, &all, true);
        let seen: Vec<_> = repo.marks(&head).into_iter().map(|(_, m)| m).collect();
        assert!(seen.iter().all(|m| *m == Some(Marked)), "{seen:?}");
        // Each changes with its own side.
        #[cfg(unix)]
        {
            std::fs::remove_file(repo.dir.join("link")).unwrap();
            std::os::unix::fs::symlink("b", repo.dir.join("link")).unwrap();
        }
        repo.write("moved", "other");
        std::fs::File::options()
            .write(true)
            .open(repo.dir.join("big"))
            .unwrap()
            .set_len(HASH_LIMIT + 2)
            .unwrap();
        let marks = repo.marks(&head);
        let changed: Vec<&str> = marks
            .iter()
            .filter(|(_, m)| *m == Some(Changed))
            .map(|(p, _)| p.as_str())
            .collect();
        let mut expected = vec!["big", "link", "moved"];
        expected.retain(|p| cfg!(unix) || *p != "link");
        assert_eq!(changed, expected);
    }

    #[test]
    fn what_stands_for_a_file_on_disk() {
        let repo = Repo::new();
        assert_eq!(on_disk(&repo.dir, "a"), None);
        assert_eq!(on_disk(&repo.dir, "gone"), Some(NONE.to_owned()));
        assert_eq!(on_disk(&repo.dir, ".git"), Some(NONE.to_owned()));
        // A name `--stdin-paths` cannot take: its size and time.
        // A leading `"` is C-unquoted by git: never sent (`"..\057x"` would be `../x`, `"bad`
        // would fail the whole batch).
        for odd in ["odd\rname", "cr\r", "\"..\\057x\"", "\"bad"] {
            repo.write(odd, "x");
            let id = on_disk(&repo.dir, odd).unwrap_or_default();
            assert!(id.starts_with("stat:1:"), "{id}");
        }
        #[cfg(unix)]
        {
            repo.write("odd\nname", "x");
            assert!(on_disk(&repo.dir, "odd\nname").is_some());
        }
        // At the cap, hashed.
        let at = std::fs::File::create(repo.dir.join("at")).unwrap();
        at.set_len(HASH_LIMIT).unwrap();
        assert_eq!(on_disk(&repo.dir, "at"), None);
    }

    #[test]
    fn a_failing_git_is_changed_or_an_error() {
        let repo = Repo::new();
        repo.write("a", "a2");
        repo.mark(&repo.head(), &["a"], true);
        let listed = changes::list(&repo.dir, None).unwrap().files;
        let paths = vec!["a".to_owned()];
        // A base git cannot read: its marks are under `branch`, none yet; marking fails.
        let gone = Against {
            commit: Some("0".repeat(40)),
            ..repo.head()
        };
        let mut files = listed.clone();
        repo.seen.apply(&repo.path(), &gone, &mut files);
        assert_eq!(files[0].seen, None);
        assert!(
            repo.seen
                .mark(&repo.path(), &gone, &files, &paths, true)
                .is_err()
        );
        // Git cannot tell (the folder is gone): changed, never seen unchecked.
        let broken = Against {
            dir: repo.dir.join("nope"),
            ..repo.head()
        };
        let mut files = listed.clone();
        repo.seen.apply(&repo.path(), &broken, &mut files);
        assert_eq!(files[0].seen, Some(Changed));
        assert!(
            repo.seen
                .mark(&repo.path(), &broken, &listed, &paths, true)
                .is_err()
        );
    }

    #[test]
    fn raw_ids_are_parsed() {
        let z = "0".repeat(40);
        let out = format!(
            ":100644 100644 {a} {z} M\0a\0:100644 100644 {b} {z} R100\0b\0c\0:000000 100644 {z} {z} A\0n\0junk\0",
            a = "1".repeat(40),
            b = "2".repeat(40)
        );
        let ids = parse_ids(out.as_bytes());
        assert_eq!(ids.len(), 3);
        assert_eq!(ids["a"], "1".repeat(40));
        assert_eq!(ids["c"], "2".repeat(40));
        assert_eq!(ids["n"], z);
        assert!(!ids.contains_key("b"));
    }

    #[test]
    fn a_worktree_removed_forgets_its_marks_and_the_file_is_capped() {
        let repo = Repo::new();
        let head = repo.head();
        repo.write("a", "a2");
        repo.mark(&head, &["a"], true);
        repo.seen.forget(&[repo.path()]).unwrap();
        assert_eq!(repo.marks(&head), of(&[("a", None)]));
        let small = Seen::with_limit(repo.file.clone(), 16);
        let files = changes::list(&repo.dir, None).unwrap().files;
        let err = small
            .mark(&repo.path(), &head, &files, &["a".into()], true)
            .unwrap_err();
        assert_eq!(err.to_string(), "seen.json would be over 16 bytes");
    }
}
