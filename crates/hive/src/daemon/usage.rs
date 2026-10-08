//! The usage windows `hive statusline` reports (12.1; the 7-day one, 15.3): the service's side,
//! apart from the statusline itself so it runs wherever the service does.

use std::collections::HashMap;
use std::path::Path;

use hive_protocol::{Control, SessionWindow};

/// Most Claude config folders the service keeps a window for; a new one past it is ignored.
const FOLDERS: usize = 64;
/// Longest Claude config folder the service keeps, in bytes.
const FOLDER_LIMIT: usize = 4096;

/// A folder's 5-hour window and its 7-day one, when it reported one.
type Windows = (SessionWindow, Option<SessionWindow>);

/// The latest windows of each Claude config folder (in memory only), and the ones the app has.
#[derive(Debug, Default)]
pub struct Usage {
    windows: HashMap<String, Windows>,
    sent: Option<Windows>,
}

impl Usage {
    /// Keeps `usage` and `week` as the latest windows of `claude_dir` (from a hook connection:
    /// untrusted).
    pub fn report(
        &mut self,
        claude_dir: String,
        usage: SessionWindow,
        week: Option<SessionWindow>,
    ) {
        let room = self.windows.len() < FOLDERS || self.windows.contains_key(&claude_dir);
        if room && claude_dir.len() <= FOLDER_LIMIT {
            let capped = |mut window: SessionWindow| {
                window.used_percentage = window.used_percentage.min(100);
                window
            };
            self.windows
                .insert(claude_dir, (capped(usage), week.map(capped)));
        }
    }

    /// A new app has none.
    pub fn unsent(&mut self) {
        self.sent = None;
    }

    /// `session_usage` with the windows of `account` when they are not the ones the app has:
    /// none once `now` (Unix seconds) reached the 5-hour window's reset, and the 7-day one only
    /// until its own.
    pub fn changed(&mut self, account: Option<&Path>, now: u64) -> Option<Control> {
        let live = |window: &SessionWindow| window.resets_at > now;
        let windows = account.and_then(|dir| self.windows.get(dir.to_str()?));
        let windows = windows
            .filter(|(usage, _)| live(usage))
            .map(|&(usage, week)| (usage, week.filter(live)));
        if windows == self.sent {
            return None;
        }
        self.sent = windows;
        let (usage, week) = windows.unzip();
        Some(Control::SessionUsage {
            usage,
            week: week.flatten(),
        })
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
        usage.report("/a".into(), window(150, 100), None);
        usage.report("/b".into(), window(7, 100), None);
        let sent = |usage: Option<SessionWindow>| Some(Control::SessionUsage { usage, week: None });
        assert_eq!(usage.changed(a, 99), sent(Some(window(100, 100))));
        // Only what changed.
        assert_eq!(usage.changed(a, 99), None);
        usage.report("/a".into(), window(12, 100), None);
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
    fn the_seven_day_window_goes_with_the_five_hour_one_until_its_own_reset() {
        let mut usage = Usage::default();
        let a = Some(Path::new("/a"));
        let sent = |usage, week| Some(Control::SessionUsage { usage, week });
        usage.report("/a".into(), window(12, 100), Some(window(250, 50)));
        assert_eq!(
            usage.changed(a, 49),
            sent(Some(window(12, 100)), Some(window(100, 50)))
        );
        assert_eq!(usage.changed(a, 49), None);
        // Past its reset, the 5-hour one alone.
        assert_eq!(usage.changed(a, 50), sent(Some(window(12, 100)), None));
        assert_eq!(usage.changed(a, 51), None);
        // A new 7-day one is sent; a report without one drops it.
        usage.report("/a".into(), window(12, 200), Some(window(41, 900)));
        assert_eq!(
            usage.changed(a, 51),
            sent(Some(window(12, 200)), Some(window(41, 900)))
        );
        usage.report("/a".into(), window(12, 200), Some(window(42, 900)));
        assert_eq!(
            usage.changed(a, 51),
            sent(Some(window(12, 200)), Some(window(42, 900)))
        );
        usage.report("/a".into(), window(12, 200), None);
        assert_eq!(usage.changed(a, 51), sent(Some(window(12, 200)), None));
        // Never without the 5-hour one: past its reset, neither.
        usage.report("/a".into(), window(12, 200), Some(window(41, 900)));
        assert_eq!(usage.changed(a, 200), sent(None, None));
    }

    #[test]
    fn the_service_keeps_a_bounded_number_of_folders_of_a_bounded_length() {
        let mut usage = Usage::default();
        let long = "/".repeat(FOLDER_LIMIT);
        usage.report(long.clone(), window(1, 9), None);
        usage.report(format!("{long}x"), window(1, 9), None);
        assert_eq!(usage.windows.len(), 1);
        for n in 1..FOLDERS {
            usage.report(format!("/{n}"), window(1, 9), None);
        }
        usage.report("/new".into(), window(1, 9), None);
        assert_eq!(usage.windows.len(), FOLDERS);
        assert!(!usage.windows.contains_key("/new"));
        // A known one is still updated.
        usage.report("/1".into(), window(2, 9), Some(window(3, 9)));
        assert_eq!(usage.windows["/1"], (window(2, 9), Some(window(3, 9))));
    }
}
