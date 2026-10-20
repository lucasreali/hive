import { openLink } from "./shell/Markdown";
import { COMMANDS, commandsBlocked } from "./shortcuts";
import { toggleSidebar, useHive } from "./store";
import { transport } from "./transport";

// The app menu (macOS, 17.13) is the app side's (`src-tauri/src/menu.rs`); a click on one of its
// own items arrives as `menu {item}` and runs here what the item's shortcut or button runs.

export const HELP_URL = "https://github.com/lucasreali/hive#readme";
export const ISSUE_URL = "https://github.com/lucasreali/hive/issues/new";

/** Runs the app command `id` (`COMMANDS`), as its shortcut does. */
const command = (id: string) => () => COMMANDS.find((c) => c.id === id)?.run();

/**
 * What each item does. `always`: it runs even under the connection block or a dialog, as the
 * title bar's buttons do; the others are blocked as the shortcuts are.
 */
const ITEMS: Record<string, { run: () => void; always?: true }> = {
  settings: { run: command("settings") },
  "toggle-sidebar": { run: toggleSidebar },
  "toggle-panel": { run: command("toggle-panel") },
  "close-tab": { run: command("close-tab") },
  "check-updates": { run: () => void transport.checkUpdate(true), always: true },
  help: { run: () => void openLink(HELP_URL), always: true },
  "report-issue": { run: () => void openLink(ISSUE_URL), always: true },
};

/** The menu item ids the UI knows: the Rust side's (`menu::MENUS`) but Quit, which it answers. */
export const MENU_ITEMS = Object.keys(ITEMS);

/** A click on the menu item `item`; an unknown one (a newer app side) does nothing. */
export function runMenuItem(item: string): void {
  const known = Object.hasOwn(ITEMS, item) ? ITEMS[item] : undefined;
  if (known && (known.always || !commandsBlocked(useHive.getState()))) known.run();
}
