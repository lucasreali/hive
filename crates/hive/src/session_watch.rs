//! Claude's session logs and its records of running `claude`s, watched with `notify` (D19.4):
//! the Sessions list is sent again when they change, instead of the app asking every 5 s.
//!
//! Each account's projects folder is watched recursively (a log written or removed in it), and
//! the folder of its records beside it non-recursively (a `claude` that starts or exits). Only
//! a project folder's own entries count, and only for the followed projects: a subagent's log
//! or another project's does not.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{
    Event, EventKind, PathOp, RecommendedWatcher, RecursiveMode, WatchPathConfig, Watcher as _,
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

pub struct Watch {
    watcher: RecommendedWatcher,
    events: mpsc::Receiver<notify::Result<Event>>,
    /// Watched folders, canonical: projects folders (recursive) and records folders (not).
    watched: BTreeMap<PathBuf, RecursiveMode>,
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

    /// Watches the projects folders `roots` and the records folders `records` that exist, and
    /// no others.
    pub fn follow(&mut self, roots: &[PathBuf], records: &[PathBuf]) {
        let recursive = roots.iter().map(|dir| (dir, RecursiveMode::Recursive));
        let flat = records.iter().map(|dir| (dir, RecursiveMode::NonRecursive));
        let wanted: BTreeMap<PathBuf, RecursiveMode> = recursive
            .chain(flat)
            .filter_map(|(dir, mode)| Some((canonical(dir).ok().filter(|d| d.is_dir())?, mode)))
            .collect();
        let gone = self.watched.keys().filter(|dir| !wanted.contains_key(*dir));
        // Unwatching fails when the folder is gone, which removed its watch already.
        let unwatch = gone.map(PathOp::unwatch);
        let added: Vec<(PathBuf, RecursiveMode)> = (wanted.iter())
            .filter(|(dir, _)| !self.watched.contains_key(*dir))
            .map(|(dir, mode)| (dir.clone(), *mode))
            .collect();
        let watch = (added.iter())
            .map(|(dir, mode)| PathOp::Watch(dir.clone(), WatchPathConfig::new(*mode)));
        let watcher = &mut self.watcher;
        let ops = unwatch.chain(watch).collect();
        let failed = files::update(|ops| watcher.update_paths(ops).map_err(Box::new), ops);
        let failed = failed.unwrap_or_default();
        self.watched.retain(|dir, _| wanted.contains_key(dir));
        let watched = added.into_iter().filter(|(dir, _)| !failed.contains(dir));
        self.watched.extend(watched);
    }

    /// Waits until the logs of the projects at `folders` (asked for at each event: the
    /// followed projects may change meanwhile) or a record changed, and the burst of events is
    /// over. Once the watcher is gone, it waits forever.
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

    /// Whether `event` may have changed the list of the projects at `folders`.
    fn saw(&mut self, event: notify::Result<Event>, folders: &impl Fn() -> Vec<String>) -> bool {
        // A watcher error may have lost events: listing again is safe.
        let Ok(event) = event else { return true };
        if matches!(
            event.kind,
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        ) {
            // A watched folder gone lost its watch; the next follow watches it again.
            for path in &event.paths {
                self.watched.remove(path);
            }
        }
        // No path: an overflow, which calls for listing again.
        if event.paths.is_empty() {
            return true;
        }
        let prefixes: Vec<String> = folders().iter().map(|f| normalized(f)).collect();
        event.paths.iter().any(|path| self.counts(path, &prefixes))
    }

    /// Whether a change to `path` counts: a record, or a followed project's log folder or an
    /// entry in it (a log). `prefixes` are the followed projects' folders as Claude names
    /// their log folders.
    fn counts(&self, path: &Path, prefixes: &[String]) -> bool {
        self.watched.iter().any(|(dir, mode)| {
            let Ok(inside) = path.strip_prefix(dir) else {
                return false;
            };
            let mut parts = inside.components();
            let Some(first) = parts.next() else {
                return false;
            };
            match mode {
                RecursiveMode::NonRecursive => true,
                RecursiveMode::Recursive => {
                    let name = normalized(&first.as_os_str().to_string_lossy());
                    parts.count() <= 1 && prefixes.iter().any(|p| name.starts_with(p.as_str()))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{DataChange, RemoveKind};

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

    #[tokio::test]
    async fn only_the_folders_that_exist_are_watched() {
        let (_tmp, projects, records) = claude();
        let mut watch = Watch::new().unwrap();
        watch.follow(
            std::slice::from_ref(&projects),
            &[records.clone(), projects.join("none")],
        );
        let expected = BTreeMap::from([
            (projects.clone(), RecursiveMode::Recursive),
            (records.clone(), RecursiveMode::NonRecursive),
        ]);
        assert_eq!(watch.watched, expected);
        // Another account's folders replace them.
        let (_other, projects2, _) = claude();
        watch.follow(std::slice::from_ref(&projects2), &[]);
        let expected = BTreeMap::from([(projects2, RecursiveMode::Recursive)]);
        assert_eq!(watch.watched, expected);
    }

    #[tokio::test]
    async fn only_the_followed_projects_logs_and_the_records_count() {
        let (_tmp, projects, records) = claude();
        let mut watch = Watch::new().unwrap();
        watch.follow(
            std::slice::from_ref(&projects),
            std::slice::from_ref(&records),
        );
        let followed = || vec!["/w/shop".to_owned()];
        let written = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        let shop = projects.join("-w-shop");
        for (path, counts) in [
            (shop.join("s.jsonl"), true),
            // A worktree of it, under its folder.
            (projects.join("-w-shop--claude-worktrees-x/s.jsonl"), true),
            (shop.clone(), true),
            // A subagent's log, and another project's.
            (shop.join("s/subagents/a.jsonl"), false),
            (projects.join("-w-api/s.jsonl"), false),
            (projects.clone(), false),
            (records.join("12.json"), true),
            (records.clone(), false),
            (PathBuf::from("/elsewhere/-w-shop/s.jsonl"), false),
        ] {
            assert_eq!(
                watch.saw(event(written, &[&path]), &followed),
                counts,
                "{path:?}"
            );
        }
        // An overflow or an error may have lost events.
        assert!(watch.saw(event(EventKind::Other, &[]), &followed));
        assert!(watch.saw(Err(notify::Error::generic("lost")), &followed));

        // A watched folder removed lost its watch, and is watched again once it is back.
        let removed = EventKind::Remove(RemoveKind::Folder);
        assert!(!watch.saw(event(removed, &[&records]), &followed));
        assert!(!watch.watched.contains_key(&records));
        watch.follow(
            std::slice::from_ref(&projects),
            std::slice::from_ref(&records),
        );
        assert!(watch.watched.contains_key(&records));
    }

    #[tokio::test]
    async fn a_log_written_is_seen_once_its_burst_is_over() {
        let (_tmp, projects, records) = claude();
        let shop = projects.join("-w-shop");
        std::fs::create_dir(&shop).unwrap();
        let mut watch = Watch::new().unwrap();
        watch.follow(
            std::slice::from_ref(&projects),
            std::slice::from_ref(&records),
        );
        let started = Instant::now();
        std::fs::write(shop.join("s.jsonl"), "{}\n").unwrap();
        let wait = watch.changed(|| vec!["/w/shop".to_owned()]);
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .expect("the log written was not seen");
        assert!(started.elapsed() >= QUIET);
    }
}
