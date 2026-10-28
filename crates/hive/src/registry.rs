//! Git's worktree registry of the current space's projects, watched with `notify` (9.36), so
//! the app's worktrees follow `git worktree add`, `remove` and `prune` run from anywhere, not
//! only Hive's own requests and Claude Code's worktree hooks.
//!
//! Two directories per repository, each watched non-recursively (object writes would flood the
//! watch; on macOS notify keeps a non-recursive watch's events to it over a recursive FSEvents
//! stream, so FSEvents still sees them, coalesced by [`files::watch_config`]'s latency):
//! the common git dir, where a `worktrees/` folder appears or goes, and that `worktrees/`,
//! where each linked worktree has an entry. A worktree folder deleted by hand leaves the
//! registry as it was: it shows as gone on the next list (git marks it prunable).
//!
//! Each group's folder (14.1) is watched once, recursively (D19.2): an entry made, removed or
//! renamed in it, or a `.git` in one of its folders, may be a repository joining or leaving the
//! group. Only those events and the registries' leave the watcher's thread ([`wanted`]), so a
//! build in a repository of the group does not fill the queue. On Linux, where inotify adds a
//! watch per folder of a recursive one (every `node_modules`, every `.git/objects/xx`), the
//! group's folder and the folders directly in it are watched non-recursively instead.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{Event, EventKind, PathOp, RecommendedWatcher, Watcher as _};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::files::{self, Debounce};
use crate::git;
use crate::paths::canonical;

/// A change is reported once events have stopped for this long (a burst of `git worktree
/// add`s or a prune is one change)…
pub const QUIET: Duration = Duration::from_millis(300);
/// …but never later than this after the first one.
pub const MAX_DELAY: Duration = Duration::from_secs(1);

/// Events waiting to be seen; more are dropped, as a burst is already pending.
const EVENT_QUEUE: usize = 256;

/// The name of git's registry of linked worktrees, inside the common git dir.
const WORKTREES: &str = "worktrees";

pub struct Registry {
    watcher: RecommendedWatcher,
    events: mpsc::Receiver<notify::Result<Event>>,
    /// Watched directories: common git dirs and their `worktrees/`, canonical, and groups'
    /// folders.
    watched: BTreeSet<PathBuf>,
    /// The groups' folders followed (14.1), where a repository may appear or go; shared with
    /// the watcher's thread, which drops the events deeper in them.
    groups: Arc<Mutex<BTreeSet<PathBuf>>>,
    /// The burst being coalesced; kept when a wait for [`Registry::changed`] is dropped.
    burst: Option<Debounce>,
}

impl Registry {
    pub fn new() -> io::Result<Self> {
        let (tx, events) = mpsc::channel(EVENT_QUEUE);
        let groups = Arc::new(Mutex::new(BTreeSet::new()));
        let shared = Arc::clone(&groups);
        let handler = move |event: notify::Result<Event>| {
            if wanted(&shared, &event) {
                // Full: a burst is already waiting to be seen.
                let _ = tx.try_send(event);
            }
        };
        let watcher =
            RecommendedWatcher::new(handler, files::watch_config()).map_err(io::Error::other)?;
        Ok(Self {
            watcher,
            events,
            watched: BTreeSet::new(),
            groups,
            burst: None,
        })
    }

    /// Watches the registries of the repositories at `roots` and the folders of `groups` (14.1),
    /// and no others. A folder whose git dir cannot be read is skipped. Blocks on git.
    pub fn follow(&mut self, roots: &[String], groups: &[String]) {
        let mut wanted = BTreeSet::new();
        for common in roots.iter().filter_map(|root| common_dir(Path::new(root))) {
            let worktrees = common.join(WORKTREES);
            if worktrees.is_dir() {
                wanted.insert(worktrees);
            }
            wanted.insert(common);
        }
        let groups: BTreeSet<PathBuf> = groups.iter().map(PathBuf::from).collect();
        let recursive: BTreeSet<PathBuf> =
            groups.iter().filter(|dir| dir.is_dir()).cloned().collect();
        wanted.extend(recursive.iter().cloned());
        // Linux: the folders directly in each group, and nothing recursively (module's doc).
        #[cfg(target_os = "linux")]
        wanted.extend(groups.iter().flat_map(|g| subfolders(g)));
        *self.groups.lock().unwrap_or_else(PoisonError::into_inner) = groups;
        let gone = self.watched.iter().filter(|dir| !wanted.contains(*dir));
        // Unwatching fails when the directory is gone, which removed its watch already.
        let unwatch = gone.map(PathOp::unwatch);
        let added: Vec<PathBuf> = wanted.difference(&self.watched).cloned().collect();
        #[cfg(not(target_os = "linux"))]
        let watch = added.iter().map(|dir| match recursive.contains(dir) {
            true => PathOp::watch_recursive(dir),
            false => PathOp::watch_non_recursive(dir),
        });
        #[cfg(target_os = "linux")]
        let watch = added.iter().map(PathOp::watch_non_recursive);
        // ponytail: a watcher that fails (FSEvents only: its stream does not restart) leaves
        // the registry unwatched until the next follow; report it if that ever shows.
        let watcher = &mut self.watcher;
        let ops = unwatch.chain(watch).collect();
        let failed = files::update(|ops| watcher.update_paths(ops).map_err(Box::new), ops);
        let failed = failed.unwrap_or_default();
        self.watched.retain(|dir| wanted.contains(dir));
        let watched = added.into_iter().filter(|dir| !failed.contains(dir));
        self.watched.extend(watched);
    }

    /// Waits until a registry changed and the burst of events is over. Once the watcher is
    /// gone, it waits forever.
    pub async fn changed(&mut self) {
        // Every wait is in this one loop, so no helper can return without waiting.
        loop {
            let deadline = self.burst.map(|burst| burst.deadline());
            let settled = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now));
            tokio::select! {
                event = self.events.recv() => {
                    // No event ever again: the watcher's thread is gone.
                    let Some(event) = event else { return std::future::pending().await };
                    if self.saw(event) {
                        let now = Instant::now();
                        let debounce = Debounce::new(now, QUIET, MAX_DELAY);
                        self.burst.get_or_insert(debounce).event(now);
                    }
                }
                () = settled, if deadline.is_some() => {
                    self.burst = None;
                    return;
                }
            }
        }
    }

    /// Whether `event` may have changed a registry.
    fn saw(&mut self, event: notify::Result<Event>) -> bool {
        // A watcher error may have lost events: listing again is safe.
        let Ok(event) = event else { return true };
        if matches!(
            event.kind,
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        ) {
            // A `worktrees/` gone lost its watch; the next follow watches it again.
            for path in &event.paths {
                self.watched.remove(path);
            }
        }
        // A group's entry made, removed or renamed (a repository may come or go), the group's
        // folder itself, or a `.git` in a folder of it; not a file written.
        let entries =
            !matches!(event.kind, EventKind::Modify(m) if !matches!(m, ModifyKind::Name(_)));
        let groups = self.groups.lock().unwrap_or_else(PoisonError::into_inner);
        let group = |path: &PathBuf| entries && in_group(&groups, path) == Some(true);
        // No path: an overflow, which calls for listing again.
        let mut paths = event.paths.iter();
        event.paths.is_empty() || paths.any(|path| self.counts(path) || group(path))
    }

    /// Whether a change to `path` counts: `worktrees/` in a common git dir, or an entry in it.
    /// The rest of the git dir (`index`, `HEAD`, locks…) changes all the time.
    fn counts(&self, path: &Path) -> bool {
        let Some(dir) = path.parent() else {
            return false;
        };
        let name = OsStr::new(WORKTREES);
        self.watched.contains(dir)
            && (dir.file_name() == Some(name) || path.file_name() == Some(name))
    }
}

/// Whether the watcher's thread passes `event` on: not an access, and not only paths deep in a
/// group's folder, where a repository's files and git objects churn; the registries' paths
/// (`worktrees/` and its entries) pass wherever they are.
fn wanted(groups: &Mutex<BTreeSet<PathBuf>>, event: &notify::Result<Event>) -> bool {
    let Ok(event) = event else { return true };
    if event.kind.is_access() {
        return false;
    }
    let groups = groups.lock().unwrap_or_else(PoisonError::into_inner);
    let registry = |path: &Path| {
        let name = Some(OsStr::new(WORKTREES));
        path.file_name() == name || path.parent().and_then(Path::file_name) == name
    };
    let kept = |path: &PathBuf| in_group(&groups, path) != Some(false) || registry(path);
    event.paths.is_empty() || event.paths.iter().any(kept)
}

/// Whether `path`, inside the nearest of `groups`, is where a repository's coming or going
/// shows: the group's folder, an entry in it, or the `.git` of one; `None` outside every group.
fn in_group(groups: &BTreeSet<PathBuf>, path: &Path) -> Option<bool> {
    let depth = path.ancestors().position(|dir| groups.contains(dir))?;
    Some(depth <= 1 || depth == 2 && path.file_name() == Some(OsStr::new(".git")))
}

/// The folders (not links) directly in the group folder `group`, as many as a group's
/// repositories are looked for among ([`crate::projects::GROUP_ENTRIES`]).
#[cfg(target_os = "linux")]
fn subfolders(group: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(group) else {
        return Vec::new();
    };
    let entries = entries.take(crate::projects::GROUP_ENTRIES).flatten();
    let folders = entries.filter(|e| e.file_type().is_ok_and(|t| t.is_dir()));
    folders.map(|e| e.path()).collect()
}

/// The common git dir of the repository at `root` (the main worktree's `.git`, even from a
/// linked worktree), canonical; `None` when git cannot tell, or hangs past
/// [`git::TIME_LIMIT`] (a hung git would stop the watch, and the service from exiting).
fn common_dir(root: &Path) -> Option<PathBuf> {
    let args = ["rev-parse", "--git-common-dir"];
    let out = git::output_within(root, &args, &[0], git::TIME_LIMIT).ok()?;
    let dir = String::from_utf8_lossy(&out);
    // Relative to `root` unless git gives an absolute path.
    canonical(&root.join(dir.trim_end_matches('\n'))).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, RemoveKind};

    /// A repository at `<temp>/repo`, so its worktrees can go next to it (`../wt`).
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical(dir.path()).unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        run_git(&root, &["init", "-q"]);
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "init"]);
        (dir, root)
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn event(kind: EventKind, paths: &[&Path]) -> notify::Result<Event> {
        let paths = paths.iter().map(|p| p.to_path_buf()).collect();
        Ok(Event {
            kind,
            paths,
            attrs: Default::default(),
        })
    }

    fn roots(dirs: &[&Path]) -> Vec<String> {
        dirs.iter().map(|d| d.display().to_string()).collect()
    }

    #[tokio::test]
    async fn only_the_registries_of_the_followed_repositories_are_watched() {
        let (_a, a) = repo();
        let (_b, b) = repo();
        let plain = tempfile::tempdir().unwrap();
        let mut registry = Registry::new().unwrap();
        // A folder outside git is skipped; the others are still watched.
        registry.follow(&roots(&[plain.path(), &a, &b]), &[]);
        let git = |root: &Path| root.join(".git");
        assert_eq!(registry.watched, BTreeSet::from([git(&a), git(&b)]));

        // The first linked worktree makes `worktrees/`, watched from the next follow on;
        // from a linked worktree, the registry is the main one's.
        run_git(&a, &["worktree", "add", "-q", "../wt-a"]);
        registry.follow(&roots(&[&a, &a.join("../wt-a")]), &[]);
        let expected = BTreeSet::from([git(&a), git(&a).join(WORKTREES)]);
        assert_eq!(registry.watched, expected);

        // A project no longer followed is no longer watched.
        registry.follow(&roots(&[&b]), &[]);
        assert_eq!(registry.watched, BTreeSet::from([git(&b)]));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_repository_whose_git_hangs_is_skipped_once_git_is_killed() {
        let (_a, a) = repo();
        let (_b, b) = repo();
        // Git reads `b`'s config, which includes a pipe nobody writes to (a hung drive).
        let git_dir = b.join(".git");
        nix::unistd::mkfifo(&git_dir.join("hang"), nix::sys::stat::Mode::S_IRWXU).unwrap();
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(git_dir.join("config"))
            .unwrap();
        std::io::Write::write_all(&mut config, b"[include]\n\tpath = hang\n").unwrap();
        let mut registry = Registry::new().unwrap();
        let started = std::time::Instant::now();
        registry.follow(&roots(&[&b, &a]), &[]);
        assert!(started.elapsed() < git::TIME_LIMIT * 2, "not killed");
        assert_eq!(registry.watched, BTreeSet::from([a.join(".git")]));
    }

    #[tokio::test]
    async fn only_changes_to_the_registry_count() {
        let (_a, a) = repo();
        let mut registry = Registry::new().unwrap();
        run_git(&a, &["worktree", "add", "-q", "../wt"]);
        registry.follow(&roots(&[&a]), &[]);
        let common = a.join(".git");
        let worktrees = common.join(WORKTREES);
        let create = EventKind::Create(CreateKind::Any);
        for (path, counts) in [
            (worktrees.as_path(), true),
            (&worktrees.join("wt"), true),
            (&common.join("index"), false),
            (&common.join("HEAD"), false),
            (&worktrees.join("wt/HEAD"), false),
            (&a.join("worktrees"), false),
            (Path::new("/"), false),
        ] {
            assert_eq!(registry.saw(event(create, &[path])), counts, "{path:?}");
        }
        // An overflow or an error may have lost events.
        assert!(registry.saw(event(EventKind::Other, &[])));
        assert!(registry.saw(Err(notify::Error::generic("lost"))));

        // A `worktrees/` removed lost its watch, and is watched again once it is back.
        let removed = EventKind::Remove(RemoveKind::Folder);
        assert!(registry.saw(event(removed, &[&worktrees])));
        assert_eq!(registry.watched, BTreeSet::from([common.clone()]));
        registry.follow(&roots(&[&a]), &[]);
        assert!(registry.watched.contains(&worktrees));
        // Other changes leave the watches alone.
        registry.saw(event(create, &[&worktrees]));
        assert!(registry.watched.contains(&worktrees));
    }

    #[tokio::test]
    async fn a_groups_entries_count_but_not_its_files_written() {
        let (_a, a) = repo();
        let tmp = tempfile::tempdir().unwrap();
        let group = canonical(tmp.path()).unwrap();
        let mut registry = Registry::new().unwrap();
        let entry = group.join("api");
        std::fs::create_dir(&entry).unwrap();
        std::fs::write(group.join("notes.txt"), "").unwrap();
        let groups = roots(&[&group, &group.join("gone")]);
        registry.follow(&roots(&[&a]), &groups);
        // A missing group folder is not watched; one is, once (on Linux the folders in it too,
        // not its files).
        #[cfg(target_os = "linux")]
        let expected = BTreeSet::from([a.join(".git"), group.clone(), entry.clone()]);
        #[cfg(not(target_os = "linux"))]
        let expected = BTreeSet::from([a.join(".git"), group.clone()]);
        assert_eq!(registry.watched, expected);
        use notify::event::{DataChange, RenameMode};
        let written = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        let renamed = EventKind::Modify(ModifyKind::Name(RenameMode::Any));
        let made = EventKind::Create(CreateKind::Folder);
        let removed = EventKind::Remove(RemoveKind::Folder);
        for (kind, path, counts) in [
            // A `git init` in a folder already there, or its `.git` removed.
            (made, &entry.join(".git"), true),
            (removed, &entry.join(".git"), true),
            (written, &entry.join(".git"), false),
            (made, &entry.join("package.json"), false),
            (made, &group.join(".git"), true),
            (made, &a.join(".git"), false),
            (made, &entry, true),
            (removed, &entry, true),
            (renamed, &entry, true),
            (written, &group.join("notes.txt"), false),
            (made, &entry.join("src"), false),
            (made, &a.join("api"), false),
            // The folder itself.
            (written, &group, false),
            (removed, &group, true),
        ] {
            assert_eq!(
                registry.saw(event(kind, &[path])),
                counts,
                "{kind:?} {path:?}"
            );
        }
        // A group no longer followed is no longer watched.
        registry.follow(&roots(&[&a]), &[]);
        assert_eq!(registry.watched, BTreeSet::from([a.join(".git")]));
        assert!(!registry.saw(event(made, &[&entry])));
    }

    #[test]
    fn deep_events_in_a_group_never_leave_the_watchers_thread() {
        let group = PathBuf::from("/g");
        let inner = group.join("repo/vendor");
        let groups = Mutex::new(BTreeSet::from([group.clone(), inner.clone()]));
        let made = EventKind::Create(CreateKind::Any);
        for (path, kept) in [
            (group.clone(), true),
            (group.join("repo"), true),
            (group.join("repo/.git"), true),
            (group.join("repo/src"), false),
            (group.join("repo/.git/objects/ab"), false),
            (group.join("repo/a/.git"), false),
            // The registries pass wherever they are.
            (group.join("repo/.git/worktrees"), true),
            (group.join("repo/.git/worktrees/wt"), true),
            (group.join("repo/.git/worktrees/wt/HEAD"), false),
            // The nearest group counts: a group inside another one.
            (inner.join("lib"), true),
            (inner.join("lib/.git"), true),
            (inner.join("lib/src"), false),
            // Outside every group (another watch's).
            (PathBuf::from("/other/a/b/c"), true),
        ] {
            assert_eq!(wanted(&groups, &event(made, &[&path])), kept, "{path:?}");
        }
        // One path enough to keep it; no path, or an error, may be lost events.
        let src = group.join("repo/src");
        assert!(wanted(&groups, &event(made, &[&src, &group])));
        assert!(wanted(&groups, &event(EventKind::Other, &[])));
        assert!(wanted(&groups, &Err(notify::Error::generic("lost"))));
        let read = EventKind::Access(notify::event::AccessKind::Any);
        assert!(!wanted(&groups, &event(read, &[&group])));
    }

    /// `count` repositories in a new group folder.
    fn group_of(count: usize) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let group = canonical(tmp.path()).unwrap();
        for i in 0..count {
            std::fs::create_dir_all(group.join(format!("r{i}/.git"))).unwrap();
        }
        (tmp, group)
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn a_group_is_one_watch_however_many_repositories_it_holds() {
        for count in [1, 50] {
            let (_tmp, group) = group_of(count);
            let mut registry = Registry::new().unwrap();
            registry.follow(&[], &roots(&[&group]));
            assert_eq!(registry.watched, BTreeSet::from([group]));
        }
    }

    /// Waits for a change, at most [`NEVER`]: whether one came.
    async fn changes(registry: &mut Registry) -> bool {
        tokio::time::timeout(NEVER, registry.changed())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn a_repository_made_or_removed_in_a_group_is_seen_but_not_its_files() {
        let (_tmp, group) = group_of(2);
        let mut registry = Registry::new().unwrap();
        registry.follow(&[], &roots(&[&group]));
        // Let FSEvents' stream start before the first change.
        while changes(&mut registry).await {}
        // Files written deep in a repository of the group.
        let src = group.join("r0/src");
        std::fs::create_dir(&src).unwrap();
        for i in 0..50 {
            std::fs::write(src.join(format!("{i}.rs")), "").unwrap();
        }
        std::fs::write(group.join("r0/.git/index"), "").unwrap();
        assert!(!changes(&mut registry).await, "files written");
        // A `git init` in a folder already there, and a `.git` removed.
        std::fs::create_dir(group.join("r1/sub")).unwrap();
        assert!(!changes(&mut registry).await, "a folder in a repository");
        std::fs::remove_dir(group.join("r1/.git")).unwrap();
        assert!(changes(&mut registry).await, ".git removed");
        std::fs::create_dir(group.join("r1/.git")).unwrap();
        assert!(changes(&mut registry).await, ".git made");
        // A repository moved in, and removed.
        std::fs::create_dir_all(group.join("new/.git")).unwrap();
        assert!(changes(&mut registry).await, "repository made");
        std::fs::remove_dir_all(group.join("new")).unwrap();
        assert!(changes(&mut registry).await, "repository removed");
    }

    #[test]
    fn a_burst_is_one_change_after_a_quiet_spell_or_the_longest_delay() {
        let start = Instant::now();
        let mut burst = Debounce::new(start, QUIET, MAX_DELAY);
        assert_eq!(burst.deadline(), start + Duration::from_millis(300));
        burst.event(start + Duration::from_millis(200));
        assert_eq!(burst.deadline(), start + Duration::from_millis(500));
        // Worktrees keep coming: the first event bounds the wait.
        burst.event(start + Duration::from_millis(900));
        assert_eq!(burst.deadline(), start + Duration::from_secs(1));
    }

    /// Longer than a burst takes to settle.
    const NEVER: Duration = Duration::from_millis(700);

    #[tokio::test]
    async fn a_burst_is_reported_once_it_settles_even_after_an_interrupted_wait() {
        let (_a, a) = repo();
        let mut registry = Registry::new().unwrap();
        registry.follow(&roots(&[&a]), &[]);
        let common = a.join(".git");
        let (tx, events) = mpsc::channel(8);
        registry.events = events;
        let made = EventKind::Create(CreateKind::Folder);
        tx.send(event(made, &[&common.join(WORKTREES)]))
            .await
            .unwrap();
        // A wait dropped before the burst settles keeps the burst: the next wait reports it
        // without another event.
        let early = tokio::time::timeout(QUIET / 3, registry.changed()).await;
        assert!(early.is_err());
        let started = Instant::now();
        let settled = tokio::time::timeout(NEVER, registry.changed()).await;
        assert!(settled.is_ok());
        assert!(started.elapsed() < QUIET, "{:?}", started.elapsed());
        // Nothing else happened: no change is reported.
        let later = tokio::time::timeout(NEVER, registry.changed()).await;
        assert!(later.is_err());
        // Events outside the registry never start a burst.
        let index = event(EventKind::Any, &[&common.join("index")]);
        tx.send(index).await.unwrap();
        let later = tokio::time::timeout(NEVER, registry.changed()).await;
        assert!(later.is_err());
        // The watcher gone: nothing is reported, ever.
        drop(tx);
        let later = tokio::time::timeout(NEVER, registry.changed()).await;
        assert!(later.is_err());
    }
}
