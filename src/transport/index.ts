import { isTauri } from "@tauri-apps/api/core";
import type { ServiceMessage, SessionTarget, Settings } from "../store";
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
  /**
   * The subfolders of the folder `path` ends in (Windows when `windows`; the home folder when
   * empty); answered by `dirs`.
   */
  listDirs(path: string, windows: boolean): Promise<void>;
  /** Answered by `branches`. */
  listBranches(project: string): Promise<void>;
  /** Checks a new worktree name with the CLI's rule; answered by `worktree_name_validated`. */
  validateWorktreeName(project: string, name: string): Promise<void>;
  /** `hive worktree create`; answered by `worktree_created` or `create_worktree_failed`. */
  createWorktree(project: string, name: string, base: string | null): Promise<void>;
  /**
   * `git worktree remove` (with `--force` when `force`); the branch stays. Answered by
   * `worktree_removed` or `remove_worktree_failed`.
   */
  removeWorktree(path: string, force: boolean): Promise<void>;
  /** Renames a Claude worktree; answered by `worktree_renamed` or `rename_worktree_failed`. */
  renameWorktree(path: string, name: string): Promise<void>;
  /** Watches one worktree instead of any other; its files arrive as `files`, again on every change. */
  watchWorktree(path: string): Promise<void>;
  /** Stops watching (the files panel closed). */
  unwatchWorktree(): Promise<void>;
  /** The terminal shown (null: none, or a file is) and whether the window has the focus. */
  setView(terminal: number | null, focused: boolean): Promise<void>;
  /** What differs from HEAD in the worktree at `path`; answered by `changes`. */
  listChanges(path: string): Promise<void>;
  /** Claude sessions of the followed projects; answered by `sessions`. */
  listSessions(): Promise<void>;
  /** Where Windows sees a session's log or folder; answered by `session_located`. */
  locateSession(id: string, target: SessionTarget): Promise<void>;
  /** Deletes a session's log; answered by `session_deleted` or `delete_session_failed`. */
  deleteSession(id: string): Promise<void>;
  /** The lines of a followed worktree's files holding `query`; answered by `search_results`. */
  searchFiles(worktree: string, query: string): Promise<void>;
  /** A file of a followed worktree on disk and at HEAD; answered by `file`. */
  openFile(worktree: string, path: string): Promise<void>;
  /**
   * Writes `content` over the file if its bytes on disk still have `version` (null: the file
   * must not exist); answered by `file_saved` or `save_failed`.
   */
  saveFile(worktree: string, path: string, content: string, version: string | null): Promise<void>;
  /**
   * The file's Windows path for an external editor (an empty `path`: the worktree's folder);
   * answered by `editor_target`.
   */
  openInEditor(worktree: string, path: string): Promise<void>;
  /** Answered by `settings` (also sent after every `welcome`). */
  getSettings(): Promise<void>;
  /** Saves the whole settings; answered by `settings`, or `settings_failed` with nothing saved. */
  setSettings(settings: Settings): Promise<void>;
  /** Asks GitHub for a newer release and downloads it; answered by `update_ready` only once one is downloaded. */
  checkUpdate(): Promise<void>;
  /** Installs the downloaded release and restarts the app; answered by `update_failed` on a failure. */
  installUpdate(): Promise<void>;
}

/**
 * The Tauri transport inside the app; the in-browser fake service otherwise or with `?mock`.
 * `?mock=mismatch` / `?mock=disconnected` make the fake service fail the connection;
 * `?mock=empty` starts it with no projects; `?mock=update` offers an update that fails; `?mock=states` adds agents in every state;
 * `?mock=load[&cast=<url>]` replays a recording into
 * every terminal (the load test, 1.11).
 */
export function pickTransport(tauri = isTauri(), search = location.search): Transport {
  const params = new URLSearchParams(search);
  const mock = params.get("mock");
  return tauri && mock === null ? tauriTransport : createMockTransport(mock, params.get("cast"));
}

export const transport = pickTransport();
