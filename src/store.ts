import { create } from "zustand";

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
  | ({ type: "changes" } & Changes)
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

/** The file shown under the files tree (its viewer and diff are task 3.3). */
export type OpenFile = { worktree: string; path: string };

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

/** A live subagent (`id` = its `agent_id`) with its own state. */
export type Subagent = { id: string; agent_type: string | null; state: AgentState };

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
  view: "main";
  modal: Modal;
  rightPanel: RightPanel;
  /** The files panel shows only changed files ("Changed") instead of every file ("All"). */
  changedOnly: boolean;
  openFile: OpenFile | null;
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
  /** By worktree path: the last `changes` the service sent for it. */
  changes: Record<string, Changes>;
};

export const initialState: HiveState = {
  view: "main",
  modal: null,
  rightPanel: null,
  changedOnly: false,
  openFile: null,
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
  changes: {},
};

export const useHive = create<HiveState>()(() => initialState);

function patchTerminal(s: HiveState, id: number, patch: Partial<Terminal>): Partial<HiveState> {
  const current = s.terminals[id] ?? { id, exited: false, code: null, unhooked: false };
  return { terminals: { ...s.terminals, [id]: { ...current, ...patch } } };
}

function patchDialog(s: HiveState, patch: Partial<WorktreeDialog>): Partial<HiveState> {
  return { worktreeDialog: { ...s.worktreeDialog, ...patch } };
}

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
    case "projects":
      return { projects: Object.fromEntries(m.projects.map((p) => [p.id, p])) };
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
    case "changes": {
      const { type: _, ...changes } = m;
      return { changes: { ...s.changes, [m.path]: changes } };
    }
    case "disconnected":
      // The service is gone, and every agent with it.
      return {
        connection: { status: "disconnected", reason: m.reason },
        agents: {},
        agentStates: {},
      };
    default:
      // Messages without a store entry yet (e.g. `agent`, `error`) change nothing.
      return {};
  }
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
export const setOpenFile = (openFile: OpenFile | null) => useHive.setState({ openFile });
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
 * The worktree the files panel shows: the selected worktree (a selected project is its main
 * worktree, which shares its id) or the selected agent's; else the shown terminal's.
 */
export function panelWorktree(s: HiveState): { project: Project; worktree: Worktree } | null {
  const all = Object.values(s.projects ?? {}).flatMap((project) =>
    project.worktrees.map((worktree) => ({ project, worktree })),
  );
  const find = (id: string | null | undefined) => all.find((e) => e.worktree.id === id);
  const agent = s.agents[s.selection ?? ""];
  const tab = s.tabs.find((t) => t.id === s.activeTab);
  return find(agent ? agent.worktree : s.selection) ?? find(tab?.cwd) ?? null;
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

/** Re-renders only when this agent's entry changes. */
export const useAgent = (id: string) => useHive((s) => s.agents[id]);
export const useTerminal = (id: number) => useHive((s) => s.terminals[id]);
