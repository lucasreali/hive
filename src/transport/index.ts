import { isTauri } from "@tauri-apps/api/core";
import type { ServiceMessage } from "../store";
import { createMockTransport } from "./mock";
import { tauriTransport } from "./tauri";

/**
 * How the UI talks to the service. Control messages (service → UI) all arrive in the
 * `connect` handler; each terminal's output arrives in the `onData` given to `openTerminal`.
 */
export interface Transport {
  /** Starts (or, after a reload, re-attaches to) the service. Resolves once requested. */
  connect(onMessage: (message: ServiceMessage) => void): Promise<void>;
  /** Opens a terminal in `cwd` and resolves with its id (the frame channel). */
  openTerminal(
    cwd: string,
    cols: number,
    rows: number,
    onData: (bytes: Uint8Array) => void,
  ): Promise<number>;
  writeTerminal(id: number, data: string): Promise<void>;
  resizeTerminal(id: number, cols: number, rows: number): Promise<void>;
  closeTerminal(id: number): Promise<void>;
  /** Asks for every project with its worktrees again; they arrive as `projects`. */
  listProjects(): Promise<void>;
  /** Asks the service to follow `path`; answered by `project_added` or `add_project_failed`. */
  addProject(path: string): Promise<void>;
  /** Answered by `branches`. */
  listBranches(project: string): Promise<void>;
  /** Checks a new worktree name with the CLI's rule; answered by `worktree_name_validated`. */
  validateWorktreeName(project: string, name: string): Promise<void>;
  /** `hive worktree create`; answered by `worktree_created` or `create_worktree_failed`. */
  createWorktree(project: string, name: string, base: string | null): Promise<void>;
}

/**
 * The Tauri transport inside the app; the in-browser fake service otherwise or with `?mock`.
 * `?mock=mismatch` / `?mock=disconnected` make the fake service fail the connection;
 * `?mock=empty` starts it with no projects; `?mock=states` adds agents in every state;
 * `?mock=load[&cast=<url>]` replays a recording into
 * every terminal (the load test, 1.11).
 */
export function pickTransport(tauri = isTauri(), search = location.search): Transport {
  const params = new URLSearchParams(search);
  const mock = params.get("mock");
  return tauri && mock === null ? tauriTransport : createMockTransport(mock, params.get("cast"));
}

export const transport = pickTransport();
