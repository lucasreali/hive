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
import { commandKey } from "./window";

// App shortcuts (#35): Ctrl+Shift+letter (Cmd+Shift+letter on macOS) and F8, taken even with
// the focus in a terminal.
// Every other key goes to the terminal untouched.

/** The selected project, the project of the selected worktree (or agent), else the first one. */
function currentProject(s: HiveState): string | null {
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
  const agent = pending[(at + 1) % pending.length] as Agent;
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

/** What `event` does as an app shortcut, or null when it is not one (or nothing may run). */
export function shortcut(event: KeyboardEvent): (() => void) | null {
  const s = useHive.getState();
  const blocked =
    s.connection.status === "version_mismatch" || s.connection.status === "disconnected";
  // Under the connection block nothing works; with a dialog open its keys are its own.
  if (blocked || s.modal !== null) return null;
  const plain = !event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey;
  if (plain && event.key === "F8") return nextPending;
  if (!commandKey(event) || !event.shiftKey || event.altKey) return null;
  switch (event.key.toUpperCase()) {
    case "T":
      return () => openModal("worktree-picker");
    case "N": {
      const project = currentProject(s);
      // No project to add a worktree to: adding a project is the step before.
      return () => (project ? openModal("new-worktree", project) : openModal("add-project"));
    }
    case "B":
      return () => setRightPanel(s.rightPanel ? null : "files");
    case "O":
      return () => openModal("add-project");
    case "L":
      return sendReference;
    default:
      return null;
  }
}

function onKeyDown(event: KeyboardEvent): void {
  const action = shortcut(event);
  if (!action) return;
  event.preventDefault();
  action();
}

/**
 * Listens on the window, where every key ends up, including a terminal's: the terminal only
 * lets shortcuts through (xterm leaves them unhandled, so they bubble up) and the window runs
 * them, once. Returns the cleanup.
 */
export function installShortcuts(): () => void {
  interceptKeys((event) => shortcut(event) !== null);
  window.addEventListener("keydown", onKeyDown);
  return () => {
    interceptKeys(() => false);
    window.removeEventListener("keydown", onKeyDown);
  };
}
