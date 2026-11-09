//! The agents' browsers (24.6, D22.3): one per worktree, started when its agent's
//! `hive-browser` MCP server first asks for it, stopped when the worktree's last agent ends,
//! at most `browser.max` at once. Its screencast runs only while the app's tab shows it (25.2);
//! what it last showed is kept after it stops.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hive_protocol::{AgentEvent, BrowserSettings, Control, EventKind};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;

use super::State;
use crate::adapter::clip;
use crate::browser::{self, Command, Event, Process};
use crate::projects;
use crate::states::Agent;

/// Where a snap's programs are: a browser there is skipped ([`browser::find`]).
const SNAP: &str = "/snap";

/// How long a stopping browser gets to give its last picture.
const CAPTURE_WAIT: Duration = Duration::from_secs(2);

/// The most often a browser's page is told to the app: a page can change its title at will, and
/// control messages go before every terminal's output.
const STATUS_EVERY: Duration = Duration::from_millis(250);

/// The most stopped browsers whose last page is kept (a frame may be 3 MiB): the oldest goes.
const MAX_STOPPED: usize = 8;

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
    /// The worktrees of `shown` whose browser stopped, the oldest first.
    stopped: Vec<String>,
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

    /// Applies browser `id`'s screencast `event`; the frame the app gets of it (`None`: its page is
    /// told at a pace, its tab does not show the frame, or the browser was stopped meanwhile).
    fn event(&mut self, worktree: &str, id: u64, event: Event) -> Option<Control> {
        let current = self.running.get(worktree).map(|r| r.id);
        let data = match event {
            // Its picture comes while it stops, unless a new browser took the worktree already
            // or its place was given up meanwhile.
            Event::Last(data) => {
                let kept = self.stopped.iter().any(|w| w == worktree);
                data.filter(|_| current.is_none() && kept)?
            }
            _ if current != Some(id) => return None,
            Event::Frame(data) => data,
            Event::Url(url) => {
                self.shown(worktree).url = url;
                return None;
            }
            Event::Title(title) => {
                self.shown(worktree).title = title;
                return None;
            }
        };
        self.shown(worktree).frame = Some(data.clone());
        (self.watched.as_deref() == Some(worktree)).then(|| Control::BrowserFrame {
            worktree: worktree.to_owned(),
            data,
        })
    }

    /// `worktree`'s browser stopped: what it showed is kept, within [`MAX_STOPPED`].
    fn stopped(&mut self, worktree: &str) {
        self.stopped.retain(|w| w != worktree);
        self.stopped.push(worktree.to_owned());
        if self.stopped.len() > MAX_STOPPED {
            let oldest = self.stopped.remove(0);
            self.shown.remove(&oldest);
        }
    }

    /// Only `followed` worktrees keep what a stopped browser showed.
    fn followed(&mut self, followed: &HashSet<String>) {
        let Self {
            running,
            shown,
            stopped,
            ..
        } = self;
        shown.retain(|worktree, _| running.contains_key(worktree) || followed.contains(worktree));
        stopped.retain(|worktree| followed.contains(worktree));
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
        browsers.stopped.retain(|w| *w != worktree);
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
        let (events, event) = mpsc::unbounded_channel();
        let state = self.clone();
        tokio::spawn(async move {
            let cast = browser::screencast(ws, received, events);
            // Ends with the screencast, which holds the sender.
            let forward = forward(&state, &worktree, id, event, &captured);
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

    /// Tells the app of browser `id`'s page, unless it was stopped meanwhile.
    async fn browser_status(&self, worktree: &str, id: u64) {
        let browsers = self.browsers.lock().await;
        if browsers.running.get(worktree).is_some_and(|r| r.id == id) {
            self.to_app(0, &browsers.status(worktree)).await;
        }
    }

    /// A screencast event of browser `id`: a frame to the app only while its tab shows it.
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
        let Browsers {
            running,
            shown,
            stopped,
            ..
        } = &mut *browsers;
        shown.retain(|worktree, _| running.contains_key(worktree));
        stopped.clear();
        for worktree in browsers.running.keys() {
            self.to_app(0, &browsers.status(worktree)).await;
        }
    }

    /// The worktrees followed changed: a stopped browser's page is kept only for those.
    pub(super) async fn browsers_unfollowed(&self) {
        let followed: HashSet<String> = tokio::task::block_in_place(|| self.projects.list())
            .iter()
            .flat_map(|project| project.worktrees.iter().map(|w| w.id.clone()))
            .collect();
        self.browsers.lock().await.followed(&followed);
    }

    /// Tells the app of `agent`'s `event` when it is a call of a tool of its worktree's running
    /// browser (25.2): the browser it drives is its own folder's, wherever a subagent went.
    pub(super) async fn browser_action(&self, id: &str, agent: &Agent, event: &AgentEvent) {
        let Some((worktree, message)) = action(id, agent, event) else {
            return;
        };
        if self.browsers.lock().await.running.contains_key(&worktree) {
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
        browsers.stopped(worktree);
        browsers.stopping(tokio::spawn(async move {
            picture(&captured).await;
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
                browsers.stopped(worktree);
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

/// Waits for a stopping browser's last picture, at most [`CAPTURE_WAIT`].
async fn picture(captured: &Notify) {
    let _ = tokio::time::timeout(CAPTURE_WAIT, captured.notified()).await;
}

/// When a browser's page may next be told to the app: at once, then at most every
/// [`STATUS_EVERY`], the latest of the changes meanwhile.
struct Pace {
    next: Instant,
    /// A change waits for `next`.
    due: bool,
}

impl Pace {
    fn new(now: Instant) -> Self {
        Self {
            next: now,
            due: false,
        }
    }

    /// The page changed at `now`: whether to tell it now, else it waits for `next`.
    fn changed(&mut self, now: Instant) -> bool {
        let tell = now >= self.next;
        self.due = !tell;
        if tell {
            self.sent(now);
        }
        tell
    }

    /// The page was told at `now`.
    fn sent(&mut self, now: Instant) {
        self.next = now + STATUS_EVERY;
        self.due = false;
    }
}

/// Applies browser `id`'s screencast `events` until they end, at the [`Pace`] for its page;
/// `captured` is notified of its last picture.
async fn forward(
    state: &State,
    worktree: &str,
    id: u64,
    mut events: mpsc::UnboundedReceiver<Event>,
    captured: &Notify,
) {
    let mut pace = Pace::new(Instant::now());
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { return };
                let page = matches!(event, Event::Url(_) | Event::Title(_));
                let last = matches!(event, Event::Last(_));
                state.browser_event(worktree, id, event).await;
                if last {
                    captured.notify_one();
                }
                if page && pace.changed(Instant::now()) {
                    state.browser_status(worktree, id).await;
                }
            }
            () = tokio::time::sleep_until(pace.next), if pace.due => {
                pace.sent(Instant::now());
                state.browser_status(worktree, id).await;
            }
        }
    }
}

/// The app's message for `event` when it is `agent`'s call of a browser tool, and the worktree
/// whose browser it drives (the agent's own folder's, wherever a subagent went).
fn action(id: &str, agent: &Agent, event: &AgentEvent) -> Option<(String, Control)> {
    let EventKind::ToolStarted { tool: Some(tool) } = &event.kind else {
        return None;
    };
    let input = event.raw.get("tool_input").unwrap_or(&Value::Null);
    let action = browser::caption(tool, input)?;
    let worktree = agent.worktree.clone()?;
    // A session's name is untrusted text too: nothing in it reorders the action after it.
    let title = agent.title.as_deref().map(|t| clip(t, browser::MAX_QUOTE));
    let message = Control::BrowserAction {
        worktree: worktree.clone(),
        agent: id.to_owned(),
        agent_title: title.filter(|t| !t.is_empty()),
        action,
    };
    Some((worktree, message))
}

/// The CDP endpoint of a browser on `port`.
fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

#[cfg(test)]
mod tests {
    use super::super::test_state;
    use super::*;

    /// A browser `id` of a worktree, for what happens around it: its program has ended.
    async fn running(id: u64) -> (Running, mpsc::UnboundedReceiver<Command>) {
        let (commands, received) = mpsc::unbounded_channel();
        let mut sh = tokio::process::Command::new("/bin/sh");
        let child = sh.args(["-c", "exit 0"]).spawn().unwrap();
        let profile = std::env::temp_dir().join(format!("hive-test-no-profile-{id}"));
        let running = Running {
            id,
            port: 1,
            commands,
            captured: Arc::new(Notify::new()),
            process: Process::of(child, profile),
            agent_seen: true,
        };
        (running, received)
    }

    fn frame(worktree: &str, data: &str) -> Control {
        Control::BrowserFrame {
            worktree: worktree.into(),
            data: data.into(),
        }
    }

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
    fn a_stopped_browsers_last_picture_is_kept_and_sent_while_shown() {
        let mut browsers = Browsers::default();
        // A picture for a place given up (the app reloaded, it was evicted) is dropped.
        assert_eq!(browsers.event("/r", 1, Event::Last(Some("X".into()))), None);
        assert!(!browsers.shown.contains_key("/r"));
        browsers.stopped("/r");
        // Stopped, and unwatched: its last picture is kept, unsent.
        assert_eq!(browsers.event("/r", 1, Event::Last(None)), None);
        assert_eq!(browsers.event("/r", 1, Event::Last(Some("A".into()))), None);
        assert_eq!(browsers.shown["/r"].frame.as_deref(), Some("A"));
        // Watched: sent too.
        browsers.watched = Some("/r".into());
        let last = browsers.event("/r", 1, Event::Last(Some("B".into())));
        assert_eq!(last, Some(frame("/r", "B")));
        // Not a running browser's: its events are dropped.
        assert_eq!(browsers.event("/r", 1, Event::Url(None)), None);
        assert_eq!(browsers.event("/r", 1, Event::Frame("C".into())), None);
        assert_eq!(browsers.shown["/r"].frame.as_deref(), Some("B"));
    }

    #[tokio::test]
    async fn a_new_browser_is_not_overwritten_by_what_an_older_one_sends_late() {
        let mut browsers = Browsers::default();
        browsers.stopped("/r");
        let (new, _commands) = running(2).await;
        browsers.running.insert("/r".into(), new);
        browsers.watched = Some("/r".into());
        // The new one's own.
        let own = browsers.event("/r", 2, Event::Frame("NEW".into()));
        assert_eq!(own, Some(frame("/r", "NEW")));
        assert_eq!(
            browsers.event("/r", 2, Event::Url(Some("new".into()))),
            None
        );
        assert_eq!(
            browsers.event("/r", 2, Event::Title(Some("T".into()))),
            None
        );
        // The older one's late picture, frame, address and title change nothing.
        assert_eq!(
            browsers.event("/r", 1, Event::Last(Some("OLD".into()))),
            None
        );
        assert_eq!(browsers.event("/r", 1, Event::Frame("OLD".into())), None);
        assert_eq!(
            browsers.event("/r", 1, Event::Url(Some("old".into()))),
            None
        );
        assert_eq!(
            browsers.event("/r", 1, Event::Title(Some("O".into()))),
            None
        );
        let shown = &browsers.shown["/r"];
        assert_eq!(shown.frame.as_deref(), Some("NEW"));
        assert_eq!(shown.url.as_deref(), Some("new"));
        assert_eq!(shown.title.as_deref(), Some("T"));
        // Nor does a late picture of its own worktree's new browser, its own id included.
        assert_eq!(
            browsers.event("/r", 2, Event::Last(Some("OLD".into()))),
            None
        );
        assert_eq!(browsers.shown["/r"].frame.as_deref(), Some("NEW"));
    }

    #[test]
    fn only_so_many_stopped_browsers_keep_their_page_the_oldest_goes() {
        let mut browsers = Browsers::default();
        let name = |i: usize| format!("/w{i}");
        for i in 0..MAX_STOPPED {
            browsers.shown(&name(i)).frame = Some("A".into());
            browsers.stopped(&name(i));
        }
        assert_eq!(browsers.shown.len(), MAX_STOPPED);
        // Stopped again it is the newest, so another one goes first.
        browsers.stopped(&name(0));
        assert_eq!(browsers.stopped.len(), MAX_STOPPED);
        browsers.shown(&name(MAX_STOPPED)).frame = Some("A".into());
        browsers.stopped(&name(MAX_STOPPED));
        assert_eq!(browsers.shown.len(), MAX_STOPPED);
        assert!(browsers.shown.contains_key(&name(0)));
        assert!(!browsers.shown.contains_key(&name(1)));
    }

    #[tokio::test]
    async fn a_worktree_no_longer_followed_keeps_nothing_unless_its_browser_runs() {
        let mut browsers = Browsers::default();
        for worktree in ["/kept", "/gone", "/running"] {
            browsers.shown(worktree).frame = Some("A".into());
            browsers.stopped(worktree);
        }
        let (new, _commands) = running(1).await;
        browsers.running.insert("/running".into(), new);
        browsers.followed(&HashSet::from(["/kept".to_owned()]));
        let mut left: Vec<_> = browsers.shown.keys().map(String::as_str).collect();
        left.sort_unstable();
        assert_eq!(left, ["/kept", "/running"]);
        assert_eq!(browsers.stopped, ["/kept"]);
    }

    /// The app's messages so far.
    fn sent(app: &mut mpsc::UnboundedReceiver<hive_protocol::Frame>) -> Vec<Control> {
        let mut messages = vec![];
        while let Ok(frame) = app.try_recv() {
            messages.push(frame.to_control().unwrap());
        }
        messages
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_changing_its_title_all_the_time_is_told_at_a_pace_the_last_one_arriving() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path());
        let (to_app, mut app) = mpsc::unbounded_channel();
        *state.app.lock().await = Some(to_app);
        let (new, _commands) = running(1).await;
        state.browsers.lock().await.running.insert("/r".into(), new);
        let (events, received) = mpsc::unbounded_channel();
        let captured = Notify::new();
        let run = forward(&state, "/r", 1, received, &captured);
        let page = async {
            let started = Instant::now();
            // Two a millisecond for a second: 2000 changes.
            for i in 0..1000 {
                events.send(Event::Title(Some(format!("t{i}")))).unwrap();
                events.send(Event::Url(Some(format!("u{i}")))).unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let mut told = sent(&mut app);
            let most = (started.elapsed().as_millis() / STATUS_EVERY.as_millis()) as usize + 2;
            // Quiet again: the last of them arrives, within the pace.
            tokio::time::sleep(STATUS_EVERY).await;
            told.extend(sent(&mut app));
            assert!(told.len() <= most, "{} over {most}", told.len());
            let last = Control::BrowserStatus {
                worktree: "/r".into(),
                running: true,
                url: Some("u999".into()),
                title: Some("t999".into()),
                error: None,
            };
            assert_eq!(told.last(), Some(&last));
            // After a quiet while, a change is told at once.
            tokio::time::sleep(STATUS_EVERY).await;
            events.send(Event::Title(Some("now".into()))).unwrap();
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let now = Control::BrowserStatus {
                worktree: "/r".into(),
                running: true,
                url: Some("u999".into()),
                title: Some("now".into()),
                error: None,
            };
            assert_eq!(sent(&mut app), [now]);
            // Stopped with a change waiting: nothing more is said of it.
            events.send(Event::Title(Some("late".into()))).unwrap();
            tokio::task::yield_now().await;
            state.browsers.lock().await.running.clear();
            tokio::time::sleep(STATUS_EVERY * 2).await;
            assert_eq!(sent(&mut app), []);
            drop(events);
        };
        tokio::join!(run, page);
    }

    #[test]
    fn the_pace_tells_at_once_then_waits() {
        let start = Instant::now();
        let mut pace = Pace::new(start);
        assert!(pace.changed(start));
        assert!(!pace.due);
        let during = start + STATUS_EVERY - Duration::from_millis(1);
        assert!(!pace.changed(during));
        assert!(pace.due);
        assert_eq!(pace.next, start + STATUS_EVERY);
        // At the very end of the wait, it may be told.
        assert!(pace.changed(start + STATUS_EVERY));
        assert!(!pace.due);
        assert_eq!(pace.next, start + STATUS_EVERY * 2);
        pace.due = true;
        pace.sent(start + STATUS_EVERY * 3);
        assert!(!pace.due);
        assert_eq!(pace.next, start + STATUS_EVERY * 4);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_goes_on_as_soon_as_the_picture_is_in_else_after_two_seconds() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path());
        // No picture: the whole wait.
        let start = Instant::now();
        let captured = Notify::new();
        picture(&captured).await;
        assert_eq!(start.elapsed(), Duration::from_secs(2));
        // The screencast's answer ends the wait at once, with or without a picture.
        for answer in [Some("A".to_owned()), None] {
            let (events, received) = mpsc::unbounded_channel();
            let captured = Notify::new();
            let start = Instant::now();
            events.send(Event::Last(answer)).unwrap();
            drop(events);
            forward(&state, "/r", 1, received, &captured).await;
            picture(&captured).await;
            assert_eq!(start.elapsed(), Duration::ZERO);
        }
        // Any other event does not.
        let (events, received) = mpsc::unbounded_channel();
        let captured = Notify::new();
        let start = Instant::now();
        events.send(Event::Url(None)).unwrap();
        events.send(Event::Frame("A".into())).unwrap();
        drop(events);
        forward(&state, "/r", 1, received, &captured).await;
        picture(&captured).await;
        assert_eq!(start.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test]
    async fn the_end_of_the_service_waits_for_a_stop_under_way_and_asks_for_no_picture() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path());
        let (finish, finished) = tokio::sync::oneshot::channel::<()>();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = tokio::spawn({
            let done = done.clone();
            async move {
                let _ = finished.await;
                done.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let (new, mut commands) = running(1).await;
        {
            let mut browsers = state.browsers.lock().await;
            browsers.stopping(stop);
            browsers.running.insert("/r".into(), new);
            browsers.shown("/r").url = Some("u".into());
        }
        let ending = tokio::spawn({
            let state = state.clone();
            async move { state.stop_browsers().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!ending.is_finished());
        assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
        finish.send(()).unwrap();
        ending.await.unwrap();
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
        // Stopped for good: no picture asked, and what it showed is still kept.
        assert!(commands.try_recv().is_err());
        let browsers = state.browsers.lock().await;
        assert_eq!(browsers.stopped, ["/r"]);
        assert_eq!(browsers.shown["/r"].url.as_deref(), Some("u"));
    }

    #[tokio::test]
    async fn a_stop_for_the_agents_end_asks_for_the_picture_and_keeps_the_place() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path());
        let (new, mut commands) = running(1).await;
        new.captured.notify_one();
        let mut browsers = state.browsers.lock().await;
        state.stop_browser(&mut browsers, "/r", new).await;
        assert_eq!(browsers.stopped, ["/r"]);
        let stop = browsers.stopping.pop().unwrap();
        drop(browsers);
        stop.await.unwrap();
        assert_eq!(commands.recv().await, Some(Command::Capture));
    }

    /// `agent` calling a tool, its event's text as hooks send it.
    fn call(tool: &str, subagent: Option<&str>) -> AgentEvent {
        use crate::adapter::Adapter as _;
        let mut payload = serde_json::json!({
            "session_id": "s",
            "tool_name": tool,
            "tool_input": { "element": "More" },
        });
        if let Some(subagent) = subagent {
            payload["agent_id"] = serde_json::json!(subagent);
            payload["cwd"] = serde_json::json!("/elsewhere");
        }
        crate::adapter::ClaudeCode.translate("PreToolUse", None, payload)
    }

    #[tokio::test]
    async fn a_browser_call_names_its_agent_and_its_worktree_a_subagents_too() {
        let mut agent = Agent::new(1, std::time::Instant::now(), 0);
        let tool = "mcp__hive-browser__browser_click";
        // Nowhere: no browser to drive.
        assert_eq!(action("s", &agent, &call(tool, None)), None);
        agent.worktree = Some("/r".into());
        let told = |title: Option<&str>| {
            (
                "/r".to_owned(),
                Control::BrowserAction {
                    worktree: "/r".into(),
                    agent: "s".into(),
                    agent_title: title.map(Into::into),
                    action: "clicked 'More'".into(),
                },
            )
        };
        assert_eq!(action("s", &agent, &call(tool, None)), Some(told(None)));
        agent.title = Some("Fix login".into());
        assert_eq!(
            action("s", &agent, &call(tool, None)),
            Some(told(Some("Fix login")))
        );
        // A subagent elsewhere drives its agent's browser, and is told as its agent.
        assert_eq!(
            action("s", &agent, &call(tool, Some("a"))),
            Some(told(Some("Fix login")))
        );
        // A session's name cannot reorder what follows it, nor be long, nor blank.
        agent.title = Some("evil\u{202E}name".into());
        assert_eq!(
            action("s", &agent, &call(tool, None)),
            Some(told(Some("evilname")))
        );
        agent.title = Some("\u{3164}\u{202E}".into());
        assert_eq!(action("s", &agent, &call(tool, None)), Some(told(None)));
        agent.title = Some("t".repeat(10_000));
        let capped = "t".repeat(browser::MAX_QUOTE - 1) + "…";
        assert_eq!(
            action("s", &agent, &call(tool, None)),
            Some(told(Some(&capped)))
        );
        // Another tool, or another event of that tool, is no action.
        assert_eq!(action("s", &agent, &call("Read", None)), None);
        use crate::adapter::Adapter as _;
        let later = crate::adapter::ClaudeCode.translate(
            "PostToolUse",
            None,
            serde_json::json!({ "session_id": "s", "tool_name": tool }),
        );
        assert_eq!(action("s", &agent, &later), None);
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
