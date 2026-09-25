import { expect, test } from "bun:test";
import { type AgentState, DEFAULT_SETTINGS, type ServiceMessage } from "../store";
import {
  agentStatus,
  createMockTransport,
  ECHO_MARK,
  LOAD_STAGGER_MS,
  LOAD_START_MS,
  MOCK_BRANCHES,
  MOCK_CHANGES,
  MOCK_DIAGNOSTICS,
  MOCK_FILES,
  MOCK_OWN_WORKTREE,
  MOCK_REPOS,
  MOCK_SESSIONS,
  MOCK_STATES,
  MOCK_TEXTS,
  mockDirs,
  mockTranscript,
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
    { type: "projects", projects: MOCK_REPOS.slice(0, 2) },
  ]);
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
    { type: "editor_target", worktree: "", path: "", windows_path, error: null },
    { type: "diagnostics", ...MOCK_DIAGNOSTICS },
  ]);
});

test("a scenario fails the connection instead", async () => {
  for (const [scenario, types] of [
    ["mismatch", ["version_mismatch"]],
    ["disconnected", ["disconnected"]],
    ["", ["welcome", "settings", "projects"]],
    [
      "states",
      [
        "welcome",
        "settings",
        "projects",
        ...MOCK_STATES.flatMap(() => ["agent_detected", "agent_state", "agent_usage"]),
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
  const [, , listed] = messages;
  const shop = listed?.type === "projects" ? listed.projects[0] : undefined;
  expect(shop?.worktrees.map((w) => w.id).at(-1)).toBe(MOCK_OWN_WORKTREE);
  const owners = MOCK_STATES.flatMap(([, , subs]) => subs).filter((s) => s.worktree);
  expect(owners.map((s) => [s.id, s.worktree])).toEqual([["a3", MOCK_OWN_WORKTREE]]);
  // Other scenarios keep the fake repositories as they are.
  expect(MOCK_REPOS[0].worktrees.map((w) => w.id)).not.toContain(MOCK_OWN_WORKTREE);
});

test("projects are added from the fake repositories only", async () => {
  const transport = createMockTransport("empty");
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  expect(messages.at(-1)).toEqual({ type: "projects", projects: [] });

  const [shop] = MOCK_REPOS;
  await transport.addProject(shop.path);
  await transport.addProject(shop.path);
  await transport.addProject("/nope");
  await transport.listProjects();
  await tick();
  expect(messages.slice(3)).toEqual([
    { type: "project_added", project: shop },
    { type: "project_added", project: shop },
    {
      type: "add_project_failed",
      path: "/nope",
      error: "not_found",
      message: "cannot open /nope: No such file or directory (os error 2)",
    },
    { type: "projects", projects: [shop] },
  ]);
  expect(shop.worktrees.map((w) => [w.name, w.branch, w.main, w.claude])).toEqual([
    ["main", "main", true, false],
    ["fix-login", "worktree-fix-login", false, true],
    ["feat-checkout", "worktree-feat-checkout", false, true],
  ]);
});

test("a terminal prints a prompt, echoes input and repeats the line", async () => {
  const { transport, messages, id, output } = await opened();
  expect(id).toBe(1);
  expect(messages.at(-1)).toEqual({ type: "terminal_opened", channel: 1 });
  expect(output()).toBe("mock$ ");
  await transport.writeTerminal(id, "hi\r\r");
  expect(output()).toBe("mock$ hi\r\nhi\r\nmock$ \r\nmock$ ");
  await transport.resizeTerminal(id, 100, 30);
  expect(await transport.openTerminal("/", 1, 1, () => {})).toBe(2);
});

test("exit ends the terminal with code 0 and close with no code", async () => {
  const { transport, messages, id, output } = await opened();
  await transport.writeTerminal(id, "exit\rignored");
  await tick();
  expect(messages.at(-1)).toEqual({ type: "terminal_exited", channel: id, code: 0 });
  expect(output()).toBe("mock$ exit");

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
  await transport.writeTerminal(id, "exit\r");
  await tick();
  expect(messages.slice(-2)).toEqual([
    { type: "agent_removed", channel: id, id: "mock-session-1" },
    { type: "terminal_exited", channel: id, code: 0 },
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
  const pending = ["waiting_you", "error", "waiting_permission"] as const;
  const doing = { activity: null, since_ms: 0 };
  expect([...calm, ...pending].map((s) => agentStatus(s))).toEqual([
    { state: "ended", urgency: 0, pending: false, ...doing },
    { state: "idle", urgency: 1, pending: false, ...doing },
    { state: "working", urgency: 2, pending: false, ...doing },
    { state: "with_subagents", urgency: 3, pending: false, ...doing },
    { state: "waiting_you", urgency: 4, pending: true, ...doing },
    { state: "error", urgency: 5, pending: true, ...doing },
    { state: "waiting_permission", urgency: 6, pending: true, ...doing },
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
    expect(out).toEqual([`x${ECHO_MARK}mock$ one two`, "mock$ one "]);
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("a watched worktree lists its files and again after a touch in its terminal", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const fix = (shop.worktrees[1] as { path: string }).path;
  const filesOf = () => messages.filter((m) => m.type === "files" || m.type === "error");
  await transport.watchWorktree("/nowhere");
  await transport.watchWorktree(shop.path);
  await tick();
  const [refused, main] = filesOf();
  const message = "/nowhere is not a worktree of a followed project";
  expect(refused).toEqual({ type: "error", message });
  expect(main).toMatchObject({ type: "files", path: shop.path, truncated: false });
  // The main worktree has enough files to scroll.
  expect(main?.type === "files" && main.files.length).toBe(MOCK_FILES.length + 400);
  await transport.watchWorktree(fix);
  await tick();
  expect(filesOf()[2]).toEqual({ type: "files", path: fix, files: MOCK_FILES, truncated: false });

  const id = await transport.openTerminal(fix, 80, 24, () => {});
  await transport.writeTerminal(id, "touch a.txt\r");
  await tick();
  const touched = ["a.txt", ...MOCK_FILES].sort();
  expect(filesOf()[3]).toEqual({ type: "files", path: fix, files: touched, truncated: false });
  // Unwatched, or outside a worktree, a touch sends nothing.
  await transport.unwatchWorktree();
  await transport.writeTerminal(id, "touch b.txt\r");
  await transport.writeTerminal(id, "cd src\r");
  await transport.writeTerminal(id, "touch c.txt\r");
  // A worktree touched before it is watched keeps the file.
  const other = await transport.openTerminal(shop.path, 80, 24, () => {});
  await transport.writeTerminal(other, "touch d.txt\r");
  await tick();
  expect(filesOf()).toHaveLength(4);
  await transport.watchWorktree(fix);
  await transport.watchWorktree(shop.path);
  await tick();
  const [, , , , again, main2] = filesOf();
  expect(again?.type === "files" && again.files).toEqual(["b.txt", ...touched].sort());
  expect(main2?.type === "files" && main2.files.includes("d.txt")).toBe(true);
});

test("changes of a followed worktree with their totals, or why not", async () => {
  const { transport, messages } = await connected();
  const [shop, api, dotfiles] = MOCK_REPOS;
  const refactor = api.worktrees[1].path;
  messages.length = 0;
  await transport.listChanges(refactor);
  await transport.listChanges(shop.path);
  await transport.listChanges(dotfiles.path);
  await tick();
  expect(messages).toEqual([
    {
      type: "changes",
      path: refactor,
      files: MOCK_CHANGES[refactor],
      added: 24,
      removed: 49,
      error: null,
    },
    { type: "changes", path: shop.path, files: [], added: 0, removed: 0, error: null },
    {
      type: "changes",
      path: dotfiles.path,
      files: [],
      added: 0,
      removed: 0,
      error: `${dotfiles.path} is not a worktree of a followed project`,
    },
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
  for (const [worktree, path] of asked) await transport.openFile(worktree, path);
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

test("a save checks the version as the service does; write stands in for an agent", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const readme = "// README.md\nexport const value = 1;\n";
  messages.length = 0;
  await transport.saveFile(shop.path, "README.md", "mine\n", mockVersion(readme));
  await transport.saveFile(shop.path, "README.md", "late\n", mockVersion(readme));
  await transport.saveFile("/nowhere", "a", "x", null);
  await transport.openFile(shop.path, "README.md");
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
  await transport.watchWorktree(shop.path);
  const id = await transport.openTerminal(shop.path, 80, 24, () => {});
  await tick();
  messages.length = 0;
  await transport.writeTerminal(id, "write README.md agent was here\r");
  await transport.openFile(shop.path, "README.md");
  await tick();
  expect(messages.map((m) => m.type)).toEqual(["files", "changes", "file"]);
  expect(messages[2]).toMatchObject({ content: "agent was here\n" });
  await transport.unwatchWorktree();
  await transport.writeTerminal(id, "write README.md again\r");
  // The view changes nothing in the mock.
  await transport.setView(id, true);
  await tick();
  expect(messages).toHaveLength(3);
});

test("a file's Windows path for an external editor, or why not", async () => {
  const { transport, messages } = await connected();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  messages.length = 0;
  await transport.openInEditor(shop.path, "src/App.tsx");
  await transport.openInEditor("/nowhere", "a");
  await tick();
  expect(messages).toEqual([
    {
      type: "editor_target",
      worktree: shop.path,
      path: "src/App.tsx",
      windows_path: "\\\\wsl.localhost\\Ubuntu\\home\\user\\projects\\shop\\src\\App.tsx",
      error: null,
    },
    {
      type: "editor_target",
      worktree: "/nowhere",
      path: "a",
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
  await transport.openInEditor(shop.path, "");
  await tick();
  expect(messages).toEqual([
    {
      type: "editor_target",
      worktree: shop.path,
      path: "",
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
    { type: "sessions", sessions: MOCK_SESSIONS, error: null },
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
  expect(await answers(() => transport.deleteSession(agent))).toEqual([
    {
      type: "delete_session_failed",
      id: agent,
      message: "the session is running: end it first",
    },
  ]);
  expect(await answers(() => transport.deleteSession(first.id))).toEqual([
    { type: "session_deleted", id: first.id },
  ]);
  const [listed] = (await answers(() => transport.listSessions())) as { sessions: unknown[] }[];
  expect(listed.sessions).toHaveLength(MOCK_SESSIONS.length - 1);
  // The shared fake list is left alone.
  expect(MOCK_SESSIONS[0]).toBe(first);
});

test("?mock=update offers an update whose install fails; otherwise none is offered", async () => {
  const { transport, messages } = await connected();
  await transport.checkUpdate();
  await tick();
  expect(messages.some((m) => m.type === "update_ready")).toBe(false);

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

test("folders are browsed on both sides of the fake machine", async () => {
  const { transport, messages } = await connected();
  messages.length = 0;
  await transport.listDirs("", false);
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
      ],
      error: null,
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
  });
});

test("states: each subagent of MOCK_STATES has a conversation; others have none", async () => {
  const none = { type: "error", message: "no transcript is known for this subagent" };
  const answers = async (scenario: string | null, asked: [string, string][]) => {
    const messages: ServiceMessage[] = [];
    const transport = createMockTransport(scenario);
    await transport.connect((m) => messages.push(m));
    await tick();
    messages.length = 0;
    for (const [agent, subagent] of asked) await transport.watchTranscript(agent, subagent);
    await transport.unwatchTranscript("mock-state-2", "a3");
    await tick();
    return messages;
  };
  expect(
    await answers("states", [
      ["mock-state-2", "a3"],
      ["mock-state-2", "a1"],
      ["other", "a1"],
    ]),
  ).toEqual([
    {
      type: "transcript",
      agent: "mock-state-2",
      subagent: "a3",
      entries: mockTranscript("a3"),
      truncated: false,
    },
    none,
    none,
  ] as ServiceMessage[]);
  // Without the states scenario there are no subagents.
  expect(await answers(null, [["mock-state-2", "a3"]])).toEqual([none] as ServiceMessage[]);
});
