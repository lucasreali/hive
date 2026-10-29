//! Claude's session logs and its records of running `claude`s, watched with `notify` (D19.4):
//! the Sessions list is sent again when they change, instead of the app asking every 5 s.
//!
//! Each account's projects folder is watched (a log written or removed in it), and the folder
//! of its records beside it non-recursively (a `claude` that starts or exits). Only a project
//! folder's own entries count, and only for the followed projects: a subagent's log or another
//! project's does not. The projects folder is watched recursively on macOS (one FSEvents
//! stream); on Linux, where inotify adds a watch per folder of a recursive one (every
//! session's subagents and tool results), it and each followed project's log folder are
//! watched non-recursively instead.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{
    Event, EventKind, PathOp, RecommendedWatcher, RecursiveMode, UpdatePathsError, WatchPathConfig,
    Watcher as _,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::files::{self, Debounce};
use crate::paths::canonical;
use crate::sessions::normalized;

/// The list is read again once events have stopped for this long…
pub const QUIET: Duration = Duration::from_millis(500);
/// …but never later than this after the first one: a streaming agent's log updates the list
/// at most this often.
pub const MAX_DELAY: Duration = Duration::from_secs(2);

/// Events waiting to be seen; more are dropped, as a burst is already pending.
const EVENT_QUEUE: usize = 256;

/// Most entries of a projects folder looked at for the followed projects' log folders (Linux).
#[cfg(target_os = "linux")]
const LOG_FOLDERS: usize = 4096;

/// What a watched folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An account's projects folder: its log folders' entries count.
    Projects,
    /// A followed project's log folder in it (Linux; its entries count through `Projects`).
    #[cfg(target_os = "linux")]
    Logs,
    /// A folder of Claude's records of running `claude`s: any entry counts.
    Records,
}

impl Kind {
    fn mode(self) -> RecursiveMode {
        // Linux: nothing recursively (the module's doc).
        if cfg!(not(target_os = "linux")) && self == Kind::Projects {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        }
    }
}

type Apply<'a> = dyn FnMut(Vec<PathOp>) -> Result<(), Box<UpdatePathsError>> + 'a;

pub struct Watch {
    watcher: RecommendedWatcher,
    events: mpsc::Receiver<notify::Result<Event>>,
    /// Watched folders, canonical.
    watched: BTreeMap<PathBuf, Kind>,
    /// The burst being coalesced; kept when a wait for [`Watch::changed`] is dropped.
    burst: Option<Debounce>,
}

impl Watch {
    pub fn new() -> io::Result<Self> {
        let (tx, events) = mpsc::channel(EVENT_QUEUE);
        let handler = move |event: notify::Result<Event>| {
            if !matches!(&event, Ok(event) if event.kind.is_access()) {
                // Full: a burst is already waiting to be seen.
                let _ = tx.try_send(event);
            }
        };
        let watcher =
            RecommendedWatcher::new(handler, files::watch_config()).map_err(io::Error::other)?;
        Ok(Self {
            watcher,
            events,
            watched: BTreeMap::new(),
            burst: None,
        })
    }

    /// Watches the projects folders `roots` (on Linux with the log folders in them of the
    /// projects at `folders`) and the records folders `records` that exist, and no others.
    pub fn follow(&mut self, roots: &[PathBuf], records: &[PathBuf], folders: &[String]) {
        let watcher = &mut self.watcher;
        let mut apply = |ops| watcher.update_paths(ops).map_err(Box::new);
        follow(&mut self.watched, roots, records, folders, &mut apply);
    }

    /// Counts as a change, coalesced with the burst (an agent detected or ended).
    pub fn poke(&mut self) {
        let now = Instant::now();
        let debounce = Debounce::new(now, QUIET, MAX_DELAY);
        self.burst.get_or_insert(debounce).event(now);
    }

    /// Waits until the logs of the projects at `folders` (asked for at each event: the
    /// followed projects may change meanwhile) or a record changed, or [`Watch::poke`] was
    /// called, and the burst is over. Once the watcher is gone, it waits forever.
    pub async fn changed(&mut self, folders: impl Fn() -> Vec<String>) {
        // Every wait is in this one loop, so no helper can return without waiting.
        loop {
            let deadline = self.burst.map(|burst| burst.deadline());
            let settled = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now));
            tokio::select! {
                event = self.events.recv() => {
                    // No event ever again: the watcher's thread is gone.
                    let Some(event) = event else { return std::future::pending().await };
                    if self.saw(event, &folders) {
                        self.poke();
                    }
                }
                () = settled, if deadline.is_some() => {
                    self.burst = None;
                    return;
                }
            }
        }
    }

    /// Whether `event` may have changed the list of the projects at `folders`.
    fn saw(&mut self, event: notify::Result<Event>, folders: &impl Fn() -> Vec<String>) -> bool {
        // A watcher error may have lost events: listing again is safe.
        let Ok(event) = event else { return true };
        if matches!(
            event.kind,
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        ) {
            // A watched folder gone or renamed: its watch goes too (a rename keeps it), and
            // the next follow watches it again.
            for path in &event.paths {
                if self.watched.remove(path).is_some() {
                    let _ = self.watcher.unwatch(path);
                }
            }
        }
        // No path: an overflow, which calls for listing again.
        if event.paths.is_empty() {
            return true;
        }
        let prefixes = prefixes(&folders());
        event.paths.iter().any(|path| self.counts(path, &prefixes))
    }

    /// Whether a change to `path` counts: a record, or a followed project's log folder or an
    /// entry in it (a log).
    fn counts(&self, path: &Path, prefixes: &[String]) -> bool {
        self.watched.iter().any(|(dir, kind)| {
            let Ok(inside) = path.strip_prefix(dir) else {
                return false;
            };
            let mut parts = inside.components();
            let Some(first) = parts.next() else {
                return false;
            };
            match kind {
                Kind::Records => true,
                #[cfg(target_os = "linux")]
                Kind::Logs => false,
                Kind::Projects => parts.count() <= 1 && followed(first.as_os_str(), prefixes),
            }
        })
    }
}

/// The followed projects' folders as Claude names their log folders.
fn prefixes(folders: &[String]) -> Vec<String> {
    folders.iter().map(|f| normalized(f)).collect()
}

/// Whether the log folder `name` may be a followed project's (or a worktree's in its folder).
fn followed(name: &std::ffi::OsStr, prefixes: &[String]) -> bool {
    let name = normalized(&name.to_string_lossy());
    prefixes.iter().any(|p| name.starts_with(p.as_str()))
}

/// [`Watch::follow`] on `watched`, with `apply` (the watcher's `update_paths`).
fn follow(
    watched: &mut BTreeMap<PathBuf, Kind>,
    roots: &[PathBuf],
    records: &[PathBuf],
    folders: &[String],
    apply: &mut Apply<'_>,
) {
    let roots = roots.iter().map(|dir| (dir.clone(), Kind::Projects));
    let flat = records.iter().map(|dir| (dir.clone(), Kind::Records));
    let wanted: BTreeMap<PathBuf, Kind> = roots
        .chain(flat)
        .filter_map(|(dir, kind)| Some((canonical(&dir).ok().filter(|d| d.is_dir())?, kind)))
        .collect();
    #[cfg(target_os = "linux")]
    let wanted = {
        let mut wanted = wanted;
        let prefixes = prefixes(folders);
        let projects = wanted.iter().filter(|(_, kind)| **kind == Kind::Projects);
        let logs: Vec<PathBuf> = projects
            .filter_map(|(dir, _)| std::fs::read_dir(dir).ok())
            .flat_map(|entries| entries.take(LOG_FOLDERS).flatten())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| followed(&e.file_name(), &prefixes))
            .map(|e| e.path())
            .collect();
        wanted.extend(logs.into_iter().map(|dir| (dir, Kind::Logs)));
        wanted
    };
    #[cfg(not(target_os = "linux"))]
    let _ = folders;
    let gone = watched.keys().filter(|dir| !wanted.contains_key(*dir));
    // Unwatching fails when the folder is gone, which removed its watch already.
    let unwatch = gone.map(PathOp::unwatch);
    let added: Vec<(PathBuf, Kind)> = (wanted.iter())
        .filter(|(dir, _)| !watched.contains_key(*dir))
        .map(|(dir, kind)| (dir.clone(), *kind))
        .collect();
    let watch = (added.iter())
        .map(|(dir, kind)| PathOp::Watch(dir.clone(), WatchPathConfig::new(kind.mode())));
    let ops = unwatch.chain(watch).collect();
    // ponytail: a watcher that fails as a whole leaves the folders unwatched until they are
    // followed again (the app's next ask).
    let failed = files::update(apply, ops).unwrap_or_default();
    watched.retain(|dir, _| wanted.contains_key(dir));
    let ok = added.into_iter().filter(|(dir, _)| !failed.contains(dir));
    watched.extend(ok);
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{DataChange, RemoveKind, RenameMode};

    fn event(kind: EventKind, paths: &[&Path]) -> notify::Result<Event> {
        let paths = paths.iter().map(|p| p.to_path_buf()).collect();
        Ok(Event {
            kind,
            paths,
            attrs: Default::default(),
        })
    }

    /// A Claude config folder with its projects folder and records folder, canonical.
    fn claude() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = canonical(tmp.path()).unwrap();
        let (projects, records) = (dir.join("projects"), dir.join("sessions"));
        std::fs::create_dir(&projects).unwrap();
        std::fs::create_dir(&records).unwrap();
        (tmp, projects, records)
    }

    fn shop() -> Vec<String> {
        vec!["/w/shop".to_owned()]
    }

    #[tokio::test]
    async fn only_the_folders_that_exist_are_watched() {
        let (_tmp, projects, records) = claude();
        // A followed project's log folder, a worktree's of it, and another project's.
        for name in ["-w-shop", "-w-shop--claude-worktrees-x", "-w-api"] {
            std::fs::create_dir(projects.join(name)).unwrap();
        }
        std::fs::write(projects.join("-w-shop-file"), "").unwrap();
        let mut watch = Watch::new().unwrap();
        let none = projects.join("none");
        watch.follow(
            std::slice::from_ref(&projects),
            &[records.clone(), none],
            &shop(),
        );
        let mut expected = BTreeMap::from([
            (projects.clone(), Kind::Projects),
            (records.clone(), Kind::Records),
        ]);
        // Linux: the followed projects' log folders one by one, never recursively.
        #[cfg(target_os = "linux")]
        for name in ["-w-shop", "-w-shop--claude-worktrees-x"] {
            expected.insert(projects.join(name), Kind::Logs);
        }
        assert_eq!(watch.watched, expected);
        #[cfg(target_os = "linux")]
        let recursive = RecursiveMode::NonRecursive;
        #[cfg(not(target_os = "linux"))]
        let recursive = RecursiveMode::Recursive;
        assert_eq!(Kind::Projects.mode(), recursive);
        assert_eq!(Kind::Records.mode(), RecursiveMode::NonRecursive);
        #[cfg(target_os = "linux")]
        assert_eq!(Kind::Logs.mode(), RecursiveMode::NonRecursive);
        // Another account's folders replace them.
        let (_other, projects2, _) = claude();
        watch.follow(std::slice::from_ref(&projects2), &[], &shop());
        expected = BTreeMap::from([(projects2, Kind::Projects)]);
        assert_eq!(watch.watched, expected);
    }

    #[test]
    fn a_folder_the_watcher_refuses_is_not_kept_as_watched() {
        let (_tmp, projects, records) = claude();
        let mut watched = BTreeMap::new();
        // The records folder fails; the projects folder is still watched.
        let mut refusing = |ops: Vec<PathOp>| {
            let mut ops = ops.into_iter();
            let origin = ops.find(|op| op.as_path().ends_with("sessions"));
            match origin {
                None => Ok(()),
                origin => Err(Box::new(UpdatePathsError {
                    source: notify::Error::generic("refused"),
                    origin,
                    remaining: ops.collect(),
                })),
            }
        };
        let roots = [projects.clone()];
        follow(
            &mut watched,
            &roots,
            std::slice::from_ref(&records),
            &[],
            &mut refusing,
        );
        assert_eq!(
            watched,
            BTreeMap::from([(projects.clone(), Kind::Projects)])
        );
        // A watcher that fails as a whole: nothing is known to have failed.
        let mut broken = |_| {
            Err(Box::new(UpdatePathsError {
                source: notify::Error::generic("no stream"),
                origin: None,
                remaining: Vec::new(),
            }))
        };
        follow(
            &mut watched,
            &roots,
            std::slice::from_ref(&records),
            &[],
            &mut broken,
        );
        assert!(watched.contains_key(&records));
    }

    #[tokio::test]
    async fn only_the_followed_projects_logs_and_the_records_count() {
        let (_tmp, projects, records) = claude();
        let shop_logs = projects.join("-w-shop");
        std::fs::create_dir(&shop_logs).unwrap();
        let mut watch = Watch::new().unwrap();
        watch.follow(
            std::slice::from_ref(&projects),
            std::slice::from_ref(&records),
            &shop(),
        );
        let written = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        for (path, counts) in [
            (shop_logs.join("s.jsonl"), true),
            // A worktree of it, under its folder.
            (projects.join("-w-shop--claude-worktrees-x/s.jsonl"), true),
            (shop_logs.clone(), true),
            // A subagent's log, and another project's.
            (shop_logs.join("s/subagents/a.jsonl"), false),
            (projects.join("-w-api/s.jsonl"), false),
            (projects.clone(), false),
            (records.join("12.json"), true),
            (records.clone(), false),
            (PathBuf::from("/elsewhere/-w-shop/s.jsonl"), false),
        ] {
            assert_eq!(
                watch.saw(event(written, &[&path]), &shop),
                counts,
                "{path:?}"
            );
        }
        // An overflow or an error may have lost events.
        assert!(watch.saw(event(EventKind::Other, &[]), &shop));
        assert!(watch.saw(Err(notify::Error::generic("lost")), &shop));

        // A watched folder removed or renamed loses its watch, and is watched again once back.
        let removed = EventKind::Remove(RemoveKind::Folder);
        assert!(!watch.saw(event(removed, &[&records]), &shop));
        assert!(!watch.watched.contains_key(&records));
        let renamed = EventKind::Modify(ModifyKind::Name(RenameMode::From));
        watch.saw(event(renamed, &[&projects]), &shop);
        assert!(!watch.watched.contains_key(&projects));
        watch.follow(
            std::slice::from_ref(&projects),
            std::slice::from_ref(&records),
            &shop(),
        );
        assert!(watch.watched.contains_key(&records));
        assert!(watch.watched.contains_key(&projects));
    }

    #[tokio::test]
    async fn a_log_written_is_seen_once_its_burst_is_over() {
        let (_tmp, projects, records) = claude();
        let shop_logs = projects.join("-w-shop");
        std::fs::create_dir(&shop_logs).unwrap();
        let mut watch = Watch::new().unwrap();
        watch.follow(std::slice::from_ref(&projects), &[records], &shop());
        let started = Instant::now();
        std::fs::write(shop_logs.join("s.jsonl"), "{}\n").unwrap();
        tokio::time::timeout(Duration::from_secs(10), watch.changed(shop))
            .await
            .expect("the log written was not seen");
        assert!(started.elapsed() >= QUIET);
    }

    #[tokio::test]
    async fn a_poke_is_a_change_once_its_burst_is_over() {
        let mut watch = Watch::new().unwrap();
        let started = Instant::now();
        watch.poke();
        watch.poke();
        tokio::time::timeout(Duration::from_secs(10), watch.changed(Vec::new))
            .await
            .expect("the poke was not seen");
        assert!(started.elapsed() >= QUIET);
        assert!(watch.burst.is_none());
    }
}
