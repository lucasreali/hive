import type { AgentState, Project, ServiceMessage, Subagent, Worktree } from "../store";
import type { Transport } from ".";
import { loadReplay, type ReplayEvent } from "./replay";

const PROMPT = "mock$ ";
/** With `?mock=load`, sent after every echo so the load test can time it; xterm ignores it. */
export const ECHO_MARK = "\x1b]7777;echo\x07";

/** What the fake service answers on `connect`, picked with `?mock=<scenario>`. */
const HANDSHAKE: Record<string, ServiceMessage> = {
  mismatch: {
    type: "version_mismatch",
    protocol: 1,
    version: "0.0.0-mock",
    app_protocol: 1,
    app_version: "mock",
  },
  disconnected: { type: "disconnected", reason: "mock: the hive bridge exited" },
};
/** `?mock=load`: when the first terminal's replay starts, and how much later each next one does. */
export const LOAD_START_MS = 500;
export const LOAD_STAGGER_MS = 100;
const WELCOME: ServiceMessage = { type: "welcome", version: "mock", distro: "Ubuntu" };

const sub = (id: string, agent_type: string | null, state: AgentState): Subagent => ({
  id,
  agent_type,
  state,
});
/** A stand-in for `AgentState::urgency`/`pending`, least urgent first; the real rule lives in Rust. */
const URGENCY: AgentState[] = [
  "ended",
  "idle",
  "working",
  "with_subagents",
  "waiting_you",
  "error",
  "waiting_permission",
];
export const agentStatus = (state: AgentState) => {
  const urgency = URGENCY.indexOf(state);
  return { state, urgency, pending: urgency >= URGENCY.indexOf("waiting_you") };
};

/**
 * `?mock=states`: agents without a terminal in every state, by worktree path (relative to
 * `/home/user/projects`), as the service would resolve them ("the most urgent wins").
 */
export const MOCK_STATES: [string, AgentState, Subagent[]][] = [
  ["shop", "with_subagents", [sub("a1", "Explore", "working"), sub("a2", null, "working")]],
  [
    "shop/.claude/worktrees/fix-login",
    "waiting_permission",
    [sub("a3", "general-purpose", "waiting_permission"), sub("a4", "Explore", "idle")],
  ],
  ["shop/.claude/worktrees/feat-checkout", "waiting_you", []],
  ["api", "error", []],
  ["api", "ended", []],
  ["api/.claude/worktrees/refactor-auth", "working", []],
  ["api/.claude/worktrees/refactor-auth", "idle", []],
];

function mockStates(): ServiceMessage[] {
  return MOCK_STATES.flatMap(([dir, state, subagents], i) => {
    const path = `/home/user/projects/${dir}`;
    const id = `mock-state-${i + 1}`;
    const project = `/home/user/projects/${dir.split("/")[0]}`;
    const place = { project, worktree: path, cwd: path };
    return [
      { type: "agent_detected", channel: 1000 + i, id, ...place },
      { type: "agent_state", id, ...agentStatus(state), subagents },
    ];
  });
}

const worktree = (root: string, name: string, main = false): Worktree => {
  const path = main ? root : `${root}/.claude/worktrees/${name}`;
  return { id: path, name, path, branch: main ? name : `worktree-${name}`, main, claude: !main };
};
const project = (path: string, name: string, worktrees: string[]): Project => ({
  id: path,
  name,
  path,
  worktrees: [worktree(path, "main", true), ...worktrees.map((w) => worktree(path, w))],
  error: null,
});

/** The fake service's git repositories; the first two are followed from the start. */
export const MOCK_REPOS = [
  project("/home/user/projects/shop", "shop", ["fix-login", "feat-checkout"]),
  project("/home/user/projects/api", "api", ["refactor-auth"]),
  project("/home/user/dotfiles", "dotfiles", []),
];

/** The fake repositories' branches; shop has enough remote ones to need scrolling. */
export const MOCK_BRANCHES: Record<string, { local: string[]; remote: string[] }> = {
  "/home/user/projects/shop": {
    local: ["main", "develop", "fix-login", "feat-checkout"],
    remote: [
      "origin/main",
      "origin/develop",
      "origin/release/2.4",
      "origin/feat-coupon",
      "origin/fix-shipping",
      ...Array.from({ length: 200 }, (_, i) => `origin/renovate/dependency-${i + 1}`),
    ],
  },
  "/home/user/projects/api": {
    local: ["main", "refactor-auth"],
    remote: ["origin/main", "origin/v2-legacy", "origin/feat-webhooks"],
  },
  "/home/user/dotfiles": { local: ["main"], remote: ["origin/main"] },
};

/**
 * What the fake service lists in every worktree: a small web app, plus enough generated
 * files in a project's main worktree to need scrolling.
 */
export const MOCK_FILES = [
  ".gitignore",
  "README.md",
  "config/shipping.json",
  "docs/api.md",
  "package.json",
  "src/App.tsx",
  "src/auth/login.ts",
  "src/auth/session.ts",
  "src/checkout/Cart.tsx",
  "src/checkout/CheckoutSummary.tsx",
  "src/checkout/validators.ts",
  "src/components/Button.tsx",
  "src/components/Header.tsx",
  "src/server.ts",
  "tests/checkout.test.ts",
  "tsconfig.json",
];
const mockFiles = (worktree: Worktree) =>
  [
    ...MOCK_FILES,
    ...(worktree.main ? Array.from({ length: 400 }, (_, i) => `src/icons/icon-${i}.tsx`) : []),
  ].sort();

// A stand-in for `hive::worktree::check_name` and its CLI wording; the real rule lives in Rust.
function nameError(project: Project, name: string): string | null {
  if (!/^[a-z0-9][a-z0-9._-]*$/.test(name)) {
    return `invalid worktree name ${JSON.stringify(name)}: use lowercase letters, digits, '.', '_' and '-', starting with a letter or digit`;
  }
  const taken = project.worktrees.find((w) => w.claude && w.name === name);
  return taken ? `worktree ${JSON.stringify(name)} already exists at ${taken.path}` : null;
}

/**
 * A fake service for the browser (`bun run dev`, Playwright): it welcomes the UI, and each
 * terminal shows a prompt, echoes what is typed, repeats the line on Enter and exits on `exit`;
 * `cd <dir>` moves it and `claude` detects an idle agent there (removed when the terminal
 * exits); every later line sets that agent working.
 * Projects come from `MOCK_REPOS`; any other path is refused as not found. Branches come from
 * `MOCK_BRANCHES`, and new worktrees are added to the fake project. A watched worktree lists
 * `MOCK_FILES`; `touch <name>` in a terminal there adds a file and sends the list again.
 * Service messages arrive asynchronously, as they do from the real service.
 * `scenario` ("mismatch" or "disconnected") answers `connect` with that failure instead;
 * "empty" starts with no projects; "states" adds `MOCK_STATES`' agents. "load" (1.11) replays a recording into every terminal right
 * after its prompt, at recorded timing, each terminal starting `LOAD_STAGGER_MS` later than
 * the previous one; `cast` is the URL of an asciinema recording to replay instead of the
 * generated one.
 */
export function createMockTransport(
  scenario: string | null = null,
  cast: string | null = null,
): Transport {
  let send: (message: ServiceMessage) => void = () => {};
  const projects = scenario === "empty" ? [] : MOCK_REPOS.slice(0, 2);
  const find = (id: string) => projects.find((p) => p.id === id);
  let last = 0;
  type MockTerminal = {
    onData: (bytes: Uint8Array) => void;
    line: string;
    cwd: string;
    agent: string | null;
  };
  const terminals = new Map<number, MockTerminal>();
  const encoder = new TextEncoder();
  const later = (message: ServiceMessage) => setTimeout(() => send(message), 0);
  const print = (id: number, text: string) => terminals.get(id)?.onData(encoder.encode(text));
  let recording: Promise<ReplayEvent[]> | undefined;
  const replay = async (id: number) => {
    recording ??= loadReplay(cast);
    const events = await recording;
    const start = performance.now() + LOAD_START_MS + (id % 20) * LOAD_STAGGER_MS;
    let next = 0;
    const step = () => {
      const now = performance.now() - start;
      while (next < events.length && (events[next] as ReplayEvent).at <= now) {
        print(id, (events[next++] as ReplayEvent).data);
      }
      if (next < events.length && terminals.has(id)) {
        setTimeout(step, (events[next] as ReplayEvent).at - now);
      }
    };
    setTimeout(step, start - performance.now());
  };
  const exit = (id: number, code: number | null) => {
    const agent = terminals.get(id)?.agent;
    if (agent) later({ type: "agent_removed", channel: id, id: agent });
    if (terminals.delete(id)) later({ type: "terminal_exited", channel: id, code });
  };
  // A stand-in for a `SessionStart` placed by `projects::place`: only an exact worktree path.
  const detect = (id: number, terminal: MockTerminal) => {
    const cwd = terminal.cwd;
    const project = projects.find((p) => p.worktrees.some((w) => w.path === cwd));
    const worktree = project ? cwd : null;
    terminal.agent = `mock-session-${id}`;
    const placed = { project: project?.id ?? null, worktree, cwd };
    later({ type: "agent_detected", channel: id, id: terminal.agent, ...placed });
    setState(terminal.agent, "idle");
  };
  const setState = (id: string, state: AgentState) =>
    later({ type: "agent_state", id, ...agentStatus(state), subagents: [] });
  // Files by worktree path, and the one watched.
  const files = new Map<string, string[]>();
  let watched: string | null = null;
  const worktreeAt = (path: string) =>
    projects.flatMap((p) => p.worktrees).find((w) => w.path === path);
  const sendFiles = (path: string) =>
    later({ type: "files", path, files: files.get(path) as string[], truncated: false });
  // A stand-in for an agent writing a file: `touch <name>` in a worktree's terminal.
  const touch = (cwd: string, name: string) => {
    const worktree = worktreeAt(cwd);
    if (!worktree) return;
    const listed = files.get(cwd) ?? mockFiles(worktree);
    files.set(cwd, [...new Set([...listed, name])].sort());
    if (watched === cwd) sendFiles(cwd);
  };

  return {
    async connect(onMessage) {
      send = onMessage;
      const failure = HANDSHAKE[scenario ?? ""];
      later(failure ?? WELCOME);
      if (failure) return;
      later({ type: "projects", projects });
      if (scenario === "states") for (const m of mockStates()) later(m);
    },
    async listProjects() {
      later({ type: "projects", projects });
    },
    async addProject(path) {
      const repo = MOCK_REPOS.find((p) => p.path === path);
      if (!repo) {
        const message = `cannot open ${path}: No such file or directory (os error 2)`;
        return void later({ type: "add_project_failed", path, error: "not_found", message });
      }
      if (!find(path)) projects.push(repo);
      later({ type: "project_added", project: find(path) as Project });
    },
    async listBranches(project) {
      const branches = MOCK_BRANCHES[project];
      const answer = find(project)
        ? { ...branches, current: "main", error: null }
        : { local: [], remote: [], current: null, error: `${project} is not a followed project` };
      later({ type: "branches", project, ...answer });
    },
    async validateWorktreeName(project, name) {
      const shown = name || "<name>";
      const folder = `.claude/worktrees/${shown}/`;
      const error = nameError(find(project) as Project, name);
      later({
        type: "worktree_name_validated",
        project,
        name,
        folder,
        branch: `worktree-${shown}`,
        error,
      });
    },
    async createWorktree(id, name) {
      const project = find(id) as Project;
      const message = nameError(project, name);
      if (message)
        return void later({ type: "create_worktree_failed", project: id, name, message });
      const updated = { ...project, worktrees: [...project.worktrees, worktree(id, name)] };
      projects[projects.indexOf(project)] = updated;
      const path = worktree(id, name).path;
      later({ type: "worktree_created", project: updated, path, notes: [] });
    },
    async openTerminal(cwd, _cols, _rows, onData) {
      const id = ++last;
      terminals.set(id, { onData, line: "", cwd, agent: null });
      later({ type: "terminal_opened", channel: id });
      setTimeout(() => print(id, PROMPT), 0);
      if (scenario === "load") void replay(id);
      return id;
    },
    async writeTerminal(id, data) {
      const terminal = terminals.get(id);
      if (!terminal) return;
      for (const char of data) {
        if (char !== "\r") {
          terminal.line += char;
          print(id, char);
          continue;
        }
        const line = terminal.line;
        terminal.line = "";
        if (line === "exit") return exit(id, 0);
        // A stand-in for `UserPromptSubmit`: any line typed to a running agent sets it working.
        if (terminal.agent && line) setState(terminal.agent, "working");
        if (line === "claude") detect(id, terminal);
        if (line.startsWith("touch ")) touch(terminal.cwd, line.slice(6));
        if (line.startsWith("cd ")) {
          const dir = line.slice(3);
          terminal.cwd = dir.startsWith("/") ? dir : `${terminal.cwd}/${dir}`;
        }
        print(id, `\r\n${line ? `${line}\r\n` : ""}${PROMPT}`);
      }
      if (scenario === "load") print(id, ECHO_MARK);
    },
    async watchWorktree(path) {
      const worktree = worktreeAt(path);
      watched = worktree ? path : null;
      if (!worktree) {
        const message = `${path} is not a worktree of a followed project`;
        return void later({ type: "error", message });
      }
      if (!files.has(path)) files.set(path, mockFiles(worktree));
      sendFiles(path);
    },
    async unwatchWorktree() {
      watched = null;
    },
    async resizeTerminal() {},
    async closeTerminal(id) {
      exit(id, null);
    },
  };
}
