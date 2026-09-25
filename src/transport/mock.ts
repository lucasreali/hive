import type {
  AgentState,
  ChangedFile,
  Dirs,
  FileStatus,
  FileText,
  Project,
  SaveError,
  ServiceMessage,
  Session,
  Settings,
  Subagent,
  TranscriptEntry,
  Worktree,
} from "../store";
import { DEFAULT_SETTINGS } from "../store";
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
export const MOCK_DIAGNOSTICS = {
  settings_file: "/home/mock/.config/hive/settings.json",
  wrapper: "/home/mock/.local/share/hive/bin/claude",
  claude: "/home/mock/.local/bin/claude",
};

const WELCOME: ServiceMessage = { type: "welcome", version: "mock", distro: "Ubuntu" };

const sub = (
  id: string,
  agent_type: string | null,
  state: AgentState,
  worktree: string | null = null,
  activity: string | null = null,
): Subagent => ({ id, agent_type, state, worktree, activity, since_ms: Date.now() - 42_000 });
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
export const agentStatus = (state: AgentState, activity: string | null = null, since_ms = 0) => {
  const urgency = URGENCY.indexOf(state);
  return { state, urgency, pending: urgency >= URGENCY.indexOf("waiting_you"), activity, since_ms };
};

/** `?mock=states`: shop's worktree that subagent a3 works in, shown as its parent row (#22). */
export const MOCK_OWN_WORKTREE = "/home/user/projects/shop/.claude/worktrees/tests-login";

/**
 * `?mock=states`: agents without a terminal in every state, by worktree path (relative to
 * `/home/user/projects`), as the service would resolve them ("the most urgent wins"), with
 * what they are doing.
 */
export const MOCK_STATES: [string, AgentState, Subagent[], string | null][] = [
  [
    "shop",
    "with_subagents",
    [
      sub("a1", "Explore", "working", null, "Searching useSession"),
      sub("a2", null, "working", null, "Reading src/auth/session.ts"),
    ],
    "Find where sessions expire",
  ],
  [
    "shop/.claude/worktrees/fix-login",
    "waiting_permission",
    [
      sub("a3", "general-purpose", "waiting_permission", MOCK_OWN_WORKTREE, "bun test src/auth"),
      sub("a4", "Explore", "idle"),
    ],
    "Editing src/auth/login.ts",
  ],
  ["shop/.claude/worktrees/feat-checkout", "waiting_you", [], null],
  ["api", "error", [], null],
  ["api", "ended", [], null],
  ["api/.claude/worktrees/refactor-auth", "working", [], "Run the API tests"],
  ["api/.claude/worktrees/refactor-auth", "idle", [], null],
];

/** `?mock=states`: the conversation of each of `MOCK_STATES`' subagents. */
export const mockTranscript = (subagent: string): TranscriptEntry[] => [
  { role: "user", text: `Find where the login form is handled (${subagent}).`, tool: null },
  { role: "assistant", text: "I'll search the code for the login handler.", tool: null },
  { role: "tool", text: '{"pattern":"login","path":"src"}', tool: "Grep" },
  { role: "assistant", text: "The login form posts to /api/session in src/auth.ts.", tool: null },
];

function mockStates(): ServiceMessage[] {
  return MOCK_STATES.flatMap(([dir, state, subagents, activity], i) => {
    const path = `/home/user/projects/${dir}`;
    const id = `mock-state-${i + 1}`;
    const project = `/home/user/projects/${dir.split("/")[0]}`;
    const place = { project, worktree: path, cwd: path };
    return [
      { type: "agent_detected", channel: 1000 + i, id, ...place },
      {
        type: "agent_state",
        id,
        ...agentStatus(state, activity, Date.now() - (i + 1) * 97_000),
        subagents,
      },
      {
        type: "agent_usage",
        id,
        context_tokens: (i + 1) * 23_000,
        context_limit: 200_000,
        output_tokens: (i + 1) * 4_100,
      },
    ];
  });
}

/** A stand-in for `hive::health`: `[changes, ahead, behind]` of a linked worktree, by name. */
const HEALTH: Record<string, [number, number, number]> = {
  "fix-login": [2, 3, 1],
  "feat-checkout": [2, 1, 0],
  "refactor-auth": [6, 0, 2],
};
/** The mock's last commits: 2026-09-20. */
const LAST_COMMIT_MS = Date.UTC(2026, 8, 20);

const worktree = (root: string, name: string, main = false): Worktree => {
  const path = main ? root : `${root}/.claude/worktrees/${name}`;
  const [changes, ahead, behind] = HEALTH[name] ?? [0, 0, 0];
  const status = main
    ? { changes: 0, ahead: null, behind: null, merged: false, last_commit_ms: LAST_COMMIT_MS }
    : { changes, ahead, behind, merged: ahead === 0, last_commit_ms: LAST_COMMIT_MS };
  const branch = main ? name : `worktree-${name}`;
  return { id: path, name, path, branch, main, claude: !main, status };
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
  // On the Windows side (C:), which WSL sees under /mnt/c.
  project("/mnt/c/Users/user/source/site", "site", []),
];

/**
 * The fake machine's folders for "Add project", besides the repositories (every folder above
 * one exists too): WSL's home and Windows' user folder (`C:\Users\user`, `/mnt/c/...`).
 */
export const MOCK_FOLDERS = [
  ...MOCK_REPOS.map((p) => p.path),
  "/home/user/Downloads",
  "/home/user/projects/notes",
  "/mnt/c/Users/user/Documents",
];

/** The folder part of `path`: up to its last separator (`\` too on Windows), else empty. */
const folderPart = (path: string, windows: boolean) =>
  path.slice(0, Math.max(path.lastIndexOf("/"), windows ? path.lastIndexOf("\\") : -1) + 1);

/** A stand-in for `hive::dirs::answer` over `MOCK_FOLDERS`. */
export function mockDirs(asked: string, windows: boolean): Dirs {
  const path = asked || (windows ? "C:\\Users\\user\\" : "/home/user/");
  const unix = windows
    ? path
        .replace(/^([A-Za-z]):/, (_, drive: string) => `/mnt/${drive.toLowerCase()}`)
        .replaceAll("\\", "/")
    : path;
  const answer = { path, windows, linux_path: null, parent: null, dirs: [] };
  if (!unix.startsWith("/")) return { ...answer, error: "type a full path" };
  const folder = unix.slice(0, unix.lastIndexOf("/") + 1);
  const inside = MOCK_FOLDERS.filter((f) => `${f}/`.startsWith(folder));
  if (inside.length === 0) {
    const error = `cannot open ${folder}: No such file or directory (os error 2)`;
    return { ...answer, error };
  }
  const names = new Set(inside.map((f) => f.slice(folder.length).split("/")[0]).filter(Boolean));
  const dirs = [...names]
    .sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()))
    .map((name) => ({ name, git: MOCK_REPOS.some((p) => p.path === folder + name) }));
  const above = folderPart(path, windows).replace(windows ? /[\\/]+$/ : /\/+$/, "");
  const parent = folderPart(above, windows) || null;
  return { ...answer, linux_path: unix, parent, dirs, error: null };
}

const MINUTE = 60_000;
const session = (
  id: string,
  cwd: string,
  title: string | null,
  last: [Session["last_role"], string] | null,
  messages: number,
  ago: number,
  state: Session["state"] = "ended",
  running = false,
): Session => {
  const place = [...MOCK_REPOS.flatMap((p) => p.worktrees.map((w) => ({ p, w })))]
    .filter(({ w }) => cwd === w.path || cwd.startsWith(`${w.path}/`))
    .sort((a, b) => b.w.path.length - a.w.path.length)[0] as {
    p: Project;
    w: Worktree;
  };
  return {
    id,
    project: place.p.id,
    worktree: place.w.id,
    cwd,
    title,
    last_role: last?.[0] ?? null,
    last_text: last?.[1] ?? null,
    messages,
    model: "claude-opus-5-5",
    // Made up: a bigger context for a longer session.
    context_tokens: messages * 2_000,
    output_tokens: messages * 300,
    branch: place.w.branch,
    updated_ms: Date.now() - ago * MINUTE,
    state,
    running,
    log: `/home/user/.claude/projects/${cwd.replaceAll(/[^A-Za-z0-9]/g, "-")}/${id}.jsonl`,
  };
};

/** The fake service's Claude sessions, the most recent first (`?mock` sessions view). */
export const MOCK_SESSIONS: Session[] = [
  session(
    "0b1d2c3e-1111-4a4a-9b9b-000000000001",
    "/home/user/projects/shop/.claude/worktrees/fix-login",
    "Fix the login redirect",
    ["assistant", "The redirect now keeps the original URL; tests pass."],
    42,
    2,
    // Running in a terminal outside Hive, done with its turn.
    "waiting_you",
    true,
  ),
  session(
    "0b1d2c3e-1111-4a4a-9b9b-000000000002",
    "/home/user/projects/shop",
    "Checkout totals",
    ["user", "[Request interrupted by user]"],
    7,
    55,
    "waiting_you",
  ),
  session(
    "0b1d2c3e-1111-4a4a-9b9b-000000000003",
    "/home/user/projects/shop/src",
    null,
    null,
    0,
    60 * 26,
  ),
  session(
    "0b1d2c3e-1111-4a4a-9b9b-000000000004",
    "/home/user/projects/api/.claude/worktrees/refactor-auth",
    "Refactor auth middleware",
    ["assistant", "Moved the token check into its own middleware."],
    118,
    60 * 24 * 9,
    "error",
  ),
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

const change = (
  path: string,
  status: FileStatus,
  added: number | null,
  removed: number | null,
  old_path: string | null = null,
): ChangedFile => ({ path, status, old_path, added, removed });

/** What differs from HEAD in the fake worktrees, by path (after the prototype's screen 1g). */
export const MOCK_CHANGES: Record<string, ChangedFile[]> = {
  "/home/user/projects/shop/.claude/worktrees/fix-login": [
    change("src/auth/constants.ts", "modified", 1, 0),
    change("src/auth/session.ts", "modified", 3, 1),
  ],
  "/home/user/projects/shop/.claude/worktrees/feat-checkout": [
    change("src/checkout/CheckoutSummary.tsx", "modified", 6, 2),
    change("src/checkout/shipping.ts", "untracked", 42, 0),
  ],
  "/home/user/projects/api": [
    change("docs/api.md", "modified", 4, 2),
    change("src/routes/orders.ts", "modified", 18, 5),
    change("test/routes/orders.test.ts", "added", 22, 0),
  ],
  "/home/user/projects/api/.claude/worktrees/refactor-auth": [
    change("assets/logo.png", "modified", null, null),
    change("package.json", "modified", 1, 1),
    change("src/auth/token.ts", "renamed", 2, 1, "src/auth/jwt-token.ts"),
    change("src/legacy/jwt.ts", "deleted", 0, 30),
    change("src/middleware/auth.ts", "modified", 21, 17),
  ],
};

/** A stand-in for `hive::changes::list`: the fake worktree's changes, or why not. */
function changes(worktrees: string[], path: string): ServiceMessage {
  if (!worktrees.includes(path)) {
    const error = `${path} is not a worktree of a followed project`;
    return { type: "changes", path, files: [], added: 0, removed: 0, error };
  }
  const files = MOCK_CHANGES[path] ?? [];
  const sum = (key: "added" | "removed") => files.reduce((n, f) => n + (f[key] ?? 0), 0);
  return {
    type: "changes",
    path,
    files,
    added: sum("added"),
    removed: sum("removed"),
    error: null,
  };
}

const SESSION_BASE = [
  'import type { User } from "../types";',
  'import { SESSION_TTL } from "./constants";',
  'import { store } from "./store";',
  "",
  ...Array.from({ length: 31 }, (_, i) => `// session helper ${i + 1}`),
  "",
  "// Creates the session and saves it in the store",
  "export function createSession(user: User, opts: SessionOptions) {",
  "  const ttl = SESSION_TTL;",
  "  return store.set(user.id, { ttl, createdAt: Date.now() });",
  "}",
  "",
].join("\n");

/** Texts of the fake files by path (after the prototype's screen 1g): `[content, base]`. */
export const MOCK_TEXTS: Record<string, [string, string]> = {
  "src/auth/session.ts": [
    SESSION_BASE.replace(
      "  const ttl = SESSION_TTL;",
      [
        "  const ttl = opts.rememberMe",
        "    ? REMEMBER_ME_TTL // 30 days",
        "    : SESSION_TTL;",
      ].join("\n"),
    ).replace("import { SESSION_TTL }", "import { REMEMBER_ME_TTL, SESSION_TTL }"),
    SESSION_BASE,
  ],
};

/**
 * A stand-in for `hive::file`: the fake worktree's file as its status in `MOCK_CHANGES` says
 * (a new file has no base, a deleted one no content, a `.png` is binary), or why not.
 */
function file(
  worktrees: string[],
  worktree: string,
  path: string,
  written: Map<string, string>,
): FileText {
  const answer = { worktree, path, binary: false, too_large: false };
  const none = { content: null, base: null, version: null };
  if (!worktrees.includes(worktree)) {
    const error = `${worktree} is not a worktree of a followed project`;
    return { ...answer, ...none, error };
  }
  const status = MOCK_CHANGES[worktree]?.find((f) => f.path === path)?.status;
  if (path.endsWith(".png")) return { ...answer, ...none, binary: true, error: null };
  const sample = `// ${path}\nexport const value = 1;\n`;
  const [text, original] = MOCK_TEXTS[path] ?? [sample.replace("1", "2"), sample];
  const content =
    written.get(`${worktree}/${path}`) ?? (status === "deleted" ? null : status ? text : original);
  const base = status === "added" || status === "untracked" ? null : original;
  return { ...answer, content, base, version: content && mockVersion(content), error: null };
}

/** A stand-in for `hive::file::version`: the length and a 32-bit hash of the text. */
export const mockVersion = (text: string) =>
  `${text.length}-${[...text].reduce((h, c) => (h * 31 + (c.codePointAt(0) ?? 0)) | 0, 0)}`;

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
 * exits); every later line sets that agent working, and `state <state>` then sets that state;
 * `worktree-remove <name>` removes that Claude worktree as a `WorktreeRemove` hook would.
 * Projects come from `MOCK_REPOS`; any other path is refused as not found. Branches come from
 * `MOCK_BRANCHES`, and new worktrees are added to the fake project. A watched worktree lists
 * `MOCK_FILES`; `touch <name>` in a terminal there adds a file and sends the list (and the
 * changes) again. `write <path> <text>` there replaces that file's text, as an agent would;
 * a save checks the version as the service does and keeps the text.
 * Service messages arrive asynchronously, as they do from the real service.
 * `scenario` ("mismatch" or "disconnected") answers `connect` with that failure instead;
 * "empty" starts with no projects; "update" offers version 9.9.9, whose install fails; "states" adds `MOCK_STATES`' agents and the worktree one of their subagents owns. "load" (1.11) replays a recording into every terminal right
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
  if (scenario === "states") {
    const shop = projects[0] as Project;
    projects[0] = { ...shop, worktrees: [...shop.worktrees, worktree(shop.path, "tests-login")] };
  }
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
  // A stand-in for a `WorktreeRemove` hook: the Claude worktree `name` goes, and the service
  // sends the new list.
  const removeWorktree = (name: string) => {
    projects.forEach((p, i) => {
      projects[i] = { ...p, worktrees: p.worktrees.filter((w) => !w.claude || w.name !== name) };
    });
    later({ type: "projects", projects });
  };
  const holder = (path: string) => projects.find((p) => p.worktrees.some((w) => w.path === path));
  // As `Projects::linked`: only a linked worktree of a followed project.
  const linkedRefusal = (path: string) => {
    if (!holder(path)) return `${path} is not a worktree of a followed project`;
    return worktreeAt(path)?.main ? `${path} is the project's main worktree` : null;
  };
  // A terminal's shell standing in a worktree, as the service finds it in `/proc`.
  const inUse = (path: string) => {
    const ids = [...terminals].filter(([, t]) => `${t.cwd}/`.startsWith(`${path}/`));
    if (ids.length === 0) return null;
    return `in use by ${ids.map(([id]) => `fish (${id})`).join(", ")}: close its terminals first`;
  };
  let sessions = MOCK_SESSIONS;
  // Kept in memory; the real service checks the ranges and saves them.
  let settings: Settings = DEFAULT_SETTINGS;
  // The typed line stands in for the tool call it is doing.
  const setState = (id: string, state: AgentState, activity: string | null = null) =>
    later({ type: "agent_state", id, ...agentStatus(state, activity, Date.now()), subagents: [] });
  // Files by worktree path, and the one watched.
  const files = new Map<string, string[]>();
  let watched: string | null = null;
  const worktreeAt = (path: string) =>
    projects.flatMap((p) => p.worktrees).find((w) => w.path === path);
  // As the service does after every refresh of the watched worktree: its files, then its changes.
  const sendFiles = (path: string) => {
    later({ type: "files", path, files: files.get(path) as string[], truncated: false });
    later(changes([path], path));
  };
  // A stand-in for an agent writing a file: `touch <name>` in a worktree's terminal.
  const touch = (cwd: string, name: string) => {
    const worktree = worktreeAt(cwd);
    if (!worktree) return;
    const listed = files.get(cwd) ?? mockFiles(worktree);
    files.set(cwd, [...new Set([...listed, name])].sort());
    if (watched === cwd) sendFiles(cwd);
  };
  // Texts saved, or written by `write`, by `<worktree>/<path>`.
  const written = new Map<string, string>();
  const fileAt = (worktree: string, path: string) =>
    file(
      projects.flatMap((p) => p.worktrees.map((w) => w.path)),
      worktree,
      path,
      written,
    );
  // A stand-in for an agent editing a file: `write <path> <text>` in a worktree's terminal.
  const write = (cwd: string, args: string) => {
    const [path = "", ...words] = args.split(" ");
    written.set(`${cwd}/${path}`, `${words.join(" ")}\n`);
    if (watched === cwd) sendFiles(cwd);
  };

  return {
    async connect(onMessage) {
      send = onMessage;
      const failure = HANDSHAKE[scenario ?? ""];
      later(failure ?? WELCOME);
      if (failure) return;
      later({ type: "settings", settings });
      later({ type: "projects", projects });
      if (scenario === "states") for (const m of mockStates()) later(m);
    },
    async listProjects() {
      later({ type: "projects", projects });
    },
    async getSettings() {
      later({ type: "settings", settings });
    },
    async setSettings(next) {
      settings = next;
      later({ type: "settings", settings });
    },
    async openSettingsFile() {
      const windows_path = "\\\\wsl.localhost\\Ubuntu\\home\\mock\\.config\\hive\\settings.json";
      later({ type: "editor_target", worktree: "", path: "", windows_path, error: null });
    },
    async getDiagnostics() {
      later({ type: "diagnostics", ...MOCK_DIAGNOSTICS });
    },
    async checkUpdate() {
      if (scenario === "update") later({ type: "update_ready", version: "9.9.9" });
    },
    async installUpdate() {
      later({ type: "update_failed", error: "mock: nothing to install" });
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
    async listDirs(path, windows) {
      later({ type: "dirs", ...mockDirs(path, windows) });
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
    async removeWorktree(path, force) {
      const failed = (message: string) =>
        void later({ type: "remove_worktree_failed", path, message });
      const refused = linkedRefusal(path);
      if (refused) return failed(refused);
      if (!force && inUse(path)) return failed(inUse(path) as string);
      if (!force && MOCK_CHANGES[path]?.length) {
        return failed(
          `git worktree remove ${path} failed: fatal: '${path}' contains modified or untracked files, use --force to delete it`,
        );
      }
      const project = holder(path) as Project;
      const updated = { ...project, worktrees: project.worktrees.filter((w) => w.path !== path) };
      projects[projects.indexOf(project)] = updated;
      later({ type: "worktree_removed", project: updated, path });
    },
    async renameWorktree(path, name) {
      const failed = (message: string) =>
        void later({ type: "rename_worktree_failed", path, name, message });
      const refused = linkedRefusal(path);
      if (refused) return failed(refused);
      if (inUse(path)) return failed(inUse(path) as string);
      const project = holder(path) as Project;
      const taken = nameError(project, name);
      if (taken) return failed(taken);
      const renamed = worktree(project.id, name);
      const updated = {
        ...project,
        worktrees: project.worktrees.map((w) => (w.path === path ? renamed : w)),
      };
      projects[projects.indexOf(project)] = updated;
      later({ type: "worktree_renamed", project: updated, from: path, path: renamed.path });
    },
    async listChanges(path) {
      later(
        changes(
          projects.flatMap((p) => p.worktrees.map((w) => w.path)),
          path,
        ),
      );
    },
    async listSessions() {
      later({ type: "sessions", sessions: [...sessions], error: null });
    },
    async locateSession(id, target) {
      const found = sessions.find((x) => x.id === id);
      const path = found && (target === "log" ? found.log : found.cwd);
      const windows_path = path ? `\\\\wsl.localhost\\Ubuntu${path.replaceAll("/", "\\")}` : null;
      const error = found ? null : `no session ${id} in the followed projects`;
      later({ type: "session_located", id, target, windows_path, error });
    },
    async deleteSession(id) {
      const running = [...terminals.values()].some((t) => t.agent === id);
      if (running) {
        const message = "the session is running: end it first";
        return void later({ type: "delete_session_failed", id, message });
      }
      sessions = sessions.filter((x) => x.id !== id);
      later({ type: "session_deleted", id });
    },
    async searchFiles(worktree, query) {
      const answer = { worktree, query, truncated: false };
      const shown = worktreeAt(worktree);
      if (!shown) {
        const error = `${worktree} is not a worktree of a followed project`;
        return void later({ type: "search_results", ...answer, matches: [], error });
      }
      // As `git grep -i -F`: every line of the fake texts holding the query, in any case.
      const q = query.toLowerCase();
      const matches = (files.get(worktree) ?? mockFiles(shown)).flatMap((path) =>
        (fileAt(worktree, path).content ?? "")
          .split("\n")
          .map((text, i) => ({ path, line: i + 1, text }))
          .filter((m) => m.text.toLowerCase().includes(q)),
      );
      later({ type: "search_results", ...answer, matches, error: null });
    },
    async openFile(worktree, path) {
      later({ type: "file", ...fileAt(worktree, path) });
    },
    async saveFile(worktree, path, content, version) {
      const now = fileAt(worktree, path);
      const failure = (error: SaveError, message: string) =>
        later({ type: "save_failed", worktree, path, error, message });
      if (now.error) return void failure("invalid_path", now.error);
      if (now.version !== version) return void failure("conflict", `${path} changed on disk`);
      written.set(`${worktree}/${path}`, content);
      later({ type: "file_saved", worktree, path, version: mockVersion(content) });
    },
    async openInEditor(worktree, path) {
      // An empty path is the worktree's folder.
      const { error } = path ? fileAt(worktree, path) : { error: null };
      const unc = `\\\\wsl.localhost\\Ubuntu${`${worktree}/${path}`.replaceAll("/", "\\")}`;
      const windows_path = error ? null : unc;
      later({ type: "editor_target", worktree, path, error, windows_path });
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
        if (terminal.agent && line) setState(terminal.agent, "working", line);
        if (line === "claude") detect(id, terminal);
        // `state <state>` then moves it there, as a hook would (the inbox's alerts, 6.5).
        const next = line.slice(6) as AgentState;
        if (terminal.agent && line.startsWith("state ") && URGENCY.includes(next)) {
          setState(terminal.agent, next);
        }
        if (line.startsWith("touch ")) touch(terminal.cwd, line.slice(6));
        if (line.startsWith("write ")) write(terminal.cwd, line.slice(6));
        if (line.startsWith("worktree-remove ")) removeWorktree(line.slice(16));
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
    // Only `MOCK_STATES`' subagents have a conversation, and it never grows.
    async watchTranscript(agent, subagent) {
      const listed = MOCK_STATES[Number(agent.slice("mock-state-".length)) - 1]?.[2];
      if (scenario !== "states" || !listed?.some((s) => s.id === subagent)) {
        return void later({ type: "error", message: "no transcript is known for this subagent" });
      }
      const entries = mockTranscript(subagent);
      later({ type: "transcript", agent, subagent, entries, truncated: false });
    },
    async unwatchTranscript() {},
    // Mock agents never finish, so nothing depends on the view.
    async setView() {},
    async resizeTerminal() {},
    async closeTerminal(id) {
      exit(id, null);
    },
  };
}
