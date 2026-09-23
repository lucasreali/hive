//! `hive daemon`: the service. Lives exactly as long as the app connection.

use std::fs::{File, Permissions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, PROTOCOL_VERSION, Role};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, mpsc};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::VERSION;
use crate::adapter::{Adapter, ClaudeCode};
use crate::paths::Paths;

pub async fn run(paths: &Paths) -> io::Result<()> {
    paths.prepare_runtime()?;
    let _lock = lock(paths)?;
    let socket = paths.socket();
    // A socket left by a crashed daemon; the lock proves nobody is serving it.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, Permissions::from_mode(0o600))?;
    let result = serve(listener).await;
    let _ = std::fs::remove_file(&socket);
    result
}

/// Single-instance guard: an exclusive lock held for the daemon's whole life.
fn lock(paths: &Paths) -> io::Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(paths.lock())?;
    file.try_lock().map_err(|err| {
        io::Error::other(format!(
            "cannot lock {}: {err}; is another hive daemon running?",
            paths.lock().display()
        ))
    })?;
    Ok(file)
}

async fn serve(listener: UnixListener) -> io::Result<()> {
    let state = Arc::new(State::default());
    let (app_gone, mut app_gone_rx) = mpsc::channel::<()>(1);
    let mut terminate = signal(SignalKind::terminate())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(connection(stream, state.clone(), app_gone.clone()));
            }
            _ = app_gone_rx.recv() => break,
            _ = terminate.recv() => break,
        }
    }
    Ok(())
}

#[derive(Default)]
struct State {
    app: Mutex<Option<Outbox>>,
}

/// Queue to the app connection's writer.
struct Outbox {
    control: mpsc::UnboundedSender<Frame>,
}

impl State {
    /// Sends a control message to the app, if one is connected.
    async fn to_app(&self, channel: u32, message: &Control) {
        if let Some(app) = &*self.app.lock().await {
            let _ = app.control.send(Frame::control(channel, message));
        }
    }
}

async fn connection(stream: UnixStream, state: Arc<State>, app_gone: mpsc::Sender<()>) {
    let (read, write) = stream.into_split();
    let mut reader = FramedRead::new(read, FrameCodec);
    let mut writer = FramedWrite::new(write, FrameCodec);
    let Some(role) = handshake(&mut reader, &mut writer).await else {
        return;
    };
    match role {
        Role::Hook => hook_connection(reader, &state).await,
        Role::App => {
            if app_connection(reader, writer, &state).await {
                let _ = app_gone.send(()).await;
            }
        }
    }
}

/// Reads the client's `Hello`; answers `Welcome`, or `VersionMismatch` and gives up.
async fn handshake<R, W>(
    reader: &mut FramedRead<R, FrameCodec>,
    writer: &mut FramedWrite<W, FrameCodec>,
) -> Option<Role>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let hello = reader.next().await?.ok()?.to_control();
    let (reply, role) = match hello {
        Ok(Control::Hello {
            protocol,
            version,
            role,
        }) if protocol == PROTOCOL_VERSION && version == VERSION => (
            Control::Welcome {
                version: VERSION.to_owned(),
            },
            Some(role),
        ),
        Ok(Control::Hello { .. }) => (
            Control::VersionMismatch {
                protocol: PROTOCOL_VERSION,
                version: VERSION.to_owned(),
            },
            None,
        ),
        _ => (
            Control::Error {
                message: "expected a hello message".to_owned(),
            },
            None,
        ),
    };
    // A hook client may already be gone after sending its event; that is fine.
    let _ = writer.send(Frame::control(0, &reply)).await;
    role
}

/// A hook connection carries exactly one event; the connection is closed after it.
async fn hook_connection<R: AsyncRead + Unpin>(
    mut reader: FramedRead<R, FrameCodec>,
    state: &State,
) {
    if let Some(Ok(frame)) = reader.next().await
        && let Ok(Control::Hook {
            event,
            terminal_id,
            payload,
        }) = frame.to_control()
    {
        let event = ClaudeCode.translate(&event, terminal_id, payload);
        state.to_app(0, &Control::Agent(event)).await;
    }
}

/// Serves the app until it disconnects. Returns false if another app was already connected.
async fn app_connection<R, W>(
    mut reader: FramedRead<R, FrameCodec>,
    mut writer: FramedWrite<W, FrameCodec>,
    state: &State,
) -> bool
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (_terminal_tx, terminal_rx) = mpsc::channel(1);
    {
        let mut app = state.app.lock().await;
        if app.is_some() {
            let message = Control::Error {
                message: "another app is already connected".to_owned(),
            };
            let _ = writer.send(Frame::control(0, &message)).await;
            return false;
        }
        *app = Some(Outbox {
            control: control_tx,
        });
    }
    let writer = tokio::spawn(write_prioritized(writer, control_rx, terminal_rx));
    while let Some(Ok(_frame)) = reader.next().await {}
    writer.abort();
    *state.app.lock().await = None;
    true
}

/// Writes queued frames, always draining control frames before terminal frames.
async fn write_prioritized<W: AsyncWrite + Unpin>(
    mut writer: FramedWrite<W, FrameCodec>,
    mut control: mpsc::UnboundedReceiver<Frame>,
    mut terminal: mpsc::Receiver<Frame>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            Some(frame) = control.recv() => frame,
            Some(frame) = terminal.recv() => frame,
            else => return,
        };
        if writer.send(frame).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn control_frames_are_written_before_queued_terminal_frames() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::channel(8);
        terminal_tx.send(Frame::terminal(1, "out")).await.unwrap();
        terminal_tx.send(Frame::terminal(1, "more")).await.unwrap();
        control_tx
            .send(Frame::control(0, &Control::CloseTerminal))
            .unwrap();
        drop((control_tx, terminal_tx));

        let (client, server) = tokio::io::duplex(1024);
        write_prioritized(
            FramedWrite::new(server, FrameCodec),
            control_rx,
            terminal_rx,
        )
        .await;
        let frames: Vec<Frame> = FramedRead::new(client, FrameCodec)
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(
            frames,
            vec![
                Frame::control(0, &Control::CloseTerminal),
                Frame::terminal(1, "out"),
                Frame::terminal(1, "more"),
            ]
        );
    }

    #[tokio::test]
    async fn writer_stops_when_the_peer_is_gone() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (_terminal_tx, terminal_rx) = mpsc::channel(1);
        control_tx.send(Frame::terminal(1, "x")).unwrap();
        let (client, server) = tokio::io::duplex(64);
        drop(client);
        // Returns instead of looping forever even though the queues stay open.
        let writer = write_prioritized(
            FramedWrite::new(server, FrameCodec),
            control_rx,
            terminal_rx,
        );
        let finished = tokio::time::timeout(std::time::Duration::from_secs(5), writer).await;
        assert!(finished.is_ok());
    }
}
