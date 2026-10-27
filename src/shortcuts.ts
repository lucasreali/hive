import type { Agent } from "./protocol";
import { closeFile } from "./shell/RightPanel";
import { findInTerminal } from "./shell/TerminalSearch";
import {
  activateTab,
  type HiveState,
  leaveSpace,
  markdownShown,
  openModal,
  owner,
  pendingAgents,
  renderedShown,
  selectedPlace,
  setOpenFile,
  setRendered,
  setRightPanel,
  spaceOf,
  spaceProjects,
  terminalShown,
  useHive,
} from "./store";
import { barItems, fileVisible } from "./tabs";
import {
  closeTerminal,
  interceptKeys,
  openTerminal,
  showOpenFailure,
  splitTerminal,
  zoomTerminals,
} from "./terminals";
import { transport } from "./transport";
import { sendReference } from "./viewer/reference";
import { startComment } from "./viewer/review";
import { commandKey, isMac, keyText } from "./window";

// App shortcuts (#35): Ctrl+Shift+letter (Cmd+Shift+letter on macOS), Ctrl+, and F8, taken even with
// the focus in a terminal; on macOS also its own (D17.2, 17.14): ⌘W, ⌘1–9, ⇧⌘[ / ⇧⌘], ⌘= / ⌘- / ⌘0,
// ⇧⌘J and ⌘F (17.21); and (18.34, D18.10) ⌘T, ⌘N, ⌃Tab / ⌃⇧Tab. Every other key goes to the
// terminal untouched.

/** The selected project, the project of the selected worktree (or agent), else the current space's first. */
export function currentProject(s: HiveState): string | null {
  return (owner(s.projects, selectedPlace(s)) ?? spaceProjects(s)[0])?.id ?? null;
}

/**
 * F8: selects the pending agent after the current one (the selected agent, else the one whose
 * terminal is shown) in tree order, wrapping; expands its project and worktree and shows its
 * terminal. Does nothing when no agent is pending.
 */
export function nextPending(): void {
  const s = useHive.getState();
  const pending = pendingAgents(s);
  if (pending.length === 0) return;
  const selected = pending.findIndex((a) => a.id === s.selection);
  const at = selected >= 0 ? selected : pending.findIndex((a) => a.terminal === s.activeTab);
  goToAgent(pending[(at + 1) % pending.length] as Agent);
}

/**
 * Selects `agent`, opening its project and worktree, and shows its terminal; when it runs in
 * another space, that space becomes the current one (6.14).
 */
export function goToAgent(agent: Agent): void {
  const s = useHive.getState();
  const space = spaceOf(s, agent.project);
  const away = space && space.id !== s.currentSpace;
  if (away) void transport.selectSpace(space.id);
  const tab = s.tabs.find((t) => t.id === agent.terminal);
  // A group's repository opens its group too (14.1).
  const group = s.projects?.[agent.project ?? ""]?.parent ?? "";
  useHive.setState({
    // The place left behind is the one that space shows again (11.5).
    ...(away ? leaveSpace(s) : {}),
    collapsed: {
      ...s.collapsed,
      [group]: false,
      [agent.project ?? ""]: false,
      [`worktree:${agent.worktree}`]: false,
    },
    selection: agent.id,
    activeTab: tab?.id ?? s.activeTab,
    fileShown: tab ? false : s.fileShown,
  });
}

/** The tab bar's item shown now: the open file's tab while it shows, else the active terminal's. */
function shownItem(s: HiveState): number {
  return barItems(s).findIndex((t) =>
    "path" in t
      ? s.fileShown && s.openFile?.worktree === t.worktree && s.openFile.path === t.path
      : !s.fileShown && t.id === s.activeTab,
  );
}

/** Shows the bar's item `i` (from 0), when there is one. */
export function showBarItem(i: number): void {
  const item = barItems(useHive.getState())[i];
  if (!item) return;
  if ("path" in item) setOpenFile(item);
  else activateTab(item);
}

/** ⇧⌘] (1) and ⇧⌘[ (-1): the bar's next or previous item, wrapping. */
export function cycleTab(step: 1 | -1): void {
  const s = useHive.getState();
  const count = barItems(s).length;
  if (count > 0) showBarItem((shownItem(s) + step + count) % count);
}

/** ⌘W: closes the tab shown, as its × does (a file with unsaved edits asks first). */
export function closeActiveTab(): void {
  const s = useHive.getState();
  if (s.fileShown && s.openFile && fileVisible(s)) closeFile(s.openFile);
  else if (!s.fileShown && s.activeTab !== null) closeTerminal(s.activeTab);
}

/** ⌘T: a terminal in the selected worktree, as the "+" menu's Terminal; with none, the picker. */
export function newTerminal(): void {
  const worktree = selectedPlace(useHive.getState());
  if (worktree) showOpenFailure(openTerminal(worktree));
  else openModal("worktree-picker");
}

/**
 * An app command: its shortcut (`keys`, written for Windows: Ctrl stands for Cmd on macOS, and
 * Control is the Control key itself, only in macOS' keys), and
 * `macKeys`, a second one on macOS. One with `mac` exists only on macOS; one with `when` only
 * while it holds: their keys are free for the terminal otherwise.
 */
export type Command = {
  id: string;
  label: string;
  keys: string;
  macKeys?: string;
  mac?: true;
  run: () => void;
  when?: (s: HiveState) => boolean;
};

/** The text of a command's keys, as this platform names them. */
export const commandKeys = (c: Command) =>
  isMac() && c.macKeys ? `${keyText(c.macKeys)} or ${keyText(c.keys)}` : keyText(c.keys);

/**
 * Every app command with a shortcut: `shortcut()` runs them, the settings' Shortcuts section
 * lists them, and the command palette (6.3) offers them.
 */
export const COMMANDS: readonly Command[] = [
  {
    id: "palette",
    label: "Command palette",
    keys: "Ctrl+Shift+P",
    run: () => openModal("palette"),
  },
  { id: "settings", label: "Open settings", keys: "Ctrl+,", run: () => openModal("settings") },
  {
    id: "worktree-picker",
    label: "New terminal in a worktree",
    keys: "Ctrl+Shift+T",
    run: () => openModal("worktree-picker"),
  },
  {
    id: "new-terminal",
    label: "New terminal in this worktree",
    keys: "Ctrl+T",
    mac: true,
    run: newTerminal,
  },
  {
    id: "new-worktree",
    label: "New worktree",
    keys: "Ctrl+Shift+N",
    macKeys: "Ctrl+N",
    run: () => {
      const s = useHive.getState();
      const project = currentProject(s);
      // No project to add a worktree to: adding a project is the step before. A group (14.1)
      // has no worktrees.
      if (s.projects?.[project ?? ""]?.group) return;
      if (project) openModal("new-worktree", project);
      else openModal("add-project");
    },
  },
  {
    id: "split-terminal",
    label: "Split terminal",
    keys: "Ctrl+Shift+D",
    run: () => showOpenFailure(splitTerminal(useHive.getState().activeTab)),
  },
  {
    id: "toggle-panel",
    label: "Show or hide files, diff and sessions",
    keys: "Ctrl+Shift+B",
    run: () => setRightPanel(useHive.getState().rightPanel ? null : "files"),
  },
  {
    id: "add-project",
    label: "Add project",
    keys: "Ctrl+Shift+O",
    run: () => openModal("add-project"),
  },
  {
    id: "send-reference",
    label: "Send the selected lines' reference to the terminal",
    keys: "Ctrl+Shift+L",
    run: sendReference,
  },
  {
    id: "comment",
    label: "Comment on the selected lines",
    keys: "Ctrl+Shift+M",
    run: startComment,
  },
  {
    // F8 is Play/Pause on a Mac's keyboard unless Fn is held (D17.2).
    id: "next-pending",
    label: "Go to the next pending agent",
    keys: "F8",
    macKeys: "Ctrl+Shift+J",
    run: nextPending,
  },
  {
    // 17.21: ⌘F on macOS (D17.2); elsewhere Ctrl+F is the shell's, so #35's Ctrl+Shift+F.
    id: "find-in-terminal",
    label: "Search the terminal",
    keys: "Ctrl+Shift+F",
    macKeys: "Ctrl+F",
    when: terminalShown,
    run: findInTerminal,
  },
  {
    // 11.2: VS Code's key, the terminal's paste everywhere but on a Markdown file's tab.
    id: "toggle-markdown",
    label: "Show a Markdown file rendered or as text",
    keys: "Ctrl+Shift+V",
    when: markdownShown,
    run: () => setRendered(!renderedShown(useHive.getState())),
  },
  { id: "close-tab", label: "Close the tab", keys: "Ctrl+W", mac: true, run: closeActiveTab },
  {
    id: "next-tab",
    label: "Show the next tab",
    keys: "Ctrl+Shift+]",
    macKeys: "Control+Tab",
    mac: true,
    run: () => cycleTab(1),
  },
  {
    id: "previous-tab",
    label: "Show the previous tab",
    keys: "Ctrl+Shift+[",
    macKeys: "Control+Shift+Tab",
    mac: true,
    run: () => cycleTab(-1),
  },
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map(
    (n): Command => ({
      id: `tab-${n}`,
      label: `Show tab ${n}`,
      keys: `Ctrl+${n}`,
      mac: true,
      run: () => showBarItem(n - 1),
    }),
  ),
  {
    id: "font-bigger",
    label: "Make the terminal font bigger",
    keys: "Ctrl+=",
    mac: true,
    run: () => zoomTerminals(1),
  },
  {
    id: "font-smaller",
    label: "Make the terminal font smaller",
    keys: "Ctrl+-",
    mac: true,
    run: () => zoomTerminals(-1),
  },
  {
    id: "font-reset",
    label: "Terminal font at the settings' size",
    keys: "Ctrl+0",
    mac: true,
    run: () => zoomTerminals(0),
  },
];

/** The commands this platform has: macOS-only ones (`mac`) only on macOS. */
export const platformCommands = () => COMMANDS.filter((c) => !c.mac || isMac());

/**
 * Where a key is, for keys whose character Shift or the layout changes (⇧⌘[ types "{" on a US
 * layout): such a key matches by its character or by this physical key.
 */
const CODES: Record<string, string> = {
  "[": "BracketLeft",
  "]": "BracketRight",
  "=": "Equal",
  "-": "Minus",
  ...Object.fromEntries([1, 2, 3, 4, 5, 6, 7, 8, 9, 0].map((n) => [`${n}`, `Digit${n}`])),
};

/** Whether `event` presses `keys` ("F8", "Ctrl+,", "Control+Tab"), with no other modifier. */
function presses(keys: string, event: KeyboardEvent): boolean {
  const parts = keys.split("+");
  const key = parts.at(-1) as string;
  const control = parts.includes("Control");
  const command = parts.includes("Ctrl")
    ? commandKey(event)
    : event.ctrlKey === control && !event.metaKey;
  return (
    command &&
    event.shiftKey === parts.includes("Shift") &&
    !event.altKey &&
    (event.key.toUpperCase() === key.toUpperCase() || event.code === CODES[key])
  );
}

/** Whether `event` presses one of `c`'s keys on this platform. */
const pressesCommand = (c: Command, event: KeyboardEvent) =>
  presses(c.keys, event) || (isMac() && !!c.macKeys && presses(c.macKeys, event));

/**
 * Whether no app command may run now: under the connection block nothing works, and with a
 * dialog open its keys are its own. A dialog closed by Esc clears `modal` only on its `close`
 * event, which the browser sends in a later task: it counts closed as soon as it is.
 */
export function commandsBlocked(s: HiveState): boolean {
  const blocked =
    s.connection.status === "version_mismatch" || s.connection.status === "disconnected";
  return blocked || (s.modal !== null && document.querySelector("dialog[open]") !== null);
}

/** What `event` does as an app shortcut, or null when it is not one (or nothing may run). */
export function shortcut(event: KeyboardEvent): (() => void) | null {
  const s = useHive.getState();
  if (commandsBlocked(s)) return null;
  const command = platformCommands().find((c) => pressesCommand(c, event) && (c.when?.(s) ?? true));
  return command?.run ?? null;
}

function onKeyDown(event: KeyboardEvent): void {
  const action = shortcut(event);
  if (!action) return;
  event.preventDefault();
  action();
}

/**
 * The WebView's own context menu (reload, print, inspect…) is off everywhere except in text
 * fields and the editor, where it gives copy and paste. Rows with their own menu open it anyway.
 */
function onContextMenu(event: MouseEvent): void {
  const target = event.target;
  if (target instanceof Element && target.closest("input, textarea, .cm-editor")) return;
  event.preventDefault();
}

/**
 * A file or a link dragged from outside that nothing on the way took:
 * its drop would make the WebView open it in place of the app. Refused, and shown as not
 * droppable. The app's own drags (tree, sidebar) are left to their targets. On macOS the drop
 * of files from Finder never reaches the page: the app side takes it and types the paths into
 * the terminal under it (18.1, `dropFiles`).
 */
function onUnhandledDrag(event: DragEvent): void {
  const types = event.dataTransfer?.types ?? [];
  const outside = types.includes("Files") || types.includes("text/uri-list");
  if (event.defaultPrevented || !outside) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
}

/**
 * Listens on the window, where every key ends up, including a terminal's: the terminal only
 * lets shortcuts through (xterm leaves them unhandled, so they bubble up) and the window runs
 * them, once. Returns the cleanup.
 */
export function installShortcuts(): () => void {
  interceptKeys((event) => shortcut(event) !== null);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("contextmenu", onContextMenu);
  window.addEventListener("dragover", onUnhandledDrag);
  window.addEventListener("drop", onUnhandledDrag);
  return () => {
    interceptKeys(() => false);
    window.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("contextmenu", onContextMenu);
    window.removeEventListener("dragover", onUnhandledDrag);
    window.removeEventListener("drop", onUnhandledDrag);
  };
}
