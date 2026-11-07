//! The agents' browsers (24.6, D22.3): one per worktree, started when its agent's
//! `hive-browser` MCP server first asks for it, stopped when the worktree's last agent ends,
//! at most `browser.max` at once. Its screencast runs only while the app's panel shows it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hive_protocol::{BrowserSettings, Control};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;

use super::State;
use crate::browser::{self, Command, Event, Process};
use crate::projects;
use crate::states::Agent;

/// Where a snap's programs are: a browser there is skipped ([`browser::find`]).
const SNAP: &str = "/snap";

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
    /// The worktree whose browser the app's panel shows.
    watched: Option<String>,
    /// The worktrees with an agent, as last seen.
    places: HashSet<String>,
    /// The last browser number given: names its profile, tells it from a later one.
    last: u64,
}

struct Running {
    id: u64,
    port: u16,
    url: Option<String>,
    commands: mpsc::UnboundedSender<Command>,
    process: Process,
    /// An agent of its worktree was seen, so it stops when the last one ends (the MCP server
    /// may ask before its agent is detected).
    agent_seen: bool,
}

impl Browsers {
    /// What the app shows of `worktree`'s browser.
    fn status(&self, worktree: &str) -> Control {
        let running = self.running.get(worktree);
        Control::BrowserStatus {
            worktree: worktree.to_owned(),
            running: running.is_some(),
            url: running.and_then(|r| r.url.clone()),
            error: self.errors.get(worktree).cloned(),
        }
    }

    /// Applies browser `id`'s screencast `event`; what the app gets of it (`None`: its panel
    /// does not show it, or the browser was stopped meanwhile).
    fn event(&mut self, worktree: &str, id: u64, event: Event) -> Option<Control> {
        let running = self.running.get_mut(worktree).filter(|r| r.id == id)?;
        let message = match event {
            Event::Url(url) => {
                running.url = url;
                self.status(worktree)
            }
            Event::Frame(data) => Control::BrowserFrame {
                worktree: worktree.to_owned(),
                data,
            },
        };
        (self.watched.as_deref() == Some(worktree)).then_some(message)
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
        // Not under the lock: the app's panel goes on meanwhile.
        let started = self.start_browser(&settings, id).await;
        let mut browsers = self.browsers.lock().await;
        browsers.starting.remove(&worktree);
        browsers.started.send_replace(());
        let (process, ws, port) = match started {
            Ok(started) => started,
            Err(why) => return Err(self.failed(&mut browsers, &worktree, why).await),
        };
        let commands = self.cast(worktree.clone(), id, ws);
        let running = Running {
            id,
            port,
            url: None,
            commands,
            process,
            agent_seen: browsers.places.contains(&worktree),
        };
        browsers.errors.remove(&worktree);
        browsers.running.insert(worktree.clone(), running);
        if browsers.watched.as_ref() == Some(&worktree) {
            browsers.command(&worktree, Command::Watch(true));
            self.to_app(0, &browsers.status(&worktree)).await;
        }
        Ok(endpoint(port))
    }

    /// Keeps why `worktree`'s browser did not start, for its panel; `why`.
    async fn failed(&self, browsers: &mut Browsers, worktree: &str, why: String) -> String {
        browsers.errors.insert(worktree.to_owned(), why.clone());
        if browsers.watched.as_deref() == Some(worktree) {
            self.to_app(0, &browsers.status(worktree)).await;
        }
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
        browser::start(&program, self.browsers_dir.join(id.to_string())).await
    }

    /// Runs browser `id`'s screencast; its commands. When it ends (the browser is gone), the
    /// browser is stopped.
    fn cast(
        self: &Arc<Self>,
        worktree: String,
        id: u64,
        ws: WebSocketStream<TcpStream>,
    ) -> mpsc::UnboundedSender<Command> {
        let (commands, received) = mpsc::unbounded_channel();
        let (events, mut event) = mpsc::unbounded_channel();
        let state = self.clone();
        tokio::spawn(async move {
            let cast = browser::screencast(ws, received, move |e| {
                let _ = events.send(e);
            });
            // Ends with the screencast, which holds the sender.
            let forward = async {
                while let Some(e) = event.recv().await {
                    state.browser_event(&worktree, id, e).await;
                }
            };
            tokio::join!(cast, forward);
            let mut browsers = state.browsers.lock().await;
            if browsers.running.get(&worktree).is_some_and(|r| r.id == id) {
                state.stop_browser(&mut browsers, &worktree).await;
            }
        });
        commands
    }

    /// A screencast event of browser `id`: to the app while its panel shows it.
    async fn browser_event(&self, worktree: &str, id: u64, event: Event) {
        let mut browsers = self.browsers.lock().await;
        if let Some(message) = browsers.event(worktree, id, event) {
            self.to_app(0, &message).await;
        }
    }

    /// The app's panel shows `worktree`'s browser (`None`: no panel in view): only that one's
    /// screencast runs. Answered by its status.
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
        let mut ended = Vec::new();
        let Browsers {
            running, places, ..
        } = &mut *browsers;
        for (worktree, running) in running {
            if places.contains(worktree) {
                running.agent_seen = true;
            } else if running.agent_seen {
                ended.push(worktree.clone());
            }
        }
        for worktree in ended {
            self.stop_browser(&mut browsers, &worktree).await;
        }
    }

    /// Stops `worktree`'s browser in the background; its panel says so.
    async fn stop_browser(&self, browsers: &mut Browsers, worktree: &str) {
        if let Some(running) = browsers.running.remove(worktree) {
            tokio::spawn(running.process.stop());
        }
        if browsers.watched.as_deref() == Some(worktree) {
            self.to_app(0, &browsers.status(worktree)).await;
        }
    }

    /// Stops every browser and waits until they have: the service ends, or the setting went
    /// off.
    pub(super) async fn stop_browsers(&self) {
        let running: Vec<Running> = {
            let mut browsers = self.browsers.lock().await;
            let running = browsers.running.drain().map(|(_, r)| r).collect();
            if let Some(worktree) = browsers.watched.clone() {
                self.to_app(0, &browsers.status(&worktree)).await;
            }
            running
        };
        futures_util::future::join_all(running.into_iter().map(|r| r.process.stop())).await;
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
    }
}
