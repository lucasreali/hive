//! The files of the worktree the app's files panel shows, kept current with inotify (#31).
//!
//! What a worktree holds is what git lists: tracked files and untracked ones that are not
//! ignored, so `node_modules` or `target` never show up. Watches go only on the directories
//! of those files (plus empty untracked ones) and on the worktree's git dir, where `HEAD` and
//! `index` change what git reports; ignored trees never cost a watch. Every relevant event
//! (an inotify queue overflow included) leads, after a short debounce, to a full re-list.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, InotifyEvent, WatchDescriptor};
use tokio::io::unix::AsyncFd;
use tokio::time::Instant;

use crate::worktree;

/// Most files sent in one list.
pub const MAX_FILES: usize = 50_000;
/// Most bytes of JSON strings in one list, well inside a frame (`MAX_PAYLOAD`, 4 MiB).
pub const LIST_BUDGET: usize = 3_145_728; // 3 MiB
/// Most bytes read from one git command; the rest is cut off.
const GIT_LIMIT: u64 = 33_554_432; // 32 MiB
/// Most directories watched in one worktree.
pub const MAX_WATCHES: usize = 8192;
/// A re-list waits until events have stopped for this long…
pub const QUIET: Duration = Duration::from_millis(200);
/// …but never longer than this after the first one, so a busy worktree still updates.
pub const MAX_DELAY: Duration = Duration::from_secs(1);

const DIR_EVENTS: AddWatchFlags = AddWatchFlags::IN_CREATE
    .union(AddWatchFlags::IN_DELETE)
    .union(AddWatchFlags::IN_MOVE)
    .union(AddWatchFlags::IN_MODIFY)
    .union(AddWatchFlags::IN_ATTRIB)
    .union(AddWatchFlags::IN_DELETE_SELF)
    .union(AddWatchFlags::IN_MOVE_SELF)
    .union(AddWatchFlags::IN_ONLYDIR)
    .union(AddWatchFlags::IN_DONT_FOLLOW);
/// Git replaces `HEAD` and `index` by renaming a lock file over them.
const GIT_EVENTS: AddWatchFlags = AddWatchFlags::IN_CREATE
    .union(AddWatchFlags::IN_MOVED_TO)
    .union(AddWatchFlags::IN_MODIFY)
    .union(AddWatchFlags::IN_DELETE)
    .union(AddWatchFlags::IN_ONLYDIR);

/// What git lists in a worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// Sorted, `/`-separated, relative to the worktree.
    pub files: Vec<String>,
    /// The list stopped at `MAX_FILES`, `LIST_BUDGET` or git's output limit.
    pub truncated: bool,
}

/// NUL-terminated paths from `git ls-files -z` → a [`Listing`]. `cut` says git's output was
/// cut off, so its last path is partial. Names that are not UTF-8 are skipped, and so are
/// nested repositories (`dir/`); a path listed twice (a conflict's stages) is kept once.
pub fn parse(out: &[u8], cut: bool) -> Listing {
    let mut pieces: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    // Empty after the last NUL, or partial when cut.
    pieces.pop();
    let mut files: Vec<&str> = pieces
        .into_iter()
        .filter_map(|p| std::str::from_utf8(p).ok())
        .filter(|p| !p.ends_with('/'))
        .collect();
    files.sort_unstable();
    files.dedup();
    let mut budget = LIST_BUDGET;
    let mut kept = Vec::new();
    for name in files.iter().take(MAX_FILES) {
        let cost = serde_json::to_string(name).map_or(usize::MAX, |json| json.len());
        let Some(left) = budget.checked_sub(cost) else {
            break;
        };
        budget = left;
        kept.push((*name).to_owned());
    }
    Listing {
        truncated: cut || kept.len() < files.len(),
        files: kept,
    }
}

/// Every directory holding one of `paths`, the worktree itself (`""`) included. A path
/// ending in `/` is a directory itself.
pub fn dirs<'a>(paths: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'a str> {
    let mut dirs = BTreeSet::from([""]);
    for path in paths {
        let mut rest = path;
        while let Some(end) = rest.rfind('/') {
            rest = &rest[..end];
            // Its parents are already in.
            if !dirs.insert(rest) {
                break;
            }
        }
    }
    dirs
}

/// Whether a change to `name` in the git dir changes what git reports.
pub fn git_state(name: Option<&OsStr>) -> bool {
    matches!(name.and_then(OsStr::to_str), Some("HEAD" | "index"))
}

/// When a burst of events is over: `QUIET` after the last one, at most `MAX_DELAY` after the
/// first. The clock is passed in.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    first: Instant,
    last: Instant,
}

impl Debounce {
    pub fn new(now: Instant) -> Self {
        Self {
            first: now,
            last: now,
        }
    }

    pub fn event(&mut self, now: Instant) {
        self.last = now;
    }

    pub fn deadline(&self) -> Instant {
        (self.last + QUIET).min(self.first + MAX_DELAY)
    }
}

/// Watches one worktree.
pub struct Watcher {
    root: PathBuf,
    inotify: AsyncFd<Fd>,
    git_dir: Option<WatchDescriptor>,
    /// Watched directories, relative to `root`.
    dirs: BTreeMap<String, WatchDescriptor>,
}

impl Watcher {
    /// Starts watching the worktree at `root`'s git dir; [`Watcher::list`] adds the rest.
    /// Blocks on git; must run inside the tokio runtime.
    pub fn new(root: &Path) -> io::Result<Self> {
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)?;
        let (out, _) = git(root, &["rev-parse", "--absolute-git-dir"], 65_536)?;
        let git_dir = String::from_utf8_lossy(&out);
        let git_dir = inotify.add_watch(git_dir.trim_end(), GIT_EVENTS).ok();
        Ok(Self {
            root: root.to_owned(),
            inotify: AsyncFd::new(Fd(inotify))?,
            git_dir,
            dirs: BTreeMap::new(),
        })
    }

    /// Lists the worktree and moves the watches to the directories it lists. Blocks on git.
    pub fn list(&mut self) -> io::Result<Listing> {
        let ls = [
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ];
        let (out, cut) = git(&self.root, &ls, GIT_LIMIT)?;
        let listing = parse(&out, cut);
        // Untracked directories too, even empty ones, so files created in them are seen.
        // ponytail: git names only the top of a new untracked tree, so `mkdir -p a/b` watches
        // `a` alone until a file appears; walk such trees if deep late writes go unseen.
        let ls = [
            "ls-files",
            "--others",
            "--exclude-standard",
            "--directory",
            "-z",
        ];
        let (untracked, _) = git(&self.root, &ls, GIT_LIMIT)?;
        let untracked = parse_dirs(&untracked);
        let files = listing.files.iter().map(String::as_str);
        let wanted: BTreeSet<&str> = dirs(files.chain(untracked))
            .into_iter()
            .take(MAX_WATCHES)
            .collect();
        let inotify = &self.inotify.get_ref().0;
        // Removed first: a renamed directory keeps its watch, which must not be reused.
        self.dirs.retain(|dir, wd| {
            let keep = wanted.contains(dir.as_str());
            if !keep {
                // Fails when the directory is gone, which removed its watch already.
                let _ = inotify.rm_watch(*wd);
            }
            keep
        });
        for dir in wanted {
            if !self.dirs.contains_key(dir)
                && let Ok(wd) = inotify.add_watch(&self.root.join(dir), DIR_EVENTS)
            {
                self.dirs.insert(dir.to_owned(), wd);
            }
        }
        Ok(listing)
    }

    /// Waits until something in the worktree changed and the burst of events is over.
    pub async fn changed(&mut self) -> io::Result<()> {
        while !self.relevant().await? {}
        let mut debounce = Debounce::new(Instant::now());
        loop {
            tokio::select! {
                relevant = self.relevant() => if relevant? { debounce.event(Instant::now()) },
                () = tokio::time::sleep_until(debounce.deadline()) => return Ok(()),
            }
        }
    }

    /// Waits for events; whether any of them may change what git lists.
    async fn relevant(&mut self) -> io::Result<bool> {
        let mut ready = self.inotify.readable().await?;
        let Ok(events) = ready.try_io(|fd| Ok(fd.get_ref().0.read_events()?)) else {
            return Ok(false);
        };
        // Every event is seen: some only update the watches.
        let seen: Vec<bool> = events?.iter().map(|e| self.saw(e)).collect();
        Ok(seen.contains(&true))
    }

    fn saw(&mut self, event: &InotifyEvent) -> bool {
        if event.mask.contains(AddWatchFlags::IN_IGNORED) {
            // The directory is gone, and its watch with it.
            self.dirs.retain(|_, wd| *wd != event.wd);
            return false;
        }
        if Some(event.wd) == self.git_dir {
            return git_state(event.name.as_deref());
        }
        // Anything else, an overflow included, calls for a full re-list.
        true
    }
}

/// What tokio needs to poll the inotify descriptor.
struct Fd(Inotify);

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_fd().as_raw_fd()
    }
}

/// The directories among `--directory` entries (`dir/`), not UTF-8 ones skipped.
fn parse_dirs(out: &[u8]) -> impl Iterator<Item = &str> {
    out.split(|&b| b == 0)
        .filter_map(|p| std::str::from_utf8(p).ok())
        .filter(|p| p.ends_with('/'))
}

/// `git -C <root> <args>`'s output, at most `limit` bytes; true when it was cut off there.
fn git(root: &Path, args: &[&str], limit: u64) -> io::Result<(Vec<u8>, bool)> {
    let mut child = worktree::git_command(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("cannot run git: {err}")))?;
    let mut out = Vec::new();
    let read = child
        .stdout
        .take()
        .map(|s| s.take(limit + 1).read_to_end(&mut out));
    let cut = out.len() as u64 > limit;
    if cut {
        out.truncate(out.len() - 1);
        let _ = child.kill();
    }
    let status = child.wait()?;
    read.transpose()?;
    if cut || status.success() {
        return Ok((out, cut));
    }
    let args = args.join(" ");
    let root = root.display();
    Err(io::Error::other(format!("git {args} failed in {root}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(listing: &Listing) -> Vec<&str> {
        listing.files.iter().map(String::as_str).collect()
    }

    #[test]
    fn listings_are_sorted_deduplicated_and_skip_bad_names() {
        let out = b"b.txt\0a/z.rs\0a/z.rs\0bad\xff\0nested/\0a b\0";
        let listing = parse(out, false);
        assert_eq!(names(&listing), ["a b", "a/z.rs", "b.txt"]);
        assert!(!listing.truncated);
        assert_eq!(parse(b"", false), Listing::default());
    }

    #[test]
    fn a_cut_listing_drops_its_partial_last_path() {
        let listing = parse(b"a\0b\0par", true);
        assert_eq!(names(&listing), ["a", "b"]);
        assert!(listing.truncated);
    }

    #[test]
    fn listings_stop_at_the_file_cap() {
        let out: Vec<u8> = (0..=MAX_FILES)
            .flat_map(|i| format!("f{i:06}\0").into_bytes())
            .collect();
        let listing = parse(&out, false);
        assert_eq!(listing.files.len(), MAX_FILES);
        assert_eq!(
            listing.files.last().unwrap(),
            &format!("f{:06}", MAX_FILES - 1)
        );
        assert!(listing.truncated);
        let exact = parse(&out[..out.len() - 8], false);
        assert_eq!(exact.files.len(), MAX_FILES);
        assert!(!exact.truncated);
    }

    #[test]
    fn listings_stop_at_the_byte_budget() {
        // Each name costs 1000 bytes of JSON: 998 characters and two quotes.
        let out: Vec<u8> = (0..4000)
            .flat_map(|i| format!("{i:04}{}\0", "x".repeat(994)).into_bytes())
            .collect();
        let listing = parse(&out, false);
        assert_eq!(listing.files.len(), LIST_BUDGET / 1000);
        assert!(listing.truncated);
        // Escapes count: a quote costs two bytes.
        let quoted = format!("{}\0", "\"".repeat(LIST_BUDGET / 2 - 1));
        assert_eq!(parse(quoted.as_bytes(), false).files.len(), 1);
        let over = format!("{}\0", "\"".repeat(LIST_BUDGET / 2));
        assert!(parse(over.as_bytes(), false).files.is_empty());
    }

    #[test]
    fn every_directory_of_the_listed_paths_is_watched() {
        let got = dirs(["a/b/c.rs", "a/d.rs", "top.rs", "new/", "x/y/"]);
        let want = BTreeSet::from(["", "a", "a/b", "new", "x", "x/y"]);
        assert_eq!(got, want);
        assert_eq!(dirs([]), BTreeSet::from([""]));
    }

    #[test]
    fn only_untracked_directories_are_taken_from_the_directory_listing() {
        let got: Vec<&str> = parse_dirs(b"a.txt\0new/\0bad\xff/\0").collect();
        assert_eq!(got, ["new/"]);
    }

    #[test]
    fn only_head_and_index_matter_in_the_git_dir() {
        assert!(git_state(Some(OsStr::new("HEAD"))));
        assert!(git_state(Some(OsStr::new("index"))));
        assert!(!git_state(Some(OsStr::new("index.lock"))));
        assert!(!git_state(Some(OsStr::new("FETCH_HEAD"))));
        assert!(!git_state(None));
    }

    #[test]
    fn a_burst_settles_after_a_quiet_spell_or_the_longest_delay() {
        let start = Instant::now();
        let mut debounce = Debounce::new(start);
        assert_eq!(debounce.deadline(), start + QUIET);
        debounce.event(start + QUIET / 2);
        assert_eq!(debounce.deadline(), start + QUIET / 2 + QUIET);
        // Events keep coming: the first one bounds the wait.
        debounce.event(start + MAX_DELAY);
        assert_eq!(debounce.deadline(), start + MAX_DELAY);
    }

    #[test]
    fn git_output_is_cut_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (full, cut) = git(dir.path(), &["--version"], 1024).unwrap();
        assert!(full.starts_with(b"git version "), "{full:?}");
        assert!(!cut);
        assert_eq!(
            git(dir.path(), &["--version"], 4).unwrap(),
            (b"git ".to_vec(), true)
        );
        let exact = full.len() as u64;
        assert_eq!(
            git(dir.path(), &["--version"], exact).unwrap(),
            (full, false)
        );
    }

    /// A new repository in a temporary directory; the tests' own git never reads the user's config.
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        run_git(&root, &["init", "-q"]);
        (dir, root)
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, rel).unwrap();
    }

    fn watched(watcher: &Watcher) -> Vec<&str> {
        watcher.dirs.keys().map(String::as_str).collect()
    }

    /// Whether the watcher reports a change within `wait`.
    async fn changes(watcher: &mut Watcher, wait: Duration) -> bool {
        tokio::time::timeout(wait, watcher.changed()).await.is_ok()
    }

    const SOON: Duration = Duration::from_secs(5);
    const NEVER: Duration = Duration::from_millis(600);

    #[tokio::test]
    async fn ignored_trees_are_neither_listed_nor_watched() {
        let (_dir, root) = repo();
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        write(&root, "ignored/deep/x.txt");
        write(&root, "src/a.rs");
        std::fs::create_dir(root.join("empty")).unwrap();
        let mut watcher = Watcher::new(&root).unwrap();
        assert!(watcher.git_dir.is_some());
        let listing = watcher.list().unwrap();
        assert_eq!(names(&listing), [".gitignore", "src/a.rs"]);
        assert_eq!(watched(&watcher), ["", "empty", "src"]);
        // Nothing happens, and nothing in an ignored tree counts.
        assert!(!changes(&mut watcher, NEVER).await);
        write(&root, "ignored/deep/y.txt");
        assert!(!changes(&mut watcher, NEVER).await);
        // A file in an empty untracked directory is seen.
        write(&root, "empty/new.txt");
        assert!(changes(&mut watcher, SOON).await);
        let listing = watcher.list().unwrap();
        assert_eq!(names(&listing), [".gitignore", "empty/new.txt", "src/a.rs"]);
    }

    #[tokio::test]
    async fn a_renamed_directory_is_watched_under_its_new_name() {
        let (_dir, root) = repo();
        write(&root, "d/a.txt");
        let mut watcher = Watcher::new(&root).unwrap();
        watcher.list().unwrap();
        std::fs::rename(root.join("d"), root.join("e")).unwrap();
        assert!(changes(&mut watcher, SOON).await);
        assert_eq!(names(&watcher.list().unwrap()), ["e/a.txt"]);
        assert_eq!(watched(&watcher), ["", "e"]);
        write(&root, "e/b.txt");
        assert!(changes(&mut watcher, SOON).await);
        assert_eq!(names(&watcher.list().unwrap()), ["e/a.txt", "e/b.txt"]);
    }

    #[tokio::test]
    async fn a_directory_removed_and_made_again_gets_a_new_watch() {
        let (_dir, root) = repo();
        write(&root, "d/a.txt");
        let mut watcher = Watcher::new(&root).unwrap();
        watcher.list().unwrap();
        // Both within one burst: the old watch is gone before the re-list.
        std::fs::remove_dir_all(root.join("d")).unwrap();
        write(&root, "d/b.txt");
        assert!(changes(&mut watcher, SOON).await);
        assert_eq!(names(&watcher.list().unwrap()), ["d/b.txt"]);
        write(&root, "d/c.txt");
        assert!(changes(&mut watcher, SOON).await);
        assert_eq!(names(&watcher.list().unwrap()), ["d/b.txt", "d/c.txt"]);
    }

    #[tokio::test]
    async fn the_index_changes_what_is_listed() {
        let (_dir, root) = repo();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        write(&root, "x.log");
        run_git(&root, &["add", "-f", "x.log"]);
        let mut watcher = Watcher::new(&root).unwrap();
        assert_eq!(names(&watcher.list().unwrap()), [".gitignore", "x.log"]);
        // Only the index changes: the file is now untracked, so ignored.
        run_git(&root, &["rm", "-q", "--cached", "x.log"]);
        assert!(changes(&mut watcher, SOON).await);
        assert_eq!(names(&watcher.list().unwrap()), [".gitignore"]);
    }

    #[tokio::test]
    async fn a_folder_outside_git_cannot_be_watched() {
        let dir = tempfile::tempdir().unwrap();
        let err = Watcher::new(dir.path()).err().unwrap();
        let message = err.to_string();
        assert!(
            message.starts_with("git rev-parse --absolute-git-dir failed"),
            "{message}"
        );
    }

    #[test]
    fn a_failing_git_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = git(dir.path(), &["no-such-command"], 1024).unwrap_err();
        let want = format!("git no-such-command failed in {}", dir.path().display());
        assert_eq!(err.to_string(), want);
    }
}
