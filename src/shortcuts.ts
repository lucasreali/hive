import { type HiveState, openModal, setRightPanel, useHive } from "./store";
import { interceptKeys } from "./terminals";

// App shortcuts (#35): Ctrl+Shift+letter and F8, taken even with the focus in a terminal.
// Every other key goes to the terminal untouched.

/** The selected project, the project of the selected worktree, or else the first project. */
function currentProject(s: HiveState): string | null {
  const list = Object.values(s.projects ?? {});
  const selected = list.find(
    (p) => p.id === s.selection || p.worktrees.some((w) => w.id === s.selection),
  );
  return (selected ?? list[0])?.id ?? null;
}

/**
 * F8: shows the next agent waiting on the user. Pending states arrive with Stage 2 (2.1, 2.3);
 * until then no agent is ever pending, so this does nothing.
 */
export function nextPending(): void {}

/** What `event` does as an app shortcut, or null when it is not one (or nothing may run). */
export function shortcut(event: KeyboardEvent): (() => void) | null {
  const s = useHive.getState();
  const blocked =
    s.connection.status === "version_mismatch" || s.connection.status === "disconnected";
  // Under the connection block nothing works; with a dialog open its keys are its own.
  if (blocked || s.modal !== null) return null;
  const plain = !event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey;
  if (plain && event.key === "F8") return nextPending;
  if (!event.ctrlKey || !event.shiftKey || event.altKey || event.metaKey) return null;
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
