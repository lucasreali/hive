//! Shell marks and terminal alerts (17.25): the OSC sequences a terminal's output carries that
//! the service acts on. OSC 133 `C` / `D;<exit>` (a shell's command start and end, as fish 4
//! emits them) time each command, for the long-command alert; OSC 9 `<text>` (iTerm2's) and
//! OSC 777 `notify;<title>;<body>` (urxvt's) ask for a notification; OSC 9;4 (ConEmu's) reports
//! progress. The output itself passes through untouched: xterm.js ignores what it does not
//! know, and keeps its own prompt marks (`src/marks.ts`).
//!
//! Terminal output is untrusted: a sequence longer than [`MAX_OSC`] is dropped whole, text is
//! cleaned of control and invisible characters and cut ([`adapter::clip`]), and notifications
//! are spaced out per terminal ([`NOTE_GAP`]).

use std::time::{Duration, Instant};

use hive_protocol::{Control, ProgressState};

use crate::adapter;

/// The longest OSC payload read; a longer one is dropped whole.
pub const MAX_OSC: usize = 1024;
/// A notification's title is cut at this many characters.
pub const MAX_TITLE: usize = 100;
/// A notification's body is cut at this many characters.
pub const MAX_BODY: usize = 300;
/// A terminal's notifications closer together than this are dropped.
pub const NOTE_GAP: Duration = Duration::from_secs(1);
/// The shortest command that can alert: the lowest `notifications.command_secs` but 0 (never).
pub const MIN_COMMAND: Duration = Duration::from_secs(1);

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// CAN and SUB cancel a sequence (ECMA-48).
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// What one OSC sequence asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// OSC 133;C: the shell runs a command.
    CommandStarted,
    /// OSC 133;D[;<exit>]: the command ended, with its exit status when given.
    CommandFinished(Option<i32>),
    /// OSC 9 or OSC 777 `notify`, cleaned and cut; never both empty.
    Notification { title: String, body: String },
    /// OSC 9;4;<state>[;<percent>].
    Progress(ProgressState, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Plain output.
    Ground,
    /// After ESC.
    Escape,
    /// Inside an OSC payload.
    Osc,
    /// ESC inside an OSC payload: `\` ends it (ST).
    OscEscape,
}

/// Finds the OSC sequences in a terminal's output, across reads.
#[derive(Debug)]
pub struct Scanner {
    mode: Mode,
    payload: Vec<u8>,
    /// The payload grew past [`MAX_OSC`]: dropped when it ends.
    overflow: bool,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            mode: Mode::Ground,
            payload: Vec::new(),
            overflow: false,
        }
    }
}

impl Scanner {
    /// Reads `bytes`, the next of the output, and returns the signals of the sequences that
    /// ended in them.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Signal> {
        let mut signals = Vec::new();
        let mut rest = bytes;
        while let Some((&byte, after)) = rest.split_first() {
            rest = after;
            match self.mode {
                Mode::Ground if byte == ESC => self.mode = Mode::Escape,
                Mode::Ground => {
                    // Most output holds no ESC at all: skip to the next one.
                    let at = rest.iter().position(|&b| b == ESC).unwrap_or(rest.len());
                    rest = &rest[at..];
                }
                Mode::Escape => self.escape(byte),
                Mode::Osc => match byte {
                    BEL => signals.extend(self.end()),
                    ESC => self.mode = Mode::OscEscape,
                    CAN | SUB => self.mode = Mode::Ground,
                    _ if self.payload.len() < MAX_OSC => self.payload.push(byte),
                    _ => self.overflow = true,
                },
                Mode::OscEscape if byte == b'\\' => signals.extend(self.end()),
                // Any other escape ends the sequence unread and starts its own.
                Mode::OscEscape => self.escape(byte),
            }
        }
        signals
    }

    /// The byte after an ESC: `]` starts an OSC sequence.
    fn escape(&mut self, byte: u8) {
        self.mode = match byte {
            b']' => {
                self.payload.clear();
                self.overflow = false;
                Mode::Osc
            }
            ESC => Mode::Escape,
            _ => Mode::Ground,
        };
    }

    /// The OSC sequence ended: its signal, unless it was too long.
    fn end(&mut self) -> Option<Signal> {
        self.mode = Mode::Ground;
        if self.overflow {
            return None;
        }
        parse(&self.payload)
    }
}

/// The signal of an OSC payload (what lies between `ESC ]` and its terminator), if it is one.
pub fn parse(payload: &[u8]) -> Option<Signal> {
    let text = std::str::from_utf8(payload).ok()?;
    let (code, rest) = text.split_once(';')?;
    match code {
        "133" => shell_mark(rest),
        "9" => osc9(rest),
        "777" => {
            let (title, body) = rest.strip_prefix("notify;")?.split_once(';')?;
            notification(title, body)
        }
        _ => None,
    }
}

/// OSC 133's `C` and `D[;<exit>]`; its other marks are the prompt's, which xterm.js keeps.
fn shell_mark(rest: &str) -> Option<Signal> {
    let mut fields = rest.split(';');
    match fields.next()? {
        "C" => Some(Signal::CommandStarted),
        "D" => {
            let exit = fields.next().and_then(|code| code.parse().ok());
            Some(Signal::CommandFinished(exit))
        }
        _ => None,
    }
}

/// OSC 9: ConEmu's commands are a number first (`4;…` is progress, the others are ignored,
/// e.g. `9;<cwd>`); anything else is iTerm2's notification text.
fn osc9(rest: &str) -> Option<Signal> {
    let (command, args) = rest.split_once(';').unwrap_or((rest, ""));
    if command.is_empty() || !command.bytes().all(|b| b.is_ascii_digit()) {
        return notification("", rest);
    }
    if command != "4" {
        return None;
    }
    let mut args = args.split(';');
    let state = match args.next()? {
        "0" => ProgressState::None,
        "1" => ProgressState::Normal,
        "2" => ProgressState::Error,
        "3" => ProgressState::Indeterminate,
        "4" => ProgressState::Paused,
        _ => return None,
    };
    let value = args.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    // At most 100, so the cast keeps it.
    Some(Signal::Progress(state, value.min(100) as u8))
}

/// A notification, its text cleaned and cut; none when nothing is left of it.
fn notification(title: &str, body: &str) -> Option<Signal> {
    let title = adapter::clip(title, MAX_TITLE);
    let body = adapter::clip(body, MAX_BODY);
    (!title.is_empty() || !body.is_empty()).then_some(Signal::Notification { title, body })
}

/// What a terminal's signals amount to, once timed and spaced out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The progress changed: sent as it is, in order with the output.
    Progress(ProgressState, u8),
    /// Sent or not by [`message`], which needs the terminal's agents and the app's view.
    Alert(Alert),
}

/// A terminal's event that may alert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// A command ran for `took` and ended with `exit`.
    Finished {
        exit: Option<i32>,
        took: Duration,
    },
    Notification {
        title: String,
        body: String,
    },
}

/// One terminal's command and notification state.
#[derive(Debug, Default)]
pub struct Shell {
    /// When the running command started.
    started: Option<Instant>,
    /// When the last notification went out.
    noted: Option<Instant>,
    /// The progress last reported.
    progress: Option<(ProgressState, u8)>,
}

impl Shell {
    /// The events of one read's `signals` at `now`: its alerts, and only the last progress
    /// change (a program flooding progress makes one message a read, not one a sequence).
    pub fn read(&mut self, signals: Vec<Signal>, now: Instant) -> (Option<Control>, Vec<Alert>) {
        let (mut progress, mut alerts) = (None, Vec::new());
        for event in signals.into_iter().filter_map(|s| self.on(s, now)) {
            match event {
                Event::Progress(state, value) => {
                    progress = Some(Control::TerminalProgress { state, value });
                }
                Event::Alert(alert) => alerts.push(alert),
            }
        }
        (progress, alerts)
    }

    /// The event `signal` makes at `now`: a command's end (only after its start), a
    /// notification at least [`NOTE_GAP`] after the last, a progress that changed.
    pub fn on(&mut self, signal: Signal, now: Instant) -> Option<Event> {
        match signal {
            Signal::CommandStarted => {
                self.started = Some(now);
                None
            }
            Signal::CommandFinished(exit) => {
                let started = self.started.take()?;
                let took = now.saturating_duration_since(started);
                // Shorter than any setting alerts: nothing to ask about, so a program marking
                // commands in a loop makes at most one alert check a second.
                (took >= MIN_COMMAND).then_some(Event::Alert(Alert::Finished { exit, took }))
            }
            Signal::Notification { title, body } => {
                if self.noted.is_some_and(|at| now < at + NOTE_GAP) {
                    return None;
                }
                self.noted = Some(now);
                Some(Event::Alert(Alert::Notification { title, body }))
            }
            Signal::Progress(state, value) => {
                let progress = Some((state, value));
                if self.progress == progress {
                    return None;
                }
                self.progress = progress;
                Some(Event::Progress(state, value))
            }
        }
    }
}

/// The long-command rule: a command that ran at least `secs` (0: never) alerts, unless an agent
/// runs in its terminal (its own alerts say more) or the terminal is in view of the focused
/// window (`watched`: the user saw it end).
pub fn command_alerts(took: Duration, secs: u32, agent: bool, watched: bool) -> bool {
    secs > 0 && took >= Duration::from_secs(secs.into()) && !agent && !watched
}

/// What the app gets for `alert` in a terminal with an `agent` or none, `watched` or not, the
/// `notifications.command_secs` setting being `secs`: a long command's end ([`command_alerts`]);
/// a program's notification unless an agent runs there (Claude Code's own OSC 9 would repeat
/// the agent's alerts), an OS notification only out of view.
pub fn message(alert: Alert, secs: u32, agent: bool, watched: bool) -> Option<Control> {
    match alert {
        Alert::Finished { exit, took } => {
            let finished = Control::CommandFinished {
                exit,
                secs: took.as_secs(),
            };
            command_alerts(took, secs, agent, watched).then_some(finished)
        }
        Alert::Notification { title, body } => {
            let notify = !watched;
            (!agent).then_some(Control::TerminalNotification {
                title,
                body,
                notify,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str, body: &str) -> Signal {
        Signal::Notification {
            title: title.into(),
            body: body.into(),
        }
    }

    #[test]
    fn shell_marks_give_command_starts_and_ends_with_their_status() {
        let mut scanner = Scanner::default();
        let out = scanner.feed(b"$ \x1b]133;A\x07ls\x1b]133;C\x1b\\files\n\x1b]133;D;2\x07");
        assert_eq!(
            out,
            [Signal::CommandStarted, Signal::CommandFinished(Some(2))]
        );
        // fish's extra fields; no status, or one that is not a number.
        assert_eq!(parse(b"133;C;cmdline=ls"), Some(Signal::CommandStarted));
        assert_eq!(parse(b"133;D"), Some(Signal::CommandFinished(None)));
        assert_eq!(parse(b"133;D;x"), Some(Signal::CommandFinished(None)));
        assert_eq!(
            parse(b"133;D;-1;aid=3"),
            Some(Signal::CommandFinished(Some(-1)))
        );
        for prompt in ["133;A", "133;B", "133;", "133;CD", "133"] {
            assert_eq!(parse(prompt.as_bytes()), None, "{prompt}");
        }
    }

    #[test]
    fn osc_9_is_iterm2_text_or_conemu_progress_and_osc_777_a_titled_notification() {
        assert_eq!(parse(b"9;Build done"), Some(note("", "Build done")));
        assert_eq!(parse(b"9;a;b"), Some(note("", "a;b")));
        assert_eq!(parse(b"9;4x;y"), Some(note("", "4x;y")));
        assert_eq!(parse(b"777;notify;make;ok"), Some(note("make", "ok")));
        assert_eq!(parse(b"777;notify;;only body"), Some(note("", "only body")));
        assert_eq!(
            parse(b"777;notify;only title;"),
            Some(note("only title", ""))
        );
        assert_eq!(parse(b"777;notify;no body"), None);
        assert_eq!(parse(b"777;other;a;b"), None);
        let progress = |state, value| Some(Signal::Progress(state, value));
        assert_eq!(parse(b"9;4;1;40"), progress(ProgressState::Normal, 40));
        assert_eq!(parse(b"9;4;0"), progress(ProgressState::None, 0));
        assert_eq!(parse(b"9;4;2;100"), progress(ProgressState::Error, 100));
        assert_eq!(parse(b"9;4;3;"), progress(ProgressState::Indeterminate, 0));
        assert_eq!(parse(b"9;4;4;101"), progress(ProgressState::Paused, 100));
        assert_eq!(parse(b"9;4;1;-5"), progress(ProgressState::Normal, 0));
        // ConEmu's other commands (a cwd, a tab title, a message box) and broken progress.
        for ignored in [
            "9;9;/home/u",
            "9;2;hello",
            "9;12",
            "9;4",
            "9;4;5;1",
            "9;4;;1",
        ] {
            assert_eq!(parse(ignored.as_bytes()), None, "{ignored}");
        }
    }

    #[test]
    fn notification_text_is_cleaned_cut_and_never_empty() {
        let bidi = "9;\u{202E}evil\u{200B}\x01 text ";
        assert_eq!(parse(bidi.as_bytes()), Some(note("", "evil text")));
        let long = format!("777;notify;{};{}", "t".repeat(150), "b".repeat(400));
        let title = format!("{}…", "t".repeat(MAX_TITLE - 1));
        let body = format!("{}…", "b".repeat(MAX_BODY - 1));
        assert_eq!(parse(long.as_bytes()), Some(note(&title, &body)));
        for empty in ["9;", "9; \u{200B} ", "777;notify; ;\x07"] {
            assert_eq!(parse(empty.as_bytes()), None, "{empty:?}");
        }
        assert_eq!(parse(b"9;caf\xff"), None, "not UTF-8");
        assert_eq!(parse(b"0;window title"), None);
        assert_eq!(parse(b"9"), None);
    }

    #[test]
    fn sequences_split_across_reads_are_found_and_both_terminators_work() {
        let mut scanner = Scanner::default();
        let whole = b"a\x1b]9;hi\x1b\\b\x1b]9;4;1;5\x07c";
        let mut out = Vec::new();
        for byte in whole {
            out.extend(scanner.feed(std::slice::from_ref(byte)));
        }
        assert_eq!(
            out,
            [note("", "hi"), Signal::Progress(ProgressState::Normal, 5)]
        );
    }

    #[test]
    fn broken_sequences_are_dropped_and_what_follows_is_still_read() {
        let mut scanner = Scanner::default();
        // CAN and SUB cancel; another escape ends it unread and is read itself.
        let out = scanner.feed(b"\x1b]9;one\x18\x1b]9;two\x1a\x1b]9;three\x1b[0m\x1b]9;four\x07");
        assert_eq!(out, [note("", "four")]);
        // ESC ESC ] still starts one; ESC then anything else is not an OSC.
        let out = scanner.feed(b"\x1b\x1b]9;five\x07\x1bx]9;six\x07");
        assert_eq!(out, [note("", "five")]);
        // An ESC inside the payload followed by `]` starts a new one at once.
        let out = scanner.feed(b"\x1b]9;lost\x1b]9;seven\x07");
        assert_eq!(out, [note("", "seven")]);
        // A terminator with no sequence open is plain output.
        assert_eq!(scanner.feed(b"\x07\x1b\\x"), []);
    }

    #[test]
    fn oversized_sequences_are_dropped_whole_and_the_next_one_is_read() {
        let mut scanner = Scanner::default();
        let fits = format!("9;{}", "x".repeat(MAX_OSC - 2));
        let fit = scanner.feed(format!("\x1b]{fits}\x07").as_bytes());
        assert_eq!(fit.len(), 1);
        let over = format!("\x1b]9;{}\x07", "x".repeat(MAX_OSC - 1));
        assert_eq!(scanner.feed(over.as_bytes()), []);
        // Kept at the limit, whatever is sent.
        let huge = format!("\x1b]9;{}", "y".repeat(10 * MAX_OSC));
        assert_eq!(scanner.feed(huge.as_bytes()), []);
        assert!(scanner.payload.len() <= MAX_OSC);
        assert_eq!(scanner.feed(b"\x07\x1b]9;next\x07"), [note("", "next")]);
    }

    fn finished(exit: Option<i32>, secs: u64) -> Alert {
        let took = Duration::from_secs(secs);
        Alert::Finished { exit, took }
    }

    #[test]
    fn a_command_ends_only_after_it_started_and_takes_the_time_between() {
        let mut shell = Shell::default();
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        assert_eq!(shell.on(Signal::CommandFinished(Some(0)), t0), None);
        assert_eq!(shell.on(Signal::CommandStarted, at(1)), None);
        // A second start (a shell's and a program's own) restarts the clock.
        assert_eq!(shell.on(Signal::CommandStarted, at(2)), None);
        let end = shell.on(Signal::CommandFinished(Some(3)), at(14));
        assert_eq!(end, Some(Event::Alert(finished(Some(3), 12))));
        // The next prompt's end with no command run in between: nothing.
        assert_eq!(shell.on(Signal::CommandFinished(None), at(15)), None);
        // A command shorter than any setting alerts is not even asked about.
        let short = at(16) + MIN_COMMAND - Duration::from_millis(1);
        assert_eq!(shell.on(Signal::CommandStarted, at(16)), None);
        assert_eq!(shell.on(Signal::CommandFinished(Some(0)), short), None);
        assert_eq!(shell.on(Signal::CommandStarted, at(20)), None);
        let end = shell.on(Signal::CommandFinished(None), at(20) + MIN_COMMAND);
        assert_eq!(end, Some(Event::Alert(finished(None, 1))));
    }

    #[test]
    fn notifications_are_spaced_out_and_progress_sent_only_when_it_changes() {
        let mut shell = Shell::default();
        let t0 = Instant::now();
        let alert = |title: &str, body: &str| {
            let (title, body) = (title.into(), body.into());
            Some(Event::Alert(Alert::Notification { title, body }))
        };
        assert_eq!(shell.on(note("", "a"), t0), alert("", "a"));
        let soon = t0 + NOTE_GAP - Duration::from_millis(1);
        assert_eq!(shell.on(note("", "b"), soon), None);
        assert_eq!(shell.on(note("t", "c"), t0 + NOTE_GAP), alert("t", "c"));
        let progress = |state, value| Some(Event::Progress(state, value));
        let normal = |value| Signal::Progress(ProgressState::Normal, value);
        assert_eq!(shell.on(normal(5), t0), progress(ProgressState::Normal, 5));
        assert_eq!(shell.on(normal(5), t0), None);
        assert_eq!(shell.on(normal(6), t0), progress(ProgressState::Normal, 6));
        let error = shell.on(Signal::Progress(ProgressState::Error, 6), t0);
        assert_eq!(error, progress(ProgressState::Error, 6));
    }

    #[test]
    fn a_read_sends_its_last_progress_change_and_every_alert() {
        let mut shell = Shell::default();
        let t0 = Instant::now();
        let normal = |value| Signal::Progress(ProgressState::Normal, value);
        let mut flood: Vec<_> = (0..1000).map(|n| normal((n % 2) as u8)).collect();
        flood.push(note("", "a"));
        flood.push(normal(7));
        let (progress, alerts) = shell.read(flood, t0);
        let seven = Control::TerminalProgress {
            state: ProgressState::Normal,
            value: 7,
        };
        assert_eq!(progress, Some(seven));
        let a = Alert::Notification {
            title: "".into(),
            body: "a".into(),
        };
        assert_eq!(alerts, [a]);
        // Nothing changed: nothing to send.
        assert_eq!(shell.read(vec![normal(7)], t0), (None, vec![]));
    }

    #[test]
    fn a_long_command_alerts_only_with_no_agent_out_of_view() {
        let secs = Duration::from_secs;
        assert!(command_alerts(secs(10), 10, false, false));
        assert!(command_alerts(secs(600), 10, false, false));
        let short = secs(10) - Duration::from_millis(1);
        assert!(!command_alerts(short, 10, false, false));
        assert!(!command_alerts(secs(600), 0, false, false), "0: never");
        assert!(!command_alerts(secs(600), 10, true, false), "an agent");
        assert!(!command_alerts(secs(600), 10, false, true), "in view");
    }

    #[test]
    fn the_app_gets_long_commands_and_notifications_of_terminals_without_agents() {
        let end = Control::CommandFinished {
            exit: Some(1),
            secs: 12,
        };
        assert_eq!(message(finished(Some(1), 12), 10, false, false), Some(end));
        assert_eq!(message(finished(Some(1), 12), 10, true, false), None);
        assert_eq!(message(finished(Some(1), 9), 10, false, false), None);
        let note = || Alert::Notification {
            title: "t".into(),
            body: "b".into(),
        };
        let sent = |notify| {
            let (title, body) = ("t".into(), "b".into());
            Some(Control::TerminalNotification {
                title,
                body,
                notify,
            })
        };
        assert_eq!(message(note(), 0, false, false), sent(true));
        // In view: only the inbox.
        assert_eq!(message(note(), 0, false, true), sent(false));
        assert_eq!(message(note(), 0, true, false), None);
    }
}
