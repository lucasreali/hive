import { Channel, type InvokeArgs, invoke } from "@tauri-apps/api/core";
import type { ServiceMessage } from "../protocol";
import { showFailure, useHive } from "../store";
import type { Transport } from ".";

/**
 * A command whose answer (if any) comes later as a service message (9.21): a failure (e.g. the
 * link is down) is shown as an error toast, never swallowed. The promise still rejects, already
 * handled, so a caller that set a pending state ("Removing…") can end it.
 */
const send = (cmd: string, args?: InvokeArgs) => showFailure(invoke<void>(cmd, args));

/**
 * The host the UI's requests about spaces, projects, worktrees, files and sessions go to (26.18):
 * the shown one's (`HiveState.host`), whose data the UI shows; absent for the local service.
 */
const shownHost = () => useHive.getState().host ?? undefined;

/** `send` to the shown host's service (`shownHost`). */
const hosted = (cmd: string, args?: Record<string, unknown>) =>
  send(cmd, { ...args, host: shownHost() });

// Commands live in src-tauri/src/lib.rs (`commands`); each terminal gets its own Channel (#24).
export const tauriTransport: Transport = {
  async connect(onMessage) {
    await invoke("connect", { onMessage: new Channel<ServiceMessage>(onMessage) });
  },
  setMode: (mode) => send("set_mode", { mode }),
  reconnectHost: (host) => send("reconnect_host", { host }),
  restartRemote: (host) => send("restart_remote", { host }),
  byeHosts: (byes) => send("bye_hosts", { byes }),
  openTerminal(cwd, cols, rows, onData, account, on) {
    const channel = new Channel<ArrayBuffer>((bytes) => onData(new Uint8Array(bytes)));
    const host = on === undefined ? shownHost() : (on ?? undefined);
    return invoke<number>("open_terminal", { host, cwd, cols, rows, account, onData: channel });
  },
  attachTerminal(id, onData) {
    const channel = new Channel<ArrayBuffer>((bytes) => onData(new Uint8Array(bytes)));
    return invoke<void>("attach_terminal", { id, onData: channel });
  },
  writeTerminal: (id, data) => send("write_terminal", { id, data }),
  resizeTerminal: (id, cols, rows) => send("resize_terminal", { id, cols, rows }),
  closeTerminal: (id) => send("close_terminal", { id }),
  ackTerminal: (id, bytes) => send("ack_terminal", { id, bytes }),
  listProjects: () => hosted("list_projects"),
  addProject: (path) => hosted("add_project", { path }),
  removeProject: (id) => hosted("remove_project", { id }),
  initGit: (id) => hosted("init_git", { id }),
  createProject: (parent, name, git) => hosted("create_project", { parent, name, git }),
  createDir: (parent, name) => hosted("create_dir", { parent, name }),
  createSpace: (name, env) => hosted("create_space", { name, env }),
  updateSpace: (id, name, env) => hosted("update_space", { id, name, env }),
  deleteSpace: (id) => hosted("delete_space", { id }),
  selectSpace: (id) => hosted("select_space", { id }),
  listClaudeLogins: () => send("list_claude_logins"),
  listGhAccounts: (ghConfigDir) => hosted("list_gh_accounts", { ghConfigDir }),
  switchGhAccount: (ghConfigDir, account) => hosted("switch_gh_account", { ghConfigDir, account }),
  getGhIdentity: (ghConfigDir, account) => hosted("get_gh_identity", { ghConfigDir, account }),
  listPulls: (project, force) => hosted("list_pulls", { project, force }),
  openPull: (project, number) => hosted("open_pull", { project, number }),
  actOnPull: (project, number, action) => hosted("act_on_pull", { project, number, action }),
  createPull: (worktree, title, body, base, draft, push) =>
    hosted("create_pull", { worktree, form: { title, body, base, draft, push } }),
  listWorkflows: (project, worktree) => hosted("list_workflows", { project, worktree }),
  runWorkflow: (project, workflow, branch, inputs) =>
    hosted("run_workflow", { project, workflow, branch, inputs }),
  stageFiles: (worktree, paths, staged) => hosted("stage_files", { worktree, paths, staged }),
  commit: (worktree, message) => hosted("commit", { worktree, message }),
  getPushTarget: (worktree) => hosted("get_push_target", { worktree }),
  push: (worktree, branch, target) => hosted("push", { worktree, branch, target }),
  listRuns: (project, branch, force) => hosted("list_runs", { project, branch, force }),
  openRun: (project, run) => hosted("open_run", { project, run }),
  openJobLog: (project, job) => hosted("open_job_log", { project, job }),
  actOnRun: (project, run, action, branch) =>
    hosted("act_on_run", { project, run, action, branch }),
  listDirs: (path, windows, hidden) => hosted("list_dirs", { path, windows, hidden }),
  listBranches: (project) => hosted("list_branches", { project }),
  validateWorktreeName: (project, name) => hosted("validate_worktree_name", { project, name }),
  createWorktree: (project, name, base) => hosted("create_worktree", { project, name, base }),
  createIssueWorktree: (project, number, base) =>
    hosted("create_issue_worktree", { project, number, base }),
  removeWorktree: (path, force) => hosted("remove_worktree", { path, force }),
  renameWorktree: (path, name) => hosted("rename_worktree", { path, name }),
  watchWorktree: (path, base) => hosted("watch_worktree", { path, base }),
  unwatchWorktree: () => hosted("unwatch_worktree"),
  expandIgnored: (path, folders) => hosted("expand_ignored", { path, folders }),
  setView: (terminal, focused, visible) => hosted("set_view", { terminal, focused, visible }),
  listChanges: (path, base) => hosted("list_changes", { path, base }),
  listScripts: (path) => hosted("list_scripts", { path }),
  listSessions: () => hosted("list_sessions"),
  locateSession: (id, target) => hosted("locate_session", { id, target }),
  deleteSession: (id) => hosted("delete_session", { id }),
  moveSession: (id, configDir) => hosted("move_session", { id, configDir }),
  searchFiles: (worktree, query) => hosted("search_files", { worktree, query }),
  openFile: (worktree, path, base) => hosted("open_file", { worktree, path, base }),
  openTable: (worktree, path) => hosted("open_table", { worktree, path }),
  editTable: (worktree, path, content, edit) =>
    hosted("edit_table", { worktree, path, content, edit }),
  saveFile: (worktree, path, content, version) =>
    invoke("save_file", { host: shownHost(), worktree, path, content, version }),
  createFile: (worktree, folder, name) => hosted("create_file", { worktree, folder, name }),
  renameFile: (worktree, path, name) => hosted("rename_file", { worktree, path, name }),
  moveFile: (worktree, path, folder) => hosted("move_file", { worktree, path, folder }),
  deleteFile: (worktree, path) => hosted("delete_file", { worktree, path }),
  createFolder: (worktree, folder, name) => hosted("create_folder", { worktree, folder, name }),
  openInEditor: (worktree, path, reveal) => hosted("open_in_editor", { worktree, path, reveal }),
  getSettings: () => send("get_settings"),
  setSettings: (settings) => send("set_settings", { settings }),
  createAccount: (name) => send("create_account", { name }),
  removeAccount: (configDir, deleteFolder) => send("remove_account", { configDir, deleteFolder }),
  openSettingsFile: () => send("open_settings_file"),
  getDiagnostics: () => send("get_diagnostics"),
  getShared: () => send("get_shared"),
  listUninstall: (keepData, accounts) => send("list_uninstall", { keepData, accounts }),
  uninstall: (keepData, accounts) => send("uninstall", { keepData, accounts }),
  openShared: () => send("open_shared"),
  checkUpdate: (manual = false) => send("check_update", { manual }),
  installUpdate: () => send("install_update"),
  showNotification: (title, body, agent) => send("show_notification", { title, body, agent }),
  checkNotifications: () => send("check_notifications"),
  openNotificationSettings: () => send("open_notification_settings"),
  checkTerminalPermissions: () => send("check_terminal_permissions"),
  openPrivacySettings: (pane) => send("open_privacy_settings", { pane }),
  // No toast: the caller falls back to the webview's tone.
  playTone: (volume) => invoke<void>("play_tone", { volume }),
  // No toast on a failure: the icon is a hint (`followTray` drops it).
  setTray: (tray) => invoke("set_tray", tray),
  // No toast: a paste with no files read pastes the clipboard's text (`terminals.ts`).
  pastedFiles: () => invoke<string>("pasted_files"),
};
