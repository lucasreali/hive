import { type AgentState, type HiveState, openModal, useHive } from "../store";
import { transport } from "../transport";
import { closeWindow } from "../window";
import { CloseIcon } from "./icons";

const cancel = () => openModal(null);

const AT_RISK = new Set<AgentState>([
  "working",
  "with_subagents",
  "waiting_permission",
  "waiting_you",
]);

/**
 * The agents that make closing ask first (#18): working, with subagents, waiting for permission
 * or waiting for you, as the service last said. An agent with no state yet is idle.
 */
export const agentsAtRisk = (s: HiveState) =>
  Object.values(s.agents).filter((a) => AT_RISK.has(s.agentStates[a.id]?.state ?? "idle"));

/**
 * The close guard: asks when agents would end with the app, unless the `confirm_close` setting
 * is off. True keeps the window open.
 */
export function confirmClose(): boolean {
  const s = useHive.getState();
  if (!s.settings.agents.confirm_close || agentsAtRisk(s).length === 0) return false;
  openModal("close-app");
  return true;
}

/** Installs the update found at startup (4.19); the app restarts, or says why it could not. */
export function installUpdate(): void {
  openModal(null);
  useHive.setState((s) => ({ update: s.update && { ...s.update, installing: true } }));
  void transport.installUpdate();
}

/** The update button: restarting ends agents as closing does, so it asks the same way. */
export function requestUpdate(): void {
  if (agentsAtRisk(useHive.getState()).length === 0) installUpdate();
  else openModal("update-app");
}

/**
 * Closing ends every terminal and the agents in them (#18), and so does the restart of an
 * update (`updating`). The prototype has no screen for it.
 */
export function CloseAppDialog({ updating = false }: { updating?: boolean }) {
  const count = useHive((s) => agentsAtRisk(s).length);
  const title = updating ? "Update Hive?" : "Close Hive?";
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
          if (updating) installUpdate();
          else closeWindow();
        }}
      >
        <header>
          <h2 id="close-app-title">{title}</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={cancel}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>
            {count === 1 ? "1 agent is" : `${count} agents are`} running.{" "}
            {updating ? "Updating restarts Hive, which ends" : "Closing Hive ends"} every terminal
            and the agents in them.
          </p>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={cancel}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary">
            {updating ? "Update and restart" : "Close Hive"} <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}
