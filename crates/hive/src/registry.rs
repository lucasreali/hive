//! Git's worktree registry of the current space's projects, watched with `notify` (9.36), so
//! the app's worktrees follow `git worktree add`, `remove` and `prune` run from anywhere, not
//! only Hive's own requests and Claude Code's worktree hooks.
//!
//! Two directories per repository, never recursively (object writes would flood the watch):
//! the common git dir, where a `worktrees/` folder appears or goes, and that `worktrees/`,
//! where each linked worktree has an entry. A worktree folder deleted by hand leaves the
//! registry as it was: it shows as gone on the next list (git marks it prunable).

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::files::Debounce;
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
    /// Watched directories: common git dirs and their `worktrees/`, canonical.
    watched: BTreeSet<PathBuf>,
    /// The burst being coalesced; kept when a wait for [`Registry::changed`] is dropped.
    burst: Option<Debounce>,
}

impl Registry {
    pub fn new() -> io::Result<Self> {
        let (tx, events) = mpsc::channel(EVENT_QUEUE);
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if !matches!(&event, Ok(event) if event.kind.is_access()) {
                // Full: a burst is already waiting to be seen.
                let _ = tx.try_send(event);
            }
        })
        .map_err(io::Error::other)?;
        Ok(Self {
            watcher,
            events,
            watched: BTreeSet::new(),
            burst: None,
        })
    }

    /// Watches the registries of the repositories at `roots` and no others. A folder whose git
    /// dir cannot be read is skipped. Blocks on git.
    pub fn follow(&mut self, roots: &[String]) {
        let mut wanted = BTreeSet::new();
        for common in roots.iter().filter_map(|root| common_dir(Path::new(root))) {
            let worktrees = common.join(WORKTREES);
            if worktrees.is_dir() {
                wanted.insert(worktrees);
            }
            wanted.insert(common);
        }
        let mut paths = self.watcher.paths_mut();
        self.watched.retain(|dir| {
            let keep = wanted.contains(dir);
            if !keep {
                // Fails when the directory is gone, which removed its watch already.
                let _ = paths.remove(dir);
            }
            keep
        });
        for dir in wanted {
            if !self.watched.contains(&dir) && paths.add(&dir, RecursiveMode::NonRecursive).is_ok()
            {
                self.watched.insert(dir);
            }
        }
        // ponytail: a failed commit (FSEvents only; inotify watches at `add`) leaves the
        // registry unwatched until the next follow; report it if that ever shows.
        let _ = paths.commit();
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
        // No path: an overflow, which calls for listing again.
        event.paths.is_empty() || event.paths.iter().any(|path| self.counts(path))
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

/// The common git dir of the repository at `root` (the main worktree's `.git`, even from a
/// linked worktree), canonical; `None` when git cannot tell.
fn common_dir(root: &Path) -> Option<PathBuf> {
    let out = git::output(root, &["rev-parse", "--git-common-dir"], &[0]).ok()?;
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
        registry.follow(&roots(&[plain.path(), &a, &b]));
        let git = |root: &Path| root.join(".git");
        assert_eq!(registry.watched, BTreeSet::from([git(&a), git(&b)]));

        // The first linked worktree makes `worktrees/`, watched from the next follow on;
        // from a linked worktree, the registry is the main one's.
        run_git(&a, &["worktree", "add", "-q", "../wt-a"]);
        registry.follow(&roots(&[&a, &a.join("../wt-a")]));
        let expected = BTreeSet::from([git(&a), git(&a).join(WORKTREES)]);
        assert_eq!(registry.watched, expected);

        // A project no longer followed is no longer watched.
        registry.follow(&roots(&[&b]));
        assert_eq!(registry.watched, BTreeSet::from([git(&b)]));
    }

    #[tokio::test]
    async fn only_changes_to_the_registry_count() {
        let (_a, a) = repo();
        let mut registry = Registry::new().unwrap();
        run_git(&a, &["worktree", "add", "-q", "../wt"]);
        registry.follow(&roots(&[&a]));
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
        registry.follow(&roots(&[&a]));
        assert!(registry.watched.contains(&worktrees));
        // Other changes leave the watches alone.
        registry.saw(event(create, &[&worktrees]));
        assert!(registry.watched.contains(&worktrees));
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
        registry.follow(&roots(&[&a]));
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
