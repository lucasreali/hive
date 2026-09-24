import { create } from "zustand";
import { type EditBuffer, failed, fromDisk, isFor, saved, startEdit } from "./viewer/buffer";

// The one store (#30, #38). UI state is set by components; service data changes
// only through `apply`, which stores what the service sent without deriving anything (#37).

/** Service → app messages the store understands. Mirrors `hive_protocol::Control`. */
export type ServiceMessage =
  | { type: "welcome"; version: string; distro: string | null }
  // `protocol`/`version` are the service's; `app_*` are added by the app side (Rust).
  | {
      type: "version_mismatch";
      protocol: number;
      version: string;
      app_protocol: number;
      app_version: string;
    }
  | { type: "terminal_opened"; channel: number }
  | { type: "terminal_exited"; channel: number; code: number | null }
  | { type: "unhooked_agent"; channel: number }
  | ({ type: "agent_detected"; channel: number } & Omit<Agent, "terminal">)
  | { type: "agent_removed"; channel: number; id: string }
  | ({ type: "agent_state"; id: string } & AgentStatus)
  | { type: "projects"; projects: Project[] }
  | { type: "project_added"; project: Project }
  | { type: "add_project_failed"; path: string; error: ProjectError; message: string }
  | ({ type: "branches" } & Branches)
  | ({ type: "worktree_name_validated" } & NameCheck)
  | { type: "worktree_created"; project: Project; path: string; notes: string[] }
  | ({ type: "create_worktree_failed" } & CreateFailure)
  | ({ type: "files" } & WorktreeFiles)
  // A refused request, e.g. watching a worktree that is not followed. Not stored.
  | { type: "error"; message: string }
  | ({ type: "changes" } & Changes)
  | ({ type: "file" } & FileText)
  | { type: "file_saved"; worktree: string; path: string; version: string }
  | { type: "save_failed"; worktree: string; path: string; error: SaveError; message: string }
  // Handled by `openExternal` (src/viewer/external.ts), not stored.
  | {
      type: "editor_target";
      worktree: string;
      path: string;
      windows_path: string | null;
      error: string | null;
    }
  // Sent by the app side (Rust) when the bridge exits or its output closes.
  | { type: "disconnected"; reason: string };

export type Connection =
  | { status: "connecting" }
  | { status: "connected"; version: string; distro: string | null }
  | {
      status: "version_mismatch";
      protocol: number;
      version: string;
      app_protocol: number;
      app_version: string;
    }
  | { status: "disconnected"; reason: string };

export type Terminal = {
  id: number;
  exited: boolean;
  code: number | null;
  unhooked: boolean;
};

/** Mirrors `hive_protocol::Worktree`: every field comes from the service. */
export type Worktree = {
  id: string;
  name: string;
  path: string;
  branch: string | null;
  main: boolean;
  claude: boolean;
};

/** Mirrors `hive_protocol::Project`; `error` says why its worktrees could not be listed. */
export type Project = {
  id: string;
  name: string;
  path: string;
  worktrees: Worktree[];
  error: string | null;
};

export type ProjectError =
  | "empty_path"
  | "not_absolute"
  | "not_found"
  | "not_a_directory"
  | "not_a_git_repository"
  | "storage";

/** A project's branches; `current` is checked out in its main worktree (the "default"). */
export type Branches = {
  project: string;
  local: string[];
  remote: string[];
  current: string | null;
  error: string | null;
};

/** The service's verdict on a new worktree name, with the folder and branch it would get. */
export type NameCheck = {
  project: string;
  name: string;
  folder: string;
  branch: string;
  error: string | null;
};

export type CreateFailure = { project: string; name: string; message: string };

/**
 * Every file git lists in the watched worktree `path`: sorted `/`-separated relative paths,
 * replaced as a whole on each `files`. `truncated` when the service's cap cut the list.
 */
export type WorktreeFiles = { path: string; files: string[]; truncated: boolean };

/** Mirrors `hive_protocol::FileStatus`: against HEAD, as `git status` shows it. */
export type FileStatus = "added" | "modified" | "deleted" | "renamed" | "untracked";

/** Mirrors `hive_protocol::ChangedFile`; line counts are null for a binary file. */
export type ChangedFile = {
  path: string;
  status: FileStatus;
  old_path: string | null;
  added: number | null;
  removed: number | null;
};

/** A worktree's changes against HEAD, sorted by path, with the service's totals. */
export type Changes = {
  path: string;
  files: ChangedFile[];
  added: number;
  removed: number;
  error: string | null;
};

/** The file shown under the files tree, in the viewer or its diff. */
export type OpenFile = { worktree: string; path: string };

/** Mirrors `hive_protocol::SaveError`. */
export type SaveError = "conflict" | "too_large" | "invalid_path" | "io";

/** A 1-based, inclusive range of lines. */
export type Lines = { from: number; to: number };

/**
 * Mirrors `Control::File`: a file's text on disk (`content`, null when gone) and at HEAD
 * (`base`, null when new), both null when `binary` or `too_large`; `version` is opaque.
 */
export type FileText = {
  worktree: string;
  path: string;
  content: string | null;
  base: string | null;
  version: string | null;
  binary: boolean;
  too_large: boolean;
  error: string | null;
};

/** Answers for the new-worktree dialog; reset whenever a dialog opens. */
export type WorktreeDialog = {
  branches: Branches | null;
  /** By name: answers can arrive out of order while the user types. */
  nameChecks: Record<string, NameCheck>;
  created: { project: string; path: string; notes: string[] } | null;
  createFailure: CreateFailure | null;
};

/**
 * A detected agent (Stage 1: presence only). `id` is its session id and `terminal` its tab.
 * `project`/`worktree` are ids placed by the service from the agent's own cwd (#19); null
 * outside every followed project.
 */
export type Agent = {
  id: string;
  terminal: number;
  project: string | null;
  worktree: string | null;
  cwd: string | null;
};

/** An agent's displayed state, already resolved by the service ("the most urgent wins"). */
export type AgentState =
  | "idle"
  | "working"
  | "waiting_permission"
  | "waiting_you"
  | "error"
  | "with_subagents"
  | "ended";

/**
 * A live subagent (`id` = its `agent_id`) with its own state, and the id of its own worktree
 * (#22) when it has one.
 */
export type Subagent = {
  id: string;
  agent_type: string | null;
  state: AgentState;
  worktree: string | null;
};

/**
 * What `agent_state` says about an agent, stored by its session id. `urgency` (higher wins)
 * and `pending` (needs the user) are the service's, so the app keeps no table of its own.
 */
export type AgentStatus = {
  state: AgentState;
  urgency: number;
  pending: boolean;
  subagents: Subagent[];
};

/** A terminal tab: the terminal and the worktree path it was opened in (its title's source). */
export type Tab = { id: number; cwd: string };

export type Modal = "new-worktree" | "add-project" | "worktree-picker" | "close-app" | null;
export type RightPanel = "files" | null;

export type HiveState = {
  // UI state
  modal: Modal;
  rightPanel: RightPanel;
  /** The files panel shows only changed files ("Changed") instead of every file ("All"). */
  changedOnly: boolean;
  openFile: OpenFile | null;
  /** The lines selected in the open file's viewer (new-file numbers, 1-based), or null. */
  selectedLines: Lines | null;
  selection: string | null;
  /**
   * Collapsed tree nodes: a project by its id, a worktree by `worktree:<id>` (a main worktree
   * has its project's id), a folder of the files panel by `folder:<worktree>/<path>`.
   */
  collapsed: Record<string, boolean>;
  /** Terminal tabs in the order they opened, and the one shown. */
  tabs: Tab[];
  activeTab: number | null;
  /** Lines of history each new terminal keeps (#28). Not persisted yet. */
  scrollback: number;
  // Service data
  connection: Connection;
  /** In the service's order; `null` until the service sent the list. */
  projects: Record<string, Project> | null;
  /** Why the last add-project request was refused. */
  addProjectError: string | null;
  /** The project a dialog opened for (e.g. the row's "New worktree"). */
  modalProject: string | null;
  worktreeDialog: WorktreeDialog;
  terminals: Record<number, Terminal>;
  agents: Record<string, Agent>;
  /** By session id; kept apart from `agents` so either message may arrive first. */
  agentStates: Record<string, AgentStatus>;
  /** The files of the worktree the files panel shows (see `panelWorktree`); check `path`. */
  worktreeFiles: WorktreeFiles | null;
  /** By worktree path: the last `changes` the service sent for it. */
  changes: Record<string, Changes>;
  /** The last `file` the service sent; shown only while it is the open file. */
  file: FileText | null;
  /** The open file shows as editable text (UI state), not as its read-only diff. */
  editing: boolean;
  /** The open file's edit buffer while editing (UI state, kept when the panel closes). */
  edit: EditBuffer | null;
  /** Why "Open in external editor" did not open the file, shown under its header. */
  editorNotice: string | null;
};

export const initialState: HiveState = {
  modal: null,
  rightPanel: null,
  changedOnly: false,
  openFile: null,
  selectedLines: null,
  selection: null,
  collapsed: {},
  tabs: [],
  activeTab: null,
  scrollback: 5000,
  connection: { status: "connecting" },
  projects: null,
  addProjectError: null,
  modalProject: null,
  worktreeDialog: { branches: null, nameChecks: {}, created: null, createFailure: null },
  terminals: {},
  agents: {},
  agentStates: {},
  worktreeFiles: null,
  changes: {},
  file: null,
  editing: false,
  edit: null,
  editorNotice: null,
};

export const useHive = create<HiveState>()(() => initialState);

function patchTerminal(s: HiveState, id: number, patch: Partial<Terminal>): Partial<HiveState> {
  const current = s.terminals[id] ?? { id, exited: false, code: null, unhooked: false };
  return { terminals: { ...s.terminals, [id]: { ...current, ...patch } } };
}

function patchDialog(s: HiveState, patch: Partial<WorktreeDialog>): Partial<HiveState> {
  return { worktreeDialog: { ...s.worktreeDialog, ...patch } };
}

/** The project that is `id` or holds the worktree `id`. */
export const owner = (projects: HiveState["projects"], id: string | null): Project | undefined =>
  Object.values(projects ?? {}).find((p) => p.id === id || p.worktrees.some((w) => w.id === id));

function reduce(s: HiveState, m: ServiceMessage): Partial<HiveState> {
  switch (m.type) {
    case "welcome":
      return { connection: { status: "connected", version: m.version, distro: m.distro } };
    case "version_mismatch": {
      const { type: _, ...versions } = m;
      return { connection: { status: "version_mismatch", ...versions } };
    }
    case "terminal_opened":
      return patchTerminal(s, m.channel, { exited: false, code: null, unhooked: false });
    case "terminal_exited":
      return patchTerminal(s, m.channel, { exited: true, code: m.code });
    case "unhooked_agent":
      return patchTerminal(s, m.channel, { unhooked: true });
    case "agent_detected": {
      const { type: _, channel, ...agent } = m;
      return { agents: { ...s.agents, [m.id]: { ...agent, terminal: channel } } };
    }
    case "agent_removed": {
      const { [m.id]: _, ...agents } = s.agents;
      const { [m.id]: __, ...agentStates } = s.agentStates;
      return { agents, agentStates };
    }
    case "agent_state": {
      const { type: _, id, ...status } = m;
      return { agentStates: { ...s.agentStates, [id]: status } };
    }
    case "projects": {
      const projects = Object.fromEntries(m.projects.map((p) => [p.id, p]));
      const ids = new Set(m.projects.flatMap((p) => [p.id, ...p.worktrees.map((w) => w.id)]));
      // A selected worktree that went away (e.g. `WorktreeRemove`) leaves its project selected.
      const was = owner(s.projects, s.selection);
      const gone = was && !ids.has(s.selection as string);
      const selection = gone ? (projects[was.id] ? was.id : null) : s.selection;
      const collapsed = Object.fromEntries(
        Object.entries(s.collapsed).filter(
          ([key]) => !key.startsWith("worktree:") || ids.has(key.slice("worktree:".length)),
        ),
      );
      return { projects, selection, collapsed };
    }
    case "project_added":
      // Only the add-project dialog asks for this, so it has done its job.
      return {
        projects: { ...s.projects, [m.project.id]: m.project },
        addProjectError: null,
        modal: s.modal === "add-project" ? null : s.modal,
      };
    case "add_project_failed":
      return { addProjectError: m.message };
    case "branches": {
      const { type: _, ...branches } = m;
      return patchDialog(s, { branches });
    }
    case "worktree_name_validated": {
      const { type: _, ...check } = m;
      return patchDialog(s, { nameChecks: { ...s.worktreeDialog.nameChecks, [m.name]: check } });
    }
    case "create_worktree_failed": {
      const { type: _, ...createFailure } = m;
      return patchDialog(s, { createFailure });
    }
    case "worktree_created": {
      const { project, path, notes } = m;
      return {
        projects: { ...s.projects, [project.id]: project },
        ...patchDialog(s, { created: { project: project.id, path, notes } }),
      };
    }
    case "files": {
      const { type: _, ...worktreeFiles } = m;
      return { worktreeFiles };
    }
    case "changes": {
      const { type: _, ...changes } = m;
      return { changes: { ...s.changes, [m.path]: changes } };
    }
    case "file": {
      const { type: _, ...file } = m;
      return { file, edit: editFor(s, file) };
    }
    case "file_saved":
      return s.edit && isFor(s.edit, m) ? { edit: saved(s.edit, m.version) } : {};
    case "save_failed":
      return s.edit && isFor(s.edit, m) ? { edit: failed(s.edit, m.error, m.message) } : {};
    case "disconnected":
      // The service is gone, and every agent and the watch with it.
      return {
        connection: { status: "disconnected", reason: m.reason },
        agents: {},
        agentStates: {},
        worktreeFiles: null,
      };
    default:
      // Messages without a store entry yet (e.g. `agent`, `error`) change nothing.
      return {};
  }
}

/**
 * The edit buffer after an answer for the open file while editing: the buffer updated from
 * it, or started from it; unchanged for any other file or when not editing.
 */
function editFor(s: HiveState, file: FileText | null): EditBuffer | null {
  if (!s.editing || !file || !s.openFile || !isFor(file, s.openFile)) return s.edit;
  return s.edit ? fromDisk(s.edit, file) : startEdit(file);
}

/** The only way service data enters the store. */
export function apply(message: ServiceMessage): void {
  useHive.setState((s) => reduce(s, message));
}

export const openModal = (modal: Modal, modalProject: string | null = null) =>
  useHive.setState({
    modal,
    modalProject,
    addProjectError: null,
    worktreeDialog: initialState.worktreeDialog,
  });
export const clearAddProjectError = () => useHive.setState({ addProjectError: null });
export const setRightPanel = (rightPanel: RightPanel) => useHive.setState({ rightPanel });
export const setChangedOnly = (changedOnly: boolean) => useHive.setState({ changedOnly });
/**
 * Opens a file (null closes it), as editable text when `editing`, dropping the previous file's
 * edit buffer. The file already open stays as it is.
 */
export const setOpenFile = (openFile: OpenFile | null, editing = false) =>
  useHive.setState((s) =>
    openFile && s.openFile && isFor(openFile, s.openFile)
      ? {}
      : { openFile, editing, edit: null, editorNotice: null },
  );
/** Shows the open file as editable text (its buffer starts from the last answer) or not. */
export const setEditing = (editing: boolean) =>
  useHive.setState((s) => ({ editing, edit: editing ? editFor({ ...s, editing }, s.file) : null }));
export const setEdit = (edit: EditBuffer | null) => useHive.setState({ edit });
export const setEditorNotice = (editorNotice: string | null) => useHive.setState({ editorNotice });
export const setSelectedLines = (selectedLines: Lines | null) =>
  useHive.setState({ selectedLines });
export const select = (selection: string | null) => useHive.setState({ selection });
export const toggleCollapsed = (id: string) =>
  useHive.setState((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } }));

/** A terminal just opened in `cwd`: its tab is shown and its worktree selected. */
export const addTab = (id: number, cwd: string) =>
  useHive.setState((s) => ({ tabs: [...s.tabs, { id, cwd }], activeTab: id, selection: cwd }));
export const activateTab = (tab: Tab) =>
  useHive.setState({ activeTab: tab.id, selection: tab.cwd });
/** Removes the tab; when it was shown, its right neighbour (or the new last tab) is. */
export const removeTab = (id: number) =>
  useHive.setState((s) => {
    const i = s.tabs.findIndex((t) => t.id === id);
    const tabs = s.tabs.filter((t) => t.id !== id);
    const next = tabs[Math.min(i, tabs.length - 1)]?.id ?? null;
    return { tabs, activeTab: s.activeTab === id ? next : s.activeTab };
  });

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
  return find(selectedPlace(s)) ?? find(tab?.cwd) ?? null;
}

/** Agents in the sidebar's order (project, worktree, arrival); those outside the tree last. */
export function treeAgents(s: HiveState): Agent[] {
  const worktrees = Object.values(s.projects ?? {}).flatMap((p) => p.worktrees.map((w) => w.id));
  const place = (a: Agent) => {
    const i = worktrees.indexOf(a.worktree ?? "");
    return i < 0 ? worktrees.length : i;
  };
  return Object.values(s.agents).sort((a, b) => place(a) - place(b));
}

/** Agents that need the user, in tree order: the "N pending" counter and F8's cycle. */
export const pendingAgents = (s: HiveState): Agent[] =>
  treeAgents(s).filter((a) => s.agentStates[a.id]?.pending);

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

/** States in which an agent may be writing files: the file view's "Agent working here". */
const WRITING: AgentState[] = ["working", "with_subagents", "waiting_permission"];

/** An agent placed in `worktree`, or a subagent in its own worktree there, may be writing. */
export const agentWorkingIn = (s: HiveState, worktree: string): boolean =>
  Object.values(s.agents).some((a) => {
    const status = s.agentStates[a.id];
    const subagents = status?.subagents ?? [];
    return (
      (a.worktree === worktree && !!status && WRITING.includes(status.state)) ||
      subagents.some((sub) => sub.worktree === worktree && WRITING.includes(sub.state))
    );
  });
