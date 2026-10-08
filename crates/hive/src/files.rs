//! The files of the worktree the app's files panel shows, kept current with the `notify`
//! crate (#31): inotify on Linux, FSEvents on macOS.
//!
//! What a worktree holds is what git lists: tracked files and untracked ones that are not
//! ignored, and apart from them what git ignores (14.2): ignored files, and ignored folders
//! (`node_modules`, `target`) as one entry each, whose contents are listed one level deep and
//! watched only while the app has them open. Watches go only on the directories of the listed
//! files (plus empty untracked ones), on the open ignored folders and on the worktree's git
//! dir, where `HEAD` and `index` change what git reports; a closed ignored folder never costs
//! a watch. Every relevant event (a queue overflow included) leads, after a short debounce, to
//! a full re-list.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{
    Event, EventKind, PathOp, RecommendedWatcher, RecursiveMode, WatchPathConfig, Watcher as _,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::git;
use crate::paths::canonical;

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

/// Events waiting to be seen; more are dropped, as a burst is already pending.
const EVENT_QUEUE: usize = 4096;

/// What git lists in a worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// Sorted, `/`-separated, relative to the worktree.
    pub files: Vec<String>,
    /// What git ignores, sorted: files, and folders as `dir/`, with one level of what each
    /// open one holds (its folders as `dir/sub/`).
    pub ignored: Vec<String>,
    /// The list stopped at `MAX_FILES`, `LIST_BUDGET` or git's output limit; files and
    /// ignored entries share the caps, files first.
    pub truncated: bool,
}

/// Room left in one list: entries, and bytes of their JSON strings.
#[derive(Debug, Clone, Copy)]
struct Room {
    entries: usize,
    bytes: usize,
}

impl Room {
    const FULL: Room = Room {
        entries: MAX_FILES,
        bytes: LIST_BUDGET,
    };

    /// The first of `names` while they fit; true when one was left out.
    fn fill<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> (Vec<String>, bool) {
        let mut kept = Vec::new();
        for name in names {
            let cost = serde_json::to_string(name).map_or(usize::MAX, |json| json.len());
            match (self.entries.checked_sub(1), self.bytes.checked_sub(cost)) {
                (Some(entries), Some(bytes)) => *self = Room { entries, bytes },
                _ => return (kept, true),
            }
            kept.push(name.to_owned());
        }
        (kept, false)
    }
}

/// The paths of `git ls-files -z`'s output, sorted, each once (a conflict's stages list one
/// several times), names that are not UTF-8 skipped. The last piece is dropped: empty after
/// the last NUL, or partial when the output was cut off.
fn paths(out: &[u8]) -> Vec<&str> {
    let mut pieces: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    pieces.pop();
    let mut paths: Vec<&str> = pieces
        .into_iter()
        .filter_map(|p| std::str::from_utf8(p).ok())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// NUL-terminated paths from `git ls-files -z` → a [`Listing`]. `cut` says git's output was
/// cut off, so its last path is partial. Names that are not UTF-8 are skipped, and so are
/// nested repositories (`dir/`); a path listed twice (a conflict's stages) is kept once.
pub fn parse(out: &[u8], cut: bool) -> Listing {
    let mut files = paths(out);
    files.retain(|p| !p.ends_with('/'));
    let mut room = Room::FULL;
    let (files, left_out) = room.fill(files);
    Listing {
        files,
        ignored: Vec::new(),
        truncated: cut || left_out,
    }
}

/// The entries of `git ls-files --others --ignored --exclude-standard --directory -z`: files,
/// and folders as `dir/` without what they hold (git also names each file of a folder it
/// lists for holding nothing but ignored files).
pub fn parse_ignored(out: &[u8]) -> Vec<&str> {
    let mut folder: Option<&str> = None;
    let mut entries = paths(out);
    // Sorted: what a folder holds comes right after it.
    entries.retain(|entry| {
        if folder.is_some_and(|folder| entry.starts_with(folder)) {
            return false;
        }
        if entry.ends_with('/') {
            folder = Some(entry);
        }
        true
    });
    entries
}

/// What tells one folder from another: its device and inode. (Windows has none: its path is
/// checked for links again instead.)
#[cfg(unix)]
type Id = (u64, u64);

/// The folder `folder` (relative, `/`-separated) of the canonical `root`, each part of its path
/// opened from the one before with `O_NOFOLLOW`: a link in any part, swapped in at any time,
/// fails the open instead of leading out of the worktree.
#[cfg(unix)]
fn open_folder(root: &Path, folder: &str) -> Option<(std::fs::File, Id)> {
    use nix::fcntl::{AT_FDCWD, OFlag, openat};
    use nix::sys::stat::Mode;
    use std::os::unix::fs::MetadataExt;
    // `O_DIRECTORY` also keeps a FIFO from blocking the open.
    let flags = [
        OFlag::O_RDONLY,
        OFlag::O_DIRECTORY,
        OFlag::O_NOFOLLOW,
        OFlag::O_CLOEXEC,
    ];
    let flags: OFlag = flags.into_iter().collect();
    let mut fd = openat(AT_FDCWD, root, flags, Mode::empty()).ok()?;
    for part in folder.split('/') {
        // Names only: `..` would climb out.
        if matches!(part, "" | "." | "..") {
            return None;
        }
        fd = openat(&fd, part, flags, Mode::empty()).ok()?;
    }
    let file = std::fs::File::from(fd);
    let meta = file.metadata().ok()?;
    Some((file, (meta.dev(), meta.ino())))
}

/// Whether `folder` of `root` is still the folder `id` names, reached without a link.
#[cfg(unix)]
fn same(root: &Path, folder: &str, id: &Id) -> bool {
    open_folder(root, folder).is_some_and(|(_, now)| now == *id)
}

// Windows has no `openat`: its folder is checked by path, then read (`crate::windows`).
#[cfg(windows)]
use crate::windows::{folder_level as level, same_folder as same};

/// One level of the ignored folder `folder` (relative, no trailing `/`) of the canonical
/// `root`, and which folder it was: at most `most` entries, its folders as `folder/name/`,
/// names that are not UTF-8 skipped. `None` when it cannot be read, or is no longer a folder
/// of `root` itself (a link in any part of its path).
#[cfg(unix)]
fn level(root: &Path, folder: &str, most: usize) -> Option<(Vec<String>, Id)> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{SFlag, fstatat};
    let (file, id) = open_folder(root, folder)?;
    let dir = nix::dir::Dir::from_fd(file.try_clone().ok()?.into()).ok()?;
    let entry = |entry: nix::dir::Entry| {
        let name = entry.file_name().to_str().ok()?;
        // Its own type, never its target's: a link to a folder is listed as a file, so it is
        // never opened as a folder.
        let stat = fstatat(&file, entry.file_name(), AtFlags::AT_SYMLINK_NOFOLLOW).ok()?;
        let kind = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT;
        let slash = if kind == SFlag::S_IFDIR { "/" } else { "" };
        Some(format!("{folder}/{name}{slash}"))
    };
    let entries = dir
        .into_iter()
        // Stops at the first error, as `read_dir` does: one that keeps failing (EIO) would
        // otherwise never end.
        .map_while(Result::ok)
        .filter(|entry| !matches!(entry.file_name().to_bytes(), b"." | b".."))
        .take(most)
        .filter_map(entry)
        .collect();
    Some((entries, id))
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

/// When a burst of events is over: `quiet` after the last one, at most `max` after the
/// first. The clock is passed in.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    first: Instant,
    last: Instant,
    quiet: Duration,
    max: Duration,
}

impl Debounce {
    pub fn new(now: Instant, quiet: Duration, max: Duration) -> Self {
        Self {
            first: now,
            last: now,
            quiet,
            max,
        }
    }

    pub fn event(&mut self, now: Instant) {
        self.last = now;
    }

    pub fn deadline(&self) -> Instant {
        (self.last + self.quiet).min(self.first + self.max)
    }
}

/// Watches one worktree.
pub struct Watcher {
    /// Canonical, as the paths of events are.
    root: PathBuf,
    watcher: RecommendedWatcher,
    events: mpsc::Receiver<notify::Result<Event>>,
    git_dir: Option<PathBuf>,
    /// Watched directories, relative to `root`.
    dirs: BTreeSet<String>,
    /// The ignored folders the app opened, relative to `root`.
    open: BTreeSet<String>,
    /// Called with each open ignored folder right before it is read and right before it is
    /// watched: tests swap the folder there.
    hook: fn(Step, &Path, &str),
}

/// Where [`Watcher::list`] is with an open ignored folder when it calls its hook.
#[derive(Debug, Clone, Copy)]
enum Step {
    Read,
    Watch,
}

impl Watcher {
    /// Starts watching the worktree at `root`'s git dir; [`Watcher::list`] adds the rest.
    /// Blocks on git; must run inside the tokio runtime.
    pub fn new(root: &Path) -> io::Result<Self> {
        let (out, _) = git(root, &["rev-parse", "--absolute-git-dir"], 65_536)?;
        let (tx, events) = mpsc::channel(EVENT_QUEUE);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            // Git reads the worktree on every re-list: reads never count.
            if !matches!(&event, Ok(event) if event.kind.is_access()) {
                // Full: a burst is already waiting to be seen.
                let _ = tx.try_send(event);
            }
        })
        .map_err(io::Error::other)?;
        let git_dir = PathBuf::from(String::from_utf8_lossy(&out).trim_end());
        let git_dir = canonical(&git_dir)
            .ok()
            .filter(|dir| watcher.watch(dir, RecursiveMode::NonRecursive).is_ok());
        Ok(Self {
            root: canonical(root)?,
            watcher,
            events,
            git_dir,
            dirs: BTreeSet::new(),
            open: BTreeSet::new(),
            hook: |_, _, _| {},
        })
    }

    /// The ignored folders the app has open (`/`-separated, relative, no trailing `/`), at
    /// most `MAX_WATCHES`. [`Watcher::list`] lists and watches one only while it is an ignored
    /// folder that listing holds, so a folder inside one counts only while that one is open.
    pub fn open(&mut self, folders: impl IntoIterator<Item = String>) {
        self.open = folders.into_iter().take(MAX_WATCHES).collect();
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
        let mut listing = parse(&out, cut);
        let ls = [
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ];
        let (out, cut) = git(&self.root, &ls, GIT_LIMIT)?;
        let mut ignored: BTreeSet<String> =
            parse_ignored(&out).into_iter().map(str::to_owned).collect();
        // Parents first: a folder inside an open one is there once that one was listed.
        let mut opened = Vec::new();
        for folder in &self.open {
            // Enough to fill the list: whatever comes next would be left out.
            if listing.files.len() + ignored.len() > MAX_FILES {
                break;
            }
            if !ignored.contains(&format!("{folder}/")) {
                continue;
            }
            // One more than fits, so a cut list says so; never more, however big the folder.
            // Git names a folder only when it tracks nothing in it, so nothing here is listed.
            // Not read, not watched either: the watch would follow the same path.
            (self.hook)(Step::Read, &self.root, folder);
            let Some((level, id)) = level(&self.root, folder, MAX_FILES + 1) else {
                continue;
            };
            ignored.extend(level);
            opened.push((folder.as_str(), id));
        }
        let mut room = Room::FULL;
        room.fill(listing.files.iter().map(String::as_str));
        let (kept, left_out) = room.fill(ignored.iter().map(String::as_str));
        listing.ignored = kept;
        listing.truncated |= cut || left_out;
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
        let mut wanted = dirs(files.chain(untracked));
        wanted.extend(opened.iter().map(|(folder, _)| *folder));
        let wanted: BTreeSet<&str> = wanted.into_iter().take(MAX_WATCHES).collect();
        // ponytail: on macOS every re-list restarts the FSEvents stream, even when no watch
        // moves; skip the restart if its cost or its short blind spot shows.
        // Removed first: a renamed directory keeps its watch, which must not be reused.
        let gone = self
            .dirs
            .iter()
            .filter(|dir| !wanted.contains(dir.as_str()));
        // Unwatching fails when the directory is gone, which removed its watch already.
        let unwatch = gone.map(|dir| PathOp::unwatch(self.root.join(dir)));
        let added: Vec<&str> = wanted
            .iter()
            .copied()
            .filter(|dir| !self.dirs.contains(*dir))
            .collect();
        // A link at the end of the path is watched as itself, never followed (inotify).
        let config = WatchPathConfig::new(RecursiveMode::NonRecursive);
        let config = config.with_dereference_symlinks(false);
        let watch = added
            .iter()
            .map(|dir| PathOp::Watch(self.root.join(dir), config.clone()));
        let ops = unwatch.chain(watch).collect();
        let keys = |watcher: &RecommendedWatcher| -> io::Result<BTreeSet<PathBuf>> {
            let paths = watcher.watched_paths().map_err(io::Error::other)?;
            Ok(paths.into_iter().map(|(path, _)| path).collect())
        };
        // Only an open folder watched anew is checked below.
        let fresh = opened
            .iter()
            .any(|(folder, _)| !self.dirs.contains(*folder));
        let before = if fresh {
            keys(&self.watcher)?
        } else {
            BTreeSet::new()
        };
        for (folder, _) in &opened {
            (self.hook)(Step::Watch, &self.root, folder);
        }
        let watcher = &mut self.watcher;
        let mut failed = update(|ops| watcher.update_paths(ops).map_err(Box::new), ops)
            .map_err(io::Error::other)?;
        // A watch takes a path, never a descriptor (inotify, FSEvents and Windows alike), so it
        // may follow a link swapped in after the read: an open folder that is no longer the one
        // read loses its new watch again. One swapped out and back between the watch and this
        // check keeps it, on a folder whose events only ever ask for a re-list.
        let swapped: Vec<PathBuf> = opened
            .iter()
            .filter(|(folder, id)| !self.dirs.contains(*folder) && !same(&self.root, folder, id))
            .map(|(folder, _)| self.root.join(folder))
            .collect();
        if !swapped.is_empty() {
            // Dropped by the key the watcher keeps: inotify keeps the path given, FSEvents what
            // it resolved to, which the path may no longer lead to. Every new watch but those
            // of the folders kept goes.
            let kept: BTreeSet<PathBuf> = added
                .iter()
                .map(|dir| self.root.join(dir))
                .filter(|path| !swapped.contains(path))
                .collect();
            let stray = keys(watcher)?
                .into_iter()
                .filter(|key| !before.contains(key) && !kept.contains(key))
                .map(PathOp::unwatch)
                .collect();
            update(|ops| watcher.update_paths(ops).map_err(Box::new), stray)
                .map_err(io::Error::other)?;
            failed.extend(swapped);
        }
        self.dirs.retain(|dir| wanted.contains(dir.as_str()));
        let watched = added
            .into_iter()
            .filter(|dir| !failed.contains(&self.root.join(dir)));
        self.dirs.extend(watched.map(str::to_owned));
        Ok(listing)
    }

    /// Waits until something in the worktree changed and the burst of events is over.
    pub async fn changed(&mut self) -> io::Result<()> {
        // Every wait is in this one loop, so no helper can return without waiting.
        let mut burst: Option<Debounce> = None;
        loop {
            let deadline = burst.map(|burst| burst.deadline());
            let settled = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now));
            tokio::select! {
                event = self.events.recv() => {
                    // No event ever again: the watcher's thread is gone.
                    let event = event.ok_or(io::ErrorKind::BrokenPipe)?;
                    if self.saw(event) {
                        let now = Instant::now();
                        let debounce = Debounce::new(now, QUIET, MAX_DELAY);
                        burst.get_or_insert(debounce).event(now);
                    }
                }
                () = settled, if deadline.is_some() => return Ok(()),
            }
        }
    }

    /// Whether `event` may change what git lists.
    fn saw(&mut self, event: notify::Result<Event>) -> bool {
        // A watcher error may have lost events: a full re-list is safe.
        let Ok(event) = event else { return true };
        if matches!(
            event.kind,
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        ) {
            // A directory gone or moved lost its watch. Kept with git's `/` (Windows: `\`).
            for path in &event.paths {
                if let Some(dir) = path.strip_prefix(&self.root).ok().and_then(Path::to_str) {
                    self.dirs.remove(&dir.replace(MAIN_SEPARATOR, "/"));
                }
            }
        }
        // A folder's own change changes nothing git lists: what it holds has its own watch, or
        // is ignored. (Windows reports one for a write anywhere under it, Unix for its mode.)
        let own = matches!(
            event.kind,
            EventKind::Modify(ModifyKind::Any | ModifyKind::Metadata(_))
        );
        // Never through a link: one to a network path would reach its host.
        let folder = |path: &PathBuf| path.symlink_metadata().is_ok_and(|meta| meta.is_dir());
        if own && event.paths.iter().all(folder) {
            return false;
        }
        // No path: an overflow, which calls for a full re-list.
        event.paths.is_empty() || event.paths.iter().any(|path| self.counts(path))
    }

    /// Whether a change to `path` counts: in the git dir, only `HEAD` and `index` do.
    fn counts(&self, path: &Path) -> bool {
        match &self.git_dir {
            Some(git_dir) if path.starts_with(git_dir) => {
                path.parent() == Some(git_dir) && git_state(path.file_name())
            }
            _ => true,
        }
    }
}

/// Applies `ops` in one batch with `apply` (a watcher's `update_paths`: FSEvents restarts its
/// stream once a batch), going on past the ones that fail: the paths of those. An error when
/// the watcher itself failed.
pub fn update(
    mut apply: impl FnMut(Vec<PathOp>) -> Result<(), Box<notify::UpdatePathsError>>,
    mut ops: Vec<PathOp>,
) -> notify::Result<Vec<PathBuf>> {
    let mut failed = Vec::new();
    // Each round leaves out the op that failed: at most one round per op.
    while let Err(err) = apply(ops).map_err(|err| *err) {
        let origin = err.origin.ok_or(err.source)?;
        failed.push(origin.into_path());
        ops = err.remaining;
    }
    Ok(failed)
}

/// The directories among `--directory` entries (`dir/`), not UTF-8 ones skipped.
fn parse_dirs(out: &[u8]) -> impl Iterator<Item = &str> {
    out.split(|&b| b == 0)
        .filter_map(|p| std::str::from_utf8(p).ok())
        .filter(|p| p.ends_with('/'))
}

/// `git -C <root> <args>`'s output, at most `limit` bytes; true when it was cut off there.
fn git(root: &Path, args: &[&str], limit: u64) -> io::Result<(Vec<u8>, bool)> {
    let mut child = git::command(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
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
    fn ignored_folders_are_listed_without_what_they_hold() {
        // As git lists a folder holding only ignored files: the folder, then each file.
        let out = b"logs/b.log\0.env\0logs/\0logs/a.log\0src/x.o\0logs-old.txt\0bad\xff\0par";
        assert_eq!(
            parse_ignored(out),
            [".env", "logs-old.txt", "logs/", "src/x.o"]
        );
        assert!(parse_ignored(b"").is_empty());
    }

    #[test]
    fn a_level_of_an_ignored_folder_never_goes_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = &canonical(dir.path()).unwrap();
        write(root, "deps/a/x.js");
        write(root, "deps/b.js");
        let names = |folder| level(root, folder, 10).map(|(names, _)| names);
        let (mut got, id) = level(root, "deps", 10).unwrap();
        got.sort();
        assert_eq!(got, ["deps/a/", "deps/b.js"]);
        assert!(same(root, "deps", &id));
        // Windows checks the path only.
        #[cfg(unix)]
        assert!(!same(root, "deps/a", &id));
        assert_eq!(level(root, "deps", 1).unwrap().0.len(), 1);
        assert_eq!(names("deps/a"), Some(vec!["deps/a/x.js".to_owned()]));
        assert_eq!(names("gone"), None);
        assert_eq!(names("deps/b.js"), None);
        #[cfg(unix)]
        {
            // Names only, never a step up or an empty part.
            assert_eq!(names("deps/.."), None);
            assert_eq!(names("deps/a/.."), None);
            assert_eq!(names("deps/."), None);
            assert_eq!(names("deps//a"), None);
            std::os::unix::fs::symlink(root.join("deps"), root.join("link")).unwrap();
            std::os::unix::fs::symlink(root.join("deps"), root.join("deps/c")).unwrap();
            assert_eq!(names("link"), None);
            // Nor through a link in the middle of the path.
            assert_eq!(names("link/a"), None);
            // A link to a folder inside one is listed as a file, so it is never opened.
            assert!(names("deps").unwrap().contains(&"deps/c".to_owned()));
        }
    }

    /// Moves `folder` of `root` out of the worktree, next to it, and puts a link to it there.
    #[cfg(unix)]
    fn swap(root: &Path, folder: &str) {
        let dir = root.join(folder);
        let outside = root.with_file_name("outside");
        std::fs::rename(&dir, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
    }

    /// Takes the link back out and the folder back in.
    #[cfg(unix)]
    fn unswap(root: &Path, folder: &str) {
        let dir = root.join(folder);
        std::fs::remove_file(&dir).unwrap();
        std::fs::rename(root.with_file_name("outside"), &dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_folder_swapped_for_a_link_out_of_the_worktree_is_neither_listed_nor_watched() {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical(dir.path()).unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        run_git(&root, &["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "deps/\n").unwrap();
        write(&root, "deps/x.js");
        write(&root, "deps/a/z.js");
        let mut watcher = Watcher::new(&root).unwrap();
        watcher.open(["deps".to_owned(), "deps/a".to_owned()]);
        // What the watcher itself watches: FSEvents keeps what a path resolved to.
        let keys = |watcher: &Watcher| {
            let paths = watcher.watcher.watched_paths().unwrap();
            paths
                .into_iter()
                .map(|(path, _)| path)
                .collect::<BTreeSet<_>>()
        };

        // Swapped after git named it an ignored folder, right before it is read.
        watcher.hook = |step, root, folder| {
            if matches!(step, Step::Read) && folder == "deps" {
                swap(root, folder);
            }
        };
        let listing = watcher.list().unwrap();
        assert_eq!(listing.ignored, ["deps/"]);
        assert_eq!(watched(&watcher), [""]);
        let base = keys(&watcher);
        assert_eq!(base.len(), 2, "the worktree and its git dir: {base:?}");
        assert!(base.iter().all(|key| key.starts_with(&root)), "{base:?}");
        unswap(&root, "deps");

        // Swapped after it was read, right before it is watched: read from the worktree, but
        // the watches made through the link, at the end of the path or in its middle, go.
        watcher.hook = |step, root, folder| {
            if matches!(step, Step::Watch) && folder == "deps" {
                swap(root, folder);
            }
        };
        let listing = watcher.list().unwrap();
        let read = ["deps/", "deps/a/", "deps/a/z.js", "deps/x.js"];
        assert_eq!(listing.ignored, read);
        assert_eq!(watched(&watcher), [""]);
        assert_eq!(keys(&watcher), base);
        // A write out there is not seen.
        changes(&mut watcher, NEVER).await;
        let outside = root.with_file_name("outside");
        std::fs::write(outside.join("y.js"), "").unwrap();
        std::fs::write(outside.join("a/w.js"), "").unwrap();
        assert!(!changes(&mut watcher, NEVER).await);
        unswap(&root, "deps");

        // Left alone, it is read and watched.
        watcher.hook = |_, _, _| {};
        let listing = watcher.list().unwrap();
        let read = [
            "deps/",
            "deps/a/",
            "deps/a/w.js",
            "deps/a/z.js",
            "deps/x.js",
            "deps/y.js",
        ];
        assert_eq!(listing.ignored, read);
        assert_eq!(watched(&watcher), ["", "deps", "deps/a"]);
        let mut all = base.clone();
        all.extend([root.join("deps"), root.join("deps/a")]);
        assert_eq!(keys(&watcher), all);
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
        let mut debounce = Debounce::new(start, QUIET, MAX_DELAY);
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

    #[test]
    fn watches_that_fail_are_named_and_the_others_still_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(name);
        std::fs::create_dir(path("a")).unwrap();
        std::fs::create_dir(path("b")).unwrap();
        // A channel, not a closure: no event comes, and coverage counts a closure never run.
        let (events, _) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(events).unwrap();
        let names = ["x", "a", "y", "b"];
        let ops = names.map(|name| PathOp::watch_non_recursive(path(name)));
        let failed = update(
            |ops| watcher.update_paths(ops).map_err(Box::new),
            ops.into(),
        )
        .unwrap();
        assert_eq!(failed, [path("x"), path("y")]);
        let mut watched: Vec<PathBuf> = watcher
            .watched_paths()
            .unwrap()
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        watched.sort();
        assert_eq!(watched, [path("a"), path("b")]);
    }

    #[test]
    fn a_watcher_that_fails_itself_is_an_error() {
        let broken = |_| {
            Err(Box::new(notify::UpdatePathsError {
                source: notify::Error::generic("no stream"),
                origin: None,
                remaining: Vec::new(),
            }))
        };
        let ops = vec![PathOp::watch_non_recursive("a")];
        let err = update(broken, ops).unwrap_err();
        assert!(matches!(err.kind, notify::ErrorKind::Generic(why) if why == "no stream"));
    }

    /// A new repository in a temporary directory; the tests' own git never reads the user's config.
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical(dir.path()).unwrap();
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
        watcher.dirs.iter().map(String::as_str).collect()
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
        // Listed apart, as one closed folder.
        assert_eq!(listing.ignored, ["ignored/"]);
        assert_eq!(watched(&watcher), ["", "empty", "src"]);
        // FSEvents (macOS) may still report the writes made just before the watch began.
        changes(&mut watcher, NEVER).await;
        // Nothing happens, and nothing in an ignored tree counts.
        assert!(!changes(&mut watcher, NEVER).await);
        write(&root, "ignored/deep/y.txt");
        assert!(!changes(&mut watcher, NEVER).await);
        // Nor does the git dir, apart from HEAD and the index.
        write(&root, ".git/other");
        assert!(!changes(&mut watcher, NEVER).await);
        // Nor a folder's own change (Windows reports one for the write in the ignored tree
        // above; Unix one for its mode), as told, since FSEvents coalesces it with the folder's
        // creation; a file's is.
        let modified = |kind, path: &str| {
            let event = Event::new(EventKind::Modify(kind)).add_path(root.join(path));
            Ok(event)
        };
        let metadata = ModifyKind::Metadata(notify::event::MetadataKind::Any);
        assert!(!watcher.saw(modified(ModifyKind::Any, "src")));
        assert!(!watcher.saw(modified(metadata, "src")));
        assert!(watcher.saw(modified(ModifyKind::Any, "src/a.rs")));
        // A new folder is.
        std::fs::create_dir(root.join("fresh")).unwrap();
        assert!(changes(&mut watcher, SOON).await);
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
    async fn a_directory_that_becomes_ignored_loses_its_watch() {
        let (_dir, root) = repo();
        write(&root, "d/a.txt");
        let mut watcher = Watcher::new(&root).unwrap();
        watcher.list().unwrap();
        assert_eq!(watched(&watcher), ["", "d"]);
        std::fs::write(root.join(".gitignore"), "d/\n").unwrap();
        assert!(changes(&mut watcher, SOON).await);
        assert_eq!(names(&watcher.list().unwrap()), [".gitignore"]);
        assert_eq!(watched(&watcher), [""]);
        write(&root, "d/b.txt");
        assert!(!changes(&mut watcher, NEVER).await);
    }

    #[tokio::test]
    async fn an_open_ignored_folder_is_listed_one_level_deep_and_watched() {
        let (_dir, root) = repo();
        std::fs::write(root.join(".gitignore"), ".env\nnode_modules/\n").unwrap();
        write(&root, ".env");
        write(&root, "node_modules/pkg/index.js");
        write(&root, "node_modules/top.js");
        let mut watcher = Watcher::new(&root).unwrap();
        let listing = watcher.list().unwrap();
        assert_eq!(names(&listing), [".gitignore"]);
        assert_eq!(listing.ignored, [".env", "node_modules/"]);
        // Closed: no watch, and nothing in it counts.
        assert_eq!(watched(&watcher), [""]);
        changes(&mut watcher, NEVER).await;
        write(&root, "node_modules/later.js");
        assert!(!changes(&mut watcher, NEVER).await);

        // A folder inside counts only while the one holding it is open too; an unknown one,
        // or one not ignored, never.
        let open = |watcher: &mut Watcher, folders: &[&str]| {
            watcher.open(folders.iter().map(|f| (*f).to_owned()));
            watcher.list().unwrap()
        };
        let listing = open(&mut watcher, &["node_modules/pkg", "src", "../x"]);
        assert_eq!(listing.ignored, [".env", "node_modules/"]);
        assert_eq!(watched(&watcher), [""]);
        let listing = open(&mut watcher, &["node_modules"]);
        let level = [".env", "node_modules/", "node_modules/later.js"];
        let level = [&level[..], &["node_modules/pkg/", "node_modules/top.js"]].concat();
        assert_eq!(listing.ignored, level);
        assert_eq!(watched(&watcher), ["", "node_modules"]);
        let listing = open(&mut watcher, &["node_modules", "node_modules/pkg"]);
        let deeper = ["node_modules/pkg/", "node_modules/pkg/index.js"];
        assert!(
            deeper
                .iter()
                .all(|path| listing.ignored.contains(&(*path).to_owned()))
        );
        assert_eq!(watched(&watcher), ["", "node_modules", "node_modules/pkg"]);
        assert!(!listing.truncated);
        // Open, a change in it counts.
        write(&root, "node_modules/pkg/more.js");
        assert!(changes(&mut watcher, SOON).await);
        let listing = watcher.list().unwrap();
        assert!(
            listing
                .ignored
                .contains(&"node_modules/pkg/more.js".to_owned())
        );

        // Closing it drops its watch and what it held.
        let listing = open(&mut watcher, &["node_modules/pkg"]);
        assert_eq!(listing.ignored, [".env", "node_modules/"]);
        assert_eq!(watched(&watcher), [""]);
        changes(&mut watcher, NEVER).await;
        write(&root, "node_modules/pkg/last.js");
        assert!(!changes(&mut watcher, NEVER).await);

        // A file that becomes ignored, or stops being ignored, moves between the lists.
        write(&root, "notes.txt");
        std::fs::write(root.join(".gitignore"), "node_modules/\nnotes.txt\n").unwrap();
        assert!(changes(&mut watcher, SOON).await);
        let listing = watcher.list().unwrap();
        assert_eq!(names(&listing), [".env", ".gitignore"]);
        assert_eq!(listing.ignored, ["node_modules/", "notes.txt"]);

        // An open folder that cannot be read is neither listed nor watched.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let modules = root.join("node_modules");
            let mode = |mode| std::fs::set_permissions(&modules, PermissionsExt::from_mode(mode));
            mode(0o000).unwrap();
            let listing = open(&mut watcher, &["node_modules"]);
            mode(0o755).unwrap();
            assert_eq!(listing.ignored, ["node_modules/", "notes.txt"]);
            assert_eq!(watched(&watcher), [""]);
        }
    }

    #[tokio::test]
    async fn the_caps_hold_for_a_huge_ignored_folder() {
        let (_dir, root) = repo();
        // Ignored without a `.gitignore`: nothing else is listed.
        std::fs::write(root.join(".git/info/exclude"), "huge/\n").unwrap();
        let huge = root.join("huge");
        std::fs::create_dir_all(huge.join("a")).unwrap();
        std::fs::write(huge.join("a/x"), "").unwrap();
        // With "huge/" and "a/": one short of the cap.
        for i in 0..MAX_FILES - 2 {
            std::fs::write(huge.join(format!("f{i:06}")), "").unwrap();
        }
        let mut watcher = Watcher::new(&root).unwrap();
        watcher.open(["huge".to_owned(), "huge/a".to_owned()]);
        let listing = watcher.list().unwrap();
        assert!(listing.files.is_empty());
        // Room for "huge/a" and its file: the last of the others is left out.
        assert_eq!(listing.ignored.len(), MAX_FILES);
        assert_eq!(listing.ignored[..3], ["huge/", "huge/a/", "huge/a/x"]);
        let last = format!("huge/f{:06}", MAX_FILES - 4);
        assert_eq!(listing.ignored.last(), Some(&last));
        assert!(listing.truncated);
        assert_eq!(watched(&watcher), ["", "huge", "huge/a"]);
        // Past the cap, a folder inside is not listed at all, nor watched.
        for i in MAX_FILES - 2..MAX_FILES {
            std::fs::write(huge.join(format!("f{i:06}")), "").unwrap();
        }
        let listing = watcher.list().unwrap();
        assert_eq!(listing.ignored.len(), MAX_FILES);
        assert_eq!(listing.ignored[..3], ["huge/", "huge/a/", "huge/f000000"]);
        let last = format!("huge/f{:06}", MAX_FILES - 3);
        assert_eq!(listing.ignored.last(), Some(&last));
        assert!(listing.truncated);
        assert_eq!(watched(&watcher), ["", "huge"]);
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
