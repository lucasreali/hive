//! The 5-hour usage windows `hive statusline` reports (12.1): the service's side, apart from
//! the statusline itself so it runs wherever the service does.

use std::collections::HashMap;
use std::path::Path;

use hive_protocol::{Control, SessionWindow};

/// Most Claude config folders the service keeps a window for; a new one past it is ignored.
const FOLDERS: usize = 64;
/// Longest Claude config folder the service keeps, in bytes.
const FOLDER_LIMIT: usize = 4096;

/// The latest 5-hour window of each Claude config folder (in memory only), and the one the
/// app has.
#[derive(Debug, Default)]
pub struct Usage {
    windows: HashMap<String, SessionWindow>,
    sent: Option<SessionWindow>,
}

impl Usage {
    /// Keeps `usage` as the latest window of `claude_dir` (from a hook connection: untrusted).
    pub fn report(&mut self, claude_dir: String, mut usage: SessionWindow) {
        let room = self.windows.len() < FOLDERS || self.windows.contains_key(&claude_dir);
        if room && claude_dir.len() <= FOLDER_LIMIT {
            usage.used_percentage = usage.used_percentage.min(100);
            self.windows.insert(claude_dir, usage);
        }
    }

    /// A new app has none.
    pub fn unsent(&mut self) {
        self.sent = None;
    }

    /// `session_usage` with the window of `account` (none once `now`, in Unix seconds, reached
    /// its reset), when it is not the one the app has.
    pub fn changed(&mut self, account: Option<&Path>, now: u64) -> Option<Control> {
        let window = account.and_then(|dir| self.windows.get(dir.to_str()?));
        let usage = window.filter(|w| w.resets_at > now).copied();
        if usage == self.sent {
            return None;
        }
        self.sent = usage;
        Some(Control::SessionUsage { usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(used_percentage: u8, resets_at: u64) -> SessionWindow {
        SessionWindow {
            used_percentage,
            resets_at,
        }
    }

    #[test]
    fn the_service_keeps_each_folders_latest_window_and_sends_the_accounts_until_it_resets() {
        let mut usage = Usage::default();
        let a = Some(Path::new("/a"));
        assert!(usage.changed(a, 0).is_none());
        usage.report("/a".into(), window(150, 100));
        usage.report("/b".into(), window(7, 100));
        let sent = |usage: Option<SessionWindow>| Some(Control::SessionUsage { usage });
        assert_eq!(usage.changed(a, 99), sent(Some(window(100, 100))));
        // Only what changed.
        assert_eq!(usage.changed(a, 99), None);
        usage.report("/a".into(), window(12, 100));
        assert_eq!(usage.changed(a, 99), sent(Some(window(12, 100))));
        // Past its reset, none; another account's.
        assert_eq!(usage.changed(a, 100), sent(None));
        assert_eq!(
            usage.changed(Some(Path::new("/b")), 1),
            sent(Some(window(7, 100)))
        );
        assert_eq!(usage.changed(None, 1), sent(None));
        // A new app gets it again.
        assert_eq!(usage.changed(a, 1), sent(Some(window(12, 100))));
        usage.unsent();
        assert_eq!(usage.changed(a, 1), sent(Some(window(12, 100))));
    }

    #[test]
    fn the_service_keeps_a_bounded_number_of_folders_of_a_bounded_length() {
        let mut usage = Usage::default();
        let long = "/".repeat(FOLDER_LIMIT);
        usage.report(long.clone(), window(1, 9));
        usage.report(format!("{long}x"), window(1, 9));
        assert_eq!(usage.windows.len(), 1);
        for n in 1..FOLDERS {
            usage.report(format!("/{n}"), window(1, 9));
        }
        usage.report("/new".into(), window(1, 9));
        assert_eq!(usage.windows.len(), FOLDERS);
        assert!(!usage.windows.contains_key("/new"));
        // A known one is still updated.
        usage.report("/1".into(), window(2, 9));
        assert_eq!(usage.windows["/1"], window(2, 9));
    }
}
