use super::commands::*;
use super::*;

use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tokio::io::{AsyncWriteExt, DuplexStream};

const WAIT: Duration = Duration::from_secs(5);

async fn next<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .unwrap()
        .unwrap()
}

/// A UI control channel and what it received.
fn ui() -> (Channel<Value>, mpsc::UnboundedReceiver<Value>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("control messages are JSON")
        };
        tx.send(serde_json::from_str(&json).unwrap()).unwrap();
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
        tx.send(bytes).unwrap();
        Ok(())
    });
    (channel, rx)
}

/// The service end of an in-memory connection.
struct Service {
    reader: FramedRead<DuplexStream, FrameCodec>,
    writer: FramedWrite<DuplexStream, FrameCodec>,
}

impl Service {
    async fn next(&mut self) -> Frame {
        tokio::time::timeout(WAIT, self.reader.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    async fn control(&mut self) -> (u32, Control) {
        let frame = self.next().await;
        (frame.channel, frame.to_control().unwrap())
    }

    async fn send(&mut self, channel: u32, message: Control) {
        self.writer
            .send(Frame::control(channel, &message))
            .await
            .unwrap();
    }
}

/// A `Hive` whose bridge cannot start, so a spawn would show up as `disconnected`.
fn hive() -> Hive {
    Hive::new("/nonexistent/hive-test".into(), vec![])
}

fn attach(hive: &Hive, reason: &'static str) -> Service {
    // One pipe per direction, so each side can close them separately.
    let (reader, writer) = tokio::io::duplex(1 << 16);
    let (to_service, from_app) = tokio::io::duplex(1 << 16);
    hive.attach(reader, to_service, async move { reason.to_owned() });
    Service {
        reader: FramedRead::new(from_app, FrameCodec),
        writer: FramedWrite::new(writer, FrameCodec),
    }
}

/// A connected `Hive` after a successful handshake.
async fn welcomed() -> (Hive, Service, mpsc::UnboundedReceiver<Value>) {
    let hive = hive();
    let (channel, mut rx) = ui();
    hive.link().ui = Some(channel);
    let mut service = attach(&hive, "bridge gone");
    assert_eq!(
        service.control().await,
        (0, Control::hello(Role::App, VERSION))
    );
    let welcome = Control::Welcome {
        version: VERSION.into(),
        distro: Some("Ubuntu".into()),
    };
    service.send(0, welcome).await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "welcome", "version": VERSION, "distro": "Ubuntu", "channel": 0})
    );
    // The projects are requested right after the handshake.
    assert_eq!(service.control().await, (0, Control::ListProjects));
    (hive, service, rx)
}

#[test]
fn bridge_runs_a_constant_script_without_a_shell_config() {
    let (program, args) = bridge_command(|_| None, None);
    assert_eq!(program, "wsl.exe");
    assert_eq!(
        args,
        ["--exec", "/bin/sh", "-c", BRIDGE_SCRIPT, "sh", "", ""].map(OsString::from)
    );
}

#[test]
fn bridge_overrides_are_separate_arguments() {
    let bundled = std::env::current_exe().unwrap();
    let (_, args) = bridge_command(
        |key| match key {
            "HIVE_WSL_DISTRO" => Some("Ubuntu".into()),
            "HIVE_BRIDGE" => Some("/src/hive; rm -rf ~".into()),
            _ => None,
        },
        Some(bundled.clone()),
    );
    let expected = [
        "-d".into(),
        "Ubuntu".into(),
        "--exec".into(),
        "/bin/sh".into(),
        "-c".into(),
        BRIDGE_SCRIPT.into(),
        "sh".into(),
        "/src/hive; rm -rf ~".into(),
        bundled.into_os_string(),
    ];
    assert_eq!(args, expected);
}

#[test]
fn empty_overrides_and_a_missing_bundle_are_ignored() {
    let missing = std::env::temp_dir().join("hive-no-such-bundle");
    assert_eq!(
        bridge_command(|_| Some("".into()), Some(missing)),
        bridge_command(|_| None, None)
    );
}

/// A temporary `HOME` with a fake `wslpath` (prints its path argument) and a bundled `hive`
/// that prints what ran it. Removed on drop.
struct ScriptHome(std::path::PathBuf);

impl ScriptHome {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("hive-bridge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("fakebin")).unwrap();
        let home = Self(root);
        home.write("fakebin/wslpath", "#!/bin/sh\nprintf '%s\\n' \"$2\"\n");
        home.write("bundle/hive", "#!/bin/sh\necho \"v1 $0 $*\"\n");
        home
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Runs the bridge script as `wsl.exe` would, with the given `$1` and `$2`.
    fn run(&self, over: &str, bundled: &str) -> String {
        let path = format!("{}:/usr/bin:/bin", self.0.join("fakebin").display());
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", BRIDGE_SCRIPT, "sh", over, bundled])
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    fn installed(&self) -> std::path::PathBuf {
        self.0.join(".local/share/hive/bin/hive")
    }
}

impl Drop for ScriptHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_bridge_script_installs_the_bundled_hive_once_per_version() {
    use std::os::unix::fs::MetadataExt;
    let home = ScriptHome::new("install");
    let installed = home.installed();
    // Windows gives a verbatim (`\\?\`) path; wslpath gets it without that prefix.
    let bundled = format!(r"\\?\{}", home.0.join("bundle/hive").display());
    assert_eq!(
        home.run("", &bundled),
        format!("v1 {} bridge\n", installed.display())
    );
    let inode = std::fs::metadata(&installed).unwrap().ino();
    home.run("", &bundled);
    assert_eq!(
        std::fs::metadata(&installed).unwrap().ino(),
        inode,
        "same file: no copy"
    );

    home.write("bundle/hive", "#!/bin/sh\necho \"v2 $*\"\n");
    assert_eq!(home.run("", &bundled), "v2 bridge\n");
    assert!(!installed.with_extension("new").exists());
}

#[test]
fn the_bridge_script_prefers_the_override_then_cargo_install() {
    let home = ScriptHome::new("override");
    home.write("dev/hive", "#!/bin/sh\necho \"dev $*\"\n");
    home.write(".cargo/bin/hive", "#!/bin/sh\necho \"cargo $*\"\n");
    let bundled = home.0.join("bundle/hive").display().to_string();
    let dev = home.0.join("dev/hive").display().to_string();
    assert_eq!(home.run(&dev, &bundled), "dev bridge\n");
    assert_eq!(home.run("", ""), "cargo bridge\n");
    assert!(!home.installed().exists());
}

#[tokio::test]
async fn terminal_messages_and_bytes_travel_on_their_channel() {
    let (hive, mut service, mut rx) = welcomed().await;
    let (channel, mut bytes) = output();
    assert_eq!(hive.open_terminal("/w".into(), 80, 24, channel), Ok(1));
    let open = Control::OpenTerminal {
        cwd: "/w".into(),
        cols: 80,
        rows: 24,
    };
    assert_eq!(service.control().await, (1, open));

    service.send(1, Control::TerminalOpened).await;
    service.writer.send(Frame::terminal(1, "$ ")).await.unwrap();
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "terminal_opened", "channel": 1})
    );
    assert_eq!(next(&mut bytes).await, b"$ ");

    hive.write_terminal(1, "ls\r").unwrap();
    assert_eq!(service.next().await, Frame::terminal(1, "ls\r"));
    hive.resize_terminal(1, 100, 30).unwrap();
    let resize = Control::Resize {
        cols: 100,
        rows: 30,
    };
    assert_eq!(service.control().await, (1, resize));
    hive.close_terminal(1).unwrap();
    assert_eq!(service.control().await, (1, Control::CloseTerminal));

    // Nothing reaches the UI for a channel it did not open; channel 0 always does.
    service.writer.send(Frame::terminal(7, "x")).await.unwrap();
    service.send(7, Control::TerminalOpened).await;
    let error = Control::Error {
        message: "m".into(),
    };
    service.send(0, error).await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "error", "message": "m", "channel": 0})
    );

    service
        .send(1, Control::TerminalExited { code: Some(0) })
        .await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "terminal_exited", "code": 0, "channel": 1})
    );
    // The exited terminal's channel is released.
    service
        .writer
        .send(Frame::terminal(1, "late"))
        .await
        .unwrap();
    service.send(0, Control::UnhookedAgent).await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "unhooked_agent", "channel": 0})
    );
    assert!(bytes.try_recv().is_err());

    let (channel, _bytes) = output();
    assert_eq!(hive.open_terminal("/w".into(), 80, 24, channel), Ok(2));
}

#[tokio::test]
async fn project_requests_go_to_the_service_and_answers_to_the_ui() {
    let (hive, mut service, mut rx) = welcomed().await;
    hive.list_projects().unwrap();
    assert_eq!(service.control().await, (0, Control::ListProjects));
    hive.add_project("/r/sub".into()).unwrap();
    let add = Control::AddProject {
        path: "/r/sub".into(),
    };
    assert_eq!(service.control().await, (0, add));
    hive.list_branches("/r".into()).unwrap();
    let branches = Control::ListBranches {
        project: "/r".into(),
    };
    assert_eq!(service.control().await, (0, branches));
    hive.validate_worktree_name("/r".into(), "x".into())
        .unwrap();
    let validate = Control::ValidateWorktreeName {
        project: "/r".into(),
        name: "x".into(),
    };
    assert_eq!(service.control().await, (0, validate));
    hive.create_worktree("/r".into(), "x".into(), Some("main".into()))
        .unwrap();
    let create = Control::CreateWorktree {
        project: "/r".into(),
        name: "x".into(),
        base: Some("main".into()),
    };
    assert_eq!(service.control().await, (0, create));
    hive.remove_worktree("/r/w".into(), true).unwrap();
    let remove = Control::RemoveWorktree {
        path: "/r/w".into(),
        force: true,
    };
    assert_eq!(service.control().await, (0, remove));
    hive.rename_worktree("/r/w".into(), "x".into()).unwrap();
    let rename = Control::RenameWorktree {
        path: "/r/w".into(),
        name: "x".into(),
    };
    assert_eq!(service.control().await, (0, rename));
    hive.watch_worktree("/r".into()).unwrap();
    let watch = Control::WatchWorktree { path: "/r".into() };
    assert_eq!(service.control().await, (0, watch));
    hive.unwatch_worktree().unwrap();
    assert_eq!(service.control().await, (0, Control::UnwatchWorktree));
    hive.list_changes("/r".into()).unwrap();
    let changes = Control::ListChanges { path: "/r".into() };
    assert_eq!(service.control().await, (0, changes));
    hive.list_sessions().unwrap();
    assert_eq!(service.control().await, (0, Control::ListSessions));
    hive.locate_session("s".into(), SessionTarget::Log).unwrap();
    let locate = Control::LocateSession {
        id: "s".into(),
        target: SessionTarget::Log,
    };
    assert_eq!(service.control().await, (0, locate));
    hive.delete_session("s".into()).unwrap();
    let delete = Control::DeleteSession { id: "s".into() };
    assert_eq!(service.control().await, (0, delete));
    hive.search_files("/r".into(), "q".into()).unwrap();
    let search = Control::SearchFiles {
        worktree: "/r".into(),
        query: "q".into(),
    };
    assert_eq!(service.control().await, (0, search));
    hive.open_file("/r".into(), "a".into()).unwrap();
    let open = Control::OpenFile {
        worktree: "/r".into(),
        path: "a".into(),
    };
    assert_eq!(service.control().await, (0, open));
    hive.save_file("/r".into(), "a".into(), "x".into(), Some("v".into()))
        .unwrap();
    let save = Control::SaveFile {
        worktree: "/r".into(),
        path: "a".into(),
        content: "x".into(),
        version: Some("v".into()),
    };
    assert_eq!(service.control().await, (0, save));
    hive.open_in_editor("/r".into(), "a".into()).unwrap();
    let editor = Control::OpenInEditor {
        worktree: "/r".into(),
        path: "a".into(),
    };
    assert_eq!(service.control().await, (0, editor));
    service
        .send(0, Control::Projects { projects: vec![] })
        .await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "projects", "projects": [], "channel": 0})
    );
}

#[tokio::test]
async fn big_input_is_split_into_frames_the_service_accepts() {
    let (hive, mut service, _rx) = welcomed().await;
    hive.write_terminal(3, &"a".repeat(MAX_PAYLOAD + 1))
        .unwrap();
    assert_eq!(service.next().await.payload.len(), MAX_PAYLOAD);
    assert_eq!(service.next().await, Frame::terminal(3, "a"));
}

#[tokio::test]
async fn a_message_over_the_frame_limit_is_refused_and_the_link_keeps_working() {
    let (hive, mut service, _rx) = welcomed().await;
    let pasted = "a".repeat(MAX_PAYLOAD);
    let refused = hive.save_file("/w".into(), "a".into(), pasted, None);
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.starts_with("frame payload of ")),
        "{refused:?}"
    );
    hive.write_terminal(1, "x").unwrap();
    assert_eq!(service.next().await, Frame::terminal(1, "x"));
}

#[tokio::test]
async fn channel_numbers_run_out_instead_of_wrapping() {
    let (hive, _service, _rx) = welcomed().await;
    hive.link().last_channel = u32::MAX;
    let (channel, _bytes) = output();
    assert_eq!(
        hive.open_terminal("/".into(), 80, 24, channel),
        Err("no terminal channel left".into())
    );
}

#[tokio::test]
async fn a_reloaded_ui_gets_welcome_again_and_its_old_terminals_close() {
    let (hive, mut service, _old) = welcomed().await;
    let (channel, _bytes) = output();
    hive.open_terminal("/w".into(), 80, 24, channel).unwrap();
    service.control().await;

    let (channel, mut rx) = ui();
    hive.connect(channel);
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "welcome", "version": VERSION, "distro": "Ubuntu", "channel": 0})
    );
    assert_eq!(service.control().await, (1, Control::CloseTerminal));
    // The new UI gets the projects again.
    assert_eq!(service.control().await, (0, Control::ListProjects));
    // The old terminal's exit is not reported to the new UI.
    service
        .send(1, Control::TerminalExited { code: None })
        .await;
    service.send(0, Control::UnhookedAgent).await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "unhooked_agent", "channel": 0})
    );
}

#[tokio::test]
async fn a_ui_reloaded_during_the_handshake_waits_for_welcome() {
    let hive = hive();
    let mut service = attach(&hive, "");
    service.control().await;
    let (channel, mut rx) = ui();
    hive.connect(channel);
    service
        .send(
            0,
            Control::Welcome {
                version: VERSION.into(),
                distro: None,
            },
        )
        .await;
    assert_eq!(next(&mut rx).await["type"], "welcome");
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn welcome_waits_for_a_ui_that_connects_later() {
    let hive = hive();
    let mut service = attach(&hive, "");
    service.control().await;
    let welcome = Control::Welcome {
        version: VERSION.into(),
        distro: Some("Ubuntu".into()),
    };
    service.send(0, welcome).await;
    let stored = async {
        while hive.link().welcome.is_none() {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(WAIT, stored).await.unwrap();
    let (channel, mut rx) = ui();
    hive.connect(channel);
    let message = next(&mut rx).await;
    assert_eq!(message["type"], "welcome");
}

#[tokio::test]
async fn bridge_exit_ends_terminals_then_disconnects() {
    let (hive, mut service, mut rx) = welcomed().await;
    let (channel, _bytes) = output();
    hive.open_terminal("/w".into(), 80, 24, channel).unwrap();
    service.control().await;
    drop(service);
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "terminal_exited", "channel": 1, "code": null})
    );
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "disconnected", "reason": "bridge gone"})
    );
    let not_connected = Err(NOT_CONNECTED.to_owned());
    assert_eq!(hive.write_terminal(1, "x"), not_connected);
    assert_eq!(hive.resize_terminal(1, 1, 1), not_connected);
    assert_eq!(hive.close_terminal(1), not_connected);
    assert_eq!(hive.list_projects(), not_connected);
    assert_eq!(hive.add_project("/r".into()), not_connected);
    assert_eq!(hive.list_branches("/r".into()), not_connected);
    assert_eq!(
        hive.validate_worktree_name("/r".into(), "x".into()),
        not_connected
    );
    assert_eq!(
        hive.create_worktree("/r".into(), "x".into(), None),
        not_connected
    );
    assert_eq!(hive.watch_worktree("/r".into()), not_connected);
    assert_eq!(hive.unwatch_worktree(), not_connected);
    assert_eq!(hive.list_changes("/r".into()), not_connected);
    assert_eq!(hive.open_file("/r".into(), "a".into()), not_connected);
    assert_eq!(
        hive.save_file("/r".into(), "a".into(), "x".into(), None),
        not_connected
    );
    assert_eq!(hive.open_in_editor("/r".into(), "a".into()), not_connected);
    assert_eq!(hive.search_files("/r".into(), "q".into()), not_connected);
    assert_eq!(hive.list_sessions(), not_connected);
    assert_eq!(
        hive.locate_session("s".into(), SessionTarget::Folder),
        not_connected
    );
    assert_eq!(hive.delete_session("s".into()), not_connected);
    assert_eq!(hive.remove_worktree("/r/w".into(), false), not_connected);
    assert_eq!(
        hive.rename_worktree("/r/w".into(), "x".into()),
        not_connected
    );
    let (channel, _bytes) = output();
    assert_eq!(
        hive.open_terminal("/".into(), 1, 1, channel),
        Err(NOT_CONNECTED.into())
    );
}

#[tokio::test]
async fn version_mismatch_is_final_and_not_a_disconnect() {
    let hive = hive();
    let (channel, mut rx) = ui();
    hive.link().ui = Some(channel);
    let mut service = attach(&hive, "bridge gone");
    service.control().await;
    let refused = Control::VersionMismatch {
        protocol: 9,
        version: "9.9.9".into(),
    };
    service.send(0, refused).await;
    assert_eq!(
        next(&mut rx).await,
        json!({
            "type": "version_mismatch",
            "protocol": 9,
            "version": "9.9.9",
            "app_version": VERSION,
            "app_protocol": PROTOCOL_VERSION,
            "channel": 0
        })
    );
    assert_eq!(hive.write_terminal(1, "x"), Err(NOT_CONNECTED.into()));
    drop(service);
    assert!(tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .is_err());
}

#[tokio::test]
async fn a_malformed_stream_disconnects_with_the_protocol_error() {
    let (_hive, mut service, mut rx) = welcomed().await;
    service
        .writer
        .get_mut()
        .write_all(&[9, 0, 0, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "disconnected", "reason": "unknown frame type 9"})
    );

    let (_hive, mut service, mut rx) = welcomed().await;
    service
        .writer
        .get_mut()
        .write_all(&[0, 0, 0, 0, 0, 0, 0, 0, 1, b'{'])
        .await
        .unwrap();
    let message = next(&mut rx).await;
    assert_eq!(message["type"], "disconnected");
    let reason = message["reason"].as_str().unwrap();
    assert!(reason.starts_with("invalid control message"), "{reason}");
}

/// A `Hive` whose bridge is `sh -c <script>`.
fn sh(script: &str) -> Hive {
    Hive::new("sh".into(), ["-c", script].map(OsString::from).into())
}

async fn disconnect_reason(hive: &Hive) -> Value {
    let (channel, mut rx) = ui();
    hive.connect(channel);
    let message = next(&mut rx).await;
    assert_eq!(message["type"], "disconnected");
    message["reason"].clone()
}

#[tokio::test]
async fn the_bridge_stderr_explains_a_disconnect() {
    let reason = disconnect_reason(&sh("echo 'the hive service did not start' >&2")).await;
    assert_eq!(reason, "the hive service did not start");
    let reason = disconnect_reason(&sh("exit 3")).await;
    assert_eq!(reason, "the hive bridge exited");
}

#[tokio::test]
async fn the_bridge_reads_the_hello_frame_on_stdin() {
    let hello = Frame::control(0, &Control::hello(Role::App, VERSION));
    let size = 9 + hello.payload.len();
    let hive = sh(&format!(
        "[ \"$(head -c {size} | wc -c)\" = {size} ] && echo hello read >&2"
    ));
    assert_eq!(disconnect_reason(&hive).await, "hello read");
}

#[test]
fn a_missing_pipe_is_an_error() {
    assert_eq!(pipe(Some(1)).unwrap(), 1);
    let error = pipe::<()>(None).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
}

#[tokio::test]
async fn a_bridge_that_cannot_start_disconnects_at_once() {
    let reason = disconnect_reason(&hive()).await;
    let reason = reason.as_str().unwrap();
    assert!(
        reason.starts_with("cannot start the hive bridge: "),
        "{reason}"
    );
}

#[tokio::test]
async fn a_service_that_stops_reading_fails_later_writes() {
    let (hive, service, _rx) = welcomed().await;
    drop(service.reader);
    let failed = async {
        while hive.write_terminal(1, "x").is_ok() {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(WAIT, failed).await.unwrap();
}

/// Calls a command through the IPC layer, as the UI does (argument names in camelCase).
fn invoke(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    args: Value,
) -> Result<Value, Value> {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn commands_reach_the_managed_hive() {
    // A stand-in bridge that stays up and reads its stdin.
    let app = mock_builder()
        .manage(sh("cat >/dev/null"))
        .invoke_handler(tauri::generate_handler![
            connect,
            open_terminal,
            write_terminal,
            resize_terminal,
            close_terminal,
            list_projects,
            add_project,
            list_branches,
            validate_worktree_name,
            create_worktree,
            remove_worktree,
            rename_worktree,
            watch_worktree,
            unwatch_worktree,
            list_changes,
            open_file,
            search_files,
            list_sessions,
            locate_session,
            delete_session,
            save_file,
            open_in_editor
        ])
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let open = json!({"cwd": "/", "cols": 80, "rows": 24, "onData": "__CHANNEL__:2"});
    let write = json!({"id": 1, "data": "x"});
    let resize = json!({"id": 1, "cols": 80, "rows": 24});
    let close = json!({"id": 1});

    let not_connected = Err(json!(NOT_CONNECTED));
    assert_eq!(
        invoke(&webview, "open_terminal", open.clone()),
        not_connected
    );
    assert_eq!(
        invoke(&webview, "write_terminal", write.clone()),
        not_connected
    );
    assert_eq!(
        invoke(&webview, "resize_terminal", resize.clone()),
        not_connected
    );
    assert_eq!(
        invoke(&webview, "close_terminal", close.clone()),
        not_connected
    );
    assert_eq!(invoke(&webview, "list_projects", json!({})), not_connected);
    let add = json!({"path": "/r"});
    assert_eq!(invoke(&webview, "add_project", add.clone()), not_connected);
    let branches = json!({"project": "/r"});
    let validate = json!({"project": "/r", "name": "x"});
    let create = json!({"project": "/r", "name": "x", "base": null});
    let remove = json!({"path": "/r/w", "force": false});
    let rename = json!({"path": "/r/w", "name": "x"});
    let watch = json!({"path": "/r"});
    let changes = json!({"path": "/r"});
    let file = json!({"worktree": "/r", "path": "a"});
    let search = json!({"worktree": "/r", "query": "q"});
    let locate = json!({"id": "s", "target": "log"});
    let delete = json!({"id": "s"});
    let save = json!({"worktree": "/r", "path": "a", "content": "x", "version": null});
    for (cmd, args) in [
        ("list_branches", &branches),
        ("validate_worktree_name", &validate),
        ("create_worktree", &create),
        ("remove_worktree", &remove),
        ("rename_worktree", &rename),
        ("watch_worktree", &watch),
        ("unwatch_worktree", &json!({})),
        ("list_changes", &changes),
        ("open_file", &file),
        ("search_files", &search),
        ("list_sessions", &json!({})),
        ("locate_session", &locate),
        ("delete_session", &delete),
        ("save_file", &save),
        ("open_in_editor", &file),
    ] {
        assert_eq!(invoke(&webview, cmd, args.clone()), not_connected, "{cmd}");
    }

    let refused = invoke(&webview, "connect", json!({})).unwrap_err();
    assert!(refused.as_str().unwrap().contains("onMessage"), "{refused}");
    let connect = json!({"onMessage": "__CHANNEL__:1"});
    assert_eq!(invoke(&webview, "connect", connect), Ok(Value::Null));

    assert_eq!(
        invoke(&webview, "open_terminal", open.clone()),
        Ok(json!(1))
    );
    assert_eq!(invoke(&webview, "open_terminal", open), Ok(json!(2)));
    assert_eq!(invoke(&webview, "write_terminal", write), Ok(Value::Null));
    assert_eq!(invoke(&webview, "resize_terminal", resize), Ok(Value::Null));
    assert_eq!(invoke(&webview, "close_terminal", close), Ok(Value::Null));
    assert_eq!(
        invoke(&webview, "list_projects", json!({})),
        Ok(Value::Null)
    );
    assert_eq!(invoke(&webview, "add_project", add), Ok(Value::Null));
    for (cmd, args) in [
        ("list_branches", branches),
        ("validate_worktree_name", validate),
        ("create_worktree", create),
        ("remove_worktree", remove),
        ("rename_worktree", rename),
        ("watch_worktree", watch),
        ("unwatch_worktree", json!({})),
        ("list_changes", changes),
        ("open_file", file.clone()),
        ("search_files", search),
        ("list_sessions", json!({})),
        ("locate_session", locate),
        ("delete_session", delete),
        ("save_file", save),
        ("open_in_editor", file),
    ] {
        assert_eq!(invoke(&webview, cmd, args), Ok(Value::Null), "{cmd}");
    }
}

#[tokio::test]
async fn shutdown_closes_the_bridge_stdin_and_waits_for_the_bridge_to_end() {
    let (hive, mut service, mut rx) = welcomed().await;
    let shutdown = hive.shutdown(WAIT);
    let service_side = async {
        // The app's side closed: the bridge would exit, closing its stdout.
        let end = tokio::time::timeout(WAIT, service.reader.next()).await;
        assert!(end.unwrap().is_none());
        drop(service.writer);
    };
    tokio::join!(shutdown, service_side);
    // The connection had fully ended before `shutdown` returned.
    assert_eq!(
        rx.try_recv().unwrap(),
        json!({"type": "disconnected", "reason": "bridge gone"})
    );
    assert_eq!(hive.list_projects(), Err(NOT_CONNECTED.to_owned()));
    // Nothing is left to end.
    tokio::time::timeout(WAIT, hive.shutdown(WAIT))
        .await
        .unwrap();
}

/// Whether the process is gone (or a zombie waiting to be reaped).
fn gone(pid: &str) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat.contains(") Z "),
        Err(_) => true,
    }
}

#[tokio::test]
async fn shutdown_kills_a_bridge_that_does_not_end() {
    let pid_file = std::env::temp_dir().join(format!("hive-app-test-{}", std::process::id()));
    let _ = std::fs::remove_file(&pid_file);
    // Never reads its stdin, so closing it changes nothing.
    let script = r#"echo $$ > "$0"; exec sleep 60"#;
    let args = vec!["-c".into(), script.into(), pid_file.clone().into()];
    let hive = Hive::new("sh".into(), args);
    let (channel, _rx) = ui();
    hive.connect(channel);
    let read_pid = async {
        loop {
            match std::fs::read_to_string(&pid_file) {
                Ok(pid) if pid.ends_with('\n') => return pid.trim().to_owned(),
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    };
    let pid = tokio::time::timeout(WAIT, read_pid).await.unwrap();
    std::fs::remove_file(&pid_file).unwrap();
    assert!(!gone(&pid));
    tokio::time::timeout(WAIT, hive.shutdown(Duration::from_millis(100)))
        .await
        .unwrap();
    let killed = async {
        while !gone(&pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(WAIT, killed).await.unwrap();
}

#[test]
fn the_app_exit_ends_the_connection() {
    let app = mock_builder()
        .manage(sh("cat >/dev/null"))
        .invoke_handler(tauri::generate_handler![connect, list_projects])
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let connect = json!({"onMessage": "__CHANNEL__:1"});
    assert_eq!(invoke(&webview, "connect", connect), Ok(Value::Null));
    on_run_event(app.handle(), RunEvent::Ready);
    assert_eq!(
        invoke(&webview, "list_projects", json!({})),
        Ok(Value::Null)
    );
    on_run_event(app.handle(), RunEvent::Exit);
    assert_eq!(
        invoke(&webview, "list_projects", json!({})),
        Err(json!(NOT_CONNECTED))
    );
    assert!(app.state::<Hive>().link().reader.is_none());
}

/// A GitHub stand-in: serves `/latest.json` announcing `version`, with an installer on this
/// server that carries no valid signature, until the test process ends. Returns its URL.
fn release_server(version: &str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let manifest = json!({
        "version": version,
        "platforms": {
            "linux-x86_64": {"url": format!("{base}/hive-setup"), "signature": "not signed"}
        }
    })
    .to_string();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().map_while(Result::ok) {
            let mut request = [0; 4096];
            let read = socket.read(&mut request).unwrap_or(0);
            let body = if request[..read].starts_with(b"GET /latest.json ") {
                manifest.as_str()
            } else {
                "not an installer"
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(format!("{head}{body}").as_bytes());
        }
    });
    format!("{base}/latest.json")
}

/// An app with the updater plugin reading `endpoint`, managing `hive`.
fn updater_app(endpoint: &str, hive: Hive) -> tauri::App<tauri::test::MockRuntime> {
    let mut context = mock_context(noop_assets());
    let updater = json!({"endpoints": [endpoint], "pubkey": "not a key"});
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), updater);
    mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(hive)
        .invoke_handler(tauri::generate_handler![check_update, install_update])
        .build(context)
        .unwrap()
}

/// A `Hive` without a service whose UI messages arrive on the receiver.
fn hive_with_ui() -> (Hive, mpsc::UnboundedReceiver<Value>) {
    let hive = hive();
    let (channel, rx) = ui();
    hive.link().ui = Some(channel);
    (hive, rx)
}

#[tokio::test]
async fn a_newer_release_is_offered_and_an_unsigned_one_is_not_installed() {
    let (hive, mut rx) = hive_with_ui();
    let app = updater_app(&release_server("99.0.0"), hive);
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    assert_eq!(invoke(&webview, "check_update", json!({})), Ok(Value::Null));
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "update_available", "version": "99.0.0"})
    );
    assert_eq!(
        invoke(&webview, "install_update", json!({})),
        Ok(Value::Null)
    );
    let failed = next(&mut rx).await;
    assert_eq!(failed["type"], "update_failed");
    assert_ne!(failed["error"], "no update to install", "{failed}");
}

#[tokio::test]
async fn no_newer_release_or_a_failed_check_offers_nothing() {
    use tauri_plugin_updater::UpdaterExt;
    let (hive, mut rx) = hive_with_ui();
    let endpoint = release_server("0.0.1");
    let app = updater_app(&endpoint, hive_with_ui().0);
    hive.check_update(app.updater()).await;
    // Anything but `/latest.json` answers the installer, which is not JSON: the check fails.
    let broken = vec![endpoint
        .replace("latest.json", "broken.json")
        .parse()
        .unwrap()];
    let updater = app.updater_builder().endpoints(broken).unwrap().build();
    hive.check_update(updater).await;
    hive.install_update().await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "update_failed", "error": "no update to install"})
    );
}

#[test]
fn an_installed_update_restarts_the_app() {
    let (hive, mut rx) = hive_with_ui();
    let (tx, restarted) = std::sync::mpsc::channel();
    let hive = hive.with_restart(move || tx.send(()).unwrap());
    hive.installed(Ok(()));
    assert_eq!(restarted.try_recv(), Ok(()));
    assert!(rx.try_recv().is_err());
}
