import { afterEach, expect, test } from "bun:test";
import type { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { DEFAULT_SETTINGS, type ServiceMessage } from "../store";
import { tauriTransport } from "./tauri";

type Args = Record<string, unknown>;

function record(result: unknown = null) {
  const calls: [string, Args][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args as Args]);
    return result;
  });
  return calls;
}

afterEach(clearMocks);

test("connect hands a channel to Rust and delivers its messages", async () => {
  const calls = record();
  const received: ServiceMessage[] = [];
  const welcome: ServiceMessage = { type: "welcome", version: "0.1.0", distro: "Ubuntu" };
  await tauriTransport.connect((m) => received.push(m));
  const [[cmd, args]] = calls;
  expect(cmd).toBe("connect");
  (args.onMessage as Channel<ServiceMessage>).onmessage(welcome);
  expect(received).toEqual([welcome]);
});

test("each terminal gets its own byte channel", async () => {
  const calls = record(7);
  const received: Uint8Array[] = [];
  expect(await tauriTransport.openTerminal("/w", 80, 24, (b) => received.push(b))).toBe(7);
  const [[cmd, { onData, ...args }]] = calls;
  expect([cmd, args]).toEqual(["open_terminal", { cwd: "/w", cols: 80, rows: 24 }]);
  (onData as Channel<ArrayBuffer>).onmessage(new Uint8Array([104, 105]).buffer);
  expect(received).toEqual([new Uint8Array([104, 105])]);
});

test("terminal and project actions call their commands", async () => {
  const calls = record();
  await tauriTransport.writeTerminal(7, "ls\r");
  await tauriTransport.resizeTerminal(7, 100, 30);
  await tauriTransport.closeTerminal(7);
  await tauriTransport.listProjects();
  await tauriTransport.addProject("/r");
  const env = { claude_config_dir: null, git_name: "Me", git_email: null, gh_config_dir: null };
  await tauriTransport.createSpace("Work", env);
  await tauriTransport.updateSpace("w", "Job", env);
  await tauriTransport.deleteSpace("w");
  await tauriTransport.selectSpace("w");
  await tauriTransport.listDirs("", true);
  await tauriTransport.listBranches("/r");
  await tauriTransport.validateWorktreeName("/r", "x");
  await tauriTransport.createWorktree("/r", "x", null);
  await tauriTransport.removeWorktree("/r/w", true);
  await tauriTransport.renameWorktree("/r/w", "y");
  await tauriTransport.watchWorktree("/r");
  await tauriTransport.unwatchWorktree();
  await tauriTransport.setView(3, true);
  await tauriTransport.listChanges("/r");
  await tauriTransport.listSessions();
  await tauriTransport.locateSession("s", "log");
  await tauriTransport.deleteSession("s");
  await tauriTransport.searchFiles("/r", "q");
  await tauriTransport.openFile("/r", "a.ts");
  await tauriTransport.saveFile("/r", "a.ts", "x", "v");
  await tauriTransport.createFile("/r", "src", "b.ts");
  await tauriTransport.renameFile("/r", "a.ts", "c.ts");
  await tauriTransport.moveFile("/r", "a.ts", "src");
  await tauriTransport.createFolder("/r", "src", "lib");
  await tauriTransport.openInEditor("/r", "a.ts");
  await tauriTransport.getSettings();
  await tauriTransport.setSettings(DEFAULT_SETTINGS);
  await tauriTransport.openSettingsFile();
  await tauriTransport.getDiagnostics();
  await tauriTransport.checkUpdate();
  await tauriTransport.installUpdate();
  expect(calls).toEqual([
    ["write_terminal", { id: 7, data: "ls\r" }],
    ["resize_terminal", { id: 7, cols: 100, rows: 30 }],
    ["close_terminal", { id: 7 }],
    ["list_projects", {}],
    ["add_project", { path: "/r" }],
    ["create_space", { name: "Work", env }],
    ["update_space", { id: "w", name: "Job", env }],
    ["delete_space", { id: "w" }],
    ["select_space", { id: "w" }],
    ["list_dirs", { path: "", windows: true }],
    ["list_branches", { project: "/r" }],
    ["validate_worktree_name", { project: "/r", name: "x" }],
    ["create_worktree", { project: "/r", name: "x", base: null }],
    ["remove_worktree", { path: "/r/w", force: true }],
    ["rename_worktree", { path: "/r/w", name: "y" }],
    ["watch_worktree", { path: "/r" }],
    ["unwatch_worktree", {}],
    ["set_view", { terminal: 3, focused: true }],
    ["list_changes", { path: "/r" }],
    ["list_sessions", {}],
    ["locate_session", { id: "s", target: "log" }],
    ["delete_session", { id: "s" }],
    ["search_files", { worktree: "/r", query: "q" }],
    ["open_file", { worktree: "/r", path: "a.ts" }],
    ["save_file", { worktree: "/r", path: "a.ts", content: "x", version: "v" }],
    ["create_file", { worktree: "/r", folder: "src", name: "b.ts" }],
    ["rename_file", { worktree: "/r", path: "a.ts", name: "c.ts" }],
    ["move_file", { worktree: "/r", path: "a.ts", folder: "src" }],
    ["create_folder", { worktree: "/r", folder: "src", name: "lib" }],
    ["open_in_editor", { worktree: "/r", path: "a.ts" }],
    ["get_settings", {}],
    ["set_settings", { settings: DEFAULT_SETTINGS }],
    ["open_settings_file", {}],
    ["get_diagnostics", {}],
    ["check_update", {}],
    ["install_update", {}],
  ]);
});

test("chat actions call their commands on the chat's id", async () => {
  const calls = record(4);
  expect(await tauriTransport.openChat("/r", null, "plan")).toBe(4);
  const image = { media_type: "image/png", data: "iVBO" };
  await tauriTransport.chatSend(4, "hi", [image]);
  await tauriTransport.chatAnswer(4, "req_1", { kind: "deny", message: null });
  await tauriTransport.chatInterrupt(4);
  await tauriTransport.chatSetMode(4, "accept_edits");
  await tauriTransport.closeChat(4);
  await tauriTransport.confirmChatFolder(4, "/r", true);
  expect(calls).toEqual([
    ["open_chat", { cwd: "/r", resume: null, mode: "plan" }],
    ["chat_send", { chat: 4, text: "hi", images: [image] }],
    ["chat_answer", { chat: 4, request: "req_1", answer: { kind: "deny", message: null } }],
    ["chat_interrupt", { chat: 4 }],
    ["chat_set_mode", { chat: 4, mode: "accept_edits" }],
    ["close_chat", { chat: 4 }],
    ["confirm_chat_folder", { chat: 4, cwd: "/r", accepted: true }],
  ]);
});
