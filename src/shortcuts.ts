import {
  type Agent,
  type HiveState,
  openModal,
  owner,
  pendingAgents,
  selectedPlace,
  setRightPanel,
  useHive,
} from "./store";
import { interceptKeys } from "./terminals";
import { sendReference } from "./viewer/reference";
import { startComment } from "./viewer/review";
import { commandKey } from "./window";

// App shortcuts (#35): Ctrl+Shift+letter (Cmd+Shift+letter on macOS), Ctrl+, and F8, taken even with
// the focus in a terminal.
// Every other key goes to the terminal untouched.

/** The selected project, the project of the selected worktree (or agent), else the first one. */
export function currentProject(s: HiveState): string | null {
  return (owner(s.projects, selectedPlace(s)) ?? Object.values(s.projects ?? {})[0])?.id ?? null;
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

/** Selects `agent`, opening its project and worktree, and shows its terminal. */
export function goToAgent(agent: Agent): void {
  const s = useHive.getState();
  const tab = s.tabs.find((t) => t.id === agent.terminal);
  useHive.setState({
    collapsed: {
      ...s.collapsed,
      [agent.project ?? ""]: false,
      [`worktree:${agent.worktree}`]: false,
    },
    selection: agent.id,
    activeTab: tab?.id ?? s.activeTab,
    fileShown: tab ? false : s.fileShown,
    transcriptShown: null,
  });
}

/** An app command: its shortcut (`keys`, written for Windows: Ctrl stands for Cmd on macOS). */
export type Command = { id: string; label: string; keys: string; run: () => void };

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
    id: "new-worktree",
    label: "New worktree",
    keys: "Ctrl+Shift+N",
    run: () => {
      const project = currentProject(useHive.getState());
      // No project to add a worktree to: adding a project is the step before.
      if (project) openModal("new-worktree", project);
      else openModal("add-project");
    },
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
  { id: "next-pending", label: "Go to the next pending agent", keys: "F8", run: nextPending },
];

/** Whether `event` presses `keys` ("F8", "Ctrl+,", "Ctrl+Shift+T"), with no other modifier. */
function presses(keys: string, event: KeyboardEvent): boolean {
  const parts = keys.split("+");
  const command = parts.includes("Ctrl") ? commandKey(event) : !event.ctrlKey && !event.metaKey;
  return (
    command &&
    event.shiftKey === parts.includes("Shift") &&
    !event.altKey &&
    event.key.toUpperCase() === parts.at(-1)?.toUpperCase()
  );
}

/** What `event` does as an app shortcut, or null when it is not one (or nothing may run). */
export function shortcut(event: KeyboardEvent): (() => void) | null {
  const s = useHive.getState();
  const blocked =
    s.connection.status === "version_mismatch" || s.connection.status === "disconnected";
  // Under the connection block nothing works; with a dialog open its keys are its own.
  if (blocked || s.modal !== null) return null;
  return COMMANDS.find((c) => presses(c.keys, event))?.run ?? null;
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
 * Listens on the window, where every key ends up, including a terminal's: the terminal only
 * lets shortcuts through (xterm leaves them unhandled, so they bubble up) and the window runs
 * them, once. Returns the cleanup.
 */
export function installShortcuts(): () => void {
  interceptKeys((event) => shortcut(event) !== null);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("contextmenu", onContextMenu);
  return () => {
    interceptKeys(() => false);
    window.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("contextmenu", onContextMenu);
  };
}
