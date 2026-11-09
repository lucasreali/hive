//! The agents' browsers (24.6, D22.3): one per worktree, started when its agent's
//! `hive-browser` MCP server first asks for it, stopped when the worktree's last agent ends,
//! at most `browser.max` at once. Its screencast runs only while the app's tab shows it (25.2);
//! what it last showed is kept after it stops.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hive_protocol::{AgentEvent, BrowserSettings, Control, EventKind};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;

use super::State;
use crate::browser::{self, Command, Event, Process};
use crate::projects;
use crate::states::Agent;

/// Where a snap's programs are: a browser there is skipped ([`browser::find`]).
const SNAP: &str = "/snap";

/// How long a stopping browser gets to give its last picture.
const CAPTURE_WAIT: Duration = Duration::from_secs(2);

/// The service's browsers, by worktree.
#[derive(Default)]
pub(super) struct Browsers {
    running: HashMap<String, Running>,
    /// The worktrees whose browser is starting.
    starting: HashSet<String>,
    /// Bumped when a browser has started (or failed to): an ask waiting for it looks again.
    started: tokio::sync::watch::Sender<()>,
    /// Why each worktree's browser last failed to start.
    errors: HashMap<String, String>,
    /// The worktree whose browser the app's tab shows.
    watched: Option<String>,
    /// What each worktree's browser last showed: kept after it stops, until the app reloads (a
    /// tab is not kept across a reload).
    shown: HashMap<String, Shown>,
    /// The browsers stopping in the background: the service's end waits for them.
    stopping: Vec<JoinHandle<()>>,
    /// The worktrees with an agent, as last seen.
    places: HashSet<String>,
    /// The last browser number given: names its profile, tells it from a later one.
    last: u64,
}

/// What a browser last showed.
#[derive(Default)]
struct Shown {
    url: Option<String>,
    title: Option<String>,
    /// The newest frame, a JPEG in base64.
    frame: Option<String>,
}

struct Running {
    id: u64,
    port: u16,
    commands: mpsc::UnboundedSender<Command>,
    /// Notified once its last picture is in, or its screencast ended.
    captured: Arc<Notify>,
    process: Process,
    /// An agent of its worktree was seen, so it stops when the last one ends (the MCP server
    /// may ask before its agent is detected).
    agent_seen: bool,
}

impl Browsers {
    /// What the app shows of `worktree`'s browser.
    fn status(&self, worktree: &str) -> Control {
        let shown = self.shown.get(worktree);
        Control::BrowserStatus {
            worktree: worktree.to_owned(),
            running: self.running.contains_key(worktree),
            url: shown.and_then(|s| s.url.clone()),
            title: shown.and_then(|s| s.title.clone()),
            error: self.errors.get(worktree).cloned(),
        }
    }

    /// Applies browser `id`'s screencast `event`; what the app gets of it (`None`: nothing new,
    /// its tab does not show the frame, or the browser was stopped meanwhile).
    fn event(&mut self, worktree: &str, id: u64, event: Event) -> Option<Control> {
        let current = self.running.get(worktree).map(|r| r.id);
        let data = match event {
            // Its picture comes while it stops, unless a new browser took the worktree already.
            Event::Last(data) => data.filter(|_| current.is_none())?,
            _ if current != Some(id) => return None,
            Event::Frame(data) => data,
            Event::Url(url) => {
                self.shown(worktree).url = url;
                return Some(self.status(worktree));
            }
            Event::Title(title) => {
                self.shown(worktree).title = title;
                return Some(self.status(worktree));
            }
        };
        self.shown(worktree).frame = Some(data.clone());
        (self.watched.as_deref() == Some(worktree)).then(|| Control::BrowserFrame {
            worktree: worktree.to_owned(),
            data,
        })
    }

    fn shown(&mut self, worktree: &str) -> &mut Shown {
        self.shown.entry(worktree.to_owned()).or_default()
    }

    /// Keeps a stop under way, forgetting those done.
    fn stopping(&mut self, stop: JoinHandle<()>) {
        self.stopping.retain(|stop| !stop.is_finished());
        self.stopping.push(stop);
    }

    fn command(&self, worktree: &str, command: Command) {
        if let Some(running) = self.running.get(worktree) {
            let _ = running.commands.send(command);
        }
    }
}

impl State {
    /// `hive browser-mcp`'s ask: the CDP endpoint of the browser of the worktree holding `cwd`,
    /// started first when it is not running.
    pub(super) async fn browser_endpoint(self: &Arc<Self>, cwd: &str) -> Result<String, String> {
        let settings = self.settings.browser();
        if !settings.enabled {
            return Err("the agent's browser is off in Hive's settings".to_owned());
        }
        let place = tokio::task::block_in_place(|| projects::place(&self.projects.list(), cwd));
        let Some((_, worktree)) = place else {
            return Err(format!("{cwd} is in no worktree Hive follows"));
        };
        let id = loop {
            let mut browsers = self.browsers.lock().await;
            if let Some(running) = browsers.running.get(&worktree) {
                return Ok(endpoint(running.port));
            }
            if browsers.starting.contains(&worktree) {
                let mut started = browsers.started.subscribe();
                drop(browsers);
                let _ = started.changed().await;
                continue;
            }
            let open = browsers.running.len() + browsers.starting.len();
            if open >= settings.max as usize {
                let why = format!(
                    "No more browsers: browser.max is {}, and all are open",
                    settings.max
                );
                return Err(self.failed(&mut browsers, &worktree, why).await);
            }
            browsers.starting.insert(worktree.clone());
            browsers.last += 1;
            break browsers.last;
        };
        // Not under the lock: the app's tab goes on meanwhile.
        let started = self.start_browser(&settings, id).await;
        let mut browsers = self.browsers.lock().await;
        browsers.starting.remove(&worktree);
        browsers.started.send_replace(());
        let (process, ws, port) = match started {
            Ok(started) => started,
            Err(why) => return Err(self.failed(&mut browsers, &worktree, why).await),
        };
        let captured = Arc::new(Notify::new());
        let commands = self.cast(worktree.clone(), id, ws, captured.clone());
        let running = Running {
            id,
            port,
            commands,
            captured,
            process,
            agent_seen: browsers.places.contains(&worktree),
        };
        browsers.errors.remove(&worktree);
        browsers.running.insert(worktree.clone(), running);
        // A new browser: nothing of the last one's is shown any more.
        browsers.shown.remove(&worktree);
        if browsers.watched.as_ref() == Some(&worktree) {
            browsers.command(&worktree, Command::Watch(true));
        }
        self.to_app(0, &browsers.status(&worktree)).await;
        Ok(endpoint(port))
    }

    /// Keeps why `worktree`'s browser did not start, for its tab; `why`.
    async fn failed(&self, browsers: &mut Browsers, worktree: &str, why: String) -> String {
        browsers.errors.insert(worktree.to_owned(), why.clone());
        self.to_app(0, &browsers.status(worktree)).await;
        why
    }

    /// Finds the browser to run and starts it with profile `id`.
    async fn start_browser(
        self: &Arc<Self>,
        settings: &BrowserSettings,
        id: u64,
    ) -> Result<(Process, WebSocketStream<TcpStream>, u16), String> {
        // The service's own `PATH`: the browsers install in the system's folders.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let path = std::env::var_os("PATH");
        let candidates = browser::candidates(home.as_deref(), path.as_deref());
        let program = browser::find(settings.path.as_deref(), candidates, Path::new(SNAP))?;
        let profile = self.browsers_dir.join(id.to_string());
        browser::start(&program, profile, browser::START_TIMEOUT).await
    }

    /// Runs browser `id`'s screencast; its commands. When it ends (the browser is gone), the
    /// browser is stopped. `captured` is notified of its last picture, and when it ends.
    fn cast(
        self: &Arc<Self>,
        worktree: String,
        id: u64,
        ws: WebSocketStream<TcpStream>,
        captured: Arc<Notify>,
    ) -> mpsc::UnboundedSender<Command> {
        let (commands, received) = mpsc::unbounded_channel();
        let (events, mut event) = mpsc::unbounded_channel();
        let state = self.clone();
        tokio::spawn(async move {
            let cast = browser::screencast(ws, received, events);
            // Ends with the screencast, which holds the sender.
            let forward = async {
                while let Some(e) = event.recv().await {
                    let last = matches!(e, Event::Last(_));
                    state.browser_event(&worktree, id, e).await;
                    if last {
                        captured.notify_one();
                    }
                }
            };
            tokio::join!(cast, forward);
            let mut browsers = state.browsers.lock().await;
            // Unless it was stopped meanwhile (and maybe another started): ids are never reused.
            let gone: Vec<_> = browsers.running.extract_if(|_, r| r.id == id).collect();
            for (worktree, running) in gone {
                state.stop_browser(&mut browsers, &worktree, running).await;
            }
            drop(browsers);
            captured.notify_one();
        });
        commands
    }

    /// A screencast event of browser `id`: to the app; a frame only while its tab shows it.
    async fn browser_event(&self, worktree: &str, id: u64, event: Event) {
        let mut browsers = self.browsers.lock().await;
        if let Some(message) = browsers.event(worktree, id, event) {
            self.to_app(0, &message).await;
        }
    }

    /// The app's tab shows `worktree`'s browser (`None`: none in view): only that one's
    /// screencast runs. Answered by its status, and for a stopped one the frame kept.
    pub(super) async fn watch_browser(&self, worktree: Option<String>) {
        let mut browsers = self.browsers.lock().await;
        let old = std::mem::replace(&mut browsers.watched, worktree.clone());
        if let Some(old) = old.filter(|old| Some(old) != worktree.as_ref()) {
            browsers.command(&old, Command::Watch(false));
        }
        if let Some(worktree) = &worktree {
            // Again when already watched: a reloaded app has no frame.
            browsers.command(worktree, Command::Watch(true));
            self.to_app(0, &browsers.status(worktree)).await;
            let kept = browsers.shown.get(worktree).and_then(|s| s.frame.clone());
            if let Some(data) = kept.filter(|_| !browsers.running.contains_key(worktree)) {
                let worktree = worktree.clone();
                self.to_app(0, &Control::BrowserFrame { worktree, data })
                    .await;
            }
        }
    }

    /// A new or reloaded app has no tabs: nothing of a stopped browser is kept for them, and it
    /// hears of the browsers running.
    pub(super) async fn browsers_snapshot(&self) {
        let mut browsers = self.browsers.lock().await;
        let Browsers { running, shown, .. } = &mut *browsers;
        shown.retain(|worktree, _| running.contains_key(worktree));
        for worktree in browsers.running.keys() {
            self.to_app(0, &browsers.status(worktree)).await;
        }
    }

    /// Tells the app of `agent`'s `event` when it is a call of a tool of its worktree's running
    /// browser (25.2): the browser it drives is its own folder's, wherever a subagent went.
    pub(super) async fn browser_action(&self, id: &str, agent: &Agent, event: &AgentEvent) {
        let EventKind::ToolStarted { tool: Some(tool) } = &event.kind else {
            return;
        };
        let input = event
            .raw
            .get("tool_input")
            .unwrap_or(&serde_json::Value::Null);
        let (Some(action), Some(worktree)) = (browser::caption(tool, input), &agent.worktree)
        else {
            return;
        };
        if self.browsers.lock().await.running.contains_key(worktree) {
            let message = Control::BrowserAction {
                worktree: worktree.clone(),
                agent: id.to_owned(),
                agent_title: agent.title.clone(),
                action,
            };
            self.to_app(0, &message).await;
        }
    }

    /// The app drew the watched browser's last frame.
    pub(super) async fn browser_drawn(&self) {
        let browsers = self.browsers.lock().await;
        if let Some(worktree) = &browsers.watched {
            browsers.command(worktree, Command::Drawn);
        }
    }

    /// Stops the browsers whose worktree's last agent ended. Called with the agents lock held.
    pub(super) async fn browsers_follow(&self, agents: &HashMap<String, Agent>) {
        let mut browsers = self.browsers.lock().await;
        browsers.places = agents.values().filter_map(|a| a.worktree.clone()).collect();
        let Browsers {
            running, places, ..
        } = &mut *browsers;
        let ended: Vec<_> = running
            .extract_if(|worktree, running| {
                running.agent_seen |= places.contains(worktree);
                running.agent_seen && !places.contains(worktree)
            })
            .collect();
        for (worktree, running) in ended {
            self.stop_browser(&mut browsers, &worktree, running).await;
        }
    }

    /// Stops `worktree`'s browser, already taken out of `browsers`, in the background, once it
    /// gave its last picture; the app is told.
    async fn stop_browser(&self, browsers: &mut Browsers, worktree: &str, running: Running) {
        let Running {
            process,
            commands,
            captured,
            ..
        } = running;
        let _ = commands.send(Command::Capture);
        browsers.stopping(tokio::spawn(async move {
            let _ = tokio::time::timeout(CAPTURE_WAIT, captured.notified()).await;
            process.stop().await;
            drop(commands);
        }));
        self.to_app(0, &browsers.status(worktree)).await;
    }

    /// Stops every browser and waits until they have: the service ends, or the setting went
    /// off.
    pub(super) async fn stop_browsers(&self) {
        let (running, stopping): (Vec<_>, Vec<_>) = {
            let mut browsers = self.browsers.lock().await;
            let running: Vec<_> = browsers.running.drain().collect();
            for (worktree, _) in &running {
                self.to_app(0, &browsers.status(worktree)).await;
            }
            (running, std::mem::take(&mut browsers.stopping))
        };
        futures_util::future::join_all(running.into_iter().map(|(_, r)| r.process.stop())).await;
        futures_util::future::join_all(stopping).await;
    }

    /// The settings were saved: the wrapper passes the MCP server while the browser is on, and
    /// turning it off stops the browsers.
    pub(super) async fn browser_settings_changed(&self) {
        let enabled = self.settings.browser().enabled;
        let configured = std::env::current_exe()
            .and_then(|hive| browser::configure(&self.bin_dir, &hive, enabled));
        if let Err(err) = configured {
            eprintln!("hive: warning: cannot set up the agent's browser: {err}");
        }
        if !enabled {
            self.stop_browsers().await;
        }
    }
}

/// The CDP endpoint of a browser on `port`.
fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

#[cfg(test)]
mod tests {
    use super::super::test_state;
    use super::*;

    #[tokio::test]
    async fn a_stop_under_way_is_kept_and_those_done_forgotten() {
        let mut browsers = Browsers::default();
        let done = tokio::spawn(async {});
        while !done.is_finished() {
            tokio::task::yield_now().await;
        }
        browsers.stopping(done);
        let pending = tokio::spawn(std::future::pending());
        let id = pending.id();
        browsers.stopping(pending);
        browsers.stopping(tokio::spawn(std::future::pending()));
        let kept: Vec<_> = browsers.stopping.iter().map(JoinHandle::id).collect();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], id);
    }

    #[test]
    fn what_a_browser_showed_is_kept_after_it_stops_and_a_new_ones_is_not_replaced() {
        let mut browsers = Browsers::default();
        let frame = |data: &str| Control::BrowserFrame {
            worktree: "/r".into(),
            data: data.into(),
        };
        // Stopped, and unwatched: its last picture is kept, unsent.
        assert_eq!(browsers.event("/r", 1, Event::Last(None)), None);
        assert_eq!(browsers.event("/r", 1, Event::Last(Some("A".into()))), None);
        assert_eq!(browsers.shown["/r"].frame.as_deref(), Some("A"));
        // Watched: sent too.
        browsers.watched = Some("/r".into());
        let last = browsers.event("/r", 1, Event::Last(Some("B".into())));
        assert_eq!(last, Some(frame("B")));
        // Not a running browser's: its events are dropped.
        assert_eq!(browsers.event("/r", 1, Event::Url(None)), None);
        assert_eq!(browsers.event("/r", 1, Event::Frame("C".into())), None);
        assert_eq!(browsers.shown["/r"].frame.as_deref(), Some("B"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_config_that_cannot_change_is_reported_and_the_service_goes_on() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path());
        // A folder where the config goes (the bin folder is `tmp`): it is not a file to remove.
        let config = tmp.path().join(crate::browser::MCP_CONFIG);
        std::fs::create_dir(&config).unwrap();
        state.browser_settings_changed().await;
        assert!(config.is_dir());
        assert!(state.browsers.lock().await.running.is_empty());
        // On: it cannot be written there either.
        let mut on = hive_protocol::Settings::default();
        on.browser.enabled = true;
        tokio::task::block_in_place(|| state.settings.set(on)).unwrap();
        state.browser_settings_changed().await;
        assert!(config.is_dir());
    }
}
