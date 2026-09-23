//! Detects a `claude` running in a Hive terminal without Hive's hooks (for example
//! started by absolute path, bypassing the wrapper): it never sends `SessionStart`.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::procs::Proc;

/// How often terminals are checked.
pub const INTERVAL: Duration = Duration::from_secs(1);

/// How long a `claude` may run before its `SessionStart` is considered missing.
pub const UNHOOKED_AFTER: Duration = Duration::from_secs(5);

/// Per-terminal detector state.
#[derive(Debug, Default)]
pub struct Watch {
    claude_since: Option<Instant>,
    hooked: bool,
    warned: bool,
}

impl Watch {
    /// A `SessionStart` arrived from this terminal.
    pub fn hooked(&mut self) {
        self.hooked = true;
    }

    /// Updates the state; true exactly once per unhooked `claude` run.
    pub fn tick(&mut self, claude_running: bool, now: Instant) -> bool {
        if !claude_running {
            *self = Self::default();
            return false;
        }
        let since = *self.claude_since.get_or_insert(now);
        let warn = !self.hooked && !self.warned && now.duration_since(since) >= UNHOOKED_AFTER;
        self.warned |= warn;
        warn
    }
}

/// Sessions with a process named `claude` in them.
pub fn claude_sessions(procs: &[Proc]) -> HashSet<i32> {
    procs
        .iter()
        .filter(|p| p.comm == "claude")
        .map(|p| p.session)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn warns_once_when_no_session_start_arrives_in_time() {
        let start = Instant::now();
        let mut watch = Watch::default();
        assert!(!watch.tick(true, start));
        assert!(!watch.tick(true, at(start, 4_999)));
        assert!(watch.tick(true, at(start, 5_000)));
        assert!(!watch.tick(true, at(start, 6_000)));
    }

    #[test]
    fn a_session_start_prevents_the_warning() {
        let start = Instant::now();
        let mut watch = Watch::default();
        assert!(!watch.tick(true, start));
        watch.hooked();
        assert!(!watch.tick(true, at(start, 10_000)));
    }

    #[test]
    fn a_new_claude_run_is_watched_from_scratch() {
        let start = Instant::now();
        let mut watch = Watch::default();
        watch.hooked();
        assert!(!watch.tick(true, start));
        // claude exits: the hook state and the timer reset.
        assert!(!watch.tick(false, at(start, 1_000)));
        assert!(!watch.tick(true, at(start, 2_000)));
        assert!(watch.tick(true, at(start, 7_000)));
        // After a warned run ends, the next unhooked run warns again.
        assert!(!watch.tick(false, at(start, 8_000)));
        assert!(!watch.tick(true, at(start, 9_000)));
        assert!(watch.tick(true, at(start, 14_000)));
    }

    #[test]
    fn claude_sessions_are_found_by_process_name() {
        let proc = |pid, session, comm: &str| Proc {
            pid,
            ppid: 1,
            pgrp: pid,
            session,
            comm: comm.into(),
        };
        let procs = [
            proc(1, 10, "fish"),
            proc(2, 10, "claude"),
            proc(3, 20, "node"),
            proc(4, 30, "claude"),
        ];
        assert_eq!(claude_sessions(&procs), HashSet::from([10, 30]));
    }
}
