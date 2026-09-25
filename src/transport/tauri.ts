import { Channel, invoke } from "@tauri-apps/api/core";
import type { ServiceMessage } from "../store";
import type { Transport } from ".";

// Commands live in src-tauri/src/lib.rs (`commands`); each terminal gets its own Channel (#24).
export const tauriTransport: Transport = {
  async connect(onMessage) {
    await invoke("connect", { onMessage: new Channel<ServiceMessage>(onMessage) });
  },
  openTerminal(cwd, cols, rows, onData) {
    const channel = new Channel<ArrayBuffer>((bytes) => onData(new Uint8Array(bytes)));
    return invoke<number>("open_terminal", { cwd, cols, rows, onData: channel });
  },
  writeTerminal: (id, data) => invoke("write_terminal", { id, data }),
  resizeTerminal: (id, cols, rows) => invoke("resize_terminal", { id, cols, rows }),
  closeTerminal: (id) => invoke("close_terminal", { id }),
  listProjects: () => invoke("list_projects"),
  addProject: (path) => invoke("add_project", { path }),
  createSpace: (name, env) => invoke("create_space", { name, env }),
  updateSpace: (id, name, env) => invoke("update_space", { id, name, env }),
  deleteSpace: (id) => invoke("delete_space", { id }),
  selectSpace: (id) => invoke("select_space", { id }),
  listDirs: (path, windows) => invoke("list_dirs", { path, windows }),
  listBranches: (project) => invoke("list_branches", { project }),
  validateWorktreeName: (project, name) => invoke("validate_worktree_name", { project, name }),
  createWorktree: (project, name, base) => invoke("create_worktree", { project, name, base }),
  removeWorktree: (path, force) => invoke("remove_worktree", { path, force }),
  renameWorktree: (path, name) => invoke("rename_worktree", { path, name }),
  watchWorktree: (path) => invoke("watch_worktree", { path }),
  unwatchWorktree: () => invoke("unwatch_worktree"),
  watchTranscript: (agent, subagent) => invoke("watch_transcript", { agent, subagent }),
  unwatchTranscript: (agent, subagent) => invoke("unwatch_transcript", { agent, subagent }),
  setView: (terminal, focused) => invoke("set_view", { terminal, focused }),
  listChanges: (path) => invoke("list_changes", { path }),
  listSessions: () => invoke("list_sessions"),
  locateSession: (id, target) => invoke("locate_session", { id, target }),
  deleteSession: (id) => invoke("delete_session", { id }),
  searchFiles: (worktree, query) => invoke("search_files", { worktree, query }),
  openFile: (worktree, path) => invoke("open_file", { worktree, path }),
  saveFile: (worktree, path, content, version) =>
    invoke("save_file", { worktree, path, content, version }),
  openInEditor: (worktree, path) => invoke("open_in_editor", { worktree, path }),
  getSettings: () => invoke("get_settings"),
  setSettings: (settings) => invoke("set_settings", { settings }),
  openSettingsFile: () => invoke("open_settings_file"),
  getDiagnostics: () => invoke("get_diagnostics"),
  checkUpdate: () => invoke("check_update"),
  installUpdate: () => invoke("install_update"),
};
