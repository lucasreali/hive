//! The menu bar icon (17.22): the agents' most urgent state as the icon, and a menu of the
//! pending agents and the usage ring's text. The UI sends what the service told it (#37); the
//! menu and the icon are built here, and `main.rs` only hands them to the tray.

use hive_protocol::AgentState;
use serde_json::json;
use tauri::{AppHandle, Runtime};

use crate::{bring_up, lock, printable, Hive};

/// The tray's id.
pub const TRAY: &str = "hive";
/// The menu item that only brings Hive up.
pub const SHOW: &str = "show";
/// The prefix of a pending agent's menu item id, before the agent's id.
pub const AGENT: &str = "agent:";
/// The most pending agents listed; the rest are counted in one line.
pub const MAX_AGENTS: usize = 20;
/// The most usage lines shown.
pub const MAX_USAGE: usize = 4;
/// The most characters of a menu text (an agent's title, a usage line).
pub const MAX_TEXT: usize = 80;
/// The icon's side in pixels: 18 points at 2x, the menu bar's height for an icon.
pub const SIZE: u32 = 36;

/// A pending agent as the UI lists it: its id, its title (or "Claude"), its state and, when
/// there are several spaces, its space's name.
pub type Agent = (String, String, AgentState, Option<String>);

/// One line of the tray's menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A menu item: with an `id`, it can be chosen (a pending agent, [`AGENT`] + its id, or
    /// "Show Hive", [`SHOW`]); without one, it is a disabled line of text.
    Item {
        id: Option<String>,
        text: String,
    },
    Separator,
}

/// A disabled line of text.
fn note(text: String) -> Entry {
    Entry::Item { id: None, text }
}

/// The icon: `SIZE`×`SIZE` RGBA pixels, drawn as a template (the menu bar's own colour) or in
/// the colour of the most urgent state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub rgba: Vec<u8>,
    pub template: bool,
}

/// What the tray shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub icon: Icon,
    pub tooltip: String,
    pub entries: Vec<Entry>,
}

/// Shows the tray's [`View`], or removes the tray (`None`); given by `main.rs` (`with_tray`).
pub type Show = dyn Fn(Option<View>) -> Result<(), String> + Send + Sync;

/// What a state says in the menu, as the sidebar says it.
pub fn state_text(state: AgentState) -> &'static str {
    match state {
        AgentState::WaitingPermission => "waiting for permission",
        AgentState::WaitingPlan => "waiting for plan approval",
        AgentState::WaitingAnswer => "waiting for your answer",
        AgentState::Error => "error",
        AgentState::WaitingYou => "waiting for you",
        AgentState::Working => "working",
        AgentState::WithSubagents => "running subagents",
        AgentState::Idle => "idle",
        AgentState::Ended => "ended",
    }
}

/// `text` without control characters, at most [`MAX_TEXT`] characters ("…" when cut).
fn short(text: &str) -> String {
    let text = printable(text);
    if text.chars().count() <= MAX_TEXT {
        return text;
    }
    let mut cut: String = text.chars().take(MAX_TEXT - 1).collect();
    cut.push('…');
    cut
}

/// The menu: the pending agents in the UI's order (tree order, at most [`MAX_AGENTS`], "N more
/// in Hive" for the rest) or "No agent needs you", the usage lines (at most [`MAX_USAGE`]), then
/// "Show Hive".
pub fn entries(agents: &[Agent], usage: &[String]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = agents
        .iter()
        .take(MAX_AGENTS)
        .map(|(id, title, state, space)| {
            let what = [space.as_deref(), Some(state_text(*state))];
            let what: Vec<&str> = what.into_iter().flatten().collect();
            Entry::Item {
                id: Some(format!("{AGENT}{id}")),
                text: short(&format!("{title} — {}", what.join(" · "))),
            }
        })
        .collect();
    if agents.is_empty() {
        entries.push(note("No agent needs you".into()));
    }
    if agents.len() > MAX_AGENTS {
        entries.push(note(format!("{} more in Hive", agents.len() - MAX_AGENTS)));
    }
    let usage: Vec<Entry> = usage
        .iter()
        .take(MAX_USAGE)
        .map(|line| note(short(line)))
        .collect();
    if !usage.is_empty() {
        entries.push(Entry::Separator);
        entries.extend(usage);
    }
    entries.push(Entry::Separator);
    entries.push(Entry::Item {
        id: Some(SHOW.into()),
        text: "Show Hive".into(),
    });
    entries
}

/// The colour of a state on the icon (the One Light state tokens, which read on a light and a
/// dark menu bar), or `None` for a state drawn as a template: idle, ended, and waiting for you
/// once seen (not pending, drawn muted in the sidebar, 15.6).
pub fn tint(state: AgentState, pending: bool) -> Option<[u8; 3]> {
    match state {
        AgentState::WaitingPermission | AgentState::WaitingPlan | AgentState::WaitingAnswer => {
            Some([0xa4, 0x88, 0x19])
        }
        AgentState::Error => Some([0xd3, 0x61, 0x51]),
        AgentState::WaitingYou if pending => Some([0xad, 0x6e, 0x25]),
        AgentState::Working => Some([0x5c, 0x78, 0xe2]),
        AgentState::WithSubagents => Some([0xa4, 0x49, 0xab]),
        AgentState::WaitingYou | AgentState::Idle | AgentState::Ended => None,
    }
}

/// How much of the pixel at (`x`, `y`) a hexagon of `radius` (centre to corner, pointy top)
/// centred on the icon covers, in 0..=255, from 4×4 samples. A sample inside the hexagon and
/// outside the one of `inner` (0 for none) counts: an outline, or a filled hexagon.
fn coverage(x: u32, y: u32, radius: f64, inner: f64) -> u8 {
    let centre = f64::from(SIZE) / 2.0;
    let inside = |px: f64, py: f64, r: f64| {
        let (dx, dy) = ((px - centre).abs(), (py - centre).abs());
        // A pointy-top hexagon: within the flat sides (|dx| ≤ r·√3/2) and the slanted ones.
        let half = r * 3f64.sqrt() / 2.0;
        dx <= half && dx / 3f64.sqrt() + dy <= r
    };
    let mut hits = 0_u32;
    for sy in 0..4 {
        for sx in 0..4 {
            let px = f64::from(x) + (f64::from(sx) + 0.5) / 4.0;
            let py = f64::from(y) + (f64::from(sy) + 0.5) / 4.0;
            hits += u32::from(inside(px, py, radius) && !inside(px, py, inner));
        }
    }
    u8::try_from(hits * 255 / 16).unwrap_or(u8::MAX)
}

/// The icon for the most urgent state (`None`: no agent): a hexagon outline as a template, or
/// a filled hexagon in the state's [`tint`].
pub fn icon(urgent: Option<(AgentState, bool)>) -> Icon {
    let tint = urgent.and_then(|(state, pending)| tint(state, pending));
    let radius = f64::from(SIZE) / 2.0 - 2.0;
    let (colour, inner) = match tint {
        Some(colour) => (colour, 0.0),
        None => ([0, 0, 0], radius - 4.0),
    };
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            rgba.extend_from_slice(&colour);
            rgba.push(coverage(x, y, radius, inner));
        }
    }
    Icon {
        rgba,
        template: tint.is_none(),
    }
}

/// The tooltip: "Hive", with how many agents need you.
pub fn tooltip(pending: usize) -> String {
    match pending {
        0 => "Hive".into(),
        1 => "Hive — 1 agent needs you".into(),
        n => format!("Hive — {n} agents need you"),
    }
}

/// The whole view for what the UI sent.
pub fn view(urgent: Option<(AgentState, bool)>, agents: &[Agent], usage: &[String]) -> View {
    View {
        icon: icon(urgent),
        tooltip: tooltip(agents.len()),
        entries: entries(agents, usage),
    }
}

impl Hive {
    /// Shows the menu bar icon with `show`, the platform's tray (17.22).
    pub fn with_tray(
        mut self,
        show: impl Fn(Option<View>) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.tray = Some(Box::new(show));
        self
    }

    /// Shows the tray for what the UI sent (`shown`: the setting is on), or removes it.
    pub fn set_tray(
        &self,
        shown: bool,
        urgent: Option<(AgentState, bool)>,
        agents: &[Agent],
        usage: &[String],
    ) -> Result<(), String> {
        let show = self.tray.as_ref().ok_or("this app has no menu bar icon")?;
        show(shown.then(|| view(urgent, agents, usage)))
    }
}

/// A menu item chosen (`id`): Hive comes to the front and, for a pending agent, the UI goes to
/// it (`tray_clicked {agent}`, as a notification's click; sent even with the window gone, as a
/// reload brings it back). An unknown id does nothing. Answers whether a window came up.
pub fn chosen<R: Runtime>(app: &AppHandle<R>, hive: &Hive, id: &str) -> bool {
    if id == SHOW {
        return bring_up(app);
    }
    let Some(agent) = id.strip_prefix(AGENT) else {
        return false;
    };
    lock(&hive.link).to_ui(json!({"type": "tray_clicked", "agent": agent}));
    bring_up(app)
}

#[cfg(test)]
mod tests;
