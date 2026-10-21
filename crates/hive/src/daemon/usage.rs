//! The usage windows `hive statusline` reports (12.1; the 7-day one, 15.3): the service's side,
//! apart from the statusline itself so it runs wherever the service does. Also their pace and
//! the alerts at 80% and 95% (17.23).

use std::collections::HashMap;
use std::path::Path;

use hive_protocol::{Control, SessionWindow, UsageWindow};

/// Most Claude config folders the service keeps a window for; a new one past it is ignored.
const FOLDERS: usize = 64;
/// Longest Claude config folder the service keeps, in bytes.
const FOLDER_LIMIT: usize = 4096;
/// How long the 5-hour window lasts, in seconds: it began this long before its reset.
const SESSION_LENGTH: u64 = 5 * 3600;
/// How long the 7-day window lasts, in seconds.
const WEEK_LENGTH: u64 = 7 * 86400;
/// The shares of a window that alert, once each per window, lowest first.
const THRESHOLDS: [u8; 2] = [80, 95];

/// A folder's 5-hour window and its 7-day one, when it reported one.
type Windows = (SessionWindow, Option<SessionWindow>);

/// What `session_usage` last told the app.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Shown {
    usage: Option<SessionWindow>,
    week: Option<SessionWindow>,
    limit_at: Option<u64>,
    week_limit_at: Option<u64>,
}

/// The highest threshold a window alerted for, and that window's reset.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Alerted {
    resets_at: u64,
    threshold: u8,
}

/// The latest windows of each Claude config folder (in memory only), and the ones the app has.
#[derive(Debug, Default)]
pub struct Usage {
    windows: HashMap<String, Windows>,
    sent: Shown,
    /// Per folder of `windows`: the 5-hour window's alerts and the 7-day one's.
    alerted: HashMap<String, [Alerted; 2]>,
}

/// When `window`, which lasts `length` seconds up to its reset, reaches 100% at its pace so far
/// (its share used over the time since it began), at `now` (Unix seconds): rounded down to the
/// minute, and `None` when it would not before its reset, when nothing or everything is used,
/// or when `now` is before the window began.
fn limit_at(window: &SessionWindow, length: u64, now: u64) -> Option<u64> {
    let used = u64::from(window.used_percentage);
    let start = window.resets_at.checked_sub(length)?;
    let elapsed = now.checked_sub(start)?;
    if used == 0 || used >= 100 {
        return None;
    }
    let at = start + elapsed * 100 / used;
    (at < window.resets_at).then_some(at / 60 * 60)
}

/// The alert `window` (of `kind`) raises at `now`: the highest threshold it reached, unless that
/// window already alerted for it. A window past its reset no longer counts.
fn alert(
    alerted: &mut Alerted,
    kind: UsageWindow,
    window: Option<SessionWindow>,
    now: u64,
) -> Option<Control> {
    let window = window?;
    if now >= alerted.resets_at {
        *alerted = Alerted::default();
    }
    let threshold = THRESHOLDS
        .into_iter()
        .rev()
        .find(|&t| window.used_percentage >= t)?;
    if threshold <= alerted.threshold {
        return None;
    }
    *alerted = Alerted {
        resets_at: window.resets_at,
        threshold,
    };
    Some(Control::UsageAlert {
        window: kind,
        threshold,
        resets_at: window.resets_at,
    })
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
        self.sent = Shown::default();
    }

    /// What to send the app for `account` at `now` (Unix seconds): `session_usage` with its
    /// windows and their pace when they are not the ones the app has (none once the 5-hour
    /// window reset, and the 7-day one only until its own), then a `usage_alert` for each window
    /// that reached a threshold it had not alerted for.
    pub fn changed(&mut self, account: Option<&Path>, now: u64) -> Vec<Control> {
        let live = |window: &SessionWindow| window.resets_at > now;
        let key = account.and_then(Path::to_str);
        let windows = key.and_then(|dir| self.windows.get(dir));
        let windows = windows
            .filter(|(usage, _)| live(usage))
            .map(|&(usage, week)| (usage, week.filter(live)));
        let (usage, week) = windows.unzip();
        let week = week.flatten();
        let shown = Shown {
            usage,
            week,
            limit_at: usage.and_then(|w| limit_at(&w, SESSION_LENGTH, now)),
            week_limit_at: week.and_then(|w| limit_at(&w, WEEK_LENGTH, now)),
        };
        let mut messages = Vec::new();
        if shown != self.sent {
            self.sent = shown;
            messages.push(Control::SessionUsage {
                usage,
                week,
                limit_at: shown.limit_at,
                week_limit_at: shown.week_limit_at,
            });
        }
        // Only a folder with windows gets alerts: as bounded as `windows`.
        if let (Some(key), Some(_)) = (key, windows) {
            let [session_alerted, week_alerted] = self.alerted.entry(key.to_owned()).or_default();
            messages.extend(alert(session_alerted, UsageWindow::Session, usage, now));
            messages.extend(alert(week_alerted, UsageWindow::Week, week, now));
        }
        messages
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

    /// The `session_usage` among what `changed` sends.
    fn shown(usage: &mut Usage, account: Option<&Path>, now: u64) -> Option<Control> {
        let messages = usage.changed(account, now);
        messages
            .into_iter()
            .find(|m| matches!(m, Control::SessionUsage { .. }))
    }

    /// The `usage_alert`s among what `changed` sends, as (window, threshold, reset).
    fn alerts(usage: &mut Usage, account: Option<&Path>, now: u64) -> Vec<(UsageWindow, u8, u64)> {
        let messages = usage.changed(account, now);
        let alert = |m: Control| match m {
            Control::UsageAlert {
                window,
                threshold,
                resets_at,
            } => Some((window, threshold, resets_at)),
            _ => None,
        };
        messages.into_iter().filter_map(alert).collect()
    }

    #[test]
    fn the_service_keeps_each_folders_latest_window_and_sends_the_accounts_until_it_resets() {
        let mut usage = Usage::default();
        let a = Some(Path::new("/a"));
        assert!(usage.changed(a, 0).is_empty());
        usage.report("/a".into(), window(150, 100), None);
        usage.report("/b".into(), window(7, 100), None);
        let sent = |usage: Option<SessionWindow>| {
            Some(Control::SessionUsage {
                usage,
                week: None,
                limit_at: None,
                week_limit_at: None,
            })
        };
        assert_eq!(shown(&mut usage, a, 99), sent(Some(window(100, 100))));
        // Only what changed.
        assert_eq!(usage.changed(a, 99), vec![]);
        usage.report("/a".into(), window(12, 100), None);
        assert_eq!(shown(&mut usage, a, 99), sent(Some(window(12, 100))));
        // Past its reset, none; another account's.
        assert_eq!(shown(&mut usage, a, 100), sent(None));
        assert_eq!(
            shown(&mut usage, Some(Path::new("/b")), 1),
            sent(Some(window(7, 100)))
        );
        assert_eq!(shown(&mut usage, None, 1), sent(None));
        // A new app gets it again.
        assert_eq!(shown(&mut usage, a, 1), sent(Some(window(12, 100))));
        usage.unsent();
        assert_eq!(shown(&mut usage, a, 1), sent(Some(window(12, 100))));
    }

    #[test]
    fn the_seven_day_window_goes_with_the_five_hour_one_until_its_own_reset() {
        let mut usage = Usage::default();
        let a = Some(Path::new("/a"));
        let sent = |usage, week| {
            Some(Control::SessionUsage {
                usage,
                week,
                limit_at: None,
                week_limit_at: None,
            })
        };
        usage.report("/a".into(), window(12, 100), Some(window(250, 50)));
        assert_eq!(
            shown(&mut usage, a, 49),
            sent(Some(window(12, 100)), Some(window(100, 50)))
        );
        assert_eq!(usage.changed(a, 49), vec![]);
        // Past its reset, the 5-hour one alone.
        assert_eq!(shown(&mut usage, a, 50), sent(Some(window(12, 100)), None));
        assert_eq!(usage.changed(a, 51), vec![]);
        // A new 7-day one is sent; a report without one drops it.
        usage.report("/a".into(), window(12, 200), Some(window(41, 900)));
        assert_eq!(
            shown(&mut usage, a, 51),
            sent(Some(window(12, 200)), Some(window(41, 900)))
        );
        usage.report("/a".into(), window(12, 200), Some(window(42, 900)));
        assert_eq!(
            shown(&mut usage, a, 51),
            sent(Some(window(12, 200)), Some(window(42, 900)))
        );
        usage.report("/a".into(), window(12, 200), None);
        assert_eq!(shown(&mut usage, a, 51), sent(Some(window(12, 200)), None));
        // Never without the 5-hour one: past its reset, neither.
        usage.report("/a".into(), window(12, 200), Some(window(41, 900)));
        assert_eq!(shown(&mut usage, a, 200), sent(None, None));
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
        // Alerts are kept only for a folder with windows.
        usage.changed(Some(Path::new("/new")), 1);
        assert!(usage.alerted.is_empty());
        usage.changed(Some(Path::new("/1")), 1);
        assert_eq!(usage.alerted.len(), 1);
    }

    /// A 5-hour window that began at this Unix second (a whole minute), and its reset.
    const START: u64 = 1_800_000;
    const RESET: u64 = START + SESSION_LENGTH;

    #[test]
    fn a_steady_pace_forecasts_the_same_limit() {
        let at = |used, elapsed| limit_at(&window(used, RESET), SESSION_LENGTH, START + elapsed);
        // 25% an hour in, 50% two hours in…: 100% four hours in, an hour before the reset.
        assert_eq!(at(25, 3600), Some(START + 4 * 3600));
        assert_eq!(at(50, 7200), Some(START + 4 * 3600));
        assert_eq!(at(75, 3 * 3600), Some(START + 4 * 3600));
        // Rounded down to the minute: 17 142 s is 285 whole minutes.
        assert_eq!(at(21, 3600), Some(START + 285 * 60));
        // As fast as the window lasts: 100% at its reset is no limit.
        assert_eq!(at(20, 3600), None);
        // Used as the window began: the limit is now.
        assert_eq!(at(50, 0), Some(START));
    }

    #[test]
    fn an_idle_window_pushes_its_limit_later_until_it_lasts() {
        // 40% in the first hour, then nothing more.
        let at = |elapsed| limit_at(&window(40, RESET), SESSION_LENGTH, START + elapsed);
        assert_eq!(at(3600), Some(START + 9000));
        assert_eq!(at(5400), Some(START + 13_500));
        assert_eq!(at(7199), Some(START + 17_940));
        assert_eq!(at(7200), None);
        // Nothing used, everything used, or before the window began: no pace.
        let at = |used, now| limit_at(&window(used, RESET), SESSION_LENGTH, now);
        assert_eq!(at(0, START + 60), None);
        assert_eq!(at(100, START + 60), None);
        assert_eq!(at(99, START + 60), Some(START + 60));
        assert_eq!(at(40, START - 1), None);
        // A reset sooner than the window's length (a clock far off): no start, no pace.
        assert_eq!(limit_at(&window(40, 100), SESSION_LENGTH, 50), None);
    }

    #[test]
    fn the_pace_goes_with_the_windows_and_restarts_at_their_reset() {
        let mut usage = Usage::default();
        let a = Some(Path::new("/a"));
        let paces = |usage: &mut Usage, now| match shown(usage, a, now) {
            Some(Control::SessionUsage {
                limit_at,
                week_limit_at,
                ..
            }) => Some((limit_at, week_limit_at)),
            _ => None,
        };
        // The week began a day before the session. 10% of it 25 h in is 100% in ten days: past
        // its reset.
        let week_start = START - 86400;
        let week_reset = week_start + WEEK_LENGTH;
        usage.report("/a".into(), window(25, RESET), Some(window(10, week_reset)));
        assert_eq!(
            paces(&mut usage, START + 3600),
            Some((Some(START + 4 * 3600), None))
        );
        // The same minute: nothing new to send.
        assert_eq!(paces(&mut usage, START + 3601), None);
        // 60% of the week 26 h in; the session idle since.
        usage.report("/a".into(), window(25, RESET), Some(window(60, week_reset)));
        assert_eq!(
            paces(&mut usage, START + 7200),
            Some((None, Some(week_start + 156_000)))
        );
        // Past the session's reset, neither.
        assert_eq!(paces(&mut usage, RESET), Some((None, None)));
        // A new window paces from its own start.
        usage.report("/a".into(), window(30, RESET + SESSION_LENGTH), None);
        assert_eq!(
            paces(&mut usage, RESET + 3600),
            Some((Some(RESET + 12_000), None))
        );
    }

    #[test]
    fn each_threshold_alerts_once_per_window() {
        let mut usage = Usage::default();
        let a = Some(Path::new("/a"));
        usage.report("/a".into(), window(79, 100), Some(window(10, 900)));
        assert_eq!(alerts(&mut usage, a, 1), vec![]);
        usage.report("/a".into(), window(80, 100), Some(window(10, 900)));
        assert_eq!(
            alerts(&mut usage, a, 1),
            vec![(UsageWindow::Session, 80, 100)]
        );
        // Once: not on the next tick, a new app, nor a higher share below the next threshold.
        assert_eq!(alerts(&mut usage, a, 2), vec![]);
        usage.unsent();
        usage.report("/a".into(), window(94, 100), Some(window(10, 900)));
        assert_eq!(alerts(&mut usage, a, 2), vec![]);
        usage.report("/a".into(), window(95, 100), Some(window(95, 900)));
        assert_eq!(
            alerts(&mut usage, a, 3),
            vec![
                (UsageWindow::Session, 95, 100),
                (UsageWindow::Week, 95, 900)
            ]
        );
        // A lower share later in the window does not alert again.
        usage.report("/a".into(), window(81, 100), Some(window(99, 900)));
        assert_eq!(alerts(&mut usage, a, 4), vec![]);
        // A new window alerts again (a jump past both thresholds: 95 alone).
        usage.report("/a".into(), window(96, 200), Some(window(99, 900)));
        assert_eq!(
            alerts(&mut usage, a, 100),
            vec![(UsageWindow::Session, 95, 200)]
        );
        assert_eq!(alerts(&mut usage, a, 101), vec![]);
        // Another account has its own; none for a folder without windows or past their reset.
        let b = Some(Path::new("/b"));
        usage.report("/b".into(), window(85, 100), None);
        assert_eq!(
            alerts(&mut usage, b, 99),
            vec![(UsageWindow::Session, 80, 100)]
        );
        assert_eq!(alerts(&mut usage, Some(Path::new("/c")), 99), vec![]);
        usage.report("/b".into(), window(99, 100), None);
        assert_eq!(alerts(&mut usage, b, 100), vec![]);
        // The week's alert lives until its own reset, past which a new week alerts.
        usage.report("/a".into(), window(1, 1000), Some(window(99, 900)));
        assert_eq!(alerts(&mut usage, a, 899), vec![]);
        usage.report("/a".into(), window(1, 1000), Some(window(80, 2000)));
        assert_eq!(
            alerts(&mut usage, a, 900),
            vec![(UsageWindow::Week, 80, 2000)]
        );
    }
}
