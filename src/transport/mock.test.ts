import { expect, test } from "bun:test";
import type {
  AgentState,
  Dirs,
  Project,
  ServiceMessage,
  Settings,
  Space,
  SpaceEnv,
} from "../protocol";
import { DEFAULT_SETTINGS } from "../store";
import {
  agentStatus,
  createMockTransport,
  ECHO_MARK,
  LOAD_STAGGER_MS,
  LOAD_START_MS,
  MOCK_ACCOUNTS_DIR,
  MOCK_BRANCHES,
  MOCK_CHANGES,
  MOCK_CLAUDE_DIR,
  MOCK_DIAGNOSTICS,
  MOCK_FILES,
  MOCK_FOLDER,
  MOCK_FOLDER_FILES,
  MOCK_GROUP,
  MOCK_IGNORED,
  MOCK_ISSUES,
  MOCK_NAME_RULE,
  MOCK_OWN_WORKTREE,
  MOCK_REPO_SCRIPTS,
  MOCK_REPOS,
  MOCK_SCRIPT_PROBLEMS,
  MOCK_SESSIONS,
  MOCK_SHARED,
  MOCK_STATES,
  MOCK_TEXTS,
  mockDirs,
  mockUninstall,
  mockVersion,
} from "./mock";

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const tick = () => wait(0);

async function connected() {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  return { transport, messages };
}

async function opened() {
  const { transport, messages } = await connected();
  let output = "";
  const decoder = new TextDecoder();
  const id = await transport.openTerminal("/w", 80, 24, (b) => {
    output += decoder.decode(b);
  });
  await tick();
  return { transport, messages, id, output: () => output };
}

test("welcomes the UI asynchronously", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  expect(messages).toEqual([]);
  await tick();
  expect(messages).toEqual([
    { type: "welcome", version: "mock", distro: "Ubuntu" },
    { type: "settings", settings: DEFAULT_SETTINGS },
    { type: "spaces", spaces: [space("default", "Default", [SHOP, API])], current: "default" },
    { type: "projects", projects: MOCK_REPOS.slice(0, 2) },
    {
      type: "session_usage",
      usage: { used_percentage: 42, resets_at: expect.any(Number) },
      week: { used_percentage: 41, resets_at: expect.any(Number) },
    },
  ]);
});

test("?mock=choose asks where the service runs and starts it once chosen (12.5.4)", async () => {
  const transport = createMockTransport("choose");
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  expect(messages).toEqual([{ type: "app_mode", mode: null, wsl: true }]);
  await transport.setMode("native");
  await tick();
  expect(messages.slice(1, 3)).toEqual([
    { type: "app_mode", mode: "native", wsl: true },
    { type: "welcome", version: "mock", distro: null },
  ]);
  // A reload keeps the mode; back to WSL, its distribution shows again.
  messages.length = 0;
  await transport.connect((m) => messages.push(m));
  await transport.setMode("wsl");
  await tick();
  expect(messages.map((m) => m.type).slice(0, 2)).toEqual(["app_mode", "welcome"]);
  expect(messages).toContainEqual({ type: "app_mode", mode: "wsl", wsl: true });
  expect(messages).toContainEqual({ type: "welcome", version: "mock", distro: "Ubuntu" });
});

test("keeps the settings it is given in memory", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.terminal.font_size = 20;
  await transport.setSettings(settings);
  await transport.getSettings();
  await tick();
  expect(messages.slice(-2)).toEqual([
    { type: "settings", settings },
    { type: "settings", settings },
  ]);
});

test("makes an account's folder by its name and deletes only its own (22.3)", async () => {
  const { transport, messages } = await connected();
  const last = () => messages.at(-1);
  const failed = (message: string) => ({ type: "settings_failed" as const, message });
  const accounts = () => {
    const m = last();
    return m?.type === "settings" ? m.settings.claude.accounts : null;
  };
  const work = { name: "Work", config_dir: `${MOCK_ACCOUNTS_DIR}/work`, hive_folder: true };
  await transport.createAccount("Work");
  await tick();
  expect(accounts()).toEqual([work]);
  for (const [name, message] of [
    ["Work", "Another account is named Work"],
    ["Default", "Another account is named Default"],
    ["work!", `${work.config_dir} exists already: give the account another name`],
    ["../..", '"../.." cannot name a folder: give the account a letter or a digit'],
  ]) {
    await transport.createAccount(name as string);
    await tick();
    expect(last()).toEqual(failed(message as string));
  }
  // An account of a folder the user chose is removed, never its folder.
  const mine = { name: "Mine", config_dir: "/home/user/.claude-mine" };
  const claude = { ...DEFAULT_SETTINGS.claude, accounts: [work, mine] };
  await transport.setSettings({ ...DEFAULT_SETTINGS, claude });
  await transport.removeAccount(mine.config_dir, true);
  await tick();
  expect(last()).toEqual(failed(`Hive did not make ${mine.config_dir}: it is never deleted`));
  await transport.removeAccount("/nope", false);
  await tick();
  expect(last()).toEqual(failed("No Claude account has the folder /nope"));
  // One a space uses is refused (22.5).
  await transport.updateSpace("default", "Default", { ...NO_ENV, claude_account: work.config_dir });
  await transport.removeAccount(work.config_dir, true);
  await tick();
  const used = "Work is the Claude account of the space Default: choose another one for it first";
  expect(last()).toEqual(failed(used));
  await transport.updateSpace("default", "Default", NO_ENV);
  await transport.removeAccount(work.config_dir, true);
  await tick();
  expect(accounts()).toEqual([mine]);
});

test("terminals get their project's space's Claude account, or the one asked for", async () => {
  const { transport, messages } = await connected();
  const work = { name: "Work", config_dir: "/w/.claude" };
  const settings = {
    ...DEFAULT_SETTINGS,
    claude: { ...DEFAULT_SETTINGS.claude, accounts: [work] },
  };
  await transport.setSettings(settings);
  // A space's account must be an account (22.5); the usage shown follows the current space's.
  const env = (claude_account: string | null) => ({ ...NO_ENV, claude_account });
  await transport.updateSpace("default", "Default", env("/nope"));
  await tick();
  expect(messages.at(-1)).toEqual({
    type: "space_failed",
    message: "No Claude account has the folder /nope",
  });
  await transport.updateSpace("default", "Default", env("/w/.claude"));
  await tick();
  expect(messages.at(-1)).toEqual({ type: "session_usage", usage: null, week: null });
  const outputs: string[] = [];
  const decoder = new TextDecoder();
  const open = async (cwd: string, account?: { config_dir: string | null }) => {
    const index = outputs.push("") - 1;
    const onData = (b: Uint8Array) => {
      outputs[index] += decoder.decode(b);
    };
    const id = await transport.openTerminal(cwd, 80, 24, onData, account);
    await transport.writeTerminal(id, "echo $CLAUDE_CONFIG_DIR\r");
  };
  await open(SHOP);
  await open(SHOP, { config_dir: null });
  await open("/w");
  await tick();
  expect(outputs[0]).toContain("echo $CLAUDE_CONFIG_DIR\r\n/w/.claude");
  expect(outputs[1]).not.toContain("/w/.claude");
  expect(outputs[2]).not.toContain("/w/.claude");
  // An account a space uses is not removed, naming the space.
  const removed = {
    ...settings,
    claude: { accounts: [], default_name: "", share_sessions: false },
  };
  await transport.setSettings(removed);
  await tick();
  expect(messages.at(-1)).toEqual({
    type: "settings_failed",
    message: "Work is the Claude account of the space Default: choose another one for it first",
  });
  // Used by none, it is; an empty default name is "Default".
  await transport.updateSpace("default", "Default", env(null));
  await tick();
  expect(messages.at(-1)?.type === "session_usage" && messages.at(-1)).toMatchObject({
    usage: { used_percentage: 42 },
  });
  await transport.setSettings(removed);
  await tick();
  const last = messages.at(-1);
  expect(last?.type === "settings" && last.settings.claude).toEqual({
    accounts: [],
    default_name: "Default",
    share_sessions: false,
  });
});

test("answers the settings file's path and the diagnostics", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  await transport.openSettingsFile();
  await transport.getDiagnostics();
  await tick();
  const windows_path = "\\\\wsl.localhost\\Ubuntu\\home\\mock\\.config\\hive\\settings.json";
  expect(messages.slice(-2)).toEqual([
    { type: "editor_target", worktree: "", path: "", reveal: false, windows_path, error: null },
    { type: "diagnostics", ...MOCK_DIAGNOSTICS },
  ]);
  await transport.getShared();
  await transport.openShared();
  await tick();
  const shared = "\\\\wsl.localhost\\Ubuntu\\home\\mock\\.local\\share\\hive\\shared";
  expect(messages.slice(-2)).toEqual([
    { type: "shared", ...MOCK_SHARED },
    {
      type: "editor_target",
      worktree: "",
      path: "shared",
      reveal: false,
      windows_path: shared,
      error: null,
    },
  ]);
});

test("lists what uninstalling would remove, and removes nothing (23.5)", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  await transport.listUninstall(false, true);
  await transport.listUninstall(true, false);
  await transport.uninstall(false, false);
  await tick();
  const [all, kept, done] = messages.slice(-3);
  const paths = (m: ServiceMessage | undefined) =>
    m?.type === "uninstall_list" ? m.items.map((i) => `${i.kind} ${i.path}`) : [];
  expect(all).toMatchObject({ type: "uninstall_list", keep_data: false, accounts: true });
  expect(paths(all)).toContain(`account ${MOCK_ACCOUNTS_DIR}/work`);
  expect(paths(all)).toContain("data /home/mock/.hive/settings.json");
  expect(paths(all).at(-1)).toBe("app /Applications/Hive.app");
  expect(paths(kept).every((p) => !p.startsWith("data") && !p.startsWith("account"))).toBe(true);
  expect(paths(kept)).toContain("program /home/mock/.hive/bin");
  expect(mockUninstall(false, false).items.some((i) => i.kind === "account")).toBe(false);
  expect(mockUninstall(false, false).account_folders).toEqual([`${MOCK_ACCOUNTS_DIR}/work`]);
  expect(done).toEqual({ type: "uninstalled", failed: [] });
});

test("a scenario fails the connection instead", async () => {
  for (const [scenario, types] of [
    ["mismatch", ["version_mismatch"]],
    ["disconnected", ["disconnected"]],
    ["", ["welcome", "settings", "spaces", "projects", "session_usage"]],
    [
      "states",
      [
        "welcome",
        "settings",
        "spaces",
        "projects",
        ...MOCK_STATES.flatMap(() => ["agent_detected", "agent_state", "agent_usage"]),
        "subagent_worktrees",
        "session_usage",
      ],
    ],
  ] as const) {
    const messages: ServiceMessage[] = [];
    await createMockTransport(scenario).connect((m) => messages.push(m));
    await tick();
    expect(messages.map((m): string => m.type)).toEqual([...types]);
  }
});

test("states: shop also lists the worktree a subagent owns", async () => {
  const messages: ServiceMessage[] = [];
  await createMockTransport("states").connect((m) => messages.push(m));
  await tick();
  const [, , , listed] = messages;
  const shop = listed?.type === "projects" ? listed.projects[0] : undefined;
  expect(shop?.worktrees.map((w) => w.id).at(-1)).toBe(MOCK_OWN_WORKTREE);
  const owners = MOCK_STATES.flatMap(([, , subs]) => subs).filter((s) => s.worktree);
  expect(owners.map((s) => [s.id, s.worktree])).toEqual([["a3", MOCK_OWN_WORKTREE]]);
  // No agent runs there, so it shows under its subagent only.
  const owned = { type: "subagent_worktrees", worktrees: [MOCK_OWN_WORKTREE] };
  expect(messages.at(-2)).toEqual(owned as ServiceMessage);
  // Other scenarios keep the fake repositories as they are.
  expect(MOCK_REPOS[0].worktrees.map((w) => w.id)).not.toContain(MOCK_OWN_WORKTREE);
});

test("projects are added from the fake repositories only", async () => {
  const transport = createMockTransport("empty");
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  expect(messages.at(-2)).toEqual({ type: "projects", projects: [] });

  const [shop] = MOCK_REPOS;
  await transport.addProject(shop.path);
  await transport.addProject(shop.path);
  await transport.addProject("/nope");
  await transport.listProjects();
  await tick();
  const spaces: ServiceMessage = {
    type: "spaces",
    spaces: [space("default", "Default", [SHOP])],
    current: "default",
  };
  expect(messages.slice(5)).toEqual([
    spaces,
    { type: "project_added", project: shop },
    spaces,
    { type: "project_added", project: shop },
    {
      type: "add_project_failed",
      path: "/nope",
      error: "not_found",
      message: "cannot open /nope: No such file or directory (os error 2)",
    },
    spaces,
    { type: "projects", projects: [shop] },
  ]);
  expect(shop.worktrees.map((w) => [w.name, w.branch, w.main, w.claude])).toEqual([
    ["main", "main", true, false],
    ["fix-login", "worktree-fix-login", false, true],
    ["feat-checkout", "worktree-feat-checkout", false, true],
  ]);
});

// The mock shell's OSC 133 marks (17.25): its prompt, a command's start and end.
const A = "\x1b]133;A\x07";
const C = "\x1b]133;C\x07";
const D = (exit: number) => `\x1b]133;D;${exit}\x07`;

test("the mock shell marks its prompts and commands; false fails", async () => {
  const { transport, id, output } = await opened();
  await transport.writeTerminal(id, "false\r");
  expect(output()).toBe(`${A}mock$ false\r\n${C}false\r\n${D(1)}${A}mock$ `);
});

test("`copy-files` stands in for files copied in Finder, as the app side reads them (18.3)", async () => {
  const { transport, id } = await opened();
  expect(await transport.pastedFiles()).toBe("");
  await transport.writeTerminal(id, "copy-files /tmp/My\\ Shot.png /tmp/b \r");
  expect(await transport.pastedFiles()).toBe("/tmp/My\\ Shot.png /tmp/b ");
  await transport.writeTerminal(id, "copy-files\r");
  expect(await transport.pastedFiles()).toBe("");
});

test("typed stand-ins send a terminal's progress, long command and notification", async () => {
  const { transport, messages, id } = await opened();
  const typed = ["progress normal 40", "progress indeterminate", "long-command 2 75"];
  for (const line of [...typed, "osc-notify make;done", "osc-notify bare"]) {
    await transport.writeTerminal(id, `${line}\r`);
  }
  await tick();
  expect(messages.slice(-5)).toEqual([
    { type: "terminal_progress", channel: id, state: "normal", value: 40 },
    { type: "terminal_progress", channel: id, state: "indeterminate", value: 0 },
    { type: "command_finished", channel: id, exit: 2, secs: 75 },
    { type: "terminal_notification", channel: id, title: "make", body: "done", notify: true },
    { type: "terminal_notification", channel: id, title: "bare", body: "", notify: true },
  ]);
});

test("a terminal prints a prompt, echoes input and repeats the line", async () => {
  const { transport, messages, id, output } = await opened();
  expect(id).toBe(1);
  expect(messages.at(-1)).toEqual({
    type: "terminal_opened",
    channel: 1,
    worktree: null,
    account: "Default",
  });
  expect(output()).toBe(`${A}mock$ `);
  await transport.writeTerminal(id, "hi\r\r");
  expect(output()).toBe(`${A}mock$ hi\r\n${C}hi\r\n${D(0)}${A}mock$ \r\n${A}mock$ `);
  await transport.resizeTerminal(id, 100, 30);
  expect(await transport.openTerminal("/", 1, 1, () => {})).toBe(2);
  // A renamed default account is named as configured (21.4/21.7).
  const claude = { ...DEFAULT_SETTINGS.claude, default_name: "Personal" };
  await transport.setSettings({ ...DEFAULT_SETTINGS, claude });
  await transport.openTerminal("/", 1, 1, () => {});
  await tick();
  expect(messages.at(-1)).toMatchObject({ type: "terminal_opened", account: "Personal" });
});

test("exit ends the terminal with code 0 and close with no code", async () => {
  const { transport, messages, id, output } = await opened();
  await transport.writeTerminal(id, "exit\rignored");
  await tick();
  expect(messages.at(-1)).toEqual({ type: "terminal_exited", channel: id, code: 0 });
  expect(output()).toBe(`${A}mock$ exit`);

  const second = await transport.openTerminal("/", 1, 1, () => {});
  await transport.closeTerminal(second);
  await transport.closeTerminal(second);
  await transport.writeTerminal(second, "x");
  await tick();
  expect(messages.filter((m) => m.type === "terminal_exited")).toEqual([
    { type: "terminal_exited", channel: id, code: 0 },
    { type: "terminal_exited", channel: second, code: null },
  ]);
});

test("claude detects an idle agent where the terminal is; lines set it working; exit removes it first", async () => {
  const { transport, messages } = await connected();
  const [shop] = MOCK_REPOS;
  const id = await transport.openTerminal(shop.path, 80, 24, () => {});
  await transport.writeTerminal(id, "cd .claude/worktrees/fix-login\rclaude\r\r");
  await transport.writeTerminal(id, "cd /tmp\rclaude\r");
  await tick();
  const fixLogin = `${shop.path}/.claude/worktrees/fix-login`;
  const agent = { type: "agent_detected", channel: id, id: "mock-session-1" } as const;
  // Its terminal is placed in the deepest worktree holding its folder.
  const inside = await transport.openTerminal(`${fixLogin}/src`, 80, 24, () => {});
  await tick();
  const placed = {
    type: "terminal_opened",
    channel: inside,
    worktree: fixLogin,
    account: "Default",
  };
  expect(messages).toContainEqual(placed as ServiceMessage);
  const state = (state: AgentState, activity: string | null = null): ServiceMessage => ({
    type: "agent_state",
    id: agent.id,
    ...agentStatus(state, activity, expect.any(Number)),
    subagents: [],
  });
  expect(messages.filter((m) => m.type.startsWith("agent"))).toEqual([
    { ...agent, project: shop.id, worktree: fixLogin, cwd: fixLogin },
    state("idle"),
    // The empty line sent nothing; `cd /tmp` and the second `claude` are prompts.
    state("working", "cd /tmp"),
    state("working", "claude"),
    { ...agent, project: null, worktree: null, cwd: "/tmp" },
    state("idle"),
  ]);
  // `state <state>` moves it there after the line set it working; an unknown state does not.
  // Entering a state that waits for the user alerts; working → waiting for you finished.
  await transport.writeTerminal(id, "state error\rstate bogus\rstate waiting_you\r");
  await tick();
  const alerts = messages.slice(-5).map((m) => m.type === "agent_state" && [m.state, m.alert]);
  expect(alerts).toEqual([
    ["working", null],
    ["error", "action"],
    ["working", null],
    ["working", null],
    ["waiting_you", "finished"],
  ]);
  await transport.writeTerminal(id, "exit\r");
  await tick();
  expect(messages.slice(-2)).toEqual([
    { type: "agent_removed", channel: id, id: "mock-session-1" },
    { type: "terminal_exited", channel: id, code: 0 },
  ]);
});

test("/clear in an agent ends its session, clears the screen and starts a new session (20.3)", async () => {
  const { transport, messages } = await connected();
  const printed: string[] = [];
  const decoder = new TextDecoder();
  const id = await transport.openTerminal("/w", 80, 24, (b) => printed.push(decoder.decode(b)));
  // Without an agent it is a line like any other.
  await transport.writeTerminal(id, "/clear\rclaude\r/clear\r");
  await tick();
  const sessions = messages.flatMap((m) =>
    m.type === "agent_detected" || m.type === "agent_removed" ? [[m.type, m.id]] : [],
  );
  expect(sessions).toEqual([
    ["agent_detected", "mock-session-1"],
    ["agent_removed", "mock-session-1"],
    ["agent_detected", "mock-session-1-clear"],
  ]);
  expect(printed.filter((p) => p === "\x1b[2J\x1b[3J\x1b[H")).toHaveLength(1);
});

test("hive badge sets and clears the terminal's badge", async () => {
  const { transport, messages } = await connected();
  const id = await transport.openTerminal("/w", 80, 24, () => {});
  await tick();
  messages.length = 0;
  await transport.writeTerminal(id, "hive badge  db tests \rhive badge --clear\r");
  await tick();
  expect(messages).toEqual([
    { type: "badge", channel: id, text: "db tests" },
    { type: "badge", channel: id, text: "" },
  ]);
});

test("worktree-remove drops that Claude worktree and sends the new list", async () => {
  const { transport, messages } = await connected();
  const [shop, api] = MOCK_REPOS;
  const id = await transport.openTerminal(shop.path, 80, 24, () => {});
  await tick();
  messages.length = 0;
  // "main" is no Claude worktree, so it stays.
  await transport.writeTerminal(id, "worktree-remove fix-login\rworktree-remove main\r");
  await tick();
  const names = (m: ServiceMessage) =>
    m.type === "projects" ? m.projects.map((p) => p.worktrees.map((w) => w.name)) : [];
  expect(messages.map(names)).toEqual([
    [
      ["main", "feat-checkout"],
      ["main", "refactor-auth"],
    ],
    [
      ["main", "feat-checkout"],
      ["main", "refactor-auth"],
    ],
  ]);
  // The shared fake repositories are left alone.
  expect(shop.worktrees.map((w) => w.name)).toEqual(["main", "fix-login", "feat-checkout"]);
  expect(api.worktrees).toHaveLength(2);
});

test("agent states carry the service's urgency and pending flag", () => {
  const calm = ["ended", "idle", "working", "with_subagents"] as const;
  const pending = [
    "waiting_you",
    "error",
    "waiting_answer",
    "waiting_plan",
    "waiting_permission",
  ] as const;
  const doing = { activity: null, since_ms: 0, interrupted: false, alert: null, notify: false };
  expect([...calm, ...pending].map((s) => agentStatus(s))).toEqual([
    { state: "ended", urgency: 0, pending: false, writing: false, ...doing },
    { state: "idle", urgency: 1, pending: false, writing: false, ...doing },
    { state: "working", urgency: 2, pending: false, writing: true, ...doing },
    { state: "with_subagents", urgency: 3, pending: false, writing: true, ...doing },
    { state: "waiting_you", urgency: 4, pending: true, writing: false, ...doing },
    { state: "error", urgency: 5, pending: true, writing: false, ...doing },
    { state: "waiting_answer", urgency: 6, pending: true, writing: true, ...doing },
    { state: "waiting_plan", urgency: 7, pending: true, writing: true, ...doing },
    { state: "waiting_permission", urgency: 8, pending: true, writing: true, ...doing },
  ]);
  expect(agentStatus("working", "Run tests", 7)).toMatchObject({
    activity: "Run tests",
    since_ms: 7,
  });
});

test("branches, name checks and new worktrees for the dialog", async () => {
  const { transport, messages } = await connected();
  const [shop, , dotfiles] = MOCK_REPOS;
  const before = structuredClone(shop);
  messages.length = 0;
  await transport.listBranches(shop.id);
  await transport.listBranches(dotfiles.id);
  await transport.validateWorktreeName(shop.id, "");
  await transport.validateWorktreeName(shop.id, "fix-login");
  await transport.createWorktree(shop.id, "fix-cart", "main");
  await transport.createWorktree(shop.id, "fix-cart", null);
  await tick();
  const path = `${shop.path}/.claude/worktrees/fix-cart`;
  const created = messages[4] as Extract<ServiceMessage, { type: "worktree_created" }>;
  expect(messages).toEqual([
    { type: "branches", project: shop.id, ...MOCK_BRANCHES[shop.id], current: "main", error: null },
    {
      type: "branches",
      project: dotfiles.id,
      local: [],
      remote: [],
      current: null,
      error: `${dotfiles.id} is not a followed project`,
    },
    {
      type: "worktree_name_validated",
      project: shop.id,
      name: "",
      folder: ".claude/worktrees/<name>/",
      branch: "worktree-<name>",
      error: `invalid worktree name "": use lowercase letters, digits, '.', '_' and '-', starting with a letter or digit`,
    },
    {
      type: "worktree_name_validated",
      project: shop.id,
      name: "fix-login",
      folder: ".claude/worktrees/fix-login/",
      branch: "worktree-fix-login",
      error: `worktree "fix-login" already exists at ${shop.path}/.claude/worktrees/fix-login`,
    },
    created,
    {
      type: "create_worktree_failed",
      project: shop.id,
      name: "fix-cart",
      message: `worktree "fix-cart" already exists at ${path}`,
    },
  ]);
  expect(created.path).toBe(path);
  expect(created.notes).toEqual([]);
  expect(created.project.worktrees.at(-1)?.path).toBe(path);
  // The shared fake repositories are never changed.
  expect(shop).toEqual(before);
  await transport.listProjects();
  await tick();
  expect(messages.at(-1)).toEqual({ type: "projects", projects: [created.project, MOCK_REPOS[1]] });
  // Adding a project again answers its current worktrees.
  await transport.addProject(shop.path);
  await tick();
  expect(messages.at(-1)).toEqual({ type: "project_added", project: created.project });
});

test("?mock=load replays a recording into each terminal and marks every echo", async () => {
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    new Response('{"version":2}\n[0,"o","one "]\n[0.05,"o","two"]')) as unknown as typeof fetch;
  try {
    const transport = createMockTransport("load", "/rec.cast");
    await transport.connect(() => {});
    const out = ["", ""];
    const decoder = new TextDecoder();
    const a = await transport.openTerminal("/w", 80, 24, (bytes) => {
      out[0] += decoder.decode(bytes);
    });
    const b = await transport.openTerminal("/w", 80, 24, (bytes) => {
      out[1] += decoder.decode(bytes);
    });
    await transport.writeTerminal(a, "x");
    expect(out[0]).toBe(`x${ECHO_MARK}`);
    // The second terminal starts later; closed before its second event, it stops there.
    await wait(LOAD_START_MS + b * LOAD_STAGGER_MS + 20);
    await transport.closeTerminal(b);
    await wait(100);
    expect(out).toEqual([`x${ECHO_MARK}${A}mock$ one two`, `${A}mock$ one `]);
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("a watched worktree lists its files and again after a touch in its terminal", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const fix = (shop.worktrees[1] as { path: string }).path;
  const filesOf = () => messages.filter((m) => m.type === "files" || m.type === "error");
  await transport.watchWorktree("/nowhere", "head");
  await transport.watchWorktree(shop.path, "head");
  await tick();
  const [refused, main] = filesOf();
  const message = "/nowhere is not a worktree of a followed project";
  expect(refused).toEqual({ type: "error", message });
  expect(main).toMatchObject({ type: "files", path: shop.path, truncated: false });
  // The main worktree has enough files to scroll.
  expect(main?.type === "files" && main.files.length).toBe(MOCK_FILES.length + 400);
  await transport.watchWorktree(fix, "head");
  await tick();
  const listing = (files: string[], ignored = MOCK_IGNORED) => ({
    type: "files" as const,
    path: fix,
    files,
    ignored,
    truncated: false,
  });
  expect(filesOf()[2]).toEqual(listing(MOCK_FILES));

  const id = await transport.openTerminal(fix, 80, 24, () => {});
  await transport.writeTerminal(id, "touch a.txt\r");
  await tick();
  const touched = ["a.txt", ...MOCK_FILES].sort();
  expect(filesOf()[3]).toEqual(listing(touched));
  // An open ignored folder lists one level, a folder inside it once it is listed too; another
  // worktree's folders change nothing.
  await transport.expandIgnored(shop.path, ["node_modules"]);
  await transport.expandIgnored(fix, ["node_modules/react"]);
  await transport.expandIgnored(fix, ["node_modules", "node_modules/react", "src"]);
  await tick();
  const levels = [
    ".env",
    "node_modules/",
    "node_modules/.package-lock.json",
    "node_modules/react/",
    "node_modules/react/index.js",
    "node_modules/react/package.json",
  ];
  expect(filesOf().slice(4)).toEqual([listing(touched), listing(touched, levels)]);
  // A new watch starts with none open.
  await transport.watchWorktree(fix, "head");
  await tick();
  expect(filesOf()[6]).toEqual(listing(touched));
  // Unwatched, or outside a worktree, a touch sends nothing.
  await transport.unwatchWorktree();
  await transport.writeTerminal(id, "touch b.txt\r");
  await transport.writeTerminal(id, "cd src\r");
  await transport.writeTerminal(id, "touch c.txt\r");
  // A worktree touched before it is watched keeps the file.
  const other = await transport.openTerminal(shop.path, 80, 24, () => {});
  await transport.writeTerminal(other, "touch d.txt\r");
  await tick();
  expect(filesOf()).toHaveLength(7);
  await transport.watchWorktree(fix, "head");
  await transport.watchWorktree(shop.path, "head");
  await tick();
  const [again, main2] = filesOf().slice(7);
  expect(again?.type === "files" && again.files).toEqual(["b.txt", ...touched].sort());
  expect(main2?.type === "files" && main2.files.includes("d.txt")).toBe(true);
});

test("a worktree's scripts: the settings' first, shop's repository's, a port each (23.4)", async () => {
  const { transport, messages } = await connected();
  const [shop, api] = MOCK_REPOS;
  const login = shop.worktrees[1].path;
  messages.length = 0;
  await transport.listScripts(login);
  await transport.listScripts(api.path);
  await transport.listScripts(login);
  await transport.listScripts("/nowhere");
  await tick();
  const shopScripts = {
    type: "scripts" as const,
    path: login,
    scripts: MOCK_REPO_SCRIPTS,
    problems: MOCK_SCRIPT_PROBLEMS,
    port: 20_000 as number | null,
  };
  const apiScripts = {
    type: "scripts" as const,
    path: api.path,
    scripts: [],
    problems: [],
    port: 20_010,
  };
  const refused = {
    type: "error" as const,
    message: "/nowhere is not a worktree of a followed project",
  };
  expect(messages).toEqual([shopScripts, apiScripts, shopScripts, refused]);
  // The settings change: the scripts the app last asked for (a worktree) follow; one replaces
  // the repository's of the same name.
  await transport.listScripts(login);
  await tick();
  const settings = structuredClone(DEFAULT_SETTINGS);
  const run = [{ name: "dev", command: "pnpm dev --port $PORT" }];
  settings.projects[shop.id] = { scripts: { setup: null, run, archive: null } };
  messages.length = 0;
  await transport.setSettings(settings);
  await tick();
  const mine = { name: "dev", command: "pnpm dev --port $PORT", source: "settings" as const };
  const rest = MOCK_REPO_SCRIPTS.filter((s) => s.name !== "dev");
  expect(messages.find((m) => m.type === "scripts")).toEqual({
    ...shopScripts,
    scripts: [mine, ...rest],
  });
  // Asked for no worktree: nothing more.
  await transport.listScripts("/nowhere");
  await tick();
  messages.length = 0;
  await transport.setSettings(settings);
  await tick();
  expect(messages.some((m) => m.type === "scripts")).toBe(false);
});

test("changes of a followed worktree with their totals, or why not", async () => {
  const { transport, messages } = await connected();
  const [shop, api, dotfiles] = MOCK_REPOS;
  const refactor = api.worktrees[1].path;
  messages.length = 0;
  await transport.listChanges(refactor, "branch");
  await transport.listChanges(shop.path, "branch");
  await transport.listChanges(dotfiles.path, "head");
  await transport.listChanges(refactor, "head");
  await tick();
  const refactored = {
    type: "changes",
    path: refactor,
    base: "branch",
    branch: "main",
    base_error: null,
    files: MOCK_CHANGES[refactor],
    added: 24,
    removed: 49,
    error: null,
  } as const;
  expect(messages).toEqual([
    refactored,
    // The main worktree has no branch to compare with.
    {
      type: "changes",
      path: shop.path,
      base: "head",
      branch: null,
      base_error: "No branch to compare with: this is the main worktree, or it is detached",
      files: [],
      added: 0,
      removed: 0,
      error: null,
    },
    {
      type: "changes",
      path: dotfiles.path,
      base: "head",
      branch: null,
      base_error: null,
      files: [],
      added: 0,
      removed: 0,
      error: `${dotfiles.path} is not a worktree of a followed project`,
    },
    { ...refactored, base: "head" },
  ]);
});

test("files of a followed worktree as their status says, or why not", async () => {
  const { transport, messages } = await connected();
  const [shop, api, dotfiles] = MOCK_REPOS;
  const refactor = api.worktrees[1].path;
  const fixLogin = shop.worktrees[1].path;
  messages.length = 0;
  const asked: [string, string][] = [
    [fixLogin, "src/auth/session.ts"],
    [refactor, "src/legacy/jwt.ts"],
    [refactor, "src/auth/token.ts"],
    [api.path, "test/routes/orders.test.ts"],
    [shop.path, "README.md"],
    [refactor, "assets/logo.png"],
    [dotfiles.path, "a"],
  ];
  for (const [worktree, path] of asked) await transport.openFile(worktree, path, "head");
  await tick();
  const texts = messages.map((m) =>
    m.type === "file" ? [m.path, m.content, m.base, m.binary, m.error, m.version] : m,
  );
  const [session, sessionBase] = MOCK_TEXTS["src/auth/session.ts"];
  const sample = (path: string, n = 1) => `// ${path}\nexport const value = ${n};\n`;
  expect(session).toContain("REMEMBER_ME_TTL // 30 days");
  expect(texts).toEqual([
    ["src/auth/session.ts", session, sessionBase, false, null, mockVersion(session)],
    ["src/legacy/jwt.ts", null, sample("src/legacy/jwt.ts"), false, null, null],
    [
      "src/auth/token.ts",
      sample("src/auth/token.ts", 2),
      sample("src/auth/token.ts"),
      false,
      null,
      mockVersion(sample("src/auth/token.ts", 2)),
    ],
    [
      "test/routes/orders.test.ts",
      sample("test/routes/orders.test.ts", 2),
      null,
      false,
      null,
      mockVersion(sample("test/routes/orders.test.ts", 2)),
    ],
    [
      "README.md",
      sample("README.md"),
      sample("README.md"),
      false,
      null,
      mockVersion(sample("README.md")),
    ],
    ["assets/logo.png", null, null, true, null, null],
    ["a", null, null, false, `${dotfiles.path} is not a worktree of a followed project`, null],
  ]);
});

test("a table file's rows, split on its delimiter, or why not", async () => {
  const { transport, messages } = await connected();
  const [shop, api, dotfiles] = MOCK_REPOS;
  messages.length = 0;
  await transport.openTable(shop.path, "data/orders.csv");
  await transport.openTable(shop.path, "a.tsv");
  await transport.openTable(api.worktrees[1].path, "src/legacy/jwt.ts");
  await transport.openTable(dotfiles.path, "a.csv");
  await tick();
  const [orders, tsv, gone, outside] = messages.map((m) => (m.type === "table" ? m : null));
  const sheet = orders?.sheets[0];
  expect([sheet?.rows.length, sheet?.total, sheet?.partial]).toEqual([42, 42, false]);
  expect(sheet?.rows[0]).toEqual(["id", "customer", "total", "status"]);
  expect(sheet?.rows[1]).toEqual(["1001", "Customer 1", "0.00", "paid"]);
  expect(sheet?.rows[41]).toEqual(["1041", "Late"]);
  expect(tsv?.sheets[0]?.rows).toEqual([["// a.tsv"], ["export const value = 1;"]]);
  expect([gone?.sheets, gone?.error]).toEqual([[], "src/legacy/jwt.ts does not exist"]);
  const error = `${dotfiles.path} is not a worktree of a followed project`;
  expect([outside?.sheets, outside?.error]).toEqual([[], error]);
});

test("a save checks the version as the service does; write stands in for an agent", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const readme = "// README.md\nexport const value = 1;\n";
  messages.length = 0;
  await transport.saveFile(shop.path, "README.md", "mine\n", mockVersion(readme));
  await transport.saveFile(shop.path, "README.md", "late\n", mockVersion(readme));
  await transport.saveFile("/nowhere", "a", "x", null);
  await transport.openFile(shop.path, "README.md", "head");
  await tick();
  const at = { worktree: shop.path, path: "README.md" };
  expect(messages.slice(0, 3)).toEqual([
    { type: "file_saved", ...at, version: mockVersion("mine\n") },
    { type: "save_failed", ...at, error: "conflict", message: "README.md changed on disk" },
    {
      type: "save_failed",
      worktree: "/nowhere",
      path: "a",
      error: "invalid_path",
      message: "/nowhere is not a worktree of a followed project",
    },
  ]);
  expect(messages[3]).toMatchObject({ type: "file", content: "mine\n" });

  // `write` in a watched worktree's terminal changes the text and sends the changes again.
  await transport.watchWorktree(shop.path, "head");
  const id = await transport.openTerminal(shop.path, 80, 24, () => {});
  await tick();
  messages.length = 0;
  await transport.writeTerminal(id, "write README.md agent was here\r");
  await transport.openFile(shop.path, "README.md", "head");
  await tick();
  expect(messages.map((m) => m.type)).toEqual(["files", "changes", "file"]);
  expect(messages[2]).toMatchObject({ content: "agent was here\n" });
  await transport.unwatchWorktree();
  await transport.writeTerminal(id, "write README.md again\r");
  // The view changes nothing in the mock.
  await transport.setView(id, true, true);
  await tick();
  expect(messages).toHaveLength(3);
});

test("files are created and renamed as the service does, never over another", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const w = shop.path;
  await transport.watchWorktree(w, "head");
  await tick();
  messages.length = 0;
  await transport.createFile(w, "src", "new.ts");
  await tick();
  expect(messages.map((m) => m.type)).toEqual(["file_created", "files", "changes"]);
  expect(messages[0]).toEqual({ type: "file_created", worktree: w, path: "src/new.ts" });
  expect((messages[1] as { files: string[] }).files).toContain("src/new.ts");
  await transport.unwatchWorktree();
  messages.length = 0;
  await transport.createFile(w, "", "top.ts");
  await transport.renameFile(w, "src/new.ts", "renamed.ts");
  await transport.openFile(w, "src/renamed.ts", "head");
  const failures: [string, string | null, string][] = [
    ["/nowhere", null, "x"],
    [w, null, ""],
    [w, null, "a/b"],
    [w, null, "README.md"],
    [w, "gone.ts", "x"],
  ];
  for (const [worktree, from, name] of failures) {
    if (from) await transport.renameFile(worktree, from, name);
    else await transport.createFile(worktree, "", name);
  }
  await tick();
  const failed = (worktree: string, message: string) => ({
    type: "file_op_failed",
    worktree,
    message,
  });
  expect(messages).toEqual([
    { type: "file_created", worktree: w, path: "top.ts" },
    { type: "file_renamed", worktree: w, path: "src/new.ts", to: "src/renamed.ts" },
    expect.objectContaining({ type: "file", path: "src/renamed.ts", content: "" }),
    failed("/nowhere", "/nowhere is not a worktree of a followed project"),
    failed(w, "not a valid file name"),
    failed(w, "not a valid file name"),
    failed(w, "README.md already exists"),
    failed(w, "gone.ts does not exist"),
  ]);
});

test("files are moved and folders created as the service does, never over another", async () => {
  const { transport, messages } = await connected();
  const w = (MOCK_REPOS[0] as (typeof MOCK_REPOS)[number]).path;
  await transport.watchWorktree(w, "head");
  await tick();
  messages.length = 0;
  await transport.createFolder(w, "", "empty");
  await transport.createFolder(w, "empty", "inner");
  await transport.moveFile(w, "README.md", "empty");
  await tick();
  const renamed = (path: string, to: string) =>
    ({ type: "file_renamed", worktree: w, path, to }) as const;
  expect(messages.filter((m) => m.type !== "files" && m.type !== "changes")).toEqual([
    { type: "folder_created", worktree: w, path: "empty" },
    { type: "folder_created", worktree: w, path: "empty/inner" },
    renamed("README.md", "empty/README.md"),
  ]);
  messages.length = 0;
  await transport.moveFile(w, "empty/README.md", "");
  // Its own folder: nothing moves.
  await transport.moveFile(w, "README.md", "");
  await transport.createFolder(w, "", "empty");
  await transport.createFolder(w, "", "src");
  await transport.moveFile(w, "gone.ts", "empty");
  await tick();
  const failed = (message: string) => ({ type: "file_op_failed", worktree: w, message }) as const;
  expect(messages.filter((m) => m.type !== "files" && m.type !== "changes")).toEqual([
    renamed("empty/README.md", "README.md"),
    renamed("README.md", "README.md"),
    failed("empty already exists"),
    failed("src already exists"),
    failed("gone.ts does not exist"),
  ]);

  // A folder moves with its files and folders, never into itself.
  messages.length = 0;
  await transport.moveFile(w, "src/auth", "empty/inner");
  await transport.renameFile(w, "empty", "full");
  await transport.moveFile(w, "full", "full/inner");
  await transport.openFile(w, "full/inner/auth/login.ts", "head");
  await tick();
  const files = messages.filter((m) => m.type === "files").at(-1) as { files: string[] };
  expect(files.files.filter((p) => p.includes("auth/"))).toEqual([
    "full/inner/auth/login.ts",
    "full/inner/auth/session.ts",
  ]);
  expect(messages.filter((m) => m.type !== "files" && m.type !== "changes")).toEqual([
    renamed("src/auth", "empty/inner/auth"),
    renamed("empty", "full"),
    failed("a folder cannot go into itself"),
    expect.objectContaining({
      type: "file",
      content: "// src/auth/login.ts\nexport const value = 1;\n",
    }),
  ]);
  messages.length = 0;
  await transport.createFolder(w, "", "empty");
  await tick();
  expect(messages[0]).toEqual({ type: "folder_created", worktree: w, path: "empty" });
});

test("files and folders are deleted as the service does, with what they hold", async () => {
  const { transport, messages } = await connected();
  const w = (MOCK_REPOS[0] as (typeof MOCK_REPOS)[number]).path;
  await transport.watchWorktree(w, "head");
  await tick();
  await transport.createFolder(w, "", "empty");
  await transport.createFolder(w, "src/auth", "inner");
  await tick();
  messages.length = 0;
  await transport.deleteFile(w, "src/auth");
  await transport.deleteFile(w, "README.md");
  await transport.deleteFile(w, "empty");
  await transport.deleteFile(w, "empty");
  await transport.deleteFile(w, "src/auth/login.ts");
  await transport.deleteFile("/nowhere", "a");
  await tick();
  const deleted = (path: string) => ({ type: "file_deleted", worktree: w, path }) as const;
  const failed = (worktree: string, message: string) =>
    ({ type: "file_op_failed", worktree, message }) as const;
  expect(messages.filter((m) => m.type !== "files" && m.type !== "changes")).toEqual([
    deleted("src/auth"),
    deleted("README.md"),
    deleted("empty"),
    failed(w, "empty does not exist"),
    failed(w, "src/auth/login.ts does not exist"),
    failed("/nowhere", "/nowhere is not a worktree of a followed project"),
  ]);
  const files = messages.filter((m) => m.type === "files").at(-1) as { files: string[] };
  expect(files.files.filter((p) => p.startsWith("src/auth") || p === "README.md")).toEqual([]);
  expect(files.files).toContain("src/App.tsx");
});

test("a file's Windows path for an external editor, or why not", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  messages.length = 0;
  await transport.openInEditor(shop.path, "src/App.tsx", false);
  await transport.openInEditor("/nowhere", "a", true);
  await tick();
  expect(messages).toEqual([
    {
      type: "editor_target",
      worktree: shop.path,
      path: "src/App.tsx",
      reveal: false,
      windows_path: "\\\\wsl.localhost\\Ubuntu\\home\\user\\projects\\shop\\src\\App.tsx",
      error: null,
    },
    {
      type: "editor_target",
      worktree: "/nowhere",
      path: "a",
      reveal: true,
      windows_path: null,
      error: "/nowhere is not a worktree of a followed project",
    },
  ]);
});

test("the worktree menu: delete and rename as the service refuses or does them", async () => {
  const { transport, messages } = await connected();
  const [shop] = MOCK_REPOS;
  const wt = (name: string) => `${shop.path}/.claude/worktrees/${name}`;
  const answers = async (request: () => Promise<void>) => {
    messages.length = 0;
    await request();
    await tick();
    return messages.map((m) => ("message" in m ? m.message : m.type));
  };
  expect(await answers(() => transport.removeWorktree(shop.path, true))).toEqual([
    `${shop.path} is the project's main worktree`,
  ]);
  expect(await answers(() => transport.renameWorktree("/nope", "x"))).toEqual([
    "/nope is not a worktree of a followed project",
  ]);
  // fix-login has changes in the fake service: only forced.
  const [dirty] = await answers(() => transport.removeWorktree(wt("fix-login"), false));
  expect(dirty).toContain("use --force to delete it");
  const id = await transport.openTerminal(`${wt("feat-checkout")}/src`, 80, 24, () => {});
  await tick();
  const busy = `in use by fish (${id}): close its terminals first`;
  expect(await answers(() => transport.renameWorktree(wt("feat-checkout"), "x"))).toEqual([busy]);
  expect(await answers(() => transport.removeWorktree(wt("feat-checkout"), false))).toEqual([busy]);
  await transport.closeTerminal(id);
  await tick();
  expect(await answers(() => transport.renameWorktree(wt("feat-checkout"), "fix-login"))).toEqual([
    `worktree "fix-login" already exists at ${wt("fix-login")}`,
  ]);
  expect(await answers(() => transport.renameWorktree(wt("feat-checkout"), "cart"))).toEqual([
    "worktree_renamed",
  ]);
  expect(messages[0]).toMatchObject({ from: wt("feat-checkout"), path: wt("cart") });
  expect(await answers(() => transport.removeWorktree(wt("fix-login"), true))).toEqual([
    "worktree_removed",
  ]);
  const names = (messages[0] as { project: { worktrees: { name: string }[] } }).project.worktrees;
  expect(names.map((w) => w.name)).toEqual(["main", "cart"]);
  // The shared fake repositories are left alone.
  expect(shop.worktrees.map((w) => w.name)).toEqual(["main", "fix-login", "feat-checkout"]);
});

test("an empty path locates the worktree's folder for the Explorer", async () => {
  const { transport, messages } = await connected();
  const [shop] = MOCK_REPOS;
  messages.length = 0;
  await transport.openInEditor(shop.path, "", false);
  await tick();
  expect(messages).toEqual([
    {
      type: "editor_target",
      worktree: shop.path,
      path: "",
      reveal: false,
      windows_path: "\\\\wsl.localhost\\Ubuntu\\home\\user\\projects\\shop\\",
      error: null,
    },
  ]);
});

test("a contents search finds the fake texts' lines in any case, or says why not", async () => {
  const { transport, messages } = await connected();
  const [shop] = MOCK_REPOS;
  messages.length = 0;
  await transport.searchFiles(shop.path, "EXPORT const");
  await transport.searchFiles("/nowhere", "x");
  await tick();
  const [found, refused] = messages as Extract<ServiceMessage, { type: "search_results" }>[];
  expect(found.error).toBeNull();
  expect(found.matches.length).toBeGreaterThan(10);
  expect(found.matches[0]).toEqual({
    path: ".gitignore",
    line: 2,
    text: "export const value = 1;",
  });
  expect(refused).toEqual({
    type: "search_results",
    worktree: "/nowhere",
    query: "x",
    matches: [],
    truncated: false,
    error: "/nowhere is not a worktree of a followed project",
  });
});

test("sessions: listed, located, and deleted unless running", async () => {
  const { transport, messages } = await connected();
  const [first] = MOCK_SESSIONS;
  const answers = async (request: () => Promise<unknown>) => {
    messages.length = 0;
    await request();
    await tick();
    return messages;
  };
  expect(await answers(() => transport.listSessions())).toEqual([
    { type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false },
  ]);
  expect(await answers(() => transport.locateSession(first.id, "folder"))).toEqual([
    {
      type: "session_located",
      id: first.id,
      target: "folder",
      windows_path:
        "\\\\wsl.localhost\\Ubuntu\\home\\user\\projects\\shop\\.claude\\worktrees\\fix-login",
      error: null,
    },
  ]);
  const [log] = (await answers(() => transport.locateSession(first.id, "log"))) as {
    windows_path: string;
  }[];
  expect(log.windows_path).toEndWith(`\\${first.id}.jsonl`);
  expect(await answers(() => transport.locateSession("nope", "log"))).toEqual([
    {
      type: "session_located",
      id: "nope",
      target: "log",
      windows_path: null,
      error: "no session nope in the followed projects",
    },
  ]);
  // `claude` in a terminal runs as a session of the fake service.
  const id = await transport.openTerminal("/home/user/projects/shop", 80, 24, () => {});
  await transport.writeTerminal(id, "claude\r");
  await tick();
  const agent = `mock-session-${id}`;
  // Moved into another account's folder (21.10), never while it runs; listed again unasked.
  const [moved, relisted] = (await answers(() => transport.moveSession(first.id, "/w"))) as [
    ServiceMessage,
    { sessions: { id: string; config_dir: string | null }[] },
  ];
  expect(moved).toEqual({ type: "session_moved", id: first.id, config_dir: "/w" });
  expect(relisted.sessions.find((s) => s.id === first.id)?.config_dir).toBe("/w");
  expect(await answers(() => transport.moveSession(agent, null))).toEqual([
    { type: "move_session_failed", id: agent, message: "the session is running: end it first" },
  ]);
  expect(await answers(() => transport.moveSession("nope", null))).toEqual([
    {
      type: "move_session_failed",
      id: "nope",
      message: "no session nope in the followed projects",
    },
  ]);
  expect(await answers(() => transport.deleteSession(agent))).toEqual([
    {
      type: "delete_session_failed",
      id: agent,
      message: "the session is running: end it first",
    },
  ]);
  // The list without it comes unasked, as the service sends it when a log goes (D19.4).
  const [deleted, listed] = (await answers(() => transport.deleteSession(first.id))) as [
    ServiceMessage,
    { sessions: unknown[] },
  ];
  expect(deleted).toEqual({ type: "session_deleted", id: first.id });
  expect(listed.sessions).toHaveLength(MOCK_SESSIONS.length - 1);
  // The shared fake list is left alone.
  expect(MOCK_SESSIONS[0]).toBe(first);
});

test("?mock=update offers an update whose install fails; otherwise none is offered", async () => {
  const { transport, messages } = await connected();
  await transport.checkUpdate();
  await tick();
  expect(messages.some((m) => m.type === "update_ready")).toBe(false);
  expect(messages.some((m) => m.type === "update_checked")).toBe(false);
  // Asked from the menu: up to date.
  await transport.checkUpdate(true);
  await tick();
  expect(messages.at(-1)).toEqual({ type: "update_checked", error: null });

  const update = createMockTransport("update");
  const offered: ServiceMessage[] = [];
  await update.connect((m) => offered.push(m));
  await update.checkUpdate();
  await update.installUpdate();
  await tick();
  expect(offered.slice(-2)).toEqual([
    { type: "update_ready", version: "9.9.9" },
    { type: "update_failed", error: "mock: nothing to install" },
  ]);
});

test("?mock=denied says notifications are off; the browser plays its own tone", async () => {
  const { transport, messages } = await connected();
  await transport.checkNotifications();
  await transport.openNotificationSettings();
  await tick();
  expect(messages.at(-1)).toEqual({ type: "notification_permission", permission: "granted" });
  expect(transport.playTone(50)).rejects.toThrow("mock: no app to play the tone");

  const denied = createMockTransport("denied");
  const said: ServiceMessage[] = [];
  await denied.connect((m) => said.push(m));
  await denied.checkNotifications();
  await tick();
  expect(said.at(-1)).toEqual({ type: "notification_permission", permission: "denied" });
});

test("folders are browsed on both sides of the fake machine", async () => {
  const { transport, messages } = await connected();
  messages.length = 0;
  await transport.listDirs("", false, false);
  await tick();
  expect(messages).toEqual([
    {
      type: "dirs",
      path: "/home/user/",
      windows: false,
      linux_path: "/home/user/",
      parent: "/home/",
      dirs: [
        { name: "dotfiles", git: true },
        { name: "Downloads", git: false },
        { name: "projects", git: false },
        // A group's folder (14.1).
        { name: "work", git: false },
      ],
      error: null,
      warning: null,
      claude: false,
    },
  ]);
  const windows = mockDirs("", true);
  expect(windows.path).toBe("C:\\Users\\user\\");
  expect(windows.linux_path).toBe("/mnt/c/Users/user/");
  expect(windows.dirs.map((d) => d.name)).toEqual(["Documents", "source"]);
  const site = mockDirs("C:/Users\\user\\source\\si", true);
  expect([site.linux_path, site.parent, site.dirs]).toEqual([
    "/mnt/c/Users/user/source/si",
    "C:/Users\\user\\",
    [{ name: "site", git: true }],
  ]);
  expect(mockDirs("/", false).parent).toBeNull();
  expect(mockDirs("/home/user/dotfiles/", false).dirs).toEqual([]);
  expect(mockDirs("x", true).error).toBe("type a full path");
  expect(mockDirs("/nowhere/x", false)).toEqual({
    path: "/nowhere/x",
    windows: false,
    linux_path: null,
    parent: null,
    dirs: [],
    error: "cannot open /nowhere/: No such file or directory (os error 2)",
    warning: null,
    claude: false,
  });
  // The service on Windows itself: Windows paths as they are, the drives for a bare name.
  const native = mockDirs("", false, true);
  expect([native.path, native.windows, native.linux_path]).toEqual([
    "C:\\Users\\user\\",
    false,
    "C:\\Users\\user\\",
  ]);
  expect(native.dirs.map((d) => d.name)).toEqual(["Documents", "source"]);
  expect(mockDirs("c", false, true)).toEqual({
    path: "c",
    windows: false,
    linux_path: null,
    parent: null,
    dirs: [{ name: "C:", git: false }],
    error: null,
    warning: null,
    claude: false,
  });
  expect(mockDirs("C:\\", false, true).parent).toBeNull();
});

test("hidden folders are listed when asked or for a dotted name; a Claude folder is told", () => {
  const names = (dirs: Dirs) => dirs.dirs.map((d) => d.name);
  expect(names(mockDirs("/home/user/", false))).not.toContain(".claude-work");
  expect(names(mockDirs("/home/user/", false, false, true))).toEqual([
    ".claude-work",
    ".config",
    "dotfiles",
    "Downloads",
    "projects",
    "work",
  ]);
  expect(names(mockDirs("/home/user/.c", false))).toContain(".config");
  expect(mockDirs("/home/user/", false).claude).toBe(false);
  expect(mockDirs(MOCK_CLAUDE_DIR, false).claude).toBe(true);
  expect(mockDirs(`${MOCK_CLAUDE_DIR}/`, false).claude).toBe(true);
});

const SHOP = "/home/user/projects/shop";
const API = "/home/user/projects/api";
const NO_ENV: SpaceEnv = {
  git_name: null,
  git_email: null,
  gh_account: null,
  claude_account: null,
};
function space(id: string, name: string, projects: string[], env = NO_ENV): Space {
  return { id, name, projects, env };
}

test("gh's accounts are listed, and the active one switched", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  await transport.listGhAccounts(null);
  await transport.switchGhAccount(null, { host: "github.com", login: "mock-personal" });
  // Another gh config folder (24.13) has an account of its own, named after it.
  await transport.listGhAccounts("/home/user/.config/gh-ci");
  await transport.switchGhAccount("/home/user/.config/gh-ci", {
    host: "github.com",
    login: "mock-gh-ci",
  });
  await tick();
  const login = (login: string, active: boolean) => ({
    host: "github.com",
    login,
    active,
    logged_in: true,
  });
  expect(messages).toEqual([
    {
      type: "gh_accounts",
      gh_config_dir: null,
      accounts: [login("mock-personal", false), login("mock-work", true)],
      problem: null,
    },
    {
      type: "gh_accounts",
      gh_config_dir: null,
      accounts: [login("mock-personal", true), login("mock-work", false)],
      problem: null,
    },
    ...[0, 1].map(() => ({
      type: "gh_accounts" as const,
      gh_config_dir: "/home/user/.config/gh-ci",
      accounts: [login("mock-gh-ci", true)],
      problem: null,
    })),
  ]);
});

test("a space's GitHub account must be in gh's own config or a settings' folder (24.13)", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  const dir = "/home/user/.config/gh-work";
  const gh_account = { host: "github.com", login: "mock-gh-work", config_dir: dir };
  await transport.updateSpace("default", "Default", { ...NO_ENV, gh_account });
  await tick();
  expect(messages.at(-1)).toEqual({
    type: "space_failed",
    message: `${dir} is not a gh config folder of Settings → Accounts: add it there first`,
  });
  await transport.setSettings({ ...DEFAULT_SETTINGS, gh: { config_dirs: [dir] } });
  await transport.updateSpace("default", "Default", { ...NO_ENV, gh_account });
  await tick();
  expect(messages.at(-1)?.type).toBe("spaces");
});

test("the default account has logged in, and an account added in the mock has not", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  await transport.listClaudeLogins();
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.claude.accounts.push({ name: "Work", config_dir: "/w" });
  await transport.setSettings(settings);
  await transport.listClaudeLogins();
  await tick();
  const logins = messages.filter((m) => m.type === "claude_logins");
  expect(logins).toEqual([
    { type: "claude_logins", logins: [{ config_dir: null, logged_in: true }] },
    {
      type: "claude_logins",
      logins: [
        { config_dir: null, logged_in: true },
        { config_dir: "/w", logged_in: false },
      ],
    },
  ]);
});

test("a logged-in account's identity is its login and noreply email", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  const work = { host: "github.com", login: "mock-work" };
  const gone = { host: "github.com", login: "gone" };
  await transport.getGhIdentity(null, work);
  await transport.getGhIdentity(null, gone);
  await tick();
  expect(messages).toEqual([
    {
      type: "gh_identity",
      account: work,
      name: "mock-work",
      email: "1002+mock-work@users.noreply.github.com",
    },
    {
      type: "gh_identity_failed",
      account: gone,
      message:
        "Could not get the identity of gone on github.com: gh gave no usable token for gone on github.com",
    },
  ]);
});

test("spaces are kept as the service keeps them", async () => {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  const env = { ...NO_ENV, git_name: "Me" };
  const def = space("default", "Default", [SHOP, API]);
  await transport.createSpace(" ", NO_ENV);
  await transport.createSpace(" Work ", env);
  await transport.listSessions();
  // A project of another space cannot join this one.
  await transport.addProject(SHOP);
  await transport.updateSpace("space-1", "Job", NO_ENV);
  await transport.updateSpace("space-1", "", NO_ENV);
  await transport.updateSpace("nope", "x", NO_ENV);
  await transport.selectSpace("nope");
  await transport.selectSpace("default");
  await transport.deleteSpace("default");
  await transport.deleteSpace("nope");
  await transport.deleteSpace("space-1");
  await transport.deleteSpace("default");
  await tick();
  const failed = (message: string) => ({ type: "space_failed", message }) as ServiceMessage;
  expect(messages).toEqual([
    failed("Enter a name for the space"),
    { type: "spaces", spaces: [def, space("space-1", "Work", [], env)], current: "space-1" },
    { type: "sessions", sessions: [], error: null, truncated: false },
    {
      type: "add_project_failed",
      path: SHOP,
      error: "in_other_space",
      message: `${SHOP} is already in the space Default`,
    },
    { type: "spaces", spaces: [def, space("space-1", "Job", [])], current: "space-1" },
    failed("Enter a name for the space"),
    failed('no space "nope"'),
    failed('no space "nope"'),
    { type: "spaces", spaces: [def, space("space-1", "Job", [])], current: "default" },
    failed("Default has projects: only an empty space can be deleted"),
    failed('no space "nope"'),
    { type: "spaces", spaces: [def], current: "default" },
    failed("Default has projects: only an empty space can be deleted"),
  ]);
});

test("deleting the current space makes the first one current, never the last one", async () => {
  const transport = createMockTransport("empty");
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await transport.createSpace("Work", NO_ENV);
  await transport.deleteSpace("space-1");
  await transport.deleteSpace("default");
  await tick();
  expect(messages.slice(-2)).toEqual([
    { type: "spaces", spaces: [space("default", "Default", [])], current: "default" },
    { type: "space_failed", message: "the last space cannot be deleted" },
  ]);
});

test("a folder of repositories is added as a group, and removed with them", async () => {
  const { transport, messages } = await connected();
  const [group, backend, frontend] = MOCK_GROUP as [Project, Project, Project];
  messages.length = 0;
  await transport.addProject(group.path);
  await transport.addProject(group.path);
  await tick();
  const added = { ...space("default", "Default", [SHOP, API, group.id]), groups: [group.id] };
  const listed = [...MOCK_REPOS.slice(0, 2), ...MOCK_GROUP];
  const answer: ServiceMessage[] = [
    { type: "spaces", spaces: [added], current: "default" },
    { type: "projects", projects: listed },
    { type: "project_added", project: group },
  ];
  // Again: nothing changes.
  expect(messages).toEqual([...answer, ...answer]);
  // From another space it is refused.
  await transport.createSpace("Work", NO_ENV);
  await tick();
  messages.length = 0;
  await transport.addProject(group.path);
  await tick();
  const message = `${group.path} is already in the space Default`;
  expect(messages).toEqual([
    { type: "add_project_failed", path: group.path, error: "in_other_space", message },
  ]);
  // Its repository leaves only with it; it leaves with its repositories.
  messages.length = 0;
  await transport.removeProject(backend.id);
  await transport.removeProject(group.id);
  await tick();
  const refused = `${backend.id} is in the group ${group.id}: remove the group to stop following it`;
  expect(messages.map((m) => ("id" in m ? [m.type, m.id] : m.type))).toEqual([
    ["remove_project_failed", backend.id],
    "spaces",
    ["project_removed", backend.id],
    ["project_removed", frontend.id],
    ["project_removed", group.id],
  ]);
  expect(messages[0]).toMatchObject({ message: refused });
});

test("a plain folder is a folder project: files without changes, until git init", async () => {
  const { transport, messages } = await connected();
  const folder = MOCK_FOLDER.path;
  messages.length = 0;
  await transport.addProject("/home/user");
  await tick();
  expect(messages).toEqual([
    {
      type: "add_project_failed",
      path: "/home/user",
      error: "protected_folder",
      message: "/home/user is the home folder: Hive does not follow it; choose a folder in it",
    },
  ]);
  messages.length = 0;
  await transport.addProject(folder);
  await tick();
  const added = { ...space("default", "Default", [SHOP, API, folder]), groups: [folder] };
  expect(messages).toEqual([
    { type: "spaces", spaces: [added], current: "default" },
    { type: "projects", projects: [...MOCK_REPOS.slice(0, 2), MOCK_FOLDER] },
    { type: "project_added", project: MOCK_FOLDER },
  ]);
  // Its files, with no changes; a file of it has no base.
  messages.length = 0;
  await transport.watchWorktree(folder, "head");
  await transport.openFile(folder, "todo.md", "head");
  await tick();
  expect(messages.map((m) => m.type)).toEqual(["files", "file"]);
  expect(messages[0]).toMatchObject({ files: MOCK_FOLDER_FILES });
  expect(messages[1]).toMatchObject({ path: "todo.md", base: null, error: null });
  // A repository's file keeps its base.
  messages.length = 0;
  await transport.openFile(SHOP, "README.md", "head");
  await tick();
  expect(messages[0]).toMatchObject({ base: expect.any(String) });
  // `git init` makes it a project, out of the groups; again it is refused.
  messages.length = 0;
  await transport.initGit(folder);
  await transport.initGit(folder);
  await tick();
  const [listed, refused] = messages as [ServiceMessage, ServiceMessage];
  expect(listed.type === "projects" && listed.projects.at(-1)).toMatchObject({
    id: folder,
    name: "notes",
  });
  expect(listed.type === "projects" && listed.projects.at(-1)?.folder).toBeUndefined();
  expect(refused).toEqual({ type: "error", message: `${folder} is not a folder project` });
  messages.length = 0;
  await transport.selectSpace("default");
  await tick();
  expect(messages[0]).toMatchObject({ spaces: [{ groups: [] }] });
});

test("a new folder and a new project are made as the service makes them (23.2)", async () => {
  const { transport, messages } = await connected();
  await tick();
  messages.length = 0;
  await transport.createDir("/home/user/", "games");
  await transport.createDir("/home/user", "games");
  await transport.createDir("/nowhere", "x");
  await transport.createDir("/home/user", "a/b");
  await transport.createDir("/home/user", "");
  // On Windows a path on drive D:, not a name.
  await transport.createDir("/home/user", "D:x");
  await tick();
  const one =
    'A name is one folder: no leading dot, no control characters, none of / \\ : * ? " < > |';
  expect(MOCK_NAME_RULE).toBe(one);
  expect(messages).toEqual([
    { type: "dir_created", parent: "/home/user/", name: "games" },
    {
      type: "create_dir_failed",
      parent: "/home/user",
      name: "games",
      message: "/home/user/games already exists",
    },
    {
      type: "create_dir_failed",
      parent: "/nowhere",
      name: "x",
      message: "cannot open /nowhere: No such file or directory (os error 2)",
    },
    { type: "create_dir_failed", parent: "/home/user", name: "a/b", message: one },
    { type: "create_dir_failed", parent: "/home/user", name: "", message: "Type a name" },
    { type: "create_dir_failed", parent: "/home/user", name: "D:x", message: one },
  ]);
  // Listed in its folder.
  messages.length = 0;
  await transport.listDirs("/home/user/", false, false);
  await tick();
  expect(messages[0]).toMatchObject({
    dirs: expect.arrayContaining([{ name: "games", git: false }]),
  });
  // A project with git, then a folder project (the list before it); refusals change nothing.
  messages.length = 0;
  await transport.createProject("/home/user/games", "chess", true);
  await tick();
  const chess = "/home/user/games/chess";
  expect(messages.map((m) => m.type)).toEqual(["spaces", "project_created"]);
  expect(messages[1]).toMatchObject({ path: chess, project: { id: chess, name: "chess" } });
  messages.length = 0;
  await transport.createProject("/home/user/games/", "go", false);
  await tick();
  const go = "/home/user/games/go";
  expect(messages.map((m) => m.type)).toEqual(["spaces", "projects", "project_created"]);
  expect(messages[2]).toMatchObject({ path: go, project: { id: go, folder: true } });
  expect(messages[0]).toMatchObject({ spaces: [{ groups: [go] }] });
  messages.length = 0;
  await transport.createProject("/home/user/games", "go", true);
  await transport.createProject("/gone", "x", true);
  await transport.createProject("/home/user/.claude-work", "x", true);
  await tick();
  expect(messages.map((m) => m.type === "add_project_failed" && m.error)).toEqual([
    "name_taken",
    "not_found",
    "protected_folder",
  ]);
});

test("a project is removed with its settings, never while a terminal works in it", async () => {
  const { transport, messages } = await connected();
  const scripts = { setup: "make", run: [], archive: null };
  const projects = { [SHOP]: { scripts } } as Settings["projects"];
  await transport.setSettings({ ...DEFAULT_SETTINGS, projects });
  const id = await transport.openTerminal(SHOP, 80, 24, () => {});
  await tick();
  messages.length = 0;
  await transport.removeProject("/nope");
  await transport.removeProject(SHOP);
  await tick();
  await transport.closeTerminal(id);
  await tick();
  expect(messages.pop()).toMatchObject({ type: "terminal_exited" });
  await transport.removeProject(SHOP);
  await transport.removeProject(API);
  await transport.listProjects();
  await tick();
  expect(messages).toEqual([
    {
      type: "remove_project_failed",
      id: "/nope",
      message: "/nope is not a followed project",
    },
    {
      type: "remove_project_failed",
      id: SHOP,
      message: `in use by fish (${id}): close its terminals first`,
    },
    { type: "spaces", spaces: [space("default", "Default", [API])], current: "default" },
    { type: "settings", settings: DEFAULT_SETTINGS },
    { type: "project_removed", id: SHOP },
    { type: "spaces", spaces: [space("default", "Default", [])], current: "default" },
    { type: "project_removed", id: API },
    { type: "spaces", spaces: [space("default", "Default", [])], current: "default" },
    { type: "projects", projects: [] },
  ]);
});

test("an issue's worktree is named by the service, and a failure by the issue's number", async () => {
  const { transport, messages } = await connected();
  const [shop] = MOCK_REPOS;
  messages.length = 0;
  await transport.createIssueWorktree(shop.id, 42, "main");
  await transport.createIssueWorktree(shop.id, 42, null);
  await transport.createIssueWorktree(shop.id, 7, null);
  await tick();
  const name = MOCK_ISSUES[42];
  const path = `${shop.path}/.claude/worktrees/${name}`;
  const created = messages[0] as Extract<ServiceMessage, { type: "worktree_created" }>;
  expect(created.path).toBe(path);
  expect(created.project.worktrees.at(-1)).toMatchObject({ path, name, branch: name });
  expect(messages.slice(1)).toEqual([
    {
      type: "create_worktree_failed",
      project: shop.id,
      name: "#42",
      message: `worktree "${name}" already exists at ${path}`,
    },
    {
      type: "create_worktree_failed",
      project: shop.id,
      name: "#7",
      message:
        "gh issue view failed: GraphQL: Could not resolve to an issue with the number of 7. (repository.issue)",
    },
  ]);
});
