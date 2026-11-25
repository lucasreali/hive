import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { AgentState } from "../protocol";
import { type HiveState, hostView, openModal, useHive } from "../store";
import { transport } from "../transport";
import { closeWindow } from "../window";
import { CloseIcon } from "./icons";

const cancel = () => openModal(null);

const AT_RISK = new Set<AgentState>([
  "working",
  "with_subagents",
  "waiting_permission",
  "waiting_plan",
  "waiting_answer",
  "waiting_you",
]);

/**
 * The agents that make closing ask first (#18): working, with subagents, waiting for permission,
 * a plan or an answer, or waiting for you, as the service last said. An agent with no state yet is idle.
 */
export const agentsAtRisk = (s: HiveState) => atRisk(hostView(s, null));

/** `s`'s agents at risk, of the host whose data it holds. */
const atRisk = (s: HiveState) =>
  Object.values(s.agents).filter((a) => AT_RISK.has(s.agentStates[a.id]?.state ?? "idle"));

/**
 * The remote hosts whose agents quitting asks about (26.18, Q2), as "<host>\n<count>": each one
 * linked with agents at risk, which keep running unless the user says otherwise.
 */
export const remoteAgents = (s: HiveState): string[] =>
  Object.keys(s.links)
    .filter((host) => s.links[host]?.state === "connected")
    .flatMap((host) => {
      const count = atRisk(hostView(s, host)).length;
      return count > 0 ? [`${host}\n${count}`] : [];
    });

/**
 * The close guard: asks when agents would end with the app, or run on a remote host (26.18),
 * unless the `confirm_close` setting is off. True keeps the window open.
 */
export function confirmClose(): boolean {
  const s = useHive.getState();
  const asks = agentsAtRisk(s).length > 0 || remoteAgents(s).length > 0;
  if (!s.settings.agents.confirm_close || !asks) return false;
  openModal("close-app");
  return true;
}

/**
 * Quits after the remote hosts were told (26.18): each keeps its agents running, or ends them
 * when the user unticked it. Without `confirm_close`, or with nothing asked, they are kept.
 */
async function quit(ending: string[]): Promise<void> {
  const hosts = remoteAgents(useHive.getState()).map((x) => x.slice(0, x.indexOf("\n")));
  if (hosts.length > 0) {
    await Promise.allSettled(hosts.map((host) => transport.byeHost(host, ending.includes(host))));
  }
  closeWindow();
}

/**
 * ⌘Q, the Dock's Quit or logging out (D17.3): the app side asks only while agents would end, so
 * this asks through the same guard as closing the window, and quits when it has nothing to ask.
 */
export function requestQuit(): void {
  if (!confirmClose()) closeWindow();
}

/** Installs the update found at startup (4.19); the app restarts, or says why it could not. */
export function installUpdate(): void {
  openModal(null);
  const installing = (installing: boolean) =>
    useHive.setState((s) => ({ update: s.update && { ...s.update, installing } }));
  installing(true);
  // Not sent (a toast says why): the button comes back.
  transport.installUpdate().catch(() => installing(false));
}

/**
 * The update button: restarting ends agents as closing does, so it asks the same way. A
 * translocated app cannot replace itself (17.10): it explains how to move it instead.
 */
export function requestUpdate(): void {
  const s = useHive.getState();
  if (s.translocated) openModal("translocated");
  else if (agentsAtRisk(s).length === 0) installUpdate();
  else openModal("update-app");
}

/**
 * Closing ends every terminal and the agents in them (#18), and so does the restart of an
 * update (`updating`). The prototype has no screen for it.
 */
export function CloseAppDialog({ updating = false }: { updating?: boolean }) {
  const count = useHive((s) => agentsAtRisk(s).length);
  // Each remote host's agents, kept unless unticked (Q2: keep is the default).
  const remotes = useHive(useShallow(remoteAgents)).map((x) => {
    const at = x.indexOf("\n");
    return { host: x.slice(0, at), count: Number(x.slice(at + 1)) };
  });
  const [ending, setEnding] = useState<string[]>([]);
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
          else void quit(ending);
        }}
      >
        <header>
          <h2 id="close-app-title">{title}</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={cancel}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          {(count > 0 || updating) && (
            <p>
              {count === 1 ? "1 agent is" : `${count} agents are`} running.{" "}
              {updating ? "Updating restarts Hive, which ends" : "Closing Hive ends"} every terminal
              and the agents in them.
            </p>
          )}
          {!updating &&
            remotes.map(({ host, count }) => (
              <label key={host} className="checkbox">
                <input
                  type="checkbox"
                  checked={!ending.includes(host)}
                  onChange={(e) =>
                    setEnding(
                      e.currentTarget.checked
                        ? ending.filter((h) => h !== host)
                        : [...ending, host],
                    )
                  }
                />
                Keep {count === 1 ? "1 agent" : `${count} agents`} running on {host}?
              </label>
            ))}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={cancel}>
            Cancel
          </button>
          <button type="submit" className="primary">
            {updating ? "Update and restart" : "Close Hive"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
