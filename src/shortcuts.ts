import { agentHost } from "./hosts";
import { altGr, type Bindable, COMMAND_TABLE, type Context, effective, presses } from "./keymap";
import type { Agent } from "./protocol";
import { closeFile } from "./shell/RightPanel";
import { findInTerminal } from "./shell/TerminalSearch";
import {
  activateTab,
  closeRefused,
  endRestore,
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
  showHost,
  spaceOf,
  spaceProjects,
  terminalShown,
  togglePin,
  useHive,
  withCollapsed,
} from "./store";
import { barItems, fileVisible, shownPane, shownTab } from "./tabs";
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
import { composing } from "./window";

// App shortcuts (#35): Ctrl+Shift+letter (Cmd+Shift+letter on macOS), Ctrl+, and F8 by default,
// taken even with the focus in a terminal; on macOS also its own (D17.2, 17.14): ⌘W, ⌘1–9,
// ⇧⌘[ / ⇧⌘], ⌘= / ⌘- / ⌘0, ⇧⌘J and ⌘F (17.21); and (18.34, D18.10) ⌘T, ⌘N, ⌃Tab / ⌃⇧Tab. The
// user's keymap (28.7, `src/keymap.ts`) rebinds any of them. Every other key goes to the
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
  // The user's move: the launch restore ends (28.27).
  endRestore();
  // Another host's agent (26.18): its host's spaces show first.
  showHost(agentHost(agent.id));
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
    collapsed: withCollapsed(s.collapsed, {
      [group]: false,
      [agent.project ?? ""]: false,
      [`worktree:${agent.worktree}`]: false,
    }),
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
      : terminalShown(s) && t.id === s.activeTab,
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

/**
 * ⌘W: closes the tab shown, as its × does (a file with unsaved edits asks first); a pinned one
 * stays, with a toast (29.2).
 */
export function closeActiveTab(): void {
  const s = useHive.getState();
  const file = s.fileShown && s.openFile && fileVisible(s) ? s.openFile : null;
  const terminal = s.fileShown ? null : s.activeTab;
  // Nothing would close: no toast either.
  if (!file && terminal === null) return;
  const tab = shownTab(s);
  if (tab && closeRefused(tab)) return;
  if (file) closeFile(file);
  else if (terminal !== null) closeTerminal(terminal);
}

/** Pins the tab shown (the focused pane's in a pair), or unpins it (29.2). */
export function pinShownTab(): void {
  const tab = shownTab(useHive.getState());
  if (tab) togglePin(tab);
}

/** ⌘T: a terminal in the selected worktree, as the "+" menu's Terminal; with none, the picker. */
export function newTerminal(): void {
  const worktree = selectedPlace(useHive.getState());
  if (worktree) showOpenFailure(openTerminal(worktree));
  else openModal("worktree-picker");
}

/** An app command (#35, 6.3): what the keymap knows of it (`Bindable`), and what it does. */
export type Command = Bindable & { run: () => void };

/** Whether each context holds now. */
const HOLDS: Record<Context, (s: HiveState) => boolean> = {
  any: () => true,
  terminal: terminalShown,
  markdown: markdownShown,
};

/** Whether the command `c` may run now, in its context. */
export const commandHolds = (c: Command, s: HiveState) => HOLDS[c.context](s);

/** What each command of `COMMAND_TABLE` does, by id. */
const RUNS: Record<string, () => void> = {
  palette: () => openModal("palette"),
  settings: () => openModal("settings"),
  "keyboard-settings": () => {
    openModal("settings");
    useHive.setState({ settingsSection: "Keyboard" });
  },
  "worktree-picker": () => openModal("worktree-picker"),
  "new-terminal": newTerminal,
  "new-worktree": () => {
    const s = useHive.getState();
    const project = currentProject(s);
    // No project to add a worktree to: adding a project is the step before. A group (14.1)
    // has no worktrees.
    if (s.projects?.[project ?? ""]?.group) return;
    if (project) openModal("new-worktree", project);
    else openModal("add-project");
  },
  "split-terminal": () => showOpenFailure(splitTerminal(shownPane(useHive.getState()))),
  "toggle-panel": () => setRightPanel(useHive.getState().rightPanel ? null : "files"),
  "add-project": () => openModal("add-project"),
  "send-reference": () => sendReference(),
  comment: startComment,
  "next-pending": nextPending,
  "find-in-terminal": findInTerminal,
  "toggle-markdown": () => setRendered(!renderedShown(useHive.getState())),
  "close-tab": closeActiveTab,
  "pin-tab": pinShownTab,
  "next-tab": () => cycleTab(1),
  "previous-tab": () => cycleTab(-1),
  ...Object.fromEntries(
    [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [`tab-${n}`, () => showBarItem(n - 1)]),
  ),
  "font-bigger": () => zoomTerminals(1),
  "font-smaller": () => zoomTerminals(-1),
  "font-reset": () => zoomTerminals(0),
};

/**
 * Every app command (`COMMAND_TABLE`) with its action: `shortcut()` runs them, Settings →
 * Keyboard lists and rebinds them, and the command palette (6.3) offers them.
 */
export const COMMANDS: readonly Command[] = COMMAND_TABLE.map((c) => ({
  ...c,
  run: RUNS[c.id] as () => void,
}));

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

/**
 * Whether `event` is a field's own: a key its handler already took (CodeMirror's), or one with
 * Alt but neither Ctrl nor Cmd typed into a text field or the editor (an Option character on
 * macOS, AltGr on Windows). A terminal's keys are never a field's: its chords are the app's.
 */
function fieldsOwn(event: KeyboardEvent): boolean {
  if (event.defaultPrevented) return true;
  const target = event.target;
  const field =
    target instanceof Element &&
    target.closest("input, textarea, [contenteditable], .cm-editor") !== null &&
    target.closest(".xterm") === null;
  return field && event.altKey && !event.ctrlKey && !event.metaKey;
}

/**
 * What `event` does as an app shortcut, or null when it is not one (or nothing may run): the
 * command one of whose effective chords it presses, in its context; of two, the one the user
 * bound beats a default, then table order.
 */
export function shortcut(event: KeyboardEvent): (() => void) | null {
  const s = useHive.getState();
  // A key of a composition (an IME, a dead key: ´ ~ ^, Option+E) is never a shortcut, even
  // one whose physical key an Alt chord names: it is the field's or the terminal's.
  const dead = composing(event) || event.key === "Dead";
  if (dead || commandsBlocked(s) || fieldsOwn(event) || altGr(event)) return null;
  const of = effective();
  const pressed = COMMANDS.filter(
    (c) => HOLDS[c.context](s) && (of.get(c.id)?.chords ?? []).some((k) => presses(k, event)),
  );
  const command = pressed.find((c) => of.get(c.id)?.user) ?? pressed[0];
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
