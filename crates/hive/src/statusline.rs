//! `hive statusline` (12.1): Claude Code's `statusLine` in Hive's terminals (set by the
//! `--settings` the `claude` wrapper injects). It reports the 5-hour usage window of its input
//! to the service, then runs the user's own statusline with the same input and prints what that
//! prints, so what the user sees does not change. The service keeps the latest window per
//! Claude config folder ([`Usage`]).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use hive_protocol::{Control, SessionWindow};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::hook::MAX_INPUT;
use crate::paths::Paths;

/// Set for the user's statusline: a `hive statusline` it starts prints nothing and reports
/// nothing, so Hive's own statusline never runs itself.
const GUARD: &str = "HIVE_STATUSLINE";
/// How long the user's statusline may run.
pub const TIME_LIMIT: Duration = Duration::from_secs(10);
/// The most the user's statusline may print; past it, nothing is printed.
const OUTPUT_LIMIT: usize = 64 * 1024;
/// Largest settings file read for the user's `statusLine`.
const SETTINGS_LIMIT: u64 = 1024 * 1024;
/// Most Claude config folders the service keeps a window for; a new one past it is ignored.
const FOLDERS: usize = 64;
/// Longest Claude config folder the service keeps, in bytes.
const FOLDER_LIMIT: usize = 4096;

/// What `hive statusline` prints and its exit code: the user's statusline's, or nothing and 0.
pub type Output = (Vec<u8>, u8);

/// `hive statusline`: Claude Code's JSON on `input` (untrusted), the environment through
/// `var`, `cwd` for the project when the input names none. The user's statusline is dropped
/// (and killed) when `cancel` resolves: Claude Code cancelled this run.
pub async fn run(
    paths: &Paths,
    mut input: impl AsyncRead + Unpin,
    var: impl Fn(&str) -> Option<OsString>,
    cwd: Option<PathBuf>,
    cancel: impl Future<Output = ()>,
) -> Output {
    if var(GUARD).is_some() {
        return Output::default();
    }
    let mut head = Vec::new();
    // Past the limit it is not parsed, but the user's statusline still gets all of it.
    let limit = MAX_INPUT as u64 + 1;
    let _ = (&mut input).take(limit).read_to_end(&mut head).await;
    let parsed = (head.len() <= MAX_INPUT)
        .then(|| serde_json::from_slice::<Value>(&head).ok())
        .flatten();
    let claude_dir = crate::sessions::claude_dir(&var);
    let report = async {
        let usage = parsed.as_ref().and_then(five_hour);
        if let (Some(dir), Some(usage)) = (&claude_dir, usage) {
            let claude_dir = dir.to_string_lossy().into_owned();
            let message = Control::StatuslineUsage { claude_dir, usage };
            // Gives up after ~200 ms, like a hook: the statusline never waits for Hive.
            let _ = crate::hook::send(paths, 0, &message).await;
        }
    };
    let project = parsed.as_ref().and_then(project_dir).or(cwd);
    let command = user_command(claude_dir.as_deref(), project.as_deref());
    let user = async {
        let command = command?;
        tokio::select! {
            output = user(&command, head, input, TIME_LIMIT) => output,
            () = cancel => None,
        }
    };
    let ((), output) = tokio::join!(report, user);
    output.unwrap_or_default()
}

/// This process's environment variable `key`, for [`run`].
pub fn env(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

/// `rate_limits.five_hour` of Claude Code's statusline input, when whole: the percentage
/// rounded into 0–100.
fn five_hour(input: &Value) -> Option<SessionWindow> {
    let window = input.pointer("/rate_limits/five_hour")?;
    let used = window.get("used_percentage")?.as_f64()?;
    Some(SessionWindow {
        used_percentage: used.clamp(0.0, 100.0).round() as u8,
        resets_at: window.get("resets_at")?.as_u64()?,
    })
}

/// The folder Claude Code was started in, where its project settings are.
fn project_dir(input: &Value) -> Option<PathBuf> {
    let dir = ["/workspace/project_dir", "/cwd"]
        .iter()
        .find_map(|at| input.pointer(at)?.as_str())?;
    Some(dir.into())
}

/// The user's own statusline command, as Claude Code picks it without Hive's `--settings`:
/// `statusLine` from the user's settings (in `claude_dir`), overridden by the project's shared
/// settings, then by its local ones. Managed settings are left out: a `statusLine` there
/// overrides Hive's too, so this never runs.
fn user_command(claude_dir: Option<&Path>, project: Option<&Path>) -> Option<String> {
    let user = claude_dir.map(|dir| dir.join("settings.json"));
    let shared = project.map(|dir| dir.join(".claude/settings.json"));
    let local = project.map(|dir| dir.join(".claude/settings.local.json"));
    let mut files = [local, shared, user].into_iter().flatten();
    files.find_map(|file| command_in(&file))
}

/// The `statusLine` command of the settings file `file`, when it has one.
fn command_in(file: &Path) -> Option<String> {
    // Never a FIFO or a device (e.g. a link to `/dev/tty`): reading it could block for good.
    // ponytail: a swap between this check and the open needs write access to the folder.
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(file).ok()?;
    let text = crate::git::read_limited(&mut file, SETTINGS_LIMIT).ok()?;
    let settings: Value = serde_json::from_slice(&text).ok()?;
    let line = settings.get("statusLine")?;
    if line.get("type")? != "command" {
        return None;
    }
    Some(line.get("command")?.as_str()?.to_owned())
}

/// Runs the user's statusline `command` with `sh -c`, as Claude Code does, in this process's
/// folder and environment, its input being `head` then whatever is left of `rest`. Its stdout
/// and exit code, when it printed at most [`OUTPUT_LIMIT`] bytes and ended within `time`.
async fn user(
    command: &str,
    head: Vec<u8>,
    mut rest: impl AsyncRead + Unpin,
    time: Duration,
) -> Option<Output> {
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .env(GUARD, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Its own process group: cancelled or late, what it started ends with it.
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut group = Group(child.id());
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let feed = async move {
        // A statusline that does not read its input is fine.
        let _ = stdin.write_all(&head).await;
        let _ = tokio::io::copy(&mut rest, &mut stdin).await;
    };
    let read = async {
        let mut out = Vec::new();
        let limit = OUTPUT_LIMIT as u64 + 1;
        stdout.take(limit).read_to_end(&mut out).await.ok()?;
        (out.len() <= OUTPUT_LIMIT).then_some(out)
    };
    let run = async {
        let ((), out) = tokio::join!(feed, read);
        let out = out?;
        let status = child.wait().await.ok()?;
        Some((out, status.code().map_or(1, |code| code as u8)))
    };
    let output = tokio::time::timeout(time, run).await.ok().flatten();
    // Ended in time: what it left running in the background (e.g. a cache refresh) stays.
    if output.is_some() {
        group.0 = None;
    }
    output
}

/// Kills the process group led by its pid, when dropped with one.
struct Group(Option<u32>);

impl Drop for Group {
    fn drop(&mut self) {
        // A group with members keeps its id, so this cannot reach another process's group.
        if let Some(pid) = self.0.and_then(|pid| i32::try_from(pid).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
    }
}

/// The service's side: the latest 5-hour window of each Claude config folder (in memory only),
/// and the one the app has.
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
    use serde_json::json;

    fn window(used_percentage: u8, resets_at: u64) -> SessionWindow {
        SessionWindow {
            used_percentage,
            resets_at,
        }
    }

    #[test]
    fn the_five_hour_window_is_read_when_whole() {
        let input = |five_hour: Value| json!({ "rate_limits": { "five_hour": five_hour } });
        let at =
            |used: Value| five_hour(&input(json!({ "used_percentage": used, "resets_at": 9 })));
        assert_eq!(at(json!(41.5)), Some(window(42, 9)));
        assert_eq!(at(json!(12)), Some(window(12, 9)));
        assert_eq!(at(json!(-3)), Some(window(0, 9)));
        assert_eq!(at(json!(250.7)), Some(window(100, 9)));
        assert_eq!(at(json!("42")), None);
        for broken in [
            json!({ "used_percentage": 1 }),
            json!({ "resets_at": 9 }),
            json!({ "used_percentage": 1, "resets_at": -9 }),
            json!({ "used_percentage": 1, "resets_at": "9" }),
            json!(null),
        ] {
            assert_eq!(five_hour(&input(broken.clone())), None, "{broken}");
        }
        let seven_day =
            json!({ "rate_limits": { "seven_day": { "used_percentage": 1, "resets_at": 9 } } });
        assert_eq!(five_hour(&seven_day), None);
        assert_eq!(five_hour(&json!("text")), None);
    }

    #[test]
    fn the_environment_is_this_processs() {
        assert_eq!(env("PATH"), std::env::var_os("PATH"));
        assert_eq!(env("HIVE_NO_SUCH_VARIABLE"), None);
    }

    #[test]
    fn the_project_is_where_claude_started_else_its_cwd() {
        let both = json!({ "cwd": "/c", "workspace": { "project_dir": "/p" } });
        assert_eq!(project_dir(&both), Some("/p".into()));
        assert_eq!(project_dir(&json!({ "cwd": "/c" })), Some("/c".into()));
        assert_eq!(project_dir(&json!({ "cwd": 1 })), None);
    }

    /// A settings file whose `statusLine` runs `command`.
    fn settings(file: &Path, command: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let line = json!({ "statusLine": { "type": "command", "command": command, "padding": 1 } });
        std::fs::write(file, line.to_string()).unwrap();
    }

    #[test]
    fn the_local_project_settings_win_then_the_shared_ones_then_the_users() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, project) = (tmp.path().join("claude"), tmp.path().join("p"));
        let pick = || user_command(Some(&home), Some(&project));
        assert_eq!(pick(), None);
        settings(&home.join("settings.json"), "user");
        assert_eq!(pick().as_deref(), Some("user"));
        assert_eq!(user_command(None, None), None);
        settings(&project.join(".claude/settings.json"), "shared");
        assert_eq!(pick().as_deref(), Some("shared"));
        assert_eq!(user_command(Some(&home), None).as_deref(), Some("user"));
        settings(&project.join(".claude/settings.local.json"), "local");
        assert_eq!(pick().as_deref(), Some("local"));
        // A file without a usable `statusLine` does not hide a lower one.
        for text in [
            "not json",
            "{}",
            r#"{"statusLine": {"type": "static", "command": "x"}}"#,
            r#"{"statusLine": {"command": "x"}}"#,
            r#"{"statusLine": {"type": "command", "command": 1}}"#,
        ] {
            std::fs::write(project.join(".claude/settings.local.json"), text).unwrap();
            assert_eq!(pick().as_deref(), Some("shared"), "{text}");
        }
        // Nor a FIFO, which is not opened.
        let local = project.join(".claude/settings.local.json");
        std::fs::remove_file(&local).unwrap();
        let made = std::process::Command::new("mkfifo").arg(&local).status();
        assert!(made.unwrap().success());
        assert_eq!(pick().as_deref(), Some("shared"));
        std::fs::remove_file(&local).unwrap();
        // Up to 1 MiB is read, no more.
        let sized = |size: usize| {
            let text = r#"{"statusLine": {"type": "command", "command": "big"}}"#;
            text.to_owned() + &" ".repeat(size - text.len())
        };
        std::fs::write(&local, sized(1 << 20)).unwrap();
        assert_eq!(pick().as_deref(), Some("big"));
        std::fs::write(&local, sized((1 << 20) + 1)).unwrap();
        assert_eq!(pick().as_deref(), Some("shared"));
    }

    fn paths(tmp: &Path) -> Paths {
        Paths {
            runtime: tmp.join("run"),
            data: tmp.join("data"),
            config: tmp.join("config"),
        }
    }

    /// [`run`] with `claude_dir` as `CLAUDE_CONFIG_DIR` (and `guard` set when asked), no service.
    async fn statusline(claude_dir: &Path, input: &[u8], guard: bool) -> Output {
        let tmp = tempfile::tempdir().unwrap();
        let dir = claude_dir.as_os_str().to_owned();
        let var = |key: &str| match key {
            "CLAUDE_CONFIG_DIR" => Some(dir.clone()),
            GUARD if guard => Some("1".into()),
            _ => None,
        };
        let cwd = Some(tmp.path().to_owned());
        run(&paths(tmp.path()), input, var, cwd, std::future::pending()).await
    }

    #[tokio::test]
    async fn the_users_statusline_gets_the_same_input_and_its_output_passes_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        // No user statusline: nothing.
        assert_eq!(statusline(&claude, b"{}", false).await, (vec![], 0));
        settings(
            &claude.join("settings.json"),
            "printf 'in:'; cat; printf '\\n\\033[32mguard=%s\\n' \"$HIVE_STATUSLINE\"; exit 3",
        );
        let input = br#"{"rate_limits": "garbage"}"#;
        let expected = b"in:{\"rate_limits\": \"garbage\"}\n\x1b[32mguard=1\n".to_vec();
        assert_eq!(statusline(&claude, input, false).await, (expected, 3));
        // Started by a statusline of its own: nothing, whatever the settings say.
        assert_eq!(statusline(&claude, input, true).await, (vec![], 0));
    }

    #[tokio::test]
    async fn the_input_names_the_project_up_to_the_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let (claude, project) = (tmp.path().join("claude"), tmp.path().join("p"));
        settings(&claude.join("settings.json"), "printf user");
        settings(&project.join(".claude/settings.json"), "printf project");
        let json = json!({ "workspace": { "project_dir": project } }).to_string();
        // Exactly the limit, then one byte more (its first bytes alone still whole JSON).
        let mut input = json.clone() + &" ".repeat(MAX_INPUT - json.len());
        assert_eq!(
            statusline(&claude, input.as_bytes(), false).await.0,
            b"project"
        );
        input.push(' ');
        assert_eq!(
            statusline(&claude, input.as_bytes(), false).await.0,
            b"user"
        );
    }

    #[tokio::test]
    async fn an_oversized_input_is_not_read_but_still_given_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        settings(&claude.join("settings.json"), "wc -c");
        let input = vec![b'x'; MAX_INPUT + 10];
        let (out, code) = statusline(&claude, &input, false).await;
        assert_eq!(
            (String::from_utf8(out).unwrap().trim(), code),
            (&*(MAX_INPUT + 10).to_string(), 0)
        );
    }

    #[tokio::test]
    async fn the_users_statusline_is_bounded_in_output_and_time() {
        async fn run(command: &str, time: u64) -> Option<Output> {
            user(command, vec![], &b""[..], Duration::from_millis(time)).await
        }
        // 64 KiB at most.
        let out = run("head -c 65536 /dev/zero", 5000).await.unwrap();
        assert_eq!((out.0.len(), out.1), (65536, 0));
        let over = "head -c 65537 /dev/zero";
        assert_eq!(run(over, 5000).await, None);
        assert_eq!(run("sleep 5", 100).await, None);
        // Late, what it started ends too; in time, what it left in the background stays.
        let tmp = tempfile::tempdir().unwrap();
        let (late, kept) = (tmp.path().join("late"), tmp.path().join("kept"));
        let background =
            |file: &Path| format!("(sleep 0.3; touch '{}') >/dev/null &", file.display());
        assert_eq!(run(&format!("{} wait", background(&late)), 100).await, None);
        let done = run(&format!("{} printf ok", background(&kept)), 5000).await;
        assert_eq!(done, Some((b"ok".to_vec(), 0)));
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(!late.exists());
        assert!(kept.exists());
        // Killed by a signal: printed, exit 1.
        assert_eq!(
            run("printf x; kill -9 $$", 5000).await,
            Some((b"x".to_vec(), 1))
        );
        // A statusline that does not read a large input.
        let big = vec![b'x'; 1 << 20];
        assert_eq!(
            user("printf ok", big, &b""[..], Duration::from_secs(5)).await,
            Some((b"ok".to_vec(), 0))
        );
    }

    #[tokio::test]
    async fn a_cancelled_run_prints_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        settings(&claude.join("settings.json"), "sleep 5; echo late");
        let var = |key: &str| (key == "CLAUDE_CONFIG_DIR").then(|| claude.as_os_str().to_owned());
        let start = std::time::Instant::now();
        let out = run(&paths(tmp.path()), &b"{}"[..], var, None, async {}).await;
        assert_eq!(out, (vec![], 0));
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
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
