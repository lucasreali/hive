//! The service's connections (9.20): the handshake, a hook call, and the app's frames,
//! answered in turn, with the prioritized writer to the app.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{
    Control, EventKind, Frame, FrameCodec, FrameError, FrameType, PROTOCOL_VERSION, Role,
    SessionTarget,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};

use super::State;
use crate::VERSION;
use crate::adapter::{self, Adapter, ClaudeCode};
use crate::terminal::Input;
use crate::{changes, dirs, file, procs, worktree};

/// Longest `hive badge` label, in characters.
const MAX_BADGE: usize = 40;

/// Serves one connection: a Unix socket's, or a named pipe instance's on Windows.
pub(super) async fn connection<S>(stream: S, state: Arc<State>, app_gone: mpsc::Sender<()>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
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
                distro: std::env::var("WSL_DISTRO_NAME").ok(),
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

/// A hook connection carries exactly one event (or one `hive badge`); the connection is
/// closed after it.
async fn hook_connection<R: AsyncRead + Unpin>(
    mut reader: FramedRead<R, FrameCodec>,
    state: &Arc<State>,
) {
    let Some(Ok(frame)) = reader.next().await else {
        return;
    };
    match frame.to_control() {
        Ok(Control::Badge { text }) => {
            let channel = frame.channel;
            // Held while sending, so a badge cannot follow the terminal's `terminal_exited`.
            let terminals = state.terminals.lock().await;
            if terminals.contains_key(&channel) {
                let text = adapter::clip(&text, MAX_BADGE);
                state.to_app(channel, &Control::Badge { text }).await;
            }
        }
        Ok(Control::StatuslineUsage { claude_dir, usage }) => {
            state.usage.lock().await.report(claude_dir, usage);
            state.send_usage().await;
        }
        Ok(Control::Hook {
            event,
            terminal_id,
            payload,
            sent_ns,
        }) => {
            // The payload stays here: the app gets only what the service makes of it.
            let event = ClaudeCode.translate(&event, terminal_id, payload);
            state.saw(&event, sent_ns).await;
            if let EventKind::WorktreeCreated { .. } | EventKind::WorktreeRemoved { .. } =
                event.kind
            {
                // The app's worktrees follow a `claude -w` or a subagent's worktree; listed
                // after the agent states it changed.
                let state = state.clone();
                tokio::spawn(async move { state.worktrees_changed().await });
            }
        }
        _ => {}
    }
}

/// Serves the app until it disconnects. Returns false if another app was already connected.
async fn app_connection<R, W>(
    mut reader: FramedRead<R, FrameCodec>,
    mut writer: FramedWrite<W, FrameCodec>,
    state: &Arc<State>,
) -> bool
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    // Unbounded, but each terminal's share is bounded by what the app has not acknowledged
    // ([`terminal::Output`]), so one terminal never makes another wait to queue.
    let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();
    {
        let mut app = state.app.lock().await;
        if app.is_some() {
            let message = Control::Error {
                message: "another app is already connected".to_owned(),
            };
            let _ = writer.send(Frame::control(0, &message)).await;
            return false;
        }
        *app = Some(control_tx);
    }
    *state.listed() = None;
    state.send_settings().await;
    state.snapshot().await;
    state.usage.lock().await.unsent();
    state.send_usage().await;
    // Only the first app after a restart resumes the sessions the last one left.
    let restore = std::mem::take(&mut *state.restore.pending.lock().await);
    if !restore.is_empty() {
        let sessions = restore;
        state
            .to_app(0, &Control::RestoreSessions { sessions })
            .await;
    }
    let writer = tokio::spawn(write_prioritized(writer, control_rx, terminal_rx));
    while let Some(Ok(frame)) = reader.next().await {
        app_frame(state, frame, &terminal_tx).await;
    }
    writer.abort();
    *state.app.lock().await = None;
    true
}

async fn app_frame(state: &Arc<State>, frame: Frame, output: &mpsc::UnboundedSender<Frame>) {
    let channel = frame.channel;
    let message = match frame.kind {
        FrameType::Terminal => return state.input(channel, Input::Data(frame.payload)),
        FrameType::Control => frame.to_control(),
    };
    match message {
        Ok(Control::OpenTerminal {
            cwd,
            cols,
            rows,
            account,
        }) => {
            let size = (cols, rows);
            state
                .open(channel, &cwd, size, account, output.clone())
                .await;
        }
        Ok(Control::Resize { cols, rows }) => state.input(channel, Input::Resize { cols, rows }),
        Ok(Control::Ack { bytes }) => state.ack(channel, bytes),
        Ok(Control::CloseTerminal) => state.close(channel),
        Ok(Control::WatchWorktree { path, base }) => state.watch_worktree(Some((path, base))).await,
        Ok(Control::UnwatchWorktree) => state.watch_worktree(None).await,
        Ok(Control::View { terminal, focused }) => {
            let watched = terminal.filter(|_| focused).unwrap_or(0);
            state.watched.store(watched, Ordering::Relaxed);
        }
        Ok(Control::GetSettings) => state.send_settings().await,
        Ok(Control::SetSettings { settings }) => {
            let reply = match tokio::task::block_in_place(|| state.settings.set(settings)) {
                Ok(settings) => Control::Settings { settings },
                Err(message) => Control::SettingsFailed { message },
            };
            state.to_app(0, &reply).await;
        }
        Ok(Control::OpenSettingsFile) => {
            let located = tokio::task::block_in_place(|| {
                file::windows(state.settings.ensure_file()?, OsStr::new("wslpath"))
            });
            let target = Control::EditorTarget {
                worktree: String::new(),
                path: String::new(),
                error: located.as_ref().err().map(ToString::to_string),
                windows_path: located.ok(),
            };
            state.to_app(0, &target).await;
        }
        Ok(Control::GetDiagnostics) => {
            let claude = state.user_claude().await;
            let diagnostics = Control::Diagnostics {
                settings_file: state.settings.file().display().to_string(),
                wrapper: state.bin_dir.join("claude").display().to_string(),
                claude: claude.map(|c| c.display().to_string()),
            };
            state.to_app(0, &diagnostics).await;
        }
        Ok(Control::ListProjects) => {
            *state.listed() = None;
            state.to_app(0, &state.projects.spaces_message()).await;
            // A new app, or a reloaded UI: listed afresh (e.g. a project folder moved), in turn
            // with git's registry, so an older list of it never follows this one.
            state.change_worktrees(|projects| {
                projects.forget();
                Control::Projects {
                    projects: projects.list(),
                }
            })
        }
        Ok(Control::AddProject { path }) => {
            let state = state.clone();
            tokio::spawn(async move {
                // In turn with git's registry, whose list may not have the project yet.
                let _turn = state.changing.lock().await;
                let added = tokio::task::block_in_place(|| {
                    let project = state.projects.add(&path)?;
                    let mut reply = Control::ProjectAdded { project };
                    state.with_health(&mut reply);
                    Ok(reply)
                });
                let reply = match added {
                    Ok(reply) => {
                        // It joined the current space.
                        state.refollow.notify_one();
                        state.to_app(0, &state.projects.spaces_message()).await;
                        reply
                    }
                    Err((error, message)) => Control::AddProjectFailed {
                        path,
                        error,
                        message,
                    },
                };
                state.to_app(0, &reply).await;
            });
        }
        Ok(Control::RemoveProject { id }) => {
            let state = state.clone();
            tokio::spawn(async move {
                // In turn with git's registry, whose list may still have the project.
                let _turn = state.changing.lock().await;
                let terminals = state.terminals.lock().await;
                let sessions: HashSet<i32> = terminals.values().map(|t| t.session).collect();
                drop(terminals);
                state.remove_project(id, &sessions).await
            });
        }
        Ok(Control::CreateSpace { name, env }) => {
            state.change_spaces(|s| s.create(&name, env)).await
        }
        Ok(Control::UpdateSpace { id, name, env }) => {
            state.change_spaces(|s| s.update(&id, &name, env)).await
        }
        Ok(Control::DeleteSpace { id }) => state.change_spaces(|s| s.delete(&id)).await,
        Ok(Control::SelectSpace { id }) => state.change_spaces(|s| s.select(&id)).await,
        Ok(Control::ListGhAccounts { gh_config_dir }) => state.gh_accounts(gh_config_dir, None),
        Ok(Control::SwitchGhAccount {
            gh_config_dir,
            account,
        }) => state.gh_accounts(gh_config_dir, Some(account)),
        Ok(
            request @ (Control::ListPulls { .. }
            | Control::OpenPull { .. }
            | Control::ActOnPull { .. }
            | Control::CreatePull { .. }
            | Control::ListRuns { .. }
            | Control::OpenRun { .. }
            | Control::OpenJobLog { .. }
            | Control::ActOnRun { .. }),
        ) => state.github(request),
        Ok(Control::ListBranches { project }) => state.projects(move |projects| {
            let (branches, error) = match projects.branches(&project) {
                Ok(branches) => (branches, None),
                Err(err) => (Default::default(), Some(err.to_string())),
            };
            Control::Branches {
                project,
                local: branches.local,
                remote: branches.remote,
                current: branches.current,
                error,
            }
        }),
        Ok(Control::ValidateWorktreeName { project, name }) => state.projects(move |projects| {
            let error = projects.validate_worktree_name(&project, &name).err();
            let (folder, branch) = worktree::planned(&name);
            Control::WorktreeNameValidated {
                project,
                name,
                folder,
                branch,
                error: error.map(|err| err.to_string()),
            }
        }),
        Ok(Control::CreateWorktree {
            project,
            name,
            base,
        }) => {
            let state = state.clone();
            tokio::spawn(async move {
                // A remote base is fetched first, outside the turn below, as the project's
                // space's terminals would: with its identity and its GitHub account's token
                // (9.30, 13.2).
                let mut fetched = None;
                if let Some(base) = &base {
                    let space = tokio::task::block_in_place(|| state.projects.space_env(&project));
                    let mut env = crate::spaces::vars(&space);
                    if space.gh_account.is_some() {
                        let gh = state.gh().await;
                        let token = tokio::task::block_in_place(|| gh.vars(&space));
                        // Without it git asks as the user's own credentials would.
                        env.extend(token.unwrap_or_default());
                    }
                    let fetch = || state.projects.fetch_base(&project, base, &env);
                    fetched = tokio::task::block_in_place(fetch);
                }
                // As `change_worktrees`, in turn with the registry watch.
                let _turn = state.changing.lock().await;
                let create = |projects: &crate::projects::Projects| match projects.create_worktree(
                    &project,
                    &name,
                    base.as_deref(),
                ) {
                    Ok((project, created)) => Control::WorktreeCreated {
                        project,
                        path: created.path.to_string_lossy().into_owned(),
                        notes: fetched.into_iter().chain(created.notes).collect(),
                    },
                    Err(err) => Control::CreateWorktreeFailed {
                        project,
                        name,
                        message: err.to_string(),
                    },
                };
                state.answer(create).await;
            });
        }
        Ok(Control::RemoveWorktree { path, force }) => {
            let archiving = state.clone();
            state.change_worktrees(move |projects| {
                let archive = |root: &str| archiving.archive(root, &path);
                match projects.remove_worktree(&path, force, procs::Source::System, archive) {
                    Ok(project) => Control::WorktreeRemoved { project, path },
                    Err(err) => Control::RemoveWorktreeFailed {
                        path,
                        message: err.to_string(),
                    },
                }
            })
        }
        Ok(Control::RenameWorktree { path, name }) => state.change_worktrees(move |projects| {
            match projects.rename_worktree(&path, &name, procs::Source::System) {
                Ok((project, to)) => Control::WorktreeRenamed {
                    project,
                    from: path,
                    path: to,
                },
                Err(err) => Control::RenameWorktreeFailed {
                    path,
                    name,
                    message: err.to_string(),
                },
            }
        }),
        Ok(Control::ListChanges { path, base }) => {
            state.projects(move |projects| changes::answer(&projects.list(), path, base).0)
        }
        Ok(Control::ListSessions) => {
            // Resume commands for the terminals' shell: native Windows' own, else POSIX.
            #[cfg(windows)]
            let shell = Some(state.settings.get().0.terminal.shell);
            #[cfg(not(windows))]
            let shell = None;
            // Hive's terminals: their hooks name their sessions.
            state.sessions(move |projects, sessions, mut running| {
                // Claude keeps a record of each running `claude` beside its projects folder.
                let roots = sessions.roots();
                let parents = roots.iter().filter_map(|root| root.parent());
                let records: Vec<PathBuf> = parents.map(|d| d.join("sessions")).collect();
                running.extend(procs::claude_sessions(procs::Source::System, &records));
                let (mut sessions, truncated, error) = sessions.list(projects, &running);
                for session in &mut sessions {
                    session.resume_command = crate::sessions::resume_command(session, shell);
                }
                Control::Sessions {
                    sessions,
                    error,
                    truncated,
                }
            })
        }
        Ok(Control::LocateSession { id, target }) => {
            state.sessions(move |projects, sessions, _| {
                let wslpath = OsStr::new("wslpath");
                let located = sessions
                    .find(projects, &id)
                    .and_then(|session| match target {
                        SessionTarget::Log => file::windows(Path::new(&session.log), wslpath),
                        // The log's `cwd` is untrusted: checked as a worktree's folder is (9.10).
                        SessionTarget::Folder => {
                            file::windows_path(Path::new(&session.cwd), "", wslpath)
                        }
                    });
                Control::SessionLocated {
                    id,
                    target,
                    error: located.as_ref().err().map(ToString::to_string),
                    windows_path: located.ok(),
                }
            })
        }
        Ok(Control::DeleteSession { id }) => {
            state.sessions(move |projects, sessions, running| {
                // A running session keeps writing its log.
                let deleted = if running.contains(&id) {
                    Err(io::Error::other("the session is running: end it first"))
                } else {
                    sessions.delete(projects, &id)
                };
                match deleted {
                    Ok(()) => Control::SessionDeleted { id },
                    Err(err) => Control::DeleteSessionFailed {
                        id,
                        message: err.to_string(),
                    },
                }
            })
        }
        Ok(Control::ListDirs { path, windows }) => state.projects(move |_| {
            let home = std::env::var_os(dirs::HOME).map(PathBuf::from);
            dirs::answer(path, windows, home.as_deref(), &dirs::WINDOWS)
        }),
        Ok(Control::SearchFiles { worktree, query }) => {
            state.projects(move |projects| file::answer::search(projects, worktree, query))
        }
        Ok(Control::OpenFile {
            worktree,
            path,
            base,
        }) => state.projects(move |projects| file::answer::open(projects, worktree, path, base)),
        Ok(Control::SaveFile {
            worktree,
            path,
            content,
            version,
        }) => state.projects(move |projects| {
            file::answer::save(projects, worktree, path, &content, version.as_deref())
        }),
        Ok(Control::CreateFile {
            worktree,
            folder,
            name,
        }) => {
            state.projects(move |projects| file::answer::create(projects, worktree, &folder, &name))
        }
        Ok(Control::CreateFolder {
            worktree,
            folder,
            name,
        }) => state.projects(move |projects| {
            file::answer::create_folder(projects, worktree, &folder, &name)
        }),
        Ok(Control::RenameFile {
            worktree,
            path,
            name,
        }) => state.projects(move |projects| file::answer::rename(projects, worktree, path, &name)),
        Ok(Control::MoveFile {
            worktree,
            path,
            folder,
        }) => {
            state.projects(move |projects| file::answer::move_to(projects, worktree, path, &folder))
        }
        Ok(Control::DeleteFile { worktree, path }) => {
            state.projects(move |projects| file::answer::delete(projects, worktree, path))
        }
        Ok(Control::OpenInEditor { worktree, path }) => {
            state.projects(move |projects| file::answer::editor(projects, worktree, path))
        }
        _ => {
            let message = "unexpected message from the app".to_owned();
            state.to_app(channel, &Control::Error { message }).await;
        }
    }
}

/// Writes queued frames, always draining control frames before terminal frames. Terminal
/// frames wait in one queue per terminal, taken in turn (9.19): a keystroke's echo waits for
/// at most one frame of each other terminal, however much a flooding one has queued.
async fn write_prioritized<W: AsyncWrite + Unpin>(
    mut writer: FramedWrite<W, FrameCodec>,
    mut control: mpsc::UnboundedReceiver<Frame>,
    mut terminal: mpsc::UnboundedReceiver<Frame>,
) {
    let mut turns = Turns::default();
    loop {
        while let Ok(frame) = terminal.try_recv() {
            turns.push(frame);
        }
        let frame = match control.try_recv() {
            Ok(frame) => frame,
            Err(_) => match turns.next() {
                Some(frame) => frame,
                None => tokio::select! {
                    biased;
                    Some(frame) = control.recv() => frame,
                    Some(frame) = terminal.recv() => frame,
                    else => return,
                },
            },
        };
        match writer.send(frame).await {
            // Nothing was written: the app misses this message, not every later one.
            Err(FrameError::Oversized(len)) => {
                eprintln!("hive: warning: dropped a {len}-byte message to the app");
            }
            Err(_) => return,
            Ok(()) => {}
        }
    }
}

/// Terminal frames by terminal, handed out one terminal after the other.
#[derive(Default)]
struct Turns {
    queues: BTreeMap<u32, VecDeque<Frame>>,
    /// The terminal whose frame went last.
    last: u32,
}

impl Turns {
    fn push(&mut self, frame: Frame) {
        self.queues
            .entry(frame.channel)
            .or_default()
            .push_back(frame);
    }

    /// The next frame of the first terminal after the last one served, wrapping around.
    fn next(&mut self) -> Option<Frame> {
        let after = (
            std::ops::Bound::Excluded(self.last),
            std::ops::Bound::Unbounded,
        );
        let first = || self.queues.keys().next();
        let channel = *self
            .queues
            .range(after)
            .next()
            .map(|(c, _)| c)
            .or_else(first)?;
        let queue = self.queues.get_mut(&channel)?;
        let frame = queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(&channel);
        }
        self.last = channel;
        frame
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Instant;

    use hive_protocol::AgentEvent;

    use super::super::test_state;
    use super::*;
    use crate::states::Agent;

    #[tokio::test]
    async fn control_frames_are_written_before_queued_terminal_frames() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();
        terminal_tx.send(Frame::terminal(1, "out")).unwrap();
        terminal_tx.send(Frame::terminal(1, "more")).unwrap();
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
    async fn terminals_take_turns_to_be_written() {
        let (_control_tx, control_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();
        for frame in [(2, "a1"), (2, "a2"), (2, "a3"), (1, "b1"), (u32::MAX, "c1")] {
            terminal_tx.send(Frame::terminal(frame.0, frame.1)).unwrap();
        }
        let (client, server) = tokio::io::duplex(1024);
        let writer = FramedWrite::new(server, FrameCodec);
        tokio::spawn(write_prioritized(writer, control_rx, terminal_rx));
        let mut read = FramedRead::new(client, FrameCodec);
        let time = std::time::Duration::from_secs(5);
        let mut next = async || tokio::time::timeout(time, read.next()).await.unwrap();
        let mut next = async || next().await.unwrap().unwrap().payload;
        // The first after the lowest channel, wrapping around after the highest.
        for want in ["b1", "a1", "c1", "a2", "a3"] {
            assert_eq!(next().await, want);
        }
        // Frames queued after the others emptied are written as they come.
        terminal_tx.send(Frame::terminal(5, "d1")).unwrap();
        assert_eq!(next().await, "d1");
    }

    #[tokio::test]
    async fn a_frame_too_big_to_write_is_dropped_and_writing_goes_on() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();
        let huge = Frame {
            kind: FrameType::Control,
            channel: 0,
            payload: bytes::Bytes::from(vec![b' '; hive_protocol::MAX_PAYLOAD + 1]),
        };
        control_tx.send(huge).unwrap();
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
        assert_eq!(frames, vec![Frame::control(0, &Control::CloseTerminal)]);
    }

    #[tokio::test]
    async fn writer_stops_when_the_peer_is_gone() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (_terminal_tx, terminal_rx) = mpsc::unbounded_channel::<Frame>();
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

    #[tokio::test]
    async fn a_connecting_app_gets_the_state_of_every_live_agent() {
        // Agents outlive an app connection only in principle (the service exits with the
        // app), so the snapshot is checked here rather than through a real daemon.
        let dir = tempfile::tempdir().unwrap();
        let mut named = Agent::new(4, Instant::now(), 0);
        named.title = Some("Named".into());
        let log = dir.path().join("s.jsonl");
        let turn =
            r#"{"type":"assistant","message":{"usage":{"input_tokens":7,"output_tokens":2}}}"#;
        std::fs::write(&log, format!("{turn}\n")).unwrap();
        let usage = Control::AgentUsage {
            id: "s".into(),
            context_tokens: 7,
            context_limit: 200_000,
            output_tokens: 2,
        };
        let read = named.usage.read("s", &[dir.path().to_owned()], &log);
        assert_eq!(read, Some(usage.clone()));
        // A subagent working in a worktree of its own.
        let mut owning = Agent::new(5, Instant::now(), 0);
        owning.worktree = Some("/r".into());
        let start = AgentEvent {
            provider: "claude-code".into(),
            terminal_id: Some("5".into()),
            session_id: Some("u".into()),
            subagent: Some(hive_protocol::Subagent {
                id: "a".into(),
                agent_type: None,
            }),
            cwd: Some("/r/w".into()),
            kind: EventKind::SubagentStarted,
            activity: None,
            raw: serde_json::Value::Null,
        };
        owning.apply("u", &start, 0, Instant::now(), &|cwd| Some(cwd.to_owned()));
        let owning_state = owning.message("u");
        let state = test_state(dir.path());
        *state.agents.lock().await =
            HashMap::from([("s".to_owned(), named), ("u".to_owned(), owning)]);
        // The same stream types as the daemon, so no second instantiation skews line coverage
        // (measured on Linux only).
        #[cfg(unix)]
        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        #[cfg(windows)]
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(server);
        let serving = tokio::spawn({
            let state = state.clone();
            async move {
                let reader = FramedRead::new(read, FrameCodec);
                app_connection(reader, FramedWrite::new(write, FrameCodec), &state).await
            }
        });
        let mut frames = FramedRead::new(client, FrameCodec);
        let mut got = Vec::new();
        for _ in 0..6 {
            let next = tokio::time::timeout(std::time::Duration::from_secs(5), frames.next());
            let frame = next.await.expect("no snapshot").unwrap().unwrap();
            got.push((frame.channel, frame.to_control().unwrap()));
        }
        let idle = |id: &str| Control::AgentState {
            id: id.into(),
            state: hive_protocol::AgentState::Idle,
            urgency: 1,
            pending: false,
            interrupted: false,
            alert: None,
            notify: false,
            writing: false,
            subagents: vec![],
            activity: None,
            since_ms: 0,
        };
        // Each agent's state; the named one's name too, and only after its state.
        let named = Control::AgentTitle {
            id: "s".into(),
            title: "Named".into(),
        };
        let at = |message: &Control| got.iter().position(|(_, m)| m == message);
        // The settings come first.
        let settings = Control::Settings {
            settings: Default::default(),
        };
        assert_eq!(got[0], (0, settings));
        assert!(got.contains(&(4, idle("s"))), "{got:?}");
        assert!(got.contains(&(5, owning_state)), "{got:?}");
        // Then the worktrees subagents own.
        let owned = Control::SubagentWorktrees {
            worktrees: vec!["/r/w".into()],
        };
        assert_eq!(got.last(), Some(&(0, owned)));
        assert!(at(&named) > at(&idle("s")), "{got:?}");
        assert_eq!(got[at(&named).unwrap()].0, 4);
        // Its usage too, once known.
        assert!(at(&usage) > at(&idle("s")), "{got:?}");
        assert_eq!(got[at(&usage).unwrap()].0, 4);
        drop(frames);
        assert!(serving.await.unwrap());
    }
}
