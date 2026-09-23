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
  | { type: "projects"; projects: Project[] }
  | { type: "project_added"; project: Project }
  | { type: "add_project_failed"; path: string; error: ProjectError; message: string }
  | ({ type: "branches" } & Branches)
  | ({ type: "worktree_name_validated" } & NameCheck)
  | { type: "worktree_created"; project: Project; path: string; notes: string[] }
  | ({ type: "create_worktree_failed" } & CreateFailure)
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

/** Answers for the new-worktree dialog; reset whenever a dialog opens. */
export type WorktreeDialog = {
  branches: Branches | null;
  /** By name: answers can arrive out of order while the user types. */
  nameChecks: Record<string, NameCheck>;
  created: { project: string; path: string; notes: string[] } | null;
  createFailure: CreateFailure | null;
};

// ponytail: id-only shape; fields and `apply` cases arrive with the service messages that feed it (1.8).
export type Agent = { id: string };

/** A terminal tab: the terminal and the worktree path it was opened in (its title's source). */
export type Tab = { id: number; cwd: string };

export type Modal = "new-worktree" | "add-project" | null;
export type RightPanel = "files" | null;

export type HiveState = {
  // UI state
  view: "main";
  modal: Modal;
  rightPanel: RightPanel;
  selection: string | null;
  /** Collapsed tree nodes, by id. */
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
};

export const initialState: HiveState = {
  view: "main",
  modal: null,
  rightPanel: null,
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
    case "disconnected":
      return { connection: { status: "disconnected", reason: m.reason } };
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
export const setRightPanel = (rightPanel: RightPanel) => useHive.setState({ rightPanel });
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

/** Re-renders only when this agent's entry changes. */
export const useAgent = (id: string) => useHive((s) => s.agents[id]);
export const useTerminal = (id: number) => useHive((s) => s.terminals[id]);
