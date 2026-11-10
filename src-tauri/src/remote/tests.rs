use super::*;
use crate::{Hive, Term};

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Frame, FrameCodec};
use tauri::ipc::InvokeResponseBody;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn the_ssh_command_is_fixed_options_then_the_alias_then_a_constant_command() {
    let alias = Alias::new("gpu-box".into()).unwrap();
    let args = ssh_args(&alias, Script::Run);
    let expected = [
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "ConnectTimeout=15",
        "--",
        "gpu-box",
    ];
    assert_eq!(args.len(), expected.len() + 1);
    for (arg, expected) in args.iter().zip(expected) {
        assert_eq!(arg, expected);
    }
    assert_eq!(args[expected.len()], OsString::from(command(Script::Run)));
    // The user's config decides `ControlMaster` (Q12).
    assert!(!args
        .iter()
        .any(|arg| arg.to_string_lossy().contains("Control")));
    assert_eq!(
        ssh_args(&alias, Script::Upload)[15],
        OsString::from(command(Script::Upload))
    );
    for bad in ["-oProxyCommand=sh", "gpu box", "", "gpu\u{7}"] {
        assert!(Alias::new(bad.into()).is_err(), "{bad:?}");
    }
}

#[test]
fn each_remote_command_is_one_line_any_login_shell_reads_alike() {
    for script in [Script::Run, Script::Replace, Script::Upload] {
        let command = command(script);
        let (head, rest) = command.split_once(" -c '").unwrap();
        assert_eq!(head, "exec /bin/sh");
        let (text, tail) = rest.rsplit_once("' ").unwrap();
        // fish reads `\` in single quotes, csh `!` and new lines.
        for c in ['\'', '\\', '!', '\n', '\r'] {
            assert!(!text.contains(c), "{script:?} holds {c:?}");
        }
        let replace = if script == Script::Replace {
            " --replace"
        } else {
            ""
        };
        assert_eq!(tail, format!("hive {VERSION}{replace}"));
    }
    assert!(VERSION
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)));
    assert_ne!(RUN, UPLOAD);
}

#[test]
fn ssh_s_stderr_says_what_went_wrong() {
    let cases = [
        (
            "No ED25519 host key is known for gpu-box and you have requested strict checking.\r\nHost key verification failed.",
            Failure::HostKey,
        ),
        (
            "@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n@@@@@@@@@@@",
            Failure::HostKey,
        ),
        ("me@gpu-box: Permission denied (publickey).", Failure::Auth),
        ("hive-needs Linux x86_64", Failure::Needs(Arch::X86_64)),
        ("a banner\n  hive-needs Linux aarch64\n", Failure::Needs(Arch::Aarch64)),
        ("hive-needs Darwin arm64", Failure::Platform("Darwin arm64".into())),
        ("hive-needs FreeBSD amd64", Failure::Platform("FreeBSD amd64".into())),
        ("hive-needs Linux armv7l", Failure::Platform("Linux armv7l".into())),
        ("hive-needs Linux\u{1b}[2J x86_64", Failure::Platform("Linux [2J x86_64".into())),
        (
            "hive-upload: the uploaded checksum does not match",
            Failure::Install("the uploaded checksum does not match".into()),
        ),
        (
            "\n  \nTimeout, server gpu-box not responding.\nmore",
            Failure::Network("Timeout, server gpu-box not responding.".into()),
        ),
        ("", Failure::Network("the link closed".into())),
    ];
    for (text, failure) in cases {
        assert_eq!(classify(text), failure, "{text:?}");
    }
    // The `uname` text is cut to 64 characters.
    let long = format!("hive-needs Linux {}", "é".repeat(100));
    let Failure::Platform(shown) = classify(&long) else {
        panic!("a platform")
    };
    assert_eq!(shown, format!("Linux {}", "é".repeat(58)));
}

#[test]
fn each_failure_has_its_kind_message_and_retry() {
    let version = Failure::Version("0.1.0".into(), Some(3));
    let cases = [
        (
            Failure::HostKey,
            "host_key",
            "gpu's host key is not known (or has changed). Run `ssh gpu` once in a terminal to check and accept its key.".to_owned(),
        ),
        (
            Failure::Auth,
            "auth",
            "gpu asked for a password or passphrase; add the key to your ssh agent (`ssh-add --apple-use-keychain`).".to_owned(),
        ),
        (
            Failure::Needs(Arch::Aarch64),
            "install",
            format!("Installing Hive on gpu did not take: it still needs Hive {VERSION} for aarch64."),
        ),
        (
            Failure::Platform("Darwin arm64".into()),
            "platform",
            "gpu runs macOS; Hive connects only to Linux hosts.".to_owned(),
        ),
        (
            Failure::Platform("FreeBSD amd64".into()),
            "platform",
            "gpu runs FreeBSD amd64; Hive connects only to Linux x86_64 and aarch64 hosts."
                .to_owned(),
        ),
        (
            Failure::Install("no space".into()),
            "install",
            "Installing Hive on gpu failed: no space".to_owned(),
        ),
        (
            version,
            "version",
            format!("gpu runs Hive 0.1.0 with 3 terminals; this app is Hive {VERSION}."),
        ),
        (
            Failure::Version("0.1.0".into(), None),
            "version",
            format!("gpu runs Hive 0.1.0 with its terminals; this app is Hive {VERSION}."),
        ),
        (
            Failure::Network("No route to host".into()),
            "network",
            "Cannot reach gpu: No route to host".to_owned(),
        ),
    ];
    for (failure, kind, message) in cases {
        assert_eq!(failure.kind(), kind);
        assert_eq!(failure.message("gpu"), message);
        assert_eq!(failure.retries(), kind == "network", "{failure:?}");
    }
    assert_eq!(Arch::X86_64.build(), "hive-linux-x86_64");
    assert_eq!(Arch::Aarch64.build(), "hive-linux-aarch64");
}

#[test]
fn reconnects_climb_the_ladder_then_wait_30_s_only_while_terminals_are_kept() {
    let secs = |attempt, kept| backoff(attempt, kept).map(|wait| wait.as_secs());
    for kept in [false, true] {
        let ladder: Vec<_> = (0..5).map(|attempt| secs(attempt, kept)).collect();
        assert_eq!(ladder, [Some(1), Some(2), Some(5), Some(10), Some(30)]);
    }
    for attempt in [5, 6, 100] {
        assert_eq!(secs(attempt, true), Some(30));
        assert_eq!(secs(attempt, false), None);
    }
}

#[test]
fn an_upload_starts_with_its_byte_count_and_checksum() {
    let sum = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    assert_eq!(
        upload_input(b"hello"),
        format!("5 {sum}\nhello").into_bytes()
    );
    let id = app_id();
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    assert_ne!(id, app_id());
}

/// A folder for one test, removed on drop, with a fake `ssh` (`tests/fixtures/fake_ssh.c`), a
/// home, a `PATH` whose `uname` says Linux, and this app's Linux build: a script that answers
/// `--version` and, as `bridge`, pipes to the socket `s` (a test service).
#[cfg(unix)]
struct Lab(PathBuf);

#[cfg(unix)]
impl Lab {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("hive-ssh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["home", "bin", "builds"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_ssh.c");
        let ssh = root.join("ssh");
        let cc = std::process::Command::new("cc")
            .arg("-o")
            .arg(&ssh)
            .arg(source)
            .status();
        assert!(cc.unwrap().success());
        let lab = Self(root);
        lab.script(
            "bin/uname",
            "case \"$1\" in -s) echo Linux ;; -m) echo x86_64 ;; esac",
        );
        lab.build(VERSION);
        lab
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.0.join(sub)
    }

    fn script(&self, sub: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.path(sub);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// This app's x86_64 build, answering `--version` with `version`.
    fn build(&self, version: &str) {
        let lab = self.0.display();
        let body = format!(
            "echo \"$*\" >> {lab}/ran\ncase \"$1\" in --version) echo \"hive {version}\" ;; bridge) exec {lab}/ssh --pipe {lab}/s ;; esac"
        );
        self.script("builds/hive-linux-x86_64", &body);
    }

    /// The `PATH` the remote commands run with.
    fn remote_path(&self) -> String {
        format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            self.path("bin").display()
        )
    }

    /// What the fake `ssh` does, run after run.
    fn plan(&self, steps: &[String]) {
        std::fs::write(self.path("plan"), steps.join("\n") + "\n").unwrap();
    }

    /// `sh <home> <path>`: the remote command runs in `home`.
    fn sh(&self, home: &str) -> String {
        format!("sh {} {}", self.path(home).display(), self.remote_path())
    }

    fn pipe(&self) -> String {
        format!("pipe {}", self.path("s").display())
    }

    /// The arguments of each run of the fake `ssh`.
    fn runs(&self) -> Vec<Vec<String>> {
        let log = std::fs::read_to_string(self.path("log")).unwrap_or_default();
        let runs = log
            .lines()
            .map(|run| run.split('\u{1f}').map(str::to_owned).collect());
        runs.collect()
    }

    fn remotes(&self, ids: Arc<AtomicU32>) -> Remotes {
        Remotes::new(self.path("ssh").into(), self.path("builds"), ids)
    }

    fn listen(&self) -> tokio::net::UnixListener {
        tokio::net::UnixListener::bind(self.path("s")).unwrap()
    }
}

#[cfg(unix)]
impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `script`'s remote command as sshd would, with `/bin/sh` as the login shell, in the
/// lab's `home`, with `stdin`.
#[cfg(unix)]
fn remote(lab: &Lab, script: Script, home: &str, stdin: &[u8]) -> std::process::Output {
    use std::io::Write as _;
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command(script))
        .env_clear()
        .env("HOME", lab.path(home))
        .env("PATH", lab.remote_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn the_check_runs_a_bridge_of_this_version_only_else_names_the_platform() {
    let lab = Lab::new("check");
    let needs = remote(&lab, Script::Run, "home", b"");
    assert_eq!(needs.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&needs.stderr),
        "hive-needs Linux x86_64\n"
    );
    assert!(needs.stdout.is_empty());
    // A `hive` of another version is not run as a bridge.
    let bin = lab.path("home/.hive/bin");
    std::fs::create_dir_all(&bin).unwrap();
    lab.build("0.0.1");
    std::fs::copy(lab.path("builds/hive-linux-x86_64"), bin.join("hive")).unwrap();
    assert_eq!(
        remote(&lab, Script::Run, "home", b"").status.code(),
        Some(3)
    );
    let ran = std::fs::read_to_string(lab.path("ran")).unwrap();
    assert_eq!(ran, "--version\n");
    // This version's runs `bridge --remote`, with `--replace` to restart the service.
    lab.build(VERSION);
    std::fs::copy(lab.path("builds/hive-linux-x86_64"), bin.join("hive")).unwrap();
    std::fs::remove_file(lab.path("ran")).unwrap();
    remote(&lab, Script::Run, "home", b"");
    remote(&lab, Script::Replace, "home", b"");
    let ran = std::fs::read_to_string(lab.path("ran")).unwrap();
    let bridges = "--version\nbridge --remote\n--version\nbridge --remote --replace\n";
    assert_eq!(ran, bridges);
}

#[cfg(unix)]
#[test]
fn an_upload_is_checked_before_it_runs_and_renamed_only_once_it_runs() {
    let lab = Lab::new("upload");
    let build = std::fs::read(lab.path("builds/hive-linux-x86_64")).unwrap();
    let hive = lab.path("home/.hive/bin/hive");
    let upload = |input: &[u8]| remote(&lab, Script::Upload, "home", input);
    let failed = |out: std::process::Output, why: &str| {
        assert_eq!(out.status.code(), Some(4));
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(stderr, format!("hive-upload: {why}\n"));
        assert!(!hive.exists());
        assert!(!lab.path("home/.hive/bin/hive.new").exists());
    };
    // A short upload, or one whose checksum differs, never runs.
    let input = upload_input(&build);
    failed(
        upload(&input[..input.len() - 1]),
        "the uploaded size does not match",
    );
    let mut changed = input.clone();
    *changed.last_mut().unwrap() ^= 1;
    failed(upload(&changed), "the uploaded checksum does not match");
    assert!(!lab.path("ran").exists());
    // One that does not answer this version is not installed.
    lab.build("0.0.1");
    let other = std::fs::read(lab.path("builds/hive-linux-x86_64")).unwrap();
    failed(
        upload(&upload_input(&other)),
        "the uploaded hive does not run",
    );
    let out = upload(&input);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(std::fs::read(&hive).unwrap(), build);
    assert_eq!(mode(&hive), 0o755);
    assert_eq!(mode(&lab.path("home/.hive/bin")), 0o700);
    // Without the header line nothing is written.
    assert_eq!(upload(b"").status.code(), Some(1));
}

/// A UI channel and what it received.
fn ui() -> (Channel<Value>, mpsc::UnboundedReceiver<Value>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("control messages are JSON")
        };
        let _ = tx.send(serde_json::from_str(&json).unwrap());
        Ok(())
    });
    (channel, rx)
}

/// A terminal output channel and the bytes it received.
fn output() -> (
    Channel<InvokeResponseBody>,
    mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Raw(bytes) = body else {
            panic!("terminal output is raw bytes")
        };
        let _ = tx.send(bytes);
        Ok(())
    });
    (channel, rx)
}

/// The UI messages up to the first `host_link` in `state`, which is last.
async fn until(rx: &mut mpsc::UnboundedReceiver<Value>, state: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    loop {
        let message = tokio::time::timeout(WAIT, rx.recv())
            .await
            .unwrap()
            .unwrap();
        let done = message["type"] == "host_link" && message["state"] == state;
        seen.push(message);
        if done {
            return seen;
        }
    }
}

/// The next UI message of type `kind`, past any other.
async fn next_of(rx: &mut mpsc::UnboundedReceiver<Value>, kind: &str) -> Value {
    loop {
        let message = tokio::time::timeout(WAIT, rx.recv())
            .await
            .unwrap()
            .unwrap();
        if message["type"] == kind {
            return message;
        }
    }
}

/// The remote's service end of a link: the test service the fake `ssh` pipes to.
struct Service {
    reader: FramedRead<tokio::net::unix::OwnedReadHalf, FrameCodec>,
    writer: FramedWrite<tokio::net::unix::OwnedWriteHalf, FrameCodec>,
}

impl Service {
    async fn accept(listener: &tokio::net::UnixListener) -> Self {
        let (stream, _) = tokio::time::timeout(WAIT, listener.accept())
            .await
            .unwrap()
            .unwrap();
        let (read, write) = stream.into_split();
        Self {
            reader: FramedRead::new(read, FrameCodec),
            writer: FramedWrite::new(write, FrameCodec),
        }
    }

    async fn control(&mut self) -> (u32, Control) {
        let frame = tokio::time::timeout(WAIT, self.reader.next())
            .await
            .unwrap();
        let frame = frame.unwrap().unwrap();
        (frame.channel, frame.to_control().unwrap())
    }

    async fn send(&mut self, channel: u32, message: Control) {
        self.writer
            .send(Frame::control(channel, &message))
            .await
            .unwrap();
    }

    /// Takes the `hello` and welcomes it; the app asks for the projects next.
    async fn welcome(&mut self, keep_secs: u64) -> String {
        let (_, hello) = self.control().await;
        let Control::Hello {
            version,
            role,
            keep_secs: keep,
            app_id: Some(app_id),
            ..
        } = hello
        else {
            panic!("a remote hello: {hello:?}")
        };
        assert_eq!(
            (version.as_str(), role, keep),
            (VERSION, Role::App, Some(keep_secs))
        );
        let welcome = Control::Welcome {
            version: VERSION.into(),
            distro: None,
            warning: None,
        };
        self.send(0, welcome).await;
        assert_eq!(self.control().await, (0, Control::ListProjects));
        app_id
    }
}

fn settings(hosts: &[&str], keep_minutes: u32) -> RemoteSettings {
    RemoteSettings {
        hosts: hosts.iter().map(|host| (*host).to_owned()).collect(),
        keep_minutes,
    }
}

/// The states the UI got for the host, in order.
fn states(messages: &[Value]) -> Vec<String> {
    let links = messages.iter().filter(|m| m["type"] == "host_link");
    links
        .map(|m| m["state"].as_str().unwrap().to_owned())
        .collect()
}

/// A fake clock: each wait is recorded, and over at once.
fn fake_clock(remotes: &mut Remotes) -> mpsc::UnboundedReceiver<Duration> {
    let (tx, rx) = mpsc::unbounded_channel();
    remotes.sleep = Box::new(move |wait| {
        let _ = tx.send(wait);
        Box::pin(async {})
    });
    rx
}

#[cfg(unix)]
#[tokio::test]
async fn a_new_host_gets_this_app_s_hive_then_links_with_its_keep_time_and_app_id() {
    let lab = Lab::new("install");
    let listener = lab.listen();
    let remotes = Arc::new(lab.remotes(Arc::default()));
    lab.plan(&[lab.sh("home")]);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 90), Some(&channel));
    let mut service = Service::accept(&listener).await;
    let app_id = service.welcome(90 * 60).await;
    assert_eq!(app_id, remotes.app_id);
    let seen = until(&mut rx, "connected").await;
    assert_eq!(
        states(&seen),
        ["connecting", "installing", "connecting", "connected"]
    );
    assert_eq!(
        seen[1],
        json!({"type": "host_link", "host": "gpu", "state": "installing", "cpu": "x86_64"})
    );
    // The service's messages reach the UI with the host.
    let welcome = rx.recv().await.unwrap();
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["host"], "gpu");
    // A check, the upload, then the check that runs the bridge.
    let runs = lab.runs();
    let commands: Vec<_> = runs.iter().map(|run| run.last().unwrap().clone()).collect();
    let [run, upload, again] = &commands[..] else {
        panic!("{runs:?}")
    };
    assert_eq!(
        (run, upload, again),
        (&command(Script::Run), &command(Script::Upload), run)
    );
    assert!(runs.iter().all(|run| run[13..15] == ["--", "gpu"]));
    let hive = lab.path("home/.hive/bin/hive");
    assert_eq!(
        std::fs::read(hive).unwrap(),
        std::fs::read(lab.path("builds/hive-linux-x86_64")).unwrap()
    );
    remotes.end();
}

#[cfg(unix)]
#[tokio::test]
async fn failures_that_will_not_pass_stop_with_their_reason_and_nothing_uploaded() {
    let lab = Lab::new("fatal");
    let cases = [
        ("stderr Host key verification failed.", "host_key"),
        (
            "stderr me@gpu: Permission denied (publickey,password).",
            "auth",
        ),
        ("stderr hive-needs Darwin arm64", "platform"),
    ];
    for (step, kind) in cases {
        let mut remotes = lab.remotes(Arc::default());
        let mut clock = fake_clock(&mut remotes);
        let remotes = Arc::new(remotes);
        std::fs::remove_file(lab.path("log")).ok();
        lab.plan(&[step.to_owned()]);
        let (channel, mut rx) = ui();
        remotes.sync(settings(&["gpu"], 1), Some(&channel));
        let seen = until(&mut rx, "disconnected").await;
        let stopped = seen.last().unwrap();
        assert_eq!(stopped["kind"], kind);
        assert_eq!(stopped["reason"], classify(&step[7..]).message("gpu"));
        // Not tried again, not even on wake; only Reconnect tries again.
        remotes.wake();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(lab.runs().len(), 1, "{kind}");
        assert!(clock.try_recv().is_err());
        remotes.reconnect("gpu").unwrap();
        until(&mut rx, "disconnected").await;
        assert_eq!(lab.runs().len(), 2, "{kind}");
        remotes.end();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_host_that_cannot_be_reached_is_retried_on_the_ladder_then_on_wake() {
    let lab = Lab::new("ladder");
    let mut remotes = lab.remotes(Arc::default());
    let mut clock = fake_clock(&mut remotes);
    let remotes = Arc::new(remotes);
    lab.plan(&["stderr ssh: connect to host gpu port 22: Connection refused".to_owned()]);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    let seen = until(&mut rx, "disconnected").await;
    let waits: Vec<_> = seen
        .iter()
        .filter_map(|m| m["retry_secs"].as_u64())
        .collect();
    assert_eq!(waits, [1, 2, 5, 10, 30]);
    let mut slept = Vec::new();
    while let Ok(wait) = clock.try_recv() {
        slept.push(wait.as_secs());
    }
    assert_eq!(slept, [1, 2, 5, 10, 30]);
    let reconnecting = &seen[1];
    let reason = "Cannot reach gpu: ssh: connect to host gpu port 22: Connection refused";
    assert_eq!(reconnecting["reason"], reason);
    assert_eq!(seen.last().unwrap()["kind"], "network");
    assert_eq!(lab.runs().len(), 6);
    // Waking tries again, from the ladder's start.
    remotes.wake();
    until(&mut rx, "disconnected").await;
    assert_eq!(lab.runs().len(), 12);
    remotes.end();
}

#[cfg(unix)]
#[tokio::test]
async fn a_wake_ends_the_wait_on_the_real_clock() {
    let lab = Lab::new("wake");
    let remotes = Arc::new(lab.remotes(Arc::default()));
    lab.plan(&["stderr Connection reset by peer".to_owned()]);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    until(&mut rx, "reconnecting").await;
    // The first wait is 1 s; a wake cuts it short. (The ladder waits 2 s next.)
    remotes.wake();
    until(&mut rx, "connecting").await;
    let seen = until(&mut rx, "reconnecting").await;
    assert_eq!(seen.last().unwrap()["retry_secs"], 2);
    remotes.end();
}

#[cfg(unix)]
#[tokio::test]
async fn a_dropped_link_keeps_its_terminals_and_retries_every_30_s_while_it_does() {
    let lab = Lab::new("kept");
    let listener = lab.listen();
    let mut remotes = lab.remotes(Arc::default());
    let mut clock = fake_clock(&mut remotes);
    let remotes = Arc::new(remotes);
    let network = "stderr Timeout, server gpu not responding.".to_owned();
    lab.plan(&[
        lab.pipe(),
        network.clone(),
        network.clone(),
        network.clone(),
        network.clone(),
        network.clone(),
        network.clone(),
        lab.pipe(),
    ]);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    let mut service = Service::accept(&listener).await;
    service.welcome(60).await;
    until(&mut rx, "connected").await;
    let link = remotes.link("gpu").unwrap();
    let (output, _bytes) = output();
    let host = crate::Host {
        link: Arc::clone(&link),
    };
    let id = host
        .open_terminal("/w".into(), 80, 24, None, output)
        .unwrap();
    assert_eq!(service.control().await.0, id);
    drop(service);
    // Seven tries on the ladder and past it, each failing, then the eighth links again.
    let mut service = Service::accept(&listener).await;
    let mut slept = Vec::new();
    while let Ok(wait) = clock.try_recv() {
        slept.push(wait.as_secs());
    }
    assert_eq!(slept, [1, 2, 5, 10, 30, 30, 30]);
    assert!(lock(&link).terminals.contains_key(&id));
    service.welcome(60).await;
    until(&mut rx, "connected").await;
    // Reattaching is 26.16's: until then the kept terminal is told ended.
    let exited = next_of(&mut rx, "terminal_exited").await;
    assert_eq!(
        exited,
        json!({"type": "terminal_exited", "channel": id, "code": null, "host": "gpu"})
    );
    assert!(lock(&link).terminals.is_empty());
    remotes.end();
}

#[cfg(unix)]
#[tokio::test]
async fn a_service_of_another_version_is_restarted_only_when_asked() {
    let lab = Lab::new("version");
    let listener = lab.listen();
    let remotes = Arc::new(lab.remotes(Arc::default()));
    lab.plan(&[lab.pipe()]);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    let mut service = Service::accept(&listener).await;
    service.control().await;
    let refused = Control::VersionMismatch {
        protocol: PROTOCOL_VERSION,
        version: "0.1.0".into(),
        terminals: Some(2),
    };
    service.send(0, refused).await;
    let seen = until(&mut rx, "disconnected").await;
    // The UI never gets the local service's mismatch dialog for a remote.
    assert!(seen.iter().all(|m| m["type"] == "host_link"), "{seen:?}");
    let stopped = seen.last().unwrap();
    assert_eq!(stopped["kind"], "version");
    assert_eq!(stopped["version"], "0.1.0");
    assert_eq!(stopped["app_version"], VERSION);
    assert_eq!(stopped["terminals"], 2);
    // "Restart the remote service": the next try replaces it.
    remotes.restart("gpu").unwrap();
    let mut service = Service::accept(&listener).await;
    service.welcome(60).await;
    until(&mut rx, "connected").await;
    let runs = lab.runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].last().unwrap(), &command(Script::Run));
    assert_eq!(runs[1].last().unwrap(), &command(Script::Replace));
    assert_eq!(
        remotes.restart("other"),
        Err("no link to the host other".into())
    );
    remotes.end();
}

#[cfg(unix)]
#[tokio::test]
async fn installs_that_fail_say_why_and_stop() {
    let lab = Lab::new("install-fails");
    let cases: [(&[String], &str); 4] = [
        // The upload fails on the host.
        (
            &[
                lab.sh("home"),
                "stderr hive-upload: the uploaded checksum does not match".into(),
            ],
            "Installing Hive on gpu failed: the uploaded checksum does not match",
        ),
        // The upload's link fails.
        (
            &[
                lab.sh("home"),
                "stderr Connection closed by 10.0.0.2 port 22".into(),
            ],
            "Installing Hive on gpu failed: Connection closed by 10.0.0.2 port 22",
        ),
        (
            &[
                lab.sh("home"),
                "stderr Host key verification failed.".into(),
            ],
            &Failure::HostKey.message("gpu"),
        ),
        // It went to another home, so the host still needs it.
        (
            &[lab.sh("home"), lab.sh("other"), lab.sh("home")],
            &Failure::Needs(Arch::X86_64).message("gpu"),
        ),
    ];
    std::fs::create_dir_all(lab.path("other")).unwrap();
    for (plan, reason) in cases {
        let remotes = Arc::new(lab.remotes(Arc::default()));
        std::fs::remove_file(lab.path("log")).ok();
        lab.plan(plan);
        let (channel, mut rx) = ui();
        remotes.sync(settings(&["gpu"], 1), Some(&channel));
        let seen = until(&mut rx, "disconnected").await;
        assert_eq!(seen.last().unwrap()["reason"], *reason);
        remotes.end();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn an_upload_without_a_build_or_ssh_or_time_fails() {
    let lab = Lab::new("upload-fails");
    let gpu = Alias::new("gpu".into()).unwrap();
    std::fs::remove_file(lab.path("builds/hive-linux-x86_64")).unwrap();
    let remotes = lab.remotes(Arc::default());
    let Err(Failure::Install(why)) = remotes.upload(&gpu, Arch::X86_64).await else {
        panic!("no build")
    };
    assert!(
        why.starts_with("this app has no hive-linux-x86_64: "),
        "{why}"
    );
    lab.build(VERSION);
    let mut remotes = Remotes::new(
        "/nonexistent/ssh".into(),
        lab.path("builds"),
        Arc::default(),
    );
    let Err(Failure::Network(why)) = remotes.upload(&gpu, Arch::X86_64).await else {
        panic!("no ssh")
    };
    assert!(why.starts_with("cannot start ssh: "), "{why}");
    remotes = lab.remotes(Arc::default());
    remotes.upload_wait = Duration::from_millis(200);
    lab.plan(&["hang".to_owned()]);
    let failed = remotes.upload(&gpu, Arch::X86_64).await;
    assert_eq!(
        failed,
        Err(Failure::Install("the upload took too long".into()))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn without_ssh_or_with_a_broken_stream_the_link_retries() {
    let lab = Lab::new("broken");
    let mut remotes = Remotes::new(
        "/nonexistent/ssh".into(),
        lab.path("builds"),
        Arc::default(),
    );
    let _clock = fake_clock(&mut remotes);
    let remotes = Arc::new(remotes);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    let seen = until(&mut rx, "reconnecting").await;
    let reason = seen.last().unwrap()["reason"].as_str().unwrap().to_owned();
    assert!(
        reason.starts_with("Cannot reach gpu: cannot start ssh: "),
        "{reason}"
    );
    remotes.end();
    // A stream that breaks the protocol (a shell's startup file printing on stdout).
    let listener = lab.listen();
    let remotes = Arc::new(lab.remotes(Arc::default()));
    lab.plan(&[lab.pipe()]);
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    let mut service = Service::accept(&listener).await;
    service.control().await;
    use tokio::io::AsyncWriteExt as _;
    service
        .writer
        .get_mut()
        .write_all(b"Welcome to gpu!\n")
        .await
        .unwrap();
    let seen = until(&mut rx, "reconnecting").await;
    let reason = seen.last().unwrap()["reason"].as_str().unwrap().to_owned();
    assert!(reason.starts_with("Cannot reach gpu: "), "{reason}");
    remotes.end();
}

/// A control frame of `message`, any JSON.
fn raw(message: &Value) -> Frame {
    Frame {
        kind: hive_protocol::FrameType::Control,
        channel: 0,
        payload: message.to_string().into_bytes().into(),
    }
}

/// A `Hive` linked to the remote hosts of `lab`, its local link up (the test service `local`).
#[cfg(unix)]
fn linked(lab: &Lab) -> (Hive, crate::tests::Service, mpsc::UnboundedReceiver<Value>) {
    let hive = Hive::new("/nonexistent/hive-test".into(), vec![])
        .with_remote(lab.path("ssh").into(), lab.path("builds"));
    let (channel, rx) = ui();
    hive.link().ui = Some(channel);
    let service = crate::tests::attach(&hive, "bridge gone");
    (hive, service, rx)
}

#[cfg(unix)]
#[tokio::test]
async fn the_local_settings_link_the_hosts_and_requests_go_to_each_host_s_link() {
    let lab = Lab::new("hosts");
    let listener = lab.listen();
    lab.plan(&[lab.pipe()]);
    let (hive, mut local, mut rx) = linked(&lab);
    local.control().await;
    let mut settings = hive_protocol::Settings::default();
    settings.remote = self::settings(&["gpu", "-bad"], 5);
    local.send(0, Control::Settings { settings }).await;
    let mut remote = Service::accept(&listener).await;
    remote.welcome(300).await;
    until(&mut rx, "connected").await;
    // A request names its host; none is the local service.
    hive.on(Some("gpu")).unwrap().list_sessions().unwrap();
    assert_eq!(remote.control().await, (0, Control::ListSessions));
    hive.on(None).unwrap().list_sessions().unwrap();
    assert_eq!(local.control().await, (0, Control::ListSessions));
    let bad = hive.on(Some("-bad")).err();
    assert_eq!(bad, Some("no link to the host -bad".into()));
    // Terminal ids never clash: each link's next terminal takes the next id of all.
    let (out_local, _) = output();
    let (out_remote, mut bytes) = output();
    let on_local = hive.on(None).unwrap();
    let one = on_local.open_terminal("/l".into(), 80, 24, None, out_local);
    let on_gpu = hive.on(Some("gpu")).unwrap();
    let two = on_gpu.open_terminal("/r".into(), 80, 24, None, out_remote);
    assert_eq!((one, two), (Ok(1), Ok(2)));
    assert_eq!(local.control().await.0, 1);
    assert_eq!(remote.control().await.0, 2);
    hive.write_terminal(2, "ls\n").unwrap();
    hive.resize_terminal(2, 100, 30).unwrap();
    hive.ack_terminal(2, 10).unwrap();
    let frame = tokio::time::timeout(WAIT, remote.reader.next()).await;
    let frame = frame.unwrap().unwrap().unwrap();
    assert_eq!((frame.channel, &frame.payload[..]), (2, &b"ls\n"[..]));
    let resize = Control::Resize {
        cols: 100,
        rows: 30,
    };
    assert_eq!(remote.control().await, (2, resize));
    assert_eq!(remote.control().await, (2, Control::Ack { bytes: 10 }));
    // The remote's terminal messages and output reach that terminal, with the host.
    let opened = Control::TerminalOpened {
        worktree: None,
        account: "Default".into(),
    };
    remote.send(2, opened).await;
    remote
        .writer
        .send(Frame::terminal(2, b"$ ".to_vec()))
        .await
        .unwrap();
    let opened = next_of(&mut rx, "terminal_opened").await;
    assert_eq!(opened["channel"], 2);
    assert_eq!(opened["host"], "gpu");
    assert_eq!(bytes.recv().await.unwrap(), b"$ ");
    hive.attach_terminal(2, output().0).unwrap();
    assert_eq!(
        remote.control().await,
        (2, Control::Ack { bytes: u32::MAX })
    );
    hive.close_terminal(2).unwrap();
    assert_eq!(remote.control().await, (2, Control::CloseTerminal));
    // A remote's path never opens here; its files get no `hive-file:` URL.
    let target = json!({"type": "editor_target", "worktree": "/r", "path": "a", "windows_path": "/etc/passwd", "reveal": false});
    remote.writer.send(raw(&target)).await.unwrap();
    let media = json!({"token": "t", "mime": "image/png"});
    let file = json!({"type": "file", "worktree": "/r", "path": "a.png", "media": media});
    remote.writer.send(raw(&file)).await.unwrap();
    let target = next_of(&mut rx, "editor_target").await;
    assert_eq!(target["host"], "gpu");
    assert!(hive.open_path("/etc/passwd".into(), false).is_err());
    let file = next_of(&mut rx, "file").await;
    assert_eq!(file["media"], media);
    // Reconnect, restart and wake reach the host's link.
    let other = hive.reconnect_host("other");
    assert_eq!(other, Err("no link to the host other".into()));
    hive.woke();
    // While linked, none of them does anything.
    hive.restart_remote("gpu").unwrap();
    hive.reconnect_host("gpu").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(lab.runs().len(), 1);
    // A reloaded UI gets each host's state and welcome again.
    let (channel, mut reloaded) = ui();
    hive.connect(channel);
    let mut seen = Vec::new();
    while !seen
        .iter()
        .any(|m: &Value| m["type"] == "welcome" && m["host"] == "gpu")
    {
        seen.push(
            tokio::time::timeout(WAIT, reloaded.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let connected = |m: &Value| m["type"] == "host_link" && m["state"] == "connected";
    assert!(seen.iter().any(connected), "{seen:?}");
    // A host taken out of the settings ends its link.
    let settings = hive_protocol::Settings::default();
    local.send(0, Control::Settings { settings }).await;
    until(&mut reloaded, "removed").await;
    let gone = hive.on(Some("gpu")).err();
    assert_eq!(gone, Some("no link to the host gpu".into()));
}

#[cfg(unix)]
#[tokio::test]
async fn a_new_terminal_skips_the_channels_its_link_already_uses() {
    let lab = Lab::new("adopted");
    let listener = lab.listen();
    lab.plan(&[lab.pipe()]);
    let ids = Arc::new(AtomicU32::new(0));
    let remotes = Arc::new(lab.remotes(Arc::clone(&ids)));
    let (channel, mut rx) = ui();
    remotes.sync(settings(&["gpu"], 1), Some(&channel));
    let mut service = Service::accept(&listener).await;
    service.welcome(60).await;
    until(&mut rx, "connected").await;
    let link = remotes.link("gpu").unwrap();
    // Kept terminals the service knows by channels 1 and 2 (26.16), shown as 7 and 8.
    for (channel, ui) in [(1, 7), (2, 8)] {
        let term = Term {
            ui,
            output: output().0,
            cwd: "/k".into(),
            worktree: Value::Null,
            account: Value::Null,
        };
        lock(&link).terminals.insert(channel, term);
    }
    let host = crate::Host {
        link: Arc::clone(&link),
    };
    let id = host.open_terminal("/w".into(), 80, 24, None, output().0);
    assert_eq!(id, Ok(3));
    let found = remotes.terminal(8).map(|(_, channel)| channel);
    assert_eq!(found, Some(2));
    assert!(remotes.terminal(9).is_none());
    // Without the remotes, nothing is linked.
    let hive = Hive::new("/nonexistent/hive-test".into(), vec![]);
    let none = Err("this app links to no remote host".to_owned());
    assert_eq!(hive.reconnect_host("gpu"), none);
    assert_eq!(hive.restart_remote("gpu"), none);
    hive.woke();
    remotes.end();
}
