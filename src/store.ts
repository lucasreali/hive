import { create } from "zustand";
import {
  clampWidth,
  ORDER_LIMIT,
  type Side,
  saveAgentOrder,
  savedAgentOrder,
  savedTabOrder,
  savedWidths,
  saveTabOrder,
  saveWidths,
  widthKey,
} from "./persist";
import type {
  Agent,
  AgentState,
  AgentStatus,
  AgentUsage,
  AppMode,
  Branches,
  Changes,
  CreateFailure,
  Diagnostics,
  DiffBase,
  Dirs,
  FileText,
  GhAccounts,
  NameCheck,
  Project,
  ProjectScripts,
  SearchResults,
  Session,
  SessionWindow,
  Settings,
  Space,
  Worktree,
  WorktreeFiles,
} from "./protocol";
import type { OpenPull, PullBusy, PullError, Pulls } from "./pulls";
import { moveNextTo } from "./reorder";
import type { JobLog, OpenRun, RunBusy, RunError, Runs } from "./runs";
import {
  barItems,
  barKey,
  dropFile,
  editFor,
  fileVisible,
  kept,
  opened,
  shownSplit,
  tabWorktree,
  visibleTabs,
  withKey,
} from "./tabs";
import { type EditBuffer, isDirty, isFor } from "./viewer/buffer";

// The one store (#30, #38). UI state is set by components; service data changes only through
// `apply` (reduce.ts).

export type Connection =
  | { status: "connecting" }
  | { status: "connected"; version: string; distro: string | null }
  | {
      status: "version_mismatch";
      protocol: number;
      version: string;
      app_protocol: number;
      app_version: string;
      bundled: boolean;
    }
  | { status: "disconnected"; reason: string; bundled: boolean };

export type Terminal = {
  id: number;
  exited: boolean;
  code: number | null;
  unhooked: boolean;
  /** Set with `hive badge`; cleared when the terminal exits. */
  badge?: string;
  /**
   * The followed worktree the service placed its cwd in (`terminal_opened`), null outside
   * every project or until the service answered: its tab's place (`tabWorktree`).
   */
  worktree: string | null;
};

/** The file shown under the files tree, in the viewer or its diff. */
export type OpenFile = { worktree: string; path: string };
/**
 * An open file's tab (8.21). While another file is the open one, it keeps that file's own
 * editor state: editable text or diff, its edit buffer, and its view (selection and scroll).
 * `rendered`: a Markdown file shows rendered, not as text (11.2).
 */
export type FileTab = OpenFile & {
  editing: boolean;
  edit: EditBuffer | null;
  view: unknown;
  /**
   * The preview tab (11.1), at most one: opening another file replaces it. It is kept (false)
   * once pinned (a double click) or given unsaved edits.
   */
  preview?: boolean;
  rendered?: boolean;
};

/** A session's context menu, at the pointer. */
export type SessionMenu = { session: string; x: number; y: number };
/**
 * A file tree's menu target: new files go in `folder` (relative to the worktree, "" for its
 * root); `path` is the file or folder to rename (a folder: `folder` itself), null for the tree's
 * background.
 */
export type FileTarget = { worktree: string; folder: string; path: string | null };
/** What the file name dialog does: a new file or folder in `folder`, or rename `path`. */
export type FileDialogKind = "file" | "folder" | "rename";
/** The "New file" / "New folder" / "Rename" dialog: its target and the service's refusal. */
export type FileDialog = FileTarget & { kind: FileDialogKind; error: string | null };

/** A 1-based, inclusive range of lines. */
export type Lines = { from: number; to: number };

/** A review comment (6.7) on lines of a worktree's file (new-file numbers). */
export type ReviewComment = Lines & { path: string; text: string };

/** Answers for the new-worktree dialog; reset whenever a dialog opens. */
export type WorktreeDialog = {
  branches: Branches | null;
  /** By name: answers can arrive out of order while the user types. */
  nameChecks: Record<string, NameCheck>;
  created: { project: string; path: string; notes: string[] } | null;
  createFailure: CreateFailure | null;
  /** Why deleting (`name` null) or renaming (to `name`) the worktree `path` failed. */
  failure: { path: string; name: string | null; message: string } | null;
  /** Why deleting each worktree failed, by path (removing merged worktrees sends several). */
  removeFailures: Record<string, string>;
};

/** One alert `notify` raised, kept for the bell's inbox (6.5). `id` grows with each alert. */
export type InboxItem = {
  id: number;
  /** The agent's session id: clicking the item goes to it while it runs. */
  agent: string;
  state: AgentState;
  /** Wall clock, ms since the epoch. */
  at: number;
  /** E.g. "fix login is waiting for permission". */
  text: string;
  /** The agent's space, once the store knows spaces (6.14): the item names it. */
  space?: string;
};
/** At most this many alerts are kept, the newest first. */
export const INBOX_LIMIT = 100;

export const NO_SCRIPTS: ProjectScripts = { setup: null, run: [], archive: null };

/** The scripts of the project `id`, none when it has no settings. */
export const scriptsOf = (settings: Settings, id: string | undefined): ProjectScripts =>
  (id !== undefined && settings.projects[id]?.scripts) || NO_SCRIPTS;

/** The service's defaults, used until its `settings` arrive. */
export const DEFAULT_SETTINGS: Settings = {
  terminal: {
    font_family: '"Hive Mono", "Symbols Nerd Font", monospace',
    font_size: 13,
    scrollback: 5000,
    cursor_style: "block",
    cursor_blink: false,
    copy_on_select: false,
  },
  appearance: { theme: "one-dark" },
  notifications: { volume: 100 },
  agents: { silence_secs: 5, confirm_close: true },
  worktrees: { default_base: null },
  projects: {},
  claude: { accounts: [], account: null },
};

/** A tab of the terminal area: a terminal, and the worktree path it was opened in (its title's source). */
export type Tab = { id: number; cwd: string };
/** Two terminals side by side (6.11), left and right, both of one worktree. */
export type Split = { left: number; right: number };

export type Modal =
  | "new-worktree"
  | "add-project"
  | "worktree-picker"
  | "close-app"
  | "update-app"
  | "remove-worktree"
  | "rename-worktree"
  | "new-space"
  | "edit-space"
  | "settings"
  | "remove-merged"
  | "palette"
  | "file-name"
  | "confirm"
  | "new-pull"
  | null;
/**
 * A message shown as a toast (10.3): an `error` (why something failed, 9.21) stays until
 * dismissed; an `info` confirmation ("Copied …") fades after `INFO_MS` (`Toasts`).
 */
export type Notice = { id: number; kind: "error" | "info"; text: string };

/**
 * A yes/no question asked in a Hive dialog (8.20), never the WebView's `confirm`: `run` happens
 * only when the user picks `action` (e.g. "Discard", "Delete").
 */
export type Question = {
  title: string;
  text: string;
  action: string;
  run: () => void;
  /** The dialog asked from, shown again (still open underneath) once answered. */
  back?: Modal;
  /** The deleted files with unsaved edits it asks about: a later deletion adds to them. */
  deleted?: OpenFile[];
};
/** A worktree row's context menu, at the pointer. */
export type WorktreeMenu = { worktree: string; x: number; y: number };
/** A project row's context menu, at the pointer. */
export type ProjectMenu = { project: string; x: number; y: number };
export type RightPanel = "files" | null;
/** What the right panel shows. */
export type PanelView = "files" | "changes" | "sessions" | "pulls" | "actions";
/** The key of a project's Actions runs on `branch` (all branches: null) in `HiveState.runs`. */
export const runsKey = (project: string, branch: string | null) => `${project}\n${branch ?? ""}`;

export type HiveState = {
  // UI state
  modal: Modal;
  menu: WorktreeMenu | null;
  projectMenu: ProjectMenu | null;
  sessionMenu: SessionMenu | null;
  fileMenu: (FileTarget & { x: number; y: number }) | null;
  fileDialog: FileDialog | null;
  /**
   * Folders created from the tree, by worktree: git lists no empty folder, so the tree shows
   * these too.
   */
  // ponytail: kept for the window's life, even if the folder goes away outside Hive.
  newFolders: Record<string, string[]>;
  /** The entry just renamed or moved: the Files tree makes its row the active one once listed. */
  movedRow: OpenFile | null;
  /** The question of the "confirm" modal (`ask`). */
  question: Question | null;
  /** The toasts shown (10.3), oldest first, at most `NOTICE_LIMIT`. */
  notices: Notice[];
  /** A downloaded release, shown as the title bar's restart button; `installing` once clicked. */
  update: { version: string; installing: boolean } | null;
  rightPanel: RightPanel;
  /** What the right panel shows, for the shown worktree. */
  panelView: PanelView;
  /** The left sidebar's and the right panel's widths, in pixels (see `shell/resize.tsx`). */
  sidebarWidth: number;
  panelWidth: number;
  /** The left pane's share of a split terminal area, in percent. */
  splitPercent: number;
  /** Session ids in the order the user put the agents in (8.2); others follow in arrival order. */
  agentOrder: string[];
  /**
   * The tab bar's order (8.21): tab keys (`barKey`) in the order they opened, as the user moved
   * them; each bar shows its own tabs in this order. Kept between runs, like the widths.
   */
  tabOrder: string[];
  /** Every open file's tab, in the order they opened. */
  openFiles: FileTab[];
  /** The file of the file tabs whose view shows (when `fileShown`) and whose state is live. */
  openFile: OpenFile | null;
  /** The open file's tab is the one shown, in place of the active terminal. */
  fileShown: boolean;
  /** The lines selected in the open file's viewer (new-file numbers, 1-based), or null. */
  selectedLines: Lines | null;
  /** Review comments not sent yet, by worktree path (6.7). */
  comments: Record<string, ReviewComment[]>;
  /** The lines the comment input is open for, in the open file of `worktree`, or null. */
  commenting: (Lines & OpenFile) | null;
  selection: string | null;
  /**
   * By space id: the place selected when the user left that space and its project (11.5), to
   * select again on coming back; this run only. Null: nothing was selected.
   */
  spacePlaces: Record<string, SpacePlace | null>;
  /**
   * Collapsed tree nodes: a project by its id, a worktree by `worktree:<id>` (a main worktree
   * has its project's id), a folder of the Files or Diff tree by `files:` or `changes:<worktree>/<path>`
   * (folders start collapsed: one is open only when its entry is false).
   */
  collapsed: Record<string, boolean>;
  /** Terminal tabs in the order they opened, and the one shown. */
  tabs: Tab[];
  activeTab: number | null;
  /**
   * The split terminals (6.11), shown while the active tab (the focused pane) is one of them;
   * any other tab shows alone.
   */
  split: Split | null;
  /** The alerts raised, the newest first (at most `INBOX_LIMIT`), and the newest id seen. */
  inbox: InboxItem[];
  inboxSeen: number;
  /**
   * The pending agents seen when the bell was last opened, with the state they were pending in
   * (8.5): the bell's badge counts the others. An agent leaves it when it stops being pending in
   * that state, so a new alert counts again.
   */
  pendingSeen: Record<string, AgentState>;
  /** Whether the app window has the focus (`watchFocus` in `src/window.ts`). */
  focused: boolean;
  // Service data
  connection: Connection;
  /**
   * Where the service runs, WSL or Windows (12.5.4; `mode` null until chosen), and whether WSL
   * is there to choose; null where the app offers no choice (macOS, or not offered yet).
   */
  appMode: { mode: AppMode | null; wsl: boolean } | null;
  /** The current account's 5-hour window, as the service decides it (12.1). */
  sessionUsage: SessionWindow | null;
  /** The service's settings (the defaults until they arrive). */
  settings: Settings;
  /** Why the last `set_settings` was refused, or the settings file was ignored. */
  settingsError: string | null;
  /** A `set_settings` sent and not answered yet: the next save waits for its answer (9.24). */
  settingsPending: boolean;
  /** The last `diagnostics`, or null until asked. */
  diagnostics: Diagnostics | null;
  /** In the service's order; `null` until the service sent the list. */
  projects: Record<string, Project> | null;
  /** Why the last add-project request was refused. */
  addProjectError: string | null;
  /** Every space and the current one's id (whose projects show); null until the service sent them. */
  spaces: Space[] | null;
  currentSpace: string | null;
  /** Why the last space request was refused. */
  spaceError: string | null;
  /** The last `gh_accounts` answer (9.30), for the space dialog; null until asked. */
  ghAccounts: GhAccounts | null;
  /** The project a dialog opened for (e.g. the row's "New worktree"). */
  modalProject: string | null;
  /** The worktree a dialog opened for (its row's menu). */
  modalWorktree: string | null;
  worktreeDialog: WorktreeDialog;
  terminals: Record<number, Terminal>;
  agents: Record<string, Agent>;
  /** By session id; kept apart from `agents` so either message may arrive first. */
  agentStates: Record<string, AgentStatus>;
  /** A running agent's session name (the user's, else Claude's), by session id. */
  agentTitles: Record<string, string>;
  /** A running agent's tokens from its transcript, by session id. */
  agentUsage: Record<string, AgentUsage>;
  /**
   * The worktrees a live subagent works in as its own and no agent runs in (the service's):
   * they show under their subagent only, not in their project's list.
   */
  subagentWorktrees: string[];
  /** The files of the worktree the files panel shows (see `panelWorktree`); check `path`. */
  worktreeFiles: WorktreeFiles | null;
  /** By worktree path: the last `changes` the service sent for it. */
  changes: Record<string, Changes>;
  /** By worktree path: the base picked in the Changes panel (UI state, this run only). */
  diffBases: Record<string, DiffBase>;
  /** The last `file` the service sent; shown only while it is the open file. */
  file: FileText | null;
  /** The open file shows as editable text (UI state), not as its read-only diff. */
  editing: boolean;
  /** The open file's edit buffer while editing (UI state, kept when the panel closes). */
  edit: EditBuffer | null;
  /** Why "Open in external editor" did not open the file, shown under its header. */
  editorNotice: string | null;
  /** The last contents search the service answered. */
  searchResults: SearchResults | null;
  /** The last folder listing for "Add project". */
  dirs: Dirs | null;
  /** Claude sessions of the followed projects, the most recent first; null until listed. */
  sessions: Session[] | null;
  /** Why the service could not list them. */
  sessionsError: string | null;
  /** Older sessions were left out of the list. */
  sessionsTruncated: boolean;
  /** A line to show once the open file's text is there (a search result), then cleared. */
  gotoLine: (OpenFile & { line: number }) | null;
  /** The last `pulls` of each project (9.31). */
  pulls: Record<string, Pulls>;
  /** The pull request whose details the Pull requests view shows, or null for the list. */
  openPull: OpenPull | null;
  pullBusy: PullBusy | null;
  pullError: PullError | null;
  /** The last `runs` of each project and branch filter, by `runsKey` (9.32). */
  runs: Record<string, Runs>;
  /** The run whose jobs the Actions view shows, or null for the list. */
  openRun: OpenRun | null;
  jobLog: JobLog | null;
  runBusy: RunBusy | null;
  runError: RunError | null;
};

export const initialState: HiveState = {
  modal: null,
  menu: null,
  projectMenu: null,
  sessionMenu: null,
  fileMenu: null,
  fileDialog: null,
  newFolders: {},
  movedRow: null,
  question: null,
  notices: [],
  update: null,
  rightPanel: "files",
  panelView: "files",
  sidebarWidth: 264,
  panelWidth: 380,
  splitPercent: 50,
  agentOrder: [],
  tabOrder: [],
  openFiles: [],
  openFile: null,
  fileShown: false,
  selectedLines: null,
  comments: {},
  commenting: null,
  selection: null,
  spacePlaces: {},
  collapsed: {},
  tabs: [],
  activeTab: null,
  split: null,
  inbox: [],
  inboxSeen: 0,
  pendingSeen: {},
  focused: false,
  connection: { status: "connecting" },
  appMode: null,
  sessionUsage: null,
  settings: DEFAULT_SETTINGS,
  settingsError: null,
  settingsPending: false,
  diagnostics: null,
  projects: null,
  addProjectError: null,
  spaces: null,
  currentSpace: null,
  spaceError: null,
  ghAccounts: null,
  modalProject: null,
  modalWorktree: null,
  worktreeDialog: {
    branches: null,
    nameChecks: {},
    created: null,
    createFailure: null,
    failure: null,
    removeFailures: {},
  },
  terminals: {},
  agents: {},
  agentStates: {},
  agentTitles: {},
  agentUsage: {},
  subagentWorktrees: [],
  worktreeFiles: null,
  changes: {},
  diffBases: {},
  file: null,
  editing: false,
  edit: null,
  editorNotice: null,
  searchResults: null,
  dirs: null,
  sessions: null,
  sessionsError: null,
  sessionsTruncated: false,
  gotoLine: null,
  pulls: {},
  openPull: null,
  pullBusy: null,
  pullError: null,
  runs: {},
  openRun: null,
  jobLog: null,
  runBusy: null,
  runError: null,
};

/** `agents` in the user's order (8.2): ordered ones first, the rest as they came (a stable sort). */
export function inAgentOrder(agents: Agent[], order: string[]): Agent[] {
  const rank = (a: Agent) => {
    const i = order.indexOf(a.id);
    return i < 0 ? order.length : i;
  };
  return [...agents].sort((a, b) => rank(a) - rank(b));
}

/**
 * Moves agent `id` next to agent `target` (after it when `after`), and remembers the order. Only
 * within one worktree: the service places an agent by its cwd, so another worktree refuses it.
 */
export function moveAgent(id: string, target: string, after: boolean): void {
  const s = useHive.getState();
  const agent = s.agents[id];
  if (!agent || id === target || s.agents[target]?.worktree !== agent.worktree) return;
  const siblings = inAgentOrder(
    Object.values(s.agents).filter((a) => a.worktree === agent.worktree),
    s.agentOrder,
  ).map((a) => a.id);
  const moved = moveNextTo(siblings, id, target, after);
  const agentOrder = [...moved, ...s.agentOrder.filter((o) => !moved.includes(o))].slice(
    0,
    ORDER_LIMIT,
  );
  useHive.setState({ agentOrder });
}

/** Moves agent `id` one place up (`-1`) or down (`1`) among its worktree's agents (Alt+↑/↓). */
export function stepAgent(id: string, step: -1 | 1): void {
  const s = useHive.getState();
  const agent = s.agents[id];
  if (!agent) return;
  const siblings = inAgentOrder(
    Object.values(s.agents).filter((a) => a.worktree === agent.worktree),
    s.agentOrder,
  );
  const target = siblings[siblings.findIndex((a) => a.id === id) + step];
  if (target) moveAgent(id, target.id, step > 0);
}

export const useHive = create<HiveState>()(() => ({
  ...initialState,
  ...savedWidths(),
  agentOrder: savedAgentOrder(),
  tabOrder: savedTabOrder(),
}));
useHive.subscribe(saveTabOrder);
useHive.subscribe(saveAgentOrder);

/**
 * Sets a side's width (kept within its limits) and, unless `persist` is false (a drag in
 * progress, saved once on release by `saveWidths`), remembers them all.
 */
export function setWidth(side: Side, width: number, persist = true): void {
  useHive.setState({ [widthKey(side)]: clampWidth(side, width) });
  if (persist) saveWidths(useHive.getState());
}

/** The project that is `id` or holds the worktree `id`. */
export const owner = (projects: HiveState["projects"], id: string | null): Project | undefined =>
  Object.values(projects ?? {}).find((p) => p.id === id || p.worktrees.some((w) => w.id === id));

/** The worktree `id` of any project. */
export const findWorktree = (projects: HiveState["projects"], id: string | null) =>
  owner(projects, id)?.worktrees.find((w) => w.id === id);

/** Whether `path` is `folder` or inside it. */
export const within = (path: string, folder: string) =>
  path === folder || path.startsWith(`${folder}/`);

export const openModal = (
  modal: Modal,
  modalProject: string | null = null,
  modalWorktree: string | null = null,
) =>
  useHive.setState({
    modal,
    modalProject,
    modalWorktree,
    addProjectError: null,
    spaceError: null,
    worktreeDialog: initialState.worktreeDialog,
  });
export const openMenu = (menu: WorktreeMenu | null) => useHive.setState({ menu });
export const openProjectMenu = (projectMenu: ProjectMenu | null) =>
  useHive.setState({ projectMenu });
export const openSessionMenu = (sessionMenu: SessionMenu | null) =>
  useHive.setState({ sessionMenu });
export const openFileMenu = (fileMenu: HiveState["fileMenu"]) => useHive.setState({ fileMenu });
/** The "New file" or "New folder" dialog for `target`, or "Rename file" for its `path`. */
export const openFileDialog = (target: FileTarget, kind: FileDialogKind = "file") =>
  useHive.setState({
    modal: "file-name",
    fileDialog: { ...target, kind, error: null },
  });
/** Asks `question` in the confirm dialog (`ConfirmDialog`). */
export const ask = (question: Question) => useHive.setState({ modal: "confirm", question });
/** Keeps an alert in the inbox, the newest first. */
export const addToInbox = (item: Omit<InboxItem, "id">) =>
  useHive.setState((s) => ({
    inbox: [{ ...item, id: (s.inbox[0]?.id ?? 0) + 1 }, ...s.inbox].slice(0, INBOX_LIMIT),
  }));
/** Opening the inbox marks every alert read and the pending agents seen (8.5). */
export const markInboxRead = () =>
  useHive.setState((s) => ({
    inboxSeen: s.inbox[0]?.id ?? 0,
    pendingSeen: Object.fromEntries(
      pendingAgents(s).map((a) => [a.id, s.agentStates[a.id]?.state as AgentState]),
    ),
  }));
/** At most this many toasts show; a new one drops the oldest (10.3). */
export const NOTICE_LIMIT = 3;
let lastNotice = 0;
/** `s`'s toasts with `text` added as the newest (for `reduce`). */
export const withNotice = (s: Pick<HiveState, "notices">, kind: Notice["kind"], text: string) => ({
  notices: [...s.notices, { id: ++lastNotice, kind, text }].slice(-NOTICE_LIMIT),
});
/** Shows `text` as a toast: an error stays until dismissed, a confirmation ("info") fades. */
export const showNotice = (kind: Notice["kind"], text: string) =>
  useHive.setState((s) => withNotice(s, kind, text));
export const dismissNotice = (id: number) =>
  useHive.setState((s) => ({ notices: s.notices.filter((n) => n.id !== id) }));
/**
 * Shows why `action` failed as an error toast (9.21), after `what` ("Cannot open a terminal"),
 * so a failed request is never swallowed. Returns `action` with the failure handled.
 */
export const showFailure = <T>(action: Promise<T>, what = ""): Promise<T> => {
  action.catch((error: unknown) => showNotice("error", what ? `${what}: ${error}` : String(error)));
  return action;
};
export const clearAddProjectError = () => useHive.setState({ addProjectError: null });
export const setRightPanel = (rightPanel: RightPanel) => useHive.setState({ rightPanel });
export const setPanelView = (panelView: PanelView) => useHive.setState({ panelView });
/**
 * Opens a file in its own tab (8.21), the preview tab (11.1), and shows it, as editable text
 * when `editing`; a file already open shows its tab as it was left. The file shown before keeps
 * its state in its tab. Null closes the open file's tab, without asking (`closeFile` asks).
 */
export const setOpenFile = (openFile: OpenFile | null, editing = false, line?: number) =>
  useHive.setState((s) =>
    openFile ? opened(s, openFile, editing, line) : s.openFile ? dropFile(s, s.openFile) : {},
  );

/** Keeps file `f`'s view (its selection and scroll) in its tab, for when it shows again. */
export const saveFileView = (f: OpenFile, view: unknown) =>
  useHive.setState((s) => ({
    openFiles: s.openFiles.map((t) => (isFor(t, f) ? { ...t, view } : t)),
  }));

/** A Markdown file (11.2): by its name, as the eye button and Ctrl+Shift+V offer it. */
export const isMarkdown = (path: string) => path.toLowerCase().endsWith(".md");

/** The open file shows rendered (11.2). */
export const renderedShown = (s: HiveState) =>
  !!s.openFile && !!s.openFiles.find((t) => s.openFile && isFor(t, s.openFile))?.rendered;

/** Shows the open file rendered or as text again (11.2), in its tab. */
export const setRendered = (rendered: boolean) =>
  useHive.setState((s) => ({
    openFiles: s.openFiles.map((t) =>
      s.openFile && isFor(t, s.openFile) ? { ...t, rendered } : t,
    ),
  }));

/** Ctrl+Shift+V (11.2): only while the shown tab is a Markdown file's. */
export const markdownShown = (s: HiveState) =>
  s.fileShown && !!s.openFile && isMarkdown(s.openFile.path);

/** The line asked for was shown. */
export const clearGotoLine = () => useHive.setState({ gotoLine: null });
/** Shows the open file as editable text (its buffer starts from the last answer) or not. */
export const setEditing = (editing: boolean) =>
  useHive.setState((s) => ({ editing, edit: editing ? editFor({ ...s, editing }, s.file) : null }));
/** Sets the open file's edit buffer; with unsaved edits, its tab is kept (11.1). */
export const setEdit = (edit: EditBuffer | null) =>
  useHive.setState((s) => ({
    edit,
    openFiles: edit && isDirty(edit) ? kept(s.openFiles, edit) : s.openFiles,
  }));
/** Keeps file `f`'s tab: it stops being the preview tab (11.1, a double click). */
export const pinFile = (f: OpenFile) =>
  useHive.setState((s) => ({ openFiles: kept(s.openFiles, f) }));
export const setEditorNotice = (editorNotice: string | null) => useHive.setState({ editorNotice });
export const setSelectedLines = (selectedLines: Lines | null) =>
  useHive.setState({ selectedLines });
/**
 * Selects a project, worktree or agent. The tab bar then shows that place's tabs: the shown
 * terminal stays when it is one of them, else the last of them is shown (none when it has
 * none), and the open file stays shown only when it belongs there.
 */
export const select = (selection: string | null) => useHive.setState((s) => selected(s, selection));

/** `select`'s change of state. */
export function selected(s: HiveState, selection: string | null): Partial<HiveState> {
  const next = { ...s, selection };
  const tabs = visibleTabs(next);
  const keep = tabs.some((t) => t.id === s.activeTab);
  const shown = {
    selection,
    activeTab: keep ? s.activeTab : (tabs.at(-1)?.id ?? null),
    fileShown: s.fileShown && fileVisible(next),
  };
  // A place with files but no terminal shows its last file.
  const file = barItems(next)
    .filter((t) => "path" in t)
    .at(-1) as FileTab | undefined;
  return !shown.fileShown && tabs.length === 0 && file
    ? { ...shown, ...opened(next, file, false) }
    : shown;
}
export const setFocused = (focused: boolean) => useHive.setState({ focused });
export const toggleCollapsed = (id: string) =>
  useHive.setState((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } }));

/** A terminal just opened in `cwd`: its tab is shown and its worktree selected. */
export const addTab = (id: number, cwd: string) =>
  useHive.setState((s) => {
    const tab = { id, cwd };
    return {
      tabs: [...s.tabs, tab],
      tabOrder: withKey(s.tabOrder, `tab:${id}`),
      activeTab: id,
      fileShown: false,
      selection: tabWorktree(s, tab),
    };
  });
export const activateTab = (tab: Tab) =>
  useHive.setState((s) => ({
    activeTab: tab.id,
    fileShown: false,
    selection: tabWorktree(s, tab),
  }));
/**
 * Removes the tab; when it was shown, the other pane of its split is, else its right neighbour
 * in the bar (the left one when it was last), which may be a file. Closing either pane ends the
 * split.
 */
export const removeTab = (id: number) =>
  useHive.setState((s) => {
    const shown = visibleTabs(s);
    const i = shown.findIndex((t) => t.id === id);
    const rest = shown.filter((t) => t.id !== id);
    const split = s.split && [s.split.left, s.split.right].includes(id) ? s.split : null;
    const other = split && (split.left === id ? split.right : split.left);
    const next = other ?? rest[Math.min(i, rest.length - 1)]?.id ?? null;
    const items = barItems(s);
    const at = items.findIndex((t) => !("path" in t) && t.id === id);
    const beside = items.filter((_, j) => j !== at)[Math.min(at, items.length - 2)];
    const gone = [barKey(s, { id, cwd: "" }), `tab:${id}`];
    const removed = {
      tabs: s.tabs.filter((t) => t.id !== id),
      tabOrder: s.tabOrder.filter((k) => !gone.includes(k)),
      activeTab: s.activeTab === id ? next : s.activeTab,
      split: split ? null : s.split,
    };
    return s.activeTab === id && !other && !s.fileShown && beside && "path" in beside
      ? { ...removed, ...opened({ ...s, ...removed }, beside, false) }
      : removed;
  });

/** Shows `left` and `right` side by side, `right` focused; null ends the split. */
export const setSplit = (split: Split | null) =>
  useHive.setState((s) => ({
    split,
    activeTab: split ? split.right : s.activeTab,
    fileShown: split ? false : s.fileShown,
  }));

/** A click in a shown pane focuses it: it becomes the active tab, the one "in view". */
export const focusPane = (id: number) =>
  useHive.setState((s) => {
    const split = shownSplit(s);
    return split && (split.left === id || split.right === id) ? { activeTab: id } : {};
  });

/**
 * Moves the bar's tab `id` next to its tab `target` (after it when `after`), both `barKey`s:
 * the bar's keys take the places they held in `tabOrder`, in the new order.
 */
export function moveTab(id: string, target: string, after: boolean): void {
  useHive.setState((s) => {
    const keys = barItems(s).map((item) => barKey(s, item));
    const moved = moveNextTo(keys, id, target, after);
    if (moved === keys) return {};
    const order = keys.reduce(withKey, s.tabOrder);
    const slots = order.flatMap((k, i) => (keys.includes(k) ? [i] : []));
    const tabOrder = [...order];
    slots.forEach((slot, i) => {
      tabOrder[slot] = moved[i] as string;
    });
    return { tabOrder };
  });
}

/**
 * The selected project or worktree id (a path). A selected agent (F8) stands for the worktree
 * the service placed it in, null when none.
 */
export function selectedPlace(s: HiveState): string | null {
  const agent = s.agents[s.selection ?? ""];
  return agent ? agent.worktree : s.selection;
}

/**
 * The worktree the files panel shows: the selected worktree (a selected project is its main
 * worktree, which shares its id) or the selected agent's; else the shown terminal's.
 */
export function panelWorktree(s: HiveState): { project: Project; worktree: Worktree } | null {
  const all = Object.values(s.projects ?? {}).flatMap((project) =>
    project.worktrees.map((worktree) => ({ project, worktree })),
  );
  const find = (id: string | null | undefined) => all.find((e) => e.worktree.id === id);
  const tab = s.tabs.find((t) => t.id === s.activeTab);
  return find(selectedPlace(s)) ?? find(tab && tabWorktree(s, tab)) ?? null;
}

/**
 * The base the worktree at `path` is compared with: the one picked, else the merge-base with the
 * main branch in a Claude worktree and HEAD anywhere else (9.11).
 */
export function diffBase(s: HiveState, path: string): DiffBase {
  const worktrees = Object.values(s.projects ?? {}).flatMap((p) => p.worktrees);
  const claude = worktrees.find((w) => w.path === path)?.claude;
  return s.diffBases[path] ?? (claude ? "branch" : "head");
}

/** Picks the base the worktree at `path` is compared with. */
export function setDiffBase(path: string, base: DiffBase): void {
  useHive.setState((s) => ({ diffBases: { ...s.diffBases, [path]: base } }));
}

/** The space holding the project `id`. */
export const spaceOf = (s: HiveState, project: string | null): Space | undefined =>
  s.spaces?.find((space) => space.projects.includes(project ?? ""));

/** The current space. */
export const currentSpace = (s: HiveState): Space | undefined =>
  s.spaces?.find((space) => space.id === s.currentSpace);

/** A space's remembered place (11.5): a project, worktree or agent id, and its project. */
export type SpacePlace = { place: string; project: string | null };

/** The project of place `id`: a project, a worktree or an agent. */
const placeProject = (s: HiveState, id: string | null): string | null =>
  s.agents[id ?? ""]?.project ?? owner(s.projects, id)?.id ?? null;

/** Whether place `id` is in the current space. */
export const inCurrentSpace = (s: HiveState, id: string | null): boolean =>
  !!currentSpace(s)?.projects.includes(placeProject(s, id) ?? "");

/** Remembers the selection as the current space's place, as the user leaves that space. */
export function leaveSpace(s: HiveState): Partial<HiveState> {
  if (s.currentSpace === null) return {};
  const place =
    s.selection === null ? null : { place: s.selection, project: placeProject(s, s.selection) };
  return { spacePlaces: { ...s.spacePlaces, [s.currentSpace]: place } };
}

/**
 * The place to select in the current space: the one remembered, else (gone) its project, else
 * (gone or never selected) the space's first project; null in an empty space.
 */
export function spacePlace(s: HiveState): string | null {
  const shown = spaceProjects(s).map((p) => p.id);
  const kept = s.spacePlaces[s.currentSpace ?? ""];
  if (kept?.project && shown.includes(kept.project)) {
    return placeProject(s, kept.place) === kept.project ? kept.place : kept.project;
  }
  return shown[0] ?? null;
}

/** The projects the sidebar shows: the current space's (every one until the spaces arrive). */
export function spaceProjects(s: HiveState): Project[] {
  const all = Object.values(s.projects ?? {});
  const space = currentSpace(s);
  return space ? all.filter((p) => space.projects.includes(p.id)) : all;
}

/**
 * Agents in the sidebar's order (project, worktree, then the user's order within it, 8.2);
 * those outside the tree last.
 */
export function treeAgents(s: HiveState): Agent[] {
  const worktrees = Object.values(s.projects ?? {}).flatMap((p) => p.worktrees.map((w) => w.id));
  const place = (a: Agent) => {
    const i = worktrees.indexOf(a.worktree ?? "");
    return i < 0 ? worktrees.length : i;
  };
  return inAgentOrder(Object.values(s.agents), s.agentOrder).sort((a, b) => place(a) - place(b));
}

/** Agents that need the user, in tree order: the "N pending" counter and F8's cycle. */
export const pendingAgents = (s: HiveState): Agent[] =>
  treeAgents(s).filter((a) => s.agentStates[a.id]?.pending);
/** Pending agents not seen since the bell was last opened: the bell's badge (8.5). */
export const unseenPending = (s: HiveState): number =>
  pendingAgents(s).filter((a) => s.pendingSeen[a.id] !== s.agentStates[a.id]?.state).length;

/** The state of highest `urgency` among `agents` (rule 1, for a collapsed node), or null. */
export function mostUrgent(s: HiveState, agents: Agent[]): AgentState | null {
  let top: AgentStatus | undefined;
  for (const a of agents) {
    const status = s.agentStates[a.id];
    if (status && (!top || status.urgency > top.urgency)) top = status;
  }
  return top?.state ?? null;
}

export const useTerminal = (id: number) => useHive((s) => s.terminals[id]);

/**
 * An agent placed in `worktree`, or a subagent in its own worktree there, may be writing (the
 * service's `writing`): the file view's "Agent working here".
 */
export const agentWorkingIn = (s: HiveState, worktree: string): boolean =>
  Object.values(s.agents).some((a) => {
    const status = s.agentStates[a.id];
    const subagents = status?.subagents ?? [];
    return (
      (a.worktree === worktree && !!status?.writing) ||
      subagents.some((sub) => sub.worktree === worktree && sub.writing)
    );
  });
