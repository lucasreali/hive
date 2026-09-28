import { Channel, type InvokeArgs, invoke } from "@tauri-apps/api/core";
import { type ServiceMessage, showFailure } from "../store";
import type { Transport } from ".";

/**
 * A command whose answer (if any) comes later as a service message (9.21): a failure (e.g. the
 * link is down) is shown as the notice, never swallowed. The promise still rejects, already
 * handled, so a caller that set a pending state ("Removing…") can end it.
 */
const send = (cmd: string, args?: InvokeArgs) => showFailure(invoke<void>(cmd, args));

// Commands live in src-tauri/src/lib.rs (`commands`); each terminal gets its own Channel (#24).
export const tauriTransport: Transport = {
  async connect(onMessage) {
    await invoke("connect", { onMessage: new Channel<ServiceMessage>(onMessage) });
  },
  openTerminal(cwd, cols, rows, onData) {
    const channel = new Channel<ArrayBuffer>((bytes) => onData(new Uint8Array(bytes)));
    return invoke<number>("open_terminal", { cwd, cols, rows, onData: channel });
  },
  writeTerminal: (id, data) => send("write_terminal", { id, data }),
  resizeTerminal: (id, cols, rows) => send("resize_terminal", { id, cols, rows }),
  closeTerminal: (id) => send("close_terminal", { id }),
  ackTerminal: (id, bytes) => send("ack_terminal", { id, bytes }),
  listProjects: () => send("list_projects"),
  addProject: (path) => send("add_project", { path }),
  removeProject: (id) => send("remove_project", { id }),
  createSpace: (name, env) => send("create_space", { name, env }),
  updateSpace: (id, name, env) => send("update_space", { id, name, env }),
  deleteSpace: (id) => send("delete_space", { id }),
  selectSpace: (id) => send("select_space", { id }),
  listGhAccounts: (ghConfigDir) => send("list_gh_accounts", { ghConfigDir }),
  switchGhAccount: (ghConfigDir, account) => send("switch_gh_account", { ghConfigDir, account }),
  listPulls: (project, force) => send("list_pulls", { project, force }),
  openPull: (project, number) => send("open_pull", { project, number }),
  actOnPull: (project, number, action) => send("act_on_pull", { project, number, action }),
  createPull: (worktree, title, body, base, draft) =>
    send("create_pull", { worktree, title, body, base, draft }),
  listDirs: (path, windows) => send("list_dirs", { path, windows }),
  listBranches: (project) => send("list_branches", { project }),
  validateWorktreeName: (project, name) => send("validate_worktree_name", { project, name }),
  createWorktree: (project, name, base) => send("create_worktree", { project, name, base }),
  removeWorktree: (path, force) => send("remove_worktree", { path, force }),
  renameWorktree: (path, name) => send("rename_worktree", { path, name }),
  watchWorktree: (path, base) => send("watch_worktree", { path, base }),
  unwatchWorktree: () => send("unwatch_worktree"),
  watchTranscript: (agent, subagent) => send("watch_transcript", { agent, subagent }),
  unwatchTranscript: (agent, subagent) => send("unwatch_transcript", { agent, subagent }),
  setView: (terminal, focused) => send("set_view", { terminal, focused }),
  listChanges: (path, base) => send("list_changes", { path, base }),
  listSessions: () => send("list_sessions"),
  locateSession: (id, target) => send("locate_session", { id, target }),
  deleteSession: (id) => send("delete_session", { id }),
  searchFiles: (worktree, query) => send("search_files", { worktree, query }),
  openFile: (worktree, path, base) => send("open_file", { worktree, path, base }),
  saveFile: (worktree, path, content, version) =>
    invoke("save_file", { worktree, path, content, version }),
  createFile: (worktree, folder, name) => send("create_file", { worktree, folder, name }),
  renameFile: (worktree, path, name) => send("rename_file", { worktree, path, name }),
  moveFile: (worktree, path, folder) => send("move_file", { worktree, path, folder }),
  deleteFile: (worktree, path) => send("delete_file", { worktree, path }),
  createFolder: (worktree, folder, name) => send("create_folder", { worktree, folder, name }),
  openInEditor: (worktree, path) => send("open_in_editor", { worktree, path }),
  getSettings: () => send("get_settings"),
  setSettings: (settings) => send("set_settings", { settings }),
  openSettingsFile: () => send("open_settings_file"),
  getDiagnostics: () => send("get_diagnostics"),
  checkUpdate: () => send("check_update"),
  installUpdate: () => send("install_update"),
};
