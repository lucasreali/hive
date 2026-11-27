//! An agent's background commands (28.8, `docs/spike/background-commands.md`): the
//! `run_in_background` Bash commands and `Monitor` watches it or its subagents started, from
//! their `PostToolUse`, corrected by Claude Code's `background_tasks` list on every
//! `Stop`/`SubagentStop`. No hook fires when one ends: it ends when it leaves that list
//! (`unknown`), or from Claude Code's task notice in the transcript ([`Commands::end`], read
//! once a recording gives the notice's shape, 28.8c).
//!
//! Every field comes from a hook payload, so it is untrusted: ids over 256 bytes are ignored,
//! text is cleaned of control and invisible characters and cut, and at most
//! [`MAX_COMMANDS`] are kept, so an `agent_state` stays far under the frame limit.

use std::time::{Duration, Instant};

use hive_protocol::{
    BackgroundAlert, BackgroundCommand, BackgroundEnd, BackgroundKind, BackgroundStatus,
};
use serde_json::Value;

use crate::adapter::clip;

/// Commands kept per agent: a new one past it replaces the oldest ended one, else is ignored.
pub const MAX_COMMANDS: usize = 32;
/// Longest command line kept, in characters, as Claude Code's own `background_tasks` cuts it.
pub const COMMAND_CHARS: usize = 1000;
/// Longest description kept, in characters.
pub const DESCRIPTION_CHARS: usize = 200;
/// Longest task id kept, in bytes; longer ones are ignored.
const MAX_ID: usize = 256;
/// How long an ended command stays listed (Q6), so a glance after its notification finds it.
pub const SHOWN_ENDED: Duration = Duration::from_secs(10);

/// One command, as listed, with what the service keeps to follow it.
#[derive(Debug)]
struct Entry {
    shown: BackgroundCommand,
    /// The `sent_ns` of the event it was first seen in: a `background_tasks` list sent before
    /// it cannot say it ended.
    seen_ns: u64,
    /// When it ended, on the monotonic clock: it is dropped [`SHOWN_ENDED`] later.
    ended_at: Option<Instant>,
}

/// A command that just ended, waiting to be sent as `background_finished`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub command: BackgroundCommand,
    pub status: BackgroundStatus,
    pub exit: Option<i32>,
    /// How long it ran, from when the service first saw it.
    pub secs: u64,
    pub alert: BackgroundAlert,
}

/// An agent's background commands, oldest first.
#[derive(Debug, Default)]
pub struct Commands {
    entries: Vec<Entry>,
    /// The `sent_ns` of the last `background_tasks` applied: an older list arrived late.
    last_list_ns: u64,
    /// Ended since last taken ([`Commands::take_ended`]).
    ended: Vec<Ended>,
}

impl Commands {
    /// The `PostToolUse` `raw` (`hive::slim`'s) that started a background command: a Bash
    /// `tool_response.backgroundTaskId` (its `tool_input.command` and `description` with it)
    /// or a `Monitor`'s `taskId`, by the subagent `by` when it is one's. A command already
    /// known (from a list that came first) gets what it lacked.
    pub fn launch(&mut self, raw: &Value, by: Option<&str>, sent_ns: u64, wall_ms: u64) {
        let (kind, key) = match raw.get("tool_name").and_then(Value::as_str) {
            Some("Bash") => (BackgroundKind::Shell, "backgroundTaskId"),
            Some("Monitor") => (BackgroundKind::Monitor, "taskId"),
            _ => return,
        };
        let Some(id) = raw
            .get("tool_response")
            .and_then(|r| r.get(key))
            .and_then(Value::as_str)
        else {
            return;
        };
        let input = raw.get("tool_input");
        let by = by.filter(|by| by.len() <= MAX_ID);
        let Some(entry) = self.entry(id, kind, sent_ns, wall_ms) else {
            return;
        };
        fill(&mut entry.shown, input);
        if entry.shown.by.is_none() {
            entry.shown.by = by.map(str::to_owned);
        }
    }

    /// Claude Code's list of the session's running background tasks (`background_tasks`, on
    /// a `Stop`/`SubagentStop` sent at `sent_ns`): a `shell` or `monitor` task not known yet
    /// is added from it (its subagent unknown), and a running command seen before the list
    /// that is not in it has ended (`unknown`, quiet). A list older than the last one applied
    /// is ignored.
    pub fn reconcile(&mut self, live: &[Value], sent_ns: u64, wall_ms: u64, now: Instant) {
        if sent_ns < self.last_list_ns {
            return;
        }
        self.last_list_ns = sent_ns;
        let mut listed = Vec::new();
        for task in live {
            let Some(id) = task.get("id").and_then(Value::as_str) else {
                continue;
            };
            listed.push(id);
            let kind = match task.get("type").and_then(Value::as_str) {
                Some("shell") => BackgroundKind::Shell,
                Some("monitor") => BackgroundKind::Monitor,
                _ => continue,
            };
            if let Some(entry) = self.entry(id, kind, sent_ns, wall_ms) {
                fill(&mut entry.shown, Some(task));
            }
        }
        let gone: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.ended_at.is_none() && e.seen_ns <= sent_ns)
            .map(|e| e.shown.id.clone())
            .filter(|id| !listed.contains(&id.as_str()))
            .collect();
        for id in gone {
            self.end(&id, BackgroundStatus::Unknown, None, wall_ms, now);
        }
    }

    /// The running command `id` ended with `status` and `exit` (Claude Code's task notice,
    /// or [`Self::reconcile`]); false when no running command has that id.
    pub fn end(
        &mut self,
        id: &str,
        status: BackgroundStatus,
        exit: Option<i32>,
        wall_ms: u64,
        now: Instant,
    ) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.shown.id == id && e.ended_at.is_none())
        else {
            return false;
        };
        entry.ended_at = Some(now);
        let alert = BackgroundAlert::of(status, exit);
        entry.shown.ended = Some(BackgroundEnd {
            status,
            exit,
            ms: wall_ms,
            alert,
        });
        self.ended.push(Ended {
            command: entry.shown.clone(),
            status,
            exit,
            secs: wall_ms.saturating_sub(entry.shown.since_ms) / 1000,
            alert,
        });
        true
    }

    /// Drops the commands that ended [`SHOWN_ENDED`] or more before `now`; whether any went.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| {
            e.ended_at
                .is_none_or(|at| now.saturating_duration_since(at) < SHOWN_ENDED)
        });
        self.entries.len() != before
    }

    /// Forgets every command, sending nothing: the session ended, and Claude Code stops its
    /// background tasks at exit.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.ended.clear();
    }

    /// The commands as `agent_state` lists them, oldest first.
    pub fn listed(&self) -> Vec<BackgroundCommand> {
        self.entries.iter().map(|e| e.shown.clone()).collect()
    }

    /// The commands that ended since the last call, in the order they ended.
    pub fn take_ended(&mut self) -> Vec<Ended> {
        std::mem::take(&mut self.ended)
    }

    /// The command `id`, added as `kind` when new and there is room; `None` for an id over
    /// [`MAX_ID`] bytes, or no room.
    fn entry(
        &mut self,
        id: &str,
        kind: BackgroundKind,
        sent_ns: u64,
        wall_ms: u64,
    ) -> Option<&mut Entry> {
        if id.len() > MAX_ID {
            return None;
        }
        if let Some(i) = self.entries.iter().position(|e| e.shown.id == id) {
            return self.entries.get_mut(i);
        }
        if self.entries.len() == MAX_COMMANDS {
            let ended = self.entries.iter().position(|e| e.ended_at.is_some())?;
            self.entries.remove(ended);
        }
        self.entries.push(Entry {
            shown: BackgroundCommand {
                id: id.to_owned(),
                kind,
                command: String::new(),
                description: None,
                by: None,
                since_ms: wall_ms,
                ended: None,
            },
            seen_ns: sent_ns,
            ended_at: None,
        });
        self.entries.last_mut()
    }
}

/// Fills what `shown` lacks of a payload's `command` and `description`, cleaned and cut.
fn fill(shown: &mut BackgroundCommand, from: Option<&Value>) {
    let text = |key: &str, most: usize| {
        let text = from?.get(key)?.as_str()?;
        Some(clip(text, most)).filter(|t| !t.is_empty())
    };
    if shown.command.is_empty()
        && let Some(command) = text("command", COMMAND_CHARS)
    {
        shown.command = command;
    }
    if shown.description.is_none() {
        shown.description = text("description", DESCRIPTION_CHARS);
    }
}

/// The exit code a task notice's summary names (`… completed (exit code 3)`): the last
/// `exit code <digits>`, with an optional `-`, ending at a character that is not a letter or
/// a digit; `None` when there is none or it does not fit an `i32`. The last, since the
/// command's description comes first in the summary.
pub fn exit_code(summary: &str) -> Option<i32> {
    const MARK: &str = "exit code ";
    let at = summary.rfind(MARK)?;
    let rest = &summary[at + MARK.len()..];
    let digits = rest.strip_prefix('-').unwrap_or(rest);
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let after = digits[end..].chars().next();
    if end == 0 || after.is_some_and(char::is_alphanumeric) {
        return None;
    }
    let sign = rest.len() - digits.len();
    rest[..sign + end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // SYNTHETIC: the payloads follow Claude Code's hooks reference (`background_tasks`, the
    // Bash `tool_input`) and the `backgroundTaskId` Hive already relies on (5.10); none is a
    // recording (`docs/spike/background-commands.md` §8).

    fn bash(id: &str, command: &str) -> Value {
        json!({
            "tool_name": "Bash",
            "tool_input": {"command": command, "description": "Run it", "run_in_background": true},
            "tool_response": {"backgroundTaskId": id},
        })
    }

    fn task(id: &str, kind: &str, command: &str) -> Value {
        json!({"id": id, "type": kind, "status": "running", "description": "d", "command": command})
    }

    fn ids(commands: &Commands) -> Vec<(String, Option<BackgroundStatus>)> {
        let listed = commands.listed().into_iter();
        listed.map(|c| (c.id, c.ended.map(|e| e.status))).collect()
    }

    #[test]
    fn a_launch_lists_the_command_with_its_text_and_who_started_it() {
        let mut commands = Commands::default();
        commands.launch(&bash("b1", "sleep 20"), None, 1, 5_000);
        commands.launch(&bash("b2", "make"), Some("sub"), 2, 6_000);
        let monitor = json!({"tool_name": "Monitor", "tool_response": {"taskId": "m1"}});
        commands.launch(&monitor, None, 3, 7_000);
        let listed = commands.listed();
        assert_eq!(
            listed[0],
            BackgroundCommand {
                id: "b1".into(),
                kind: BackgroundKind::Shell,
                command: "sleep 20".into(),
                description: Some("Run it".into()),
                by: None,
                since_ms: 5_000,
                ended: None,
            }
        );
        assert_eq!(listed[1].by.as_deref(), Some("sub"));
        assert_eq!(
            (listed[2].kind, listed[2].command.as_str()),
            (BackgroundKind::Monitor, "")
        );
        assert_eq!(listed[2].description, None);
        // A launch seen again changes nothing it already has.
        commands.launch(&bash("b1", "other"), None, 4, 9_000);
        commands.launch(&bash("b2", "other"), Some("x"), 4, 9_000);
        assert_eq!(commands.listed()[..2], listed[..2]);
    }

    #[test]
    fn only_bash_and_monitor_launches_with_their_own_id_count() {
        let mut commands = Commands::default();
        let cases = [
            json!({"tool_name": "Agent", "tool_response": {"agentId": "a"}}),
            json!({"tool_name": "Bash", "tool_response": {"taskId": "t"}}),
            json!({"tool_name": "Monitor", "tool_response": {"backgroundTaskId": "t"}}),
            json!({"tool_name": "Bash", "tool_response": {"backgroundTaskId": 3}}),
            json!({"tool_name": "Bash"}),
            json!({"tool_response": {"backgroundTaskId": "t"}}),
        ];
        for raw in &cases {
            commands.launch(raw, None, 1, 1);
        }
        assert!(commands.listed().is_empty());
    }

    #[test]
    fn text_is_cleaned_and_cut_and_long_ids_or_subagents_are_ignored() {
        let mut commands = Commands::default();
        let long = "x".repeat(COMMAND_CHARS + 5);
        let mut raw = bash("b1", &format!("\u{1b}[31m{long}"));
        raw["tool_input"]["description"] = json!("d".repeat(DESCRIPTION_CHARS + 1));
        commands.launch(&raw, Some(&"s".repeat(MAX_ID + 1)), 1, 1);
        let shown = &commands.listed()[0];
        assert_eq!(shown.command.chars().count(), COMMAND_CHARS);
        assert!(shown.command.starts_with("[31mxx") && shown.command.ends_with('…'));
        let description = shown.description.as_deref().unwrap();
        assert_eq!(description.chars().count(), DESCRIPTION_CHARS);
        assert_eq!(shown.by, None);
        commands.launch(&bash("b2", "ok"), Some(&"s".repeat(MAX_ID)), 1, 1);
        assert_eq!(
            commands.listed()[1].by.as_ref().map(String::len),
            Some(MAX_ID)
        );
        // An id too long is ignored; one at the limit is kept.
        commands.launch(&bash(&"i".repeat(MAX_ID + 1), "x"), None, 1, 1);
        commands.launch(&bash(&"i".repeat(MAX_ID), "x"), None, 1, 1);
        assert_eq!(commands.listed().len(), 3);
        // Blank text is no text.
        commands.launch(&bash("b3", " \u{7} "), None, 1, 1);
        assert_eq!(commands.listed()[3].command, "");
    }

    #[test]
    fn the_list_adds_shells_and_monitors_it_did_not_see_start_and_nothing_else() {
        let now = Instant::now();
        let mut commands = Commands::default();
        let live = [
            task("b1", "shell", "npm run dev"),
            task("m1", "monitor", "tail -f x"),
            task("a1", "subagent", ""),
            task("w1", "workflow", ""),
            json!({"type": "shell"}),
            json!({"id": "n1"}),
        ];
        commands.reconcile(&live, 5, 3_000, now);
        assert_eq!(ids(&commands), [("b1".into(), None), ("m1".into(), None)]);
        let shown = &commands.listed()[0];
        assert_eq!(
            (shown.command.as_str(), shown.by.as_ref()),
            ("npm run dev", None)
        );
        assert_eq!(
            (shown.since_ms, shown.description.as_deref()),
            (3_000, Some("d"))
        );
        // Its launch, late, says who started it and keeps the list's text.
        commands.launch(&bash("b1", "other"), Some("sub"), 4, 9_000);
        let shown = &commands.listed()[0];
        assert_eq!(
            (shown.command.as_str(), shown.by.as_deref()),
            ("npm run dev", Some("sub"))
        );
        // A list fills in a command its launch did not give.
        let monitor = json!({"tool_name": "Monitor", "tool_response": {"taskId": "m2"}});
        commands.launch(&monitor, None, 6, 1);
        commands.reconcile(&[task("m2", "monitor", "watch it")], 7, 1, now);
        assert_eq!(commands.listed()[2].command, "watch it");
        assert!(commands.take_ended().iter().all(|e| e.command.id != "m2"));
    }

    #[test]
    fn a_running_command_missing_from_a_later_list_ended_unknown_and_quiet() {
        let start = Instant::now();
        let mut commands = Commands::default();
        commands.launch(&bash("b1", "sleep 20"), None, 10, 1_000);
        commands.launch(&bash("b2", "sleep 30"), None, 11, 2_000);
        commands.reconcile(&[task("b2", "shell", "sleep 30")], 12, 21_500, start);
        assert_eq!(
            ids(&commands),
            [
                ("b1".into(), Some(BackgroundStatus::Unknown)),
                ("b2".into(), None)
            ]
        );
        let ended = commands.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].command.id, "b1");
        assert_eq!(
            (
                ended[0].status,
                ended[0].exit,
                ended[0].secs,
                ended[0].alert
            ),
            (BackgroundStatus::Unknown, None, 20, BackgroundAlert::Quiet)
        );
        assert_eq!(
            ended[0].command.ended,
            Some(BackgroundEnd {
                status: BackgroundStatus::Unknown,
                exit: None,
                ms: 21_500,
                alert: BackgroundAlert::Quiet,
            })
        );
        assert!(commands.take_ended().is_empty());
        // Ended once: a later list without it ends nothing again.
        commands.reconcile(&[], 13, 30_000, start);
        let again = commands.take_ended();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].command.id, "b2");
    }

    #[test]
    fn a_list_sent_before_a_launch_or_older_than_the_last_one_ends_nothing() {
        let now = Instant::now();
        let mut commands = Commands::default();
        commands.launch(&bash("b1", "x"), None, 20, 1);
        // Sent before the launch, read after it.
        commands.reconcile(&[], 19, 2, now);
        assert_eq!(ids(&commands), [("b1".into(), None)]);
        // A list at the launch's own moment, then one older than it, which adds nothing.
        commands.reconcile(&[task("b1", "shell", "x")], 30, 2, now);
        commands.reconcile(&[task("b9", "shell", "y")], 29, 2, now);
        assert_eq!(ids(&commands), [("b1".into(), None)]);
        commands.reconcile(&[], 30, 2, now);
        assert_eq!(commands.take_ended().len(), 1);
        // Seen in the same event as the list: that list ends it.
        let mut same = Commands::default();
        same.launch(&bash("b1", "x"), None, 7, 1);
        same.reconcile(&[], 7, 2, now);
        assert_eq!(same.take_ended().len(), 1);
    }

    #[test]
    fn a_notice_ends_a_running_command_with_its_status_and_exit() {
        let now = Instant::now();
        let mut commands = Commands::default();
        for id in ["c", "f", "s", "x"] {
            commands.launch(&bash(id, id), None, 1, 1_000);
        }
        let cases = [
            (
                "c",
                BackgroundStatus::Completed,
                Some(0),
                BackgroundAlert::Quiet,
            ),
            (
                "f",
                BackgroundStatus::Completed,
                Some(3),
                BackgroundAlert::Failed,
            ),
            ("s", BackgroundStatus::Stopped, None, BackgroundAlert::Quiet),
            ("x", BackgroundStatus::Failed, None, BackgroundAlert::Failed),
        ];
        for (id, status, exit, _) in cases {
            assert!(commands.end(id, status, exit, 4_999, now));
        }
        let ended = commands.take_ended();
        for ((id, status, exit, alert), ended) in cases.iter().zip(&ended) {
            assert_eq!(ended.command.id, *id);
            assert_eq!(
                (ended.status, ended.exit, ended.alert),
                (*status, *exit, *alert)
            );
            assert_eq!(ended.secs, 3);
        }
        // Ended already, or never known: nothing.
        assert!(!commands.end("c", BackgroundStatus::Failed, None, 1, now));
        assert!(!commands.end("zz", BackgroundStatus::Failed, None, 1, now));
        assert!(commands.take_ended().is_empty());
        // A clock that went back gives 0 s, not a wrap.
        commands.launch(&bash("late", "x"), None, 1, 9_000);
        commands.end("late", BackgroundStatus::Completed, None, 1_000, now);
        assert_eq!(commands.take_ended()[0].secs, 0);
    }

    #[test]
    fn ended_commands_stay_listed_ten_seconds() {
        let start = Instant::now();
        let mut commands = Commands::default();
        commands.launch(&bash("b1", "x"), None, 1, 1);
        commands.launch(&bash("b2", "y"), None, 1, 1);
        commands.end("b1", BackgroundStatus::Completed, Some(0), 2, start);
        assert!(!commands.expire(start + SHOWN_ENDED - Duration::from_millis(1)));
        assert_eq!(commands.listed().len(), 2);
        assert!(commands.expire(start + SHOWN_ENDED));
        assert_eq!(ids(&commands), [("b2".into(), None)]);
        // A running one never expires.
        assert!(!commands.expire(start + SHOWN_ENDED * 100));
    }

    #[test]
    fn at_most_32_are_kept_and_a_new_one_takes_the_oldest_ended_place() {
        let now = Instant::now();
        let mut commands = Commands::default();
        for i in 0..MAX_COMMANDS + 1 {
            commands.launch(&bash(&format!("b{i}"), "x"), None, 1, 1);
        }
        assert_eq!(commands.listed().len(), MAX_COMMANDS);
        assert!(
            commands
                .listed()
                .iter()
                .all(|c| c.id != format!("b{MAX_COMMANDS}"))
        );
        let live = [task("new", "shell", "y")];
        commands.reconcile(&live, 0, 1, now);
        assert!(commands.listed().iter().all(|c| c.id != "new"));
        commands.end("b3", BackgroundStatus::Completed, None, 1, now);
        commands.end("b5", BackgroundStatus::Completed, None, 1, now);
        commands.launch(&bash("late", "x"), None, 1, 1);
        let listed = commands.listed();
        assert_eq!(listed.len(), MAX_COMMANDS);
        assert_eq!(listed.last().map(|c| c.id.as_str()), Some("late"));
        assert!(listed.iter().all(|c| c.id != "b3"));
        assert!(listed.iter().any(|c| c.id == "b5"));
    }

    #[test]
    fn clear_forgets_every_command_and_what_ended() {
        let now = Instant::now();
        let mut commands = Commands::default();
        commands.launch(&bash("b1", "x"), None, 1, 1);
        commands.launch(&bash("b2", "x"), None, 1, 1);
        commands.end("b1", BackgroundStatus::Completed, None, 1, now);
        commands.clear();
        assert!(commands.listed().is_empty());
        assert!(commands.take_ended().is_empty());
    }

    #[test]
    fn the_exit_code_is_read_only_from_a_strict_pattern() {
        let cases = [
            ("Background command \"x\" completed (exit code 0)", Some(0)),
            ("Background command \"x\" completed (exit code 3)", Some(3)),
            ("Background command \"x\" failed (exit code -1)", Some(-1)),
            ("exit code 42", Some(42)),
            ("exit code 7.", Some(7)),
            ("exit code 2147483647", Some(i32::MAX)),
            ("exit code -2147483648", Some(i32::MIN)),
            ("Background command \"x\" completed", None),
            ("exit code ", None),
            ("exit code -", None),
            ("exit code x", None),
            ("exit code 3x", None),
            ("exit code 3é", None),
            ("exit code --3", None),
            ("exit code 2147483648", None),
            ("exit code 99999999999999999999999", None),
            ("exit code: 3", None),
            ("Exit Code 3", None),
            // The description comes first: the last one is the result's.
            (
                "Background command \"exit code 0\" completed (exit code 1)",
                Some(1),
            ),
            ("exit code 1 then exit code x", None),
            ("", None),
        ];
        for (summary, exit) in cases {
            assert_eq!(exit_code(summary), exit, "{summary}");
        }
    }
}
