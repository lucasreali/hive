use super::commands::*;
use super::*;

use tauri::Manager;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

const WAIT: Duration = Duration::from_secs(10);

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
    reader: FramedRead<ReadHalf<DuplexStream>, FrameCodec>,
    writer: FramedWrite<WriteHalf<DuplexStream>, FrameCodec>,
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
    let (app, service) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(app);
    hive.attach(reader, writer, async move { reason.to_owned() });
    let (reader, writer) = tokio::io::split(service);
    Service {
        reader: FramedRead::new(reader, FrameCodec),
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
    };
    service.send(0, welcome).await;
    assert_eq!(
        next(&mut rx).await,
        json!({"type": "welcome", "version": VERSION, "channel": 0})
    );
    (hive, service, rx)
}

#[test]
fn bridge_runs_the_installed_hive_without_a_shell_config() {
    let (program, args) = bridge_command(|_| None);
    assert_eq!(program, "wsl.exe");
    assert_eq!(
        args,
        ["--exec", "/bin/sh", "-c", BRIDGE_SCRIPT, "sh"].map(OsString::from)
    );
    assert_eq!(
        BRIDGE_SCRIPT,
        r#"exec "${1:-$HOME/.cargo/bin/hive}" bridge"#
    );
}

#[test]
fn bridge_overrides_are_separate_arguments() {
    let (_, args) = bridge_command(|key| match key {
        "HIVE_WSL_DISTRO" => Some("Ubuntu".into()),
        "HIVE_BRIDGE" => Some("/src/hive; rm -rf ~".into()),
        _ => None,
    });
    let expected = [
        "-d",
        "Ubuntu",
        "--exec",
        "/bin/sh",
        "-c",
        BRIDGE_SCRIPT,
        "sh",
        "/src/hive; rm -rf ~",
    ];
    assert_eq!(args, expected.map(OsString::from));
}

#[test]
fn empty_overrides_are_ignored() {
    assert_eq!(
        bridge_command(|_| Some("".into())),
        bridge_command(|_| None)
    );
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
async fn big_input_is_split_into_frames_the_service_accepts() {
    let (hive, mut service, _rx) = welcomed().await;
    hive.write_terminal(3, &"a".repeat(MAX_PAYLOAD + 1))
        .unwrap();
    assert_eq!(service.next().await.payload.len(), MAX_PAYLOAD);
    assert_eq!(service.next().await, Frame::terminal(3, "a"));
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
        json!({"type": "welcome", "version": VERSION, "channel": 0})
    );
    assert_eq!(service.control().await, (1, Control::CloseTerminal));
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
            },
        )
        .await;
    assert_eq!(next(&mut rx).await["type"], "welcome");
    assert!(rx.try_recv().is_err());
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
        json!({"type": "version_mismatch", "protocol": 9, "version": "9.9.9", "channel": 0})
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
async fn commands_act_on_the_managed_hive() {
    let app = tauri::test::mock_app();
    app.manage(hive());
    let (channel, mut rx) = ui();
    connect(app.state(), channel).await.unwrap();
    assert_eq!(next(&mut rx).await["type"], "disconnected");

    let not_connected = Err(NOT_CONNECTED.to_owned());
    let (channel, _bytes) = output();
    assert_eq!(
        open_terminal(app.state(), "/".into(), 80, 24, channel),
        Err(NOT_CONNECTED.into())
    );
    assert_eq!(write_terminal(app.state(), 1, "x".into()), not_connected);
    assert_eq!(resize_terminal(app.state(), 1, 80, 24), not_connected);
    assert_eq!(close_terminal(app.state(), 1), not_connected);
}
