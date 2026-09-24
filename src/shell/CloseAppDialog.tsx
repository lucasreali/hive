import { type AgentState, type HiveState, openModal, useHive } from "../store";
import { CloseIcon } from "./icons";
import { closeWindow } from "./window";

const cancel = () => openModal(null);

const AT_RISK = new Set<AgentState>(["working", "waiting_permission", "waiting_you"]);

/**
 * The agents that make closing ask first (#18): working, waiting for permission or waiting
 * for you, as the service last said. An agent with no state yet is idle.
 */
export const agentsAtRisk = (s: HiveState) =>
  Object.values(s.agents).filter((a) => AT_RISK.has(s.agentStates[a.id]?.state ?? "idle"));

/** The close guard: asks when agents would end with the app. True keeps the window open. */
export function confirmClose(): boolean {
  if (agentsAtRisk(useHive.getState()).length === 0) return false;
  openModal("close-app");
  return true;
}

/** Closing ends every terminal and the agents in them (#18). The prototype has no screen for it. */
export function CloseAppDialog() {
  const count = useHive((s) => agentsAtRisk(s).length);
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog"
      aria-labelledby="close-app-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          dialog.querySelector<HTMLButtonElement>(".primary")?.focus();
        }
      }}
      onClose={cancel}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          closeWindow();
        }}
      >
        <header>
          <h2 id="close-app-title">Close Hive?</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={cancel}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>
            {count === 1 ? "1 agent is" : `${count} agents are`} running. Closing Hive ends every
            terminal and the agents in them.
          </p>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={cancel}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary">
            Close Hive <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}
