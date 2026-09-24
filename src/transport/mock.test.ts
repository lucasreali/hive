import { expect, test } from "bun:test";
import type { AgentState, ServiceMessage } from "../store";
import {
  agentStatus,
  createMockTransport,
  ECHO_MARK,
  LOAD_STAGGER_MS,
  LOAD_START_MS,
  MOCK_BRANCHES,
  MOCK_FILES,
  MOCK_OWN_WORKTREE,
  MOCK_REPOS,
  MOCK_STATES,
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
    { type: "projects", projects: MOCK_REPOS.slice(0, 2) },
  ]);
});

test("a scenario fails the connection instead", async () => {
  for (const [scenario, types] of [
    ["mismatch", ["version_mismatch"]],
    ["disconnected", ["disconnected"]],
    ["", ["welcome", "projects"]],
    [
      "states",
      ["welcome", "projects", ...MOCK_STATES.flatMap(() => ["agent_detected", "agent_state"])],
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
  const [, listed] = messages;
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
  expect(messages.slice(2)).toEqual([
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
  const state = (state: AgentState): ServiceMessage => ({
    type: "agent_state",
    id: agent.id,
    ...agentStatus(state),
    subagents: [],
  });
  expect(messages.filter((m) => m.type.startsWith("agent"))).toEqual([
    { ...agent, project: shop.id, worktree: fixLogin, cwd: fixLogin },
    state("idle"),
    // The empty line sent nothing; `cd /tmp` and the second `claude` are prompts.
    state("working"),
    state("working"),
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
  expect([...calm, ...pending].map(agentStatus)).toEqual([
    { state: "ended", urgency: 0, pending: false },
    { state: "idle", urgency: 1, pending: false },
    { state: "working", urgency: 2, pending: false },
    { state: "with_subagents", urgency: 3, pending: false },
    { state: "waiting_you", urgency: 4, pending: true },
    { state: "error", urgency: 5, pending: true },
    { state: "waiting_permission", urgency: 6, pending: true },
  ]);
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
