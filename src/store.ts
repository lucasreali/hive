import { create } from "zustand";

// The one store (#30, #38). UI state is set by components; service data changes
// only through `apply`, which stores what the service sent without deriving anything (#37).

/** Service → app messages the store understands. Mirrors `hive_protocol::Control`. */
export type ServiceMessage =
  | { type: "welcome"; version: string }
  | { type: "version_mismatch"; protocol: number; version: string }
  | { type: "terminal_opened"; channel: number }
  | { type: "terminal_exited"; channel: number; code: number | null }
  | { type: "unhooked_agent"; channel: number }
  // Sent by the app side (Rust) when the bridge exits or its output closes.
  | { type: "disconnected"; reason: string };

export type Connection =
  | { status: "connecting" }
  | { status: "connected"; version: string }
  | { status: "version_mismatch"; protocol: number; version: string }
  | { status: "disconnected"; reason: string };

export type Terminal = {
  id: number;
  exited: boolean;
  code: number | null;
  unhooked: boolean;
};

// ponytail: id-only shapes; fields and `apply` cases arrive with the service messages that feed them (1.5, 1.8).
export type Project = { id: string };
export type Worktree = { id: string };
export type Agent = { id: string };

export type Modal = "new-worktree" | "add-project" | null;
export type RightPanel = "files" | null;

export type HiveState = {
  // UI state
  view: "main";
  modal: Modal;
  rightPanel: RightPanel;
  selection: string | null;
  // Service data
  connection: Connection;
  projects: Record<string, Project>;
  worktrees: Record<string, Worktree>;
  terminals: Record<number, Terminal>;
  agents: Record<string, Agent>;
};

export const initialState: HiveState = {
  view: "main",
  modal: null,
  rightPanel: null,
  selection: null,
  connection: { status: "connecting" },
  projects: {},
  worktrees: {},
  terminals: {},
  agents: {},
};

export const useHive = create<HiveState>()(() => initialState);

function patchTerminal(s: HiveState, id: number, patch: Partial<Terminal>): Partial<HiveState> {
  const current = s.terminals[id] ?? { id, exited: false, code: null, unhooked: false };
  return { terminals: { ...s.terminals, [id]: { ...current, ...patch } } };
}

function reduce(s: HiveState, m: ServiceMessage): Partial<HiveState> {
  switch (m.type) {
    case "welcome":
      return { connection: { status: "connected", version: m.version } };
    case "version_mismatch":
      return {
        connection: { status: "version_mismatch", protocol: m.protocol, version: m.version },
      };
    case "terminal_opened":
      return patchTerminal(s, m.channel, { exited: false, code: null, unhooked: false });
    case "terminal_exited":
      return patchTerminal(s, m.channel, { exited: true, code: m.code });
    case "unhooked_agent":
      return patchTerminal(s, m.channel, { unhooked: true });
    case "disconnected":
      return { connection: { status: "disconnected", reason: m.reason } };
    default:
      // Messages without a store entry yet (e.g. `agent`, `error`) change nothing.
      return {};
  }
}

/** The only way service data enters the store. */
export function apply(message: ServiceMessage): void {
  useHive.setState((s) => reduce(s, message));
}

export const openModal = (modal: Modal) => useHive.setState({ modal });
export const setRightPanel = (rightPanel: RightPanel) => useHive.setState({ rightPanel });
export const select = (selection: string | null) => useHive.setState({ selection });

/** Re-renders only when this agent's entry changes. */
export const useAgent = (id: string) => useHive((s) => s.agents[id]);
export const useTerminal = (id: number) => useHive((s) => s.terminals[id]);
