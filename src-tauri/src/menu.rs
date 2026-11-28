//! The app menu on macOS (17.13): its structure ([`MENUS`]) and what Hive's own items do
//! ([`on_menu`]). `main.rs` turns [`MENUS`] into Tauri's menu, item for item.
//!
//! The menu only sends user actions (#37): Quit goes through [`Hive::request_quit`] (17.7), every
//! other item of Hive's own goes to the UI as `menu {item}`, which runs the action its shortcut or
//! button runs. The native items are AppKit's: Edit's Undo/Redo/Cut/Copy/Paste/Select All are what
//! lets ⌘C/⌘V reach text fields, the editor and the terminal's paste in WKWebView (17.8).

use hive_protocol::keymap::{Chord, Keymap, Platform, COMMAND_IDS};
use serde_json::json;
use std::sync::{Arc, Mutex, PoisonError};

use tauri::{AppHandle, Manager, Runtime};

use crate::Hive;

/// The id of the menu's Quit, which the app side answers itself.
pub const QUIT: &str = "quit";

/// One of the menus in the menu bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub role: Role,
    pub items: &'static [Item],
}

/// What macOS makes of a menu besides its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plain,
    /// The Window menu: macOS lists the app's windows in it.
    Window,
    /// The Help menu: macOS adds its search field.
    Help,
}

/// An item of a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// Hive's own: a click sends `id` to [`on_menu`]; `keys` is its accelerator in Tauri's form.
    Own {
        id: &'static str,
        text: &'static str,
        keys: Option<&'static str>,
    },
    /// One AppKit provides, with its own text, shortcut and action.
    Native(Native),
    Separator,
}

/// The predefined items Hive's menu uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Fullscreen,
    Minimize,
    Zoom,
}

/// The menu bar, in order. The first menu is the app's (macOS titles it with the app's name).
pub const MENUS: &[Menu] = &[
    Menu {
        title: "Hive",
        role: Role::Plain,
        items: &[
            Item::Native(Native::About),
            Item::Separator,
            Item::Own {
                id: "settings",
                text: "Settings…",
                keys: Some("CmdOrCtrl+,"),
            },
            Item::Own {
                id: "check-updates",
                text: "Check for Updates…",
                keys: None,
            },
            Item::Own {
                id: "uninstall",
                text: "Uninstall Hive…",
                keys: None,
            },
            Item::Separator,
            Item::Native(Native::Services),
            Item::Separator,
            Item::Native(Native::Hide),
            Item::Native(Native::HideOthers),
            Item::Native(Native::ShowAll),
            Item::Separator,
            Item::Own {
                id: QUIT,
                text: "Quit Hive",
                keys: Some("CmdOrCtrl+Q"),
            },
        ],
    },
    Menu {
        title: "Edit",
        role: Role::Plain,
        items: &[
            Item::Native(Native::Undo),
            Item::Native(Native::Redo),
            Item::Separator,
            Item::Native(Native::Cut),
            Item::Native(Native::Copy),
            Item::Native(Native::Paste),
            Item::Native(Native::SelectAll),
        ],
    },
    Menu {
        title: "View",
        role: Role::Plain,
        items: &[
            Item::Own {
                id: "toggle-sidebar",
                text: "Toggle Sidebar",
                keys: None,
            },
            Item::Own {
                id: "toggle-panel",
                text: "Toggle Side Panel View",
                keys: Some("CmdOrCtrl+Shift+B"),
            },
            Item::Separator,
            Item::Native(Native::Fullscreen),
        ],
    },
    Menu {
        title: "Window",
        role: Role::Window,
        items: &[
            Item::Native(Native::Minimize),
            Item::Native(Native::Zoom),
            Item::Separator,
            Item::Own {
                id: "close-tab",
                text: "Close Tab",
                keys: Some("CmdOrCtrl+W"),
            },
        ],
    },
    Menu {
        title: "Help",
        role: Role::Help,
        items: &[
            Item::Own {
                id: "help",
                text: "Hive Help",
                keys: None,
            },
            Item::Own {
                id: "report-issue",
                text: "Report an Issue…",
                keys: None,
            },
        ],
    },
];

/// What a click on the item `id` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Quit, asking first when agents would end (17.7).
    Quit,
    /// The UI runs the item's action.
    Ui(&'static str),
    /// Not one of Hive's own items (a native one is AppKit's): nothing.
    None,
}

/// The action of the item `id`: only Hive's own items have one.
pub fn action(id: &str) -> Action {
    let own = MENUS
        .iter()
        .flat_map(|menu| menu.items)
        .find_map(|item| match item {
            Item::Own { id: own, .. } if *own == id => Some(*own),
            _ => None,
        });
    match own {
        Some(QUIT) => Action::Quit,
        Some(own) => Action::Ui(own),
        None => Action::None,
    }
}

/// A click on the menu item `id` (Tauri's menu event).
pub fn on_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if let Some(hive) = app.try_state::<Hive>() {
        match action(id) {
            Action::Quit => hive.request_quit(app),
            Action::Ui(item) => {
                hive.link().to_ui(json!({"type": "menu", "item": item}));
            }
            Action::None => {}
        }
    }
}

/// Hive's own items that are app commands (28.7), each with its command's macOS default (#35):
/// their accelerators follow the keymap ([`accelerators`]).
pub const COMMAND_ITEMS: &[(&str, &str)] = &[
    ("settings", "Cmd+,"),
    ("toggle-panel", "Shift+Cmd+B"),
    ("close-tab", "Cmd+W"),
];

/// The accelerator (Tauri's form) of each item of [`COMMAND_ITEMS`] for the settings'
/// `keymap`: the first of its command's chords (its entry's, else its default) that the UI
/// gives to that command, as `shortcut()` decides it: a chord another command's entry holds
/// beats a default, and of two entries the command first in [`COMMAND_IDS`] wins. None when it
/// wins none, so the menu never shows (or takes) a key the UI gives to another command.
pub fn accelerators(keymap: &Keymap) -> Vec<(&'static str, Option<String>)> {
    let place = |id: &str| COMMAND_IDS.iter().position(|c| *c == id);
    let entry = |id: &str| keymap.chords(id, Platform::Mac);
    let accelerator = |id: &'static str, default: &str| {
        let own = entry(id);
        let default = Chord::parse(default, Platform::Mac).into_iter().collect();
        let mine: Vec<Chord> = own.clone().unwrap_or(default);
        let wins = |chord: &Chord| {
            COMMAND_IDS
                .iter()
                .filter(|other| **other != id)
                .all(|other| {
                    let theirs = entry(other).unwrap_or_default();
                    !theirs.contains(chord) || (own.is_some() && place(other) > place(id))
                })
        };
        mine.iter().find(|c| wins(c)).map(Chord::accelerator)
    };
    COMMAND_ITEMS
        .iter()
        .map(|(id, default)| (*id, accelerator(id, default)))
        .collect()
}

/// The accelerators set on the menu, by item: what [`apply`] sent, less what failed.
pub type Applied = Arc<Mutex<Vec<(&'static str, Option<String>)>>>;

/// Sets the menu's accelerators (28.7), behind a trait so the app side is tested without
/// AppKit (`main.rs` holds Tauri's). The menu's calls wait for the main thread, so
/// [`MenuKeys::post`] never waits: the reader posts them while it holds the link, which the main
/// thread may be waiting for.
pub trait MenuKeys: Send + Sync {
    /// Runs `job` on the main thread later, never waiting for it; why when it cannot.
    fn post(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), String>;
    /// On the main thread: the menu bar's submenus, in order.
    fn submenus(&self) -> Result<Vec<Box<dyn Submenu>>, String>;
}

/// One menu of the menu bar.
pub trait Submenu {
    /// Gives its item `id` the accelerator `keys` (none: no key); `None` when it has no such
    /// item.
    fn set(&self, id: &str, keys: Option<&str>) -> Option<Result<(), String>>;
}

/// On the main thread: gives the item `id`, in whichever submenu holds it, the accelerator
/// `keys`.
pub fn set_key(menu: &dyn MenuKeys, id: &str, keys: Option<&str>) -> Result<(), String> {
    let submenus = menu.submenus()?;
    let set = submenus.iter().find_map(|submenu| submenu.set(id, keys));
    set.unwrap_or_else(|| Err(format!("the menu has no item {id}")))
}

fn lock(applied: &Applied) -> std::sync::MutexGuard<'_, Vec<(&'static str, Option<String>)>> {
    applied.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Forgets what was sent for the item `id`, so the next keymap sends it again.
fn forget(applied: &Applied, id: &str) {
    lock(applied).retain(|(item, _)| *item != id);
}

/// The accelerators of `wanted` that `applied` does not hold yet.
pub fn changed(
    applied: &Applied,
    wanted: Vec<(&'static str, Option<String>)>,
) -> Vec<(&'static str, Option<String>)> {
    let applied = lock(applied);
    wanted
        .into_iter()
        .filter(|w| !applied.contains(w))
        .collect()
}

/// Posts the accelerator `keys` of the item `id` to `menu`, recorded in `applied` first: a
/// post or a set that fails forgets it, so the next keymap tries again.
pub fn apply(menu: &Arc<dyn MenuKeys>, applied: &Applied, id: &'static str, keys: Option<String>) {
    {
        let mut applied = lock(applied);
        applied.retain(|(item, _)| *item != id);
        applied.push((id, keys.clone()));
    }
    let (job_menu, job_applied) = (Arc::clone(menu), Arc::clone(applied));
    let job = Box::new(move || {
        if let Err(error) = set_key(&*job_menu, id, keys.as_deref()) {
            eprintln!("hive-app: the menu item {id} keeps its key: {error}");
            forget(&job_applied, id);
        }
    });
    if let Err(error) = menu.post(job) {
        eprintln!("hive-app: the menu's keys are not set: {error}");
        forget(applied, id);
    }
}
