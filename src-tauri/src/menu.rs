//! The app menu on macOS (17.13): its structure ([`MENUS`]) and what Hive's own items do
//! ([`on_menu`]). `main.rs` turns [`MENUS`] into Tauri's menu, item for item.
//!
//! The menu only sends user actions (#37): Quit goes through [`Hive::request_quit`] (17.7), every
//! other item of Hive's own goes to the UI as `menu {item}`, which runs the action its shortcut or
//! button runs. The native items are AppKit's: Edit's Undo/Redo/Cut/Copy/Paste/Select All are what
//! lets ⌘C/⌘V reach text fields, the editor and the terminal's paste in WKWebView (17.8).

use serde_json::json;
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

const fn own(id: &'static str, text: &'static str, keys: Option<&'static str>) -> Item {
    Item::Own { id, text, keys }
}

/// The menu bar, in order. The first menu is the app's (macOS titles it with the app's name).
pub const MENUS: &[Menu] = &[
    Menu {
        title: "Hive",
        role: Role::Plain,
        items: &[
            Item::Native(Native::About),
            Item::Separator,
            own("settings", "Settings…", Some("CmdOrCtrl+,")),
            own("check-updates", "Check for Updates…", None),
            Item::Separator,
            Item::Native(Native::Services),
            Item::Separator,
            Item::Native(Native::Hide),
            Item::Native(Native::HideOthers),
            Item::Native(Native::ShowAll),
            Item::Separator,
            own(QUIT, "Quit Hive", Some("CmdOrCtrl+Q")),
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
            own("toggle-sidebar", "Toggle Sidebar", None),
            own(
                "toggle-panel",
                "Toggle Right Panel",
                Some("CmdOrCtrl+Shift+B"),
            ),
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
            own("close-tab", "Close Tab", Some("CmdOrCtrl+W")),
        ],
    },
    Menu {
        title: "Help",
        role: Role::Help,
        items: &[
            own("help", "Hive Help", None),
            own("report-issue", "Report an Issue…", None),
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
