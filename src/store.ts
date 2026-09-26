import { create } from "zustand";
import { moveNextTo } from "./reorder";
import {
  type EditBuffer,
  failed,
  fromDisk,
  isDirty,
  isFor,
  saved,
  startEdit,
} from "./viewer/buffer";

// The one store (#30, #38). UI state is set by components; service data changes
// only through `apply`, which stores what the service sent without deriving anything (#37).

/** Service → app messages the store understands. Mirrors `hive_protocol::Control`. */
export type ServiceMessage =
  | { type: "welcome"; version: string; distro: string | null }
  | { type: "settings"; settings: Settings }
  // A refused `set_settings`, or a settings file the service ignored (then `settings` holds
  // the defaults).
  | { type: "settings_failed"; message: string }
  | ({ type: "diagnostics" } & Diagnostics)
  // From the app side (Rust), not the service: a newer release on GitHub (4.19).
  | { type: "update_ready"; version: string }
  | { type: "update_failed"; error: string }
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
  // `hive badge` in that terminal; empty clears it.
  | { type: "badge"; channel: number; text: string }
  | ({ type: "agent_detected"; channel: number } & Omit<Agent, "terminal">)
  | { type: "agent_removed"; channel: number; id: string }
  | { type: "agent_title"; channel: number; id: string; title: string }
  | ({ type: "agent_state"; id: string } & AgentStatus)
  | ({ type: "agent_usage"; id: string } & AgentUsage)
  | { type: "projects"; projects: Project[] }
  | { type: "project_added"; project: Project }
  | { type: "add_project_failed"; path: string; error: ProjectError; message: string }
  | { type: "spaces"; spaces: Space[]; current: string }
  | { type: "space_failed"; message: string }
  | ({ type: "branches" } & Branches)
  | ({ type: "worktree_name_validated" } & NameCheck)
  | { type: "worktree_created"; project: Project; path: string; notes: string[] }
  | ({ type: "create_worktree_failed" } & CreateFailure)
  | { type: "worktree_removed"; project: Project; path: string }
  | { type: "remove_worktree_failed"; path: string; message: string }
  | { type: "worktree_renamed"; project: Project; from: string; path: string }
  | { type: "rename_worktree_failed"; path: string; name: string; message: string }
  | { type: "worktree_status"; path: string; status: WorktreeStatus | null }
  | ({ type: "files" } & WorktreeFiles)
  // A refused request, e.g. watching a worktree that is not followed. Not stored.
  | { type: "error"; message: string }
  | ({ type: "changes" } & Changes)
  | ({ type: "file" } & FileText)
  | ({ type: "search_results" } & SearchResults)
  | ({ type: "dirs" } & Dirs)
  | { type: "sessions"; sessions: Session[]; error: string | null }
  // Handled by `openSession` (src/sessions.ts), not stored.
  | {
      type: "session_located";
      id: string;
      target: SessionTarget;
      windows_path: string | null;
      error: string | null;
    }
  | { type: "session_deleted"; id: string }
  // Handled by `restore` (src/sessions.ts), not stored.
  | { type: "restore_sessions"; sessions: OpenSession[] }
  | { type: "delete_session_failed"; id: string; message: string }
  | { type: "file_saved"; worktree: string; path: string; version: string }
  | { type: "save_failed"; worktree: string; path: string; error: SaveError; message: string }
  | { type: "file_created"; worktree: string; path: string }
  | { type: "file_renamed"; worktree: string; path: string; to: string }
  | { type: "folder_created"; worktree: string; path: string }
  | { type: "file_op_failed"; worktree: string; message: string }
  // Handled by `openExternal` (src/viewer/external.ts), not stored.
  // An empty `path` is the worktree's folder (`openFolder`); an empty `worktree` too, the
  // settings file.
  | {
      type: "editor_target";
      worktree: string;
      path: string;
      windows_path: string | null;
      error: string | null;
    }
  | ({ type: "transcript" } & Transcript)
  | {
      type: "transcript_appended";
      agent: string;
      subagent: string;
      entries: TranscriptEntry[];
    }
  // The chat (7.3), on the chat's channel (`chat` is that channel).
  | ({ type: "chat_opened"; channel: number } & ChatOpened)
  | {
      type: "chat_entries";
      channel: number;
      chat: number;
      entries: ChatEntry[];
      replace_last: boolean;
    }
  | { type: "chat_request"; channel: number; chat: number; request: ChatRequest }
  | { type: "chat_request_gone"; channel: number; chat: number; request: string }
  | ({ type: "chat_status"; channel: number } & ChatStatus)
  // Also sent by the app side (Rust) for every open chat when the bridge exits.
  | { type: "chat_closed"; channel: number; chat: number; error: string | null }
  // The first chat in a project waits for the human's answer (`confirmChatFolder`).
  | { type: "confirm_chat_folder"; channel: number; chat: number; cwd: string }
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
  /** Set with `hive badge`; cleared when the terminal exits. */
  badge?: string;
};

/** Mirrors `hive_protocol::Worktree`: every field comes from the service. */
export type Worktree = {
  id: string;
  name: string;
  path: string;
  branch: string | null;
  main: boolean;
  claude: boolean;
  /** Null when git could not tell. */
  status: WorktreeStatus | null;
};

/**
 * Mirrors `hive_protocol::WorktreeStatus`: files changed, commits ahead of and behind the main
 * worktree's branch (null for the main worktree), all merged there, and the last commit's time.
 */
export type WorktreeStatus = {
  changes: number;
  ahead: number | null;
  behind: number | null;
  merged: boolean;
  last_commit_ms: number;
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
  | "in_other_space"
  | "storage";

/** Mirrors `hive_protocol::SpaceEnv`: what a space's terminals get; null leaves the user's own. */
export type SpaceEnv = {
  claude_config_dir: string | null;
  git_name: string | null;
  git_email: string | null;
  gh_config_dir: string | null;
};

/** Mirrors `hive_protocol::Space` (6.14): its projects' ids and its terminals' environment. */
export type Space = { id: string; name: string; projects: string[]; env: SpaceEnv };

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

/** Mirrors `hive_protocol::OpenSession`: a session that ran in a Hive terminal or chat. */
export type OpenSession = { id: string; cwd: string; kind: "terminal" | "chat" };

/** A Claude Code session of a followed project, as the service read it from its log. */
export type Session = {
  id: string;
  project: string;
  worktree: string;
  cwd: string;
  title: string | null;
  last_role: "user" | "assistant" | null;
  last_text: string | null;
  messages: number;
  model: string | null;
  branch: string | null;
  /** The last turn's context and the output so far, in tokens. */
  context_tokens: number;
  output_tokens: number;
  updated_ms: number;
  log: string;
  /** By how its log ends; a session running in a Hive terminal shows its live state instead. */
  state: AgentState;
  /** A `claude` is known to run it: in a Hive terminal or chat (see `agents`), or outside Hive. */
  running: boolean;
};
export type SessionTarget = "log" | "folder";
/** A session's context menu, at the pointer. */
export type SessionMenu = { session: string; x: number; y: number };
/**
 * A file tree's menu target: new files go in `folder` (relative to the worktree, "" for its
 * root); `path` is the file to rename, null for a folder or the tree's background.
 */
export type FileTarget = { worktree: string; folder: string; path: string | null };
/** What the file name dialog does: a new file or folder in `folder`, or rename `path`. */
export type FileDialogKind = "file" | "folder" | "rename";
/** The "New file" / "New folder" / "Rename file" dialog: its target and the service's refusal. */
export type FileDialog = FileTarget & { kind: FileDialogKind; error: string | null };

/** A line of a file holding the searched text (`line` is 1-based). */
export type SearchMatch = { path: string; line: number; text: string };
/** The service's answer to a search of a worktree's file contents. */
export type SearchResults = {
  worktree: string;
  query: string;
  matches: SearchMatch[];
  truncated: boolean;
  error: string | null;
};

/**
 * The service's answer to `list_dirs`: `path` as asked (the home folder for an empty one), as a
 * Linux path for `add_project`, the folder above the listed one and the listed subfolders.
 */
export type Dirs = {
  path: string;
  windows: boolean;
  linux_path: string | null;
  parent: string | null;
  dirs: { name: string; git: boolean }[];
  error: string | null;
};

/** Mirrors `hive_protocol::SaveError`. */
export type SaveError = "conflict" | "too_large" | "invalid_path" | "io";

/** Mirrors `hive_protocol::TranscriptEntry`: a message, or a tool call (`tool` is its name). */
export type TranscriptEntry = {
  role: "user" | "assistant" | "tool";
  text: string;
  tool: string | null;
};
/** A subagent: its agent's session id and its own `agent_id`. */
export type SubagentRef = { agent: string; subagent: string };
/** A subagent's conversation as the service read it, then grown by `transcript_appended`. */
export type Transcript = SubagentRef & { entries: TranscriptEntry[]; truncated: boolean };
/** Entries kept of a followed conversation; older ones are dropped. */
export const TRANSCRIPT_LIMIT = 1000;

/** Mirrors `hive_protocol::ChatMode`; `bypassPermissions` is never offered. */
export type ChatMode = "default" | "accept_edits" | "plan";
/** Mirrors `hive_protocol::ChatImage`: `data` is base64. */
export type ChatImage = { media_type: string; data: string };
export type ChatEntryKind =
  | "user"
  | "assistant"
  | "thinking"
  | "tool"
  | "error"
  | "note"
  | "divider"
  | "usage";
export type ToolStatus = "running" | "ok" | "error";
/**
 * Mirrors `hive_protocol::ChatEntry`: one row of a chat. An entry with a known `id` replaces
 * that one (a tool's result); `parent` is the `Agent` call a subagent's entry belongs to.
 */
export type ChatEntry = {
  id: number;
  kind: ChatEntryKind;
  text: string;
  tool: string | null;
  parent: string | null;
  status: ToolStatus | null;
  output: string | null;
  image: ChatImage | null;
};
/** Mirrors `hive_protocol::ChatQuestion`: one question of an `AskUserQuestion`. */
export type ChatQuestion = {
  question: string;
  header: string;
  multi: boolean;
  options: { label: string; description: string }[];
};
/** Mirrors `hive_protocol::ChatRequest`: a permission, question or plan waiting on the human. */
export type ChatRequest = {
  id: string;
  kind: "permission" | "question" | "plan";
  tool: string;
  /** The full command, path or input JSON. */
  detail: string;
  reason: string | null;
  questions: ChatQuestion[];
  plan: string | null;
};
/** Mirrors `hive_protocol::ChatAnswer`; `answers` has, per question, labels or one free text. */
export type ChatAnswer =
  | { kind: "allow" }
  | { kind: "deny"; message: string | null }
  | { kind: "answers"; answers: string[][] }
  | { kind: "approve_plan"; accept_edits: boolean }
  | { kind: "keep_planning"; feedback: string };
export type ChatOpened = {
  chat: number;
  cwd: string;
  session: string | null;
  model: string | null;
  mode: ChatMode;
  commands: string[];
  /** Set when the chat runs on an API key rather than the subscription login. */
  api_key_source: string | null;
};
export type ChatStatus = {
  chat: number;
  busy: boolean;
  mode: ChatMode;
  model: string | null;
  /** A transient API retry, e.g. "Retrying 2/10…". */
  retry: string | null;
  compacting: boolean;
  session: string | null;
  /** Set when the chat runs on an API key rather than the subscription login. */
  api_key_source: string | null;
};
/** A chat tab's data, as the service sent it (keyed by the chat's channel). */
export type Chat = {
  cwd: string;
  /** Null until `chat_opened`. */
  opened: ChatOpened | null;
  status: ChatStatus | null;
  /** At most `CHAT_LIMIT`, oldest first. */
  entries: ChatEntry[];
  /** Permissions, questions and plans waiting on the human, oldest first. */
  requests: ChatRequest[];
  /** The service asks to confirm the first chat in this folder (`confirm_chat_folder`). */
  confirm: boolean;
  /** Set once `chat_closed` arrived; `error` holds claude's last words when it failed. */
  closed: { error: string | null } | null;
};
/** Entries kept of a chat; older ones are dropped. */
export const CHAT_LIMIT = 2000;

/** A 1-based, inclusive range of lines. */
export type Lines = { from: number; to: number };

/** A review comment (6.7) on lines of a worktree's file (new-file numbers). */
export type ReviewComment = Lines & { path: string; text: string };

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
  /** Why deleting (`name` null) or renaming (to `name`) the worktree `path` failed. */
  failure: { path: string; name: string | null; message: string } | null;
  /** Why deleting each worktree failed, by path (removing merged worktrees sends several). */
  removeFailures: Record<string, string>;
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
  | "waiting_plan"
  | "waiting_answer"
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
} & Doing;

/** What an agent or subagent is doing (its current tool call) and since when (ms since the epoch) its state lasts. */
export type Doing = { activity: string | null; since_ms: number };

/**
 * What `agent_state` says about an agent, stored by its session id. `urgency` (higher wins)
 * and `pending` (needs the user) are the service's, so the app keeps no table of its own.
 */
export type AgentStatus = {
  state: AgentState;
  urgency: number;
  pending: boolean;
  /** It waits for you because the user interrupted it: nothing alerts. */
  interrupted: boolean;
  subagents: Subagent[];
} & Doing;

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

/**
 * What `agent_usage` says: the last turn's context, the window the service assumes, and the
 * session's output tokens.
 */
export type AgentUsage = { context_tokens: number; context_limit: number; output_tokens: number };

/**
 * Mirrors `hive_protocol::Settings`: the service's settings file, read and saved whole. The
 * service checks the ranges (#37).
 */
export type Settings = {
  terminal: {
    font_family: string;
    font_size: number;
    scrollback: number;
    cursor_style: "block" | "bar" | "underline";
    cursor_blink: boolean;
    copy_on_select: boolean;
  };
  appearance: { theme: "one-dark" | "one-light" };
  /** `volume`: the alert tone's, in percent; 0 mutes it. */
  notifications: { volume: number };
  agents: { silence_secs: number; confirm_close: boolean };
  worktrees: { default_base: string | null };
  /** By project id. */
  /** `chat_confirmed`: chats (7.3) allowed in the project; set by the service only. */
  projects: Record<string, { scripts: ProjectScripts; chat_confirmed?: boolean }>;
};

/** A project's scripts (6.8): the user's own, kept only in the settings. */
export type ProjectScripts = {
  /** Typed into a new terminal in each worktree the app creates. */
  setup: string | null;
  /** Typed into a new terminal when chosen. */
  run: { name: string; command: string }[];
  /** Run by the service before it removes a worktree. */
  archive: string | null;
};

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
};

/** What the settings' About section shows (the service's `diagnostics`). */
export type Diagnostics = {
  settings_file: string;
  /** The `claude` wrapper Hive terminals run first. */
  wrapper: string;
  /** The `claude` it runs, as found on the service's `PATH`; null when none is. */
  claude: string | null;
};

/**
 * A tab of the terminal area: a terminal, or a chat (7.3) when `kind` is "chat" (absent is a
 * terminal), and the worktree path it was opened in (its title's source).
 */
export type Tab = { id: number; cwd: string; kind?: "terminal" | "chat" };
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
  | null;
/**
 * A yes/no question asked in a Hive dialog (8.20), never the WebView's `confirm`: `run` happens
 * only when the user picks `action` (e.g. "Discard", "Delete").
 */
export type Question = { title: string; text: string; action: string; run: () => void };
/** A worktree row's context menu, at the pointer. */
export type WorktreeMenu = { worktree: string; x: number; y: number };
/** A project row's context menu, at the pointer. */
export type ProjectMenu = { project: string; x: number; y: number };
export type RightPanel = "files" | null;
/** What the right panel shows. */
export type PanelView = "files" | "changes" | "sessions";

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
  /** The question of the "confirm" modal (`ask`). */
  question: Question | null;
  /** A short message in the status bar, e.g. why the Explorer did not open. */
  notice: string | null;
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
  openFile: OpenFile | null;
  /** The open file's tab is the one shown, in place of the active terminal. */
  fileShown: boolean;
  /** The lines selected in the open file's viewer (new-file numbers, 1-based), or null. */
  selectedLines: Lines | null;
  /** The subagent whose conversation shows in place of the terminals (6.10), or null. */
  transcriptShown: SubagentRef | null;
  /** Review comments not sent yet, by worktree path (6.7). */
  comments: Record<string, ReviewComment[]>;
  /** The lines the comment input is open for, in the open file of `worktree`, or null. */
  commenting: (Lines & OpenFile) | null;
  selection: string | null;
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
  /** Whether the app window has the focus (`watchFocus` in `src/window.ts`). */
  focused: boolean;
  // Service data
  connection: Connection;
  /** The service's settings (the defaults until they arrive). */
  settings: Settings;
  /** Why the last `set_settings` was refused, or the settings file was ignored. */
  settingsError: string | null;
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
  /** The last contents search the service answered. */
  searchResults: SearchResults | null;
  /** The last folder listing for "Add project". */
  dirs: Dirs | null;
  /** Claude sessions of the followed projects, the most recent first; null until listed. */
  sessions: Session[] | null;
  /** Why the service could not list them. */
  sessionsError: string | null;
  /** A line to show once the open file's text is there (a search result), then cleared. */
  gotoLine: (OpenFile & { line: number }) | null;
  /** The followed subagent's conversation; check `agent` and `subagent`. */
  transcript: Transcript | null;
  /** Chats (7.3) by their channel, the id of their tab. */
  chats: Record<number, Chat>;
};

export const initialState: HiveState = {
  modal: null,
  menu: null,
  projectMenu: null,
  sessionMenu: null,
  fileMenu: null,
  fileDialog: null,
  newFolders: {},
  question: null,
  notice: null,
  update: null,
  rightPanel: "files",
  panelView: "files",
  sidebarWidth: 264,
  panelWidth: 380,
  splitPercent: 50,
  agentOrder: [],
  openFile: null,
  fileShown: false,
  selectedLines: null,
  transcriptShown: null,
  comments: {},
  commenting: null,
  selection: null,
  collapsed: {},
  tabs: [],
  activeTab: null,
  split: null,
  inbox: [],
  inboxSeen: 0,
  focused: false,
  connection: { status: "connecting" },
  settings: DEFAULT_SETTINGS,
  settingsError: null,
  diagnostics: null,
  projects: null,
  addProjectError: null,
  spaces: null,
  currentSpace: null,
  spaceError: null,
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
  worktreeFiles: null,
  changes: {},
  file: null,
  editing: false,
  edit: null,
  editorNotice: null,
  searchResults: null,
  dirs: null,
  sessions: null,
  sessionsError: null,
  gotoLine: null,
  transcript: null,
  chats: {},
};

// Side panel widths: UI preferences, kept in the window's storage between runs.
/** How a side panel may be sized, in pixels. */
export const LIMITS = {
  sidebar: { min: 200, max: 480 },
  panel: { min: 280, max: 640 },
  /** The split's left pane, in percent of the terminal area. */
  split: { min: 20, max: 80 },
} as const;
/** Dragged this narrow, the right panel closes instead. */
export const PANEL_CLOSE_AT = 200;
export type Side = keyof typeof LIMITS;
const KEYS = { sidebar: "sidebarWidth", panel: "panelWidth", split: "splitPercent" } as const;
export const widthKey = (side: Side) => KEYS[side];

/** A width kept within the side's limits. */
export const clampWidth = (side: Side, width: number) =>
  Math.round(Math.max(LIMITS[side].min, Math.min(LIMITS[side].max, width)));

/** Where the widths are remembered between runs (a per-window preference, not service data). */
const STORAGE = "hive.widths";

/** The widths remembered from the last run, within the limits; the defaults otherwise. */
export function savedWidths(storage: Pick<Storage, "getItem"> | null = safeStorage()) {
  try {
    const saved = JSON.parse(storage?.getItem(STORAGE) ?? "{}");
    return {
      sidebarWidth: clampWidth("sidebar", Number(saved.sidebarWidth) || 264),
      panelWidth: clampWidth("panel", Number(saved.panelWidth) || 380),
      splitPercent: clampWidth("split", Number(saved.splitPercent) || 50),
    };
  } catch {
    return { sidebarWidth: 264, panelWidth: 380, splitPercent: 50 };
  }
}

export function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** Sets a side's width (kept within its limits) and remembers both. */
export function setWidth(side: Side, width: number): void {
  useHive.setState({ [widthKey(side)]: clampWidth(side, width) });
  const { sidebarWidth, panelWidth, splitPercent } = useHive.getState();
  try {
    safeStorage()?.setItem(STORAGE, JSON.stringify({ sidebarWidth, panelWidth, splitPercent }));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/** Where the agents' order is remembered between runs (a per-window preference, #37). */
const ORDER_STORAGE = "hive.agentOrder";
/** How many session ids are remembered; the oldest moves are forgotten first. */
const ORDER_LIMIT = 500;

/** The agents' order remembered from the last run; empty when none or unreadable. */
export function savedAgentOrder(
  storage: Pick<Storage, "getItem"> | null = safeStorage(),
): string[] {
  try {
    const saved: unknown = JSON.parse(storage?.getItem(ORDER_STORAGE) ?? "[]");
    return Array.isArray(saved)
      ? saved.filter((id): id is string => typeof id === "string").slice(0, ORDER_LIMIT)
      : [];
  } catch {
    return [];
  }
}

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
  try {
    safeStorage()?.setItem(ORDER_STORAGE, JSON.stringify(agentOrder));
  } catch {
    // A full or blocked storage only loses the preference.
  }
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
}));

function patchTerminal(s: HiveState, id: number, patch: Partial<Terminal>): Partial<HiveState> {
  const current = s.terminals[id] ?? { id, exited: false, code: null, unhooked: false };
  return { terminals: { ...s.terminals, [id]: { ...current, ...patch } } };
}

/** Changes chat `id`, made empty first when the service speaks of it before its tab exists. */
function patchChat(s: HiveState, id: number, patch: (chat: Chat) => Partial<Chat>) {
  const chat = s.chats[id] ?? {
    cwd: "",
    opened: null,
    status: null,
    entries: [],
    requests: [],
    confirm: false,
    closed: null,
  };
  return { chats: { ...s.chats, [id]: { ...chat, ...patch(chat) } } };
}

/**
 * A chat's entries after `chat_entries`: an entry replaces the one with its `id` (a tool's
 * result), or the last one when `replaceLast` (live text); any other is added at the end.
 */
export function mergeEntries(
  entries: ChatEntry[],
  more: ChatEntry[],
  replaceLast: boolean,
): ChatEntry[] {
  const next = [...entries];
  more.forEach((entry, i) => {
    // ponytail: a linear search per entry over at most CHAT_LIMIT; index by id if it shows.
    const at = i === 0 && replaceLast ? next.length - 1 : next.findIndex((e) => e.id === entry.id);
    if (at >= 0) next[at] = entry;
    else next.push(entry);
  });
  return next.slice(-CHAT_LIMIT);
}

function patchDialog(s: HiveState, patch: Partial<WorktreeDialog>): Partial<HiveState> {
  return { worktreeDialog: { ...s.worktreeDialog, ...patch } };
}

/** The project that is `id` or holds the worktree `id`. */
export const owner = (projects: HiveState["projects"], id: string | null): Project | undefined =>
  Object.values(projects ?? {}).find((p) => p.id === id || p.worktrees.some((w) => w.id === id));

/** The worktree `id` of any project. */
export const findWorktree = (projects: HiveState["projects"], id: string | null) =>
  owner(projects, id)?.worktrees.find((w) => w.id === id);

function reduce(s: HiveState, m: ServiceMessage): Partial<HiveState> {
  switch (m.type) {
    case "welcome":
      return { connection: { status: "connected", version: m.version, distro: m.distro } };
    case "settings":
      return { settings: m.settings, settingsError: null };
    case "settings_failed":
      return { settingsError: m.message, notice: m.message };
    case "diagnostics": {
      const { type: _, ...diagnostics } = m;
      return { diagnostics };
    }
    case "update_ready":
      return { update: { version: m.version, installing: false } };
    case "update_failed":
      return {
        update: s.update && { ...s.update, installing: false },
        notice: `Update failed: ${m.error}`,
      };
    case "version_mismatch": {
      const { type: _, ...versions } = m;
      return { connection: { status: "version_mismatch", ...versions } };
    }
    case "terminal_opened":
      return patchTerminal(s, m.channel, { exited: false, code: null, unhooked: false });
    case "terminal_exited":
      return patchTerminal(s, m.channel, { exited: true, code: m.code, badge: "" });
    case "unhooked_agent":
      return patchTerminal(s, m.channel, { unhooked: true });
    case "badge":
      return patchTerminal(s, m.channel, { badge: m.text });
    case "agent_detected": {
      const { type: _, channel, ...agent } = m;
      return { agents: { ...s.agents, [m.id]: { ...agent, terminal: channel } } };
    }
    case "agent_removed": {
      const { [m.id]: _, ...agents } = s.agents;
      const { [m.id]: __, ...agentStates } = s.agentStates;
      const { [m.id]: ___, ...agentTitles } = s.agentTitles;
      const { [m.id]: ____, ...agentUsage } = s.agentUsage;
      const shown = s.transcriptShown?.agent === m.id ? null : s.transcriptShown;
      return { agents, agentStates, agentTitles, agentUsage, transcriptShown: shown };
    }
    case "agent_title":
      return { agentTitles: { ...s.agentTitles, [m.id]: m.title } };
    case "agent_usage": {
      const { type: _, id, ...usage } = m;
      return { agentUsage: { ...s.agentUsage, [id]: usage } };
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
    case "spaces":
      // The answer to the space dialog's request: it has done its job.
      return {
        spaces: m.spaces,
        currentSpace: m.current,
        spaceError: null,
        modal: s.modal === "new-space" || s.modal === "edit-space" ? null : s.modal,
      };
    case "space_failed":
      return { spaceError: m.message };
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
    case "worktree_removed":
    case "worktree_renamed": {
      // As a new list: a worktree that went away is dropped as `projects` drops it.
      const list = Object.values(s.projects ?? {}).map((p) =>
        p.id === m.project.id ? m.project : p,
      );
      const next = reduce(s, { type: "projects", projects: list });
      const renamed = m.type === "worktree_renamed" && s.selection === m.from;
      const dialog = m.type === "worktree_removed" ? "remove-worktree" : "rename-worktree";
      const gone = m.type === "worktree_removed" ? m.path : m.from;
      return {
        ...next,
        selection: renamed ? m.path : next.selection,
        // Only the dialog opened for that worktree has done its job.
        modal: s.modal === dialog && s.modalWorktree === gone ? null : s.modal,
      };
    }
    case "remove_worktree_failed":
      return patchDialog(s, {
        failure: { path: m.path, name: null, message: m.message },
        removeFailures: { ...s.worktreeDialog.removeFailures, [m.path]: m.message },
      });
    case "worktree_status": {
      if (!s.projects) return {};
      const patch = (w: Worktree) => (w.path === m.path ? { ...w, status: m.status } : w);
      const projects = Object.values(s.projects).map((p) => ({
        ...p,
        worktrees: p.worktrees.map(patch),
      }));
      return { projects: Object.fromEntries(projects.map((p) => [p.id, p])) };
    }
    case "rename_worktree_failed": {
      const { type: _, ...failure } = m;
      return patchDialog(s, { failure });
    }
    case "files": {
      const { type: _, ...worktreeFiles } = m;
      return { worktreeFiles };
    }
    case "changes": {
      const { type: _, ...changes } = m;
      return { changes: { ...s.changes, [m.path]: changes } };
    }
    case "sessions":
      return { sessions: m.sessions, sessionsError: m.error };
    case "session_deleted":
      return { sessions: s.sessions?.filter((x) => x.id !== m.id) ?? null };
    case "delete_session_failed":
      return { notice: `Cannot delete the session: ${m.message}` };
    case "search_results": {
      const { type: _, ...searchResults } = m;
      return { searchResults };
    }
    case "dirs": {
      const { type: _, ...dirs } = m;
      return { dirs };
    }
    case "file": {
      const { type: _, ...file } = m;
      return { file, edit: editFor(s, file) };
    }
    case "file_saved":
      return s.edit && isFor(s.edit, m) ? { edit: saved(s.edit, m.version) } : {};
    case "save_failed":
      return s.edit && isFor(s.edit, m) ? { edit: failed(s.edit, m.error, m.message) } : {};
    case "file_created": {
      // The new file opens as editable text, unless that would drop unsaved edits.
      const keep = s.edit && isDirty(s.edit);
      const open = keep ? {} : opened(s, { worktree: m.worktree, path: m.path }, true);
      return { ...open, ...fileDialogDone(s, m.worktree) };
    }
    case "file_renamed": {
      // The open file, its text and its edits follow the rename.
      const moved = <T extends OpenFile>(f: T | null) =>
        f && isFor(f, m) ? { ...f, path: m.to } : f;
      return {
        openFile: moved(s.openFile),
        file: moved(s.file),
        edit: moved(s.edit),
        ...fileDialogDone(s, m.worktree),
      };
    }
    case "folder_created": {
      // It shows at once, even empty, with the folders around it open.
      const open = m.path
        .split("/")
        .slice(0, -1)
        .map((_, i, parts) => [`files:${m.worktree}/${parts.slice(0, i + 1).join("/")}`, false]);
      const shown = s.newFolders[m.worktree] ?? [];
      return {
        newFolders: { ...s.newFolders, [m.worktree]: [...shown, m.path] },
        collapsed: { ...s.collapsed, ...Object.fromEntries(open) },
        ...fileDialogDone(s, m.worktree),
      };
    }
    case "file_op_failed":
      return s.fileDialog?.worktree === m.worktree
        ? { fileDialog: { ...s.fileDialog, error: m.message } }
        : {};
    case "transcript": {
      const { type: _, ...transcript } = m;
      return { transcript };
    }
    case "transcript_appended": {
      const t = s.transcript;
      if (t?.agent !== m.agent || t.subagent !== m.subagent) return {};
      const entries = [...t.entries, ...m.entries];
      const over = entries.length > TRANSCRIPT_LIMIT;
      return {
        transcript: {
          ...t,
          entries: entries.slice(-TRANSCRIPT_LIMIT),
          truncated: t.truncated || over,
        },
      };
    }
    case "chat_opened": {
      const { type: _, channel: __, ...opened } = m;
      return patchChat(s, m.chat, () => ({ opened, cwd: m.cwd, confirm: false }));
    }
    case "chat_entries":
      return patchChat(s, m.chat, (c) => ({
        entries: mergeEntries(c.entries, m.entries, m.replace_last),
      }));
    case "chat_request":
      return patchChat(s, m.chat, (c) => ({ requests: [...c.requests, m.request] }));
    case "chat_request_gone":
      return patchChat(s, m.chat, (c) => ({
        requests: c.requests.filter((r) => r.id !== m.request),
      }));
    case "chat_status": {
      const { type: _, channel: __, ...status } = m;
      return patchChat(s, m.chat, () => ({ status }));
    }
    case "chat_closed":
      return patchChat(s, m.chat, (c) => ({
        closed: { error: m.error },
        confirm: false,
        requests: [],
        status: c.status && { ...c.status, busy: false },
      }));
    case "confirm_chat_folder":
      return patchChat(s, m.chat, () => ({ cwd: m.cwd, confirm: true }));
    case "disconnected":
      // The service is gone, and every agent and the watches with it.
      return {
        connection: { status: "disconnected", reason: m.reason },
        agents: {},
        agentStates: {},
        worktreeFiles: null,
        transcriptShown: null,
        transcript: null,
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

/** Closes the file dialog when the answer is for its worktree. */
function fileDialogDone(s: HiveState, worktree: string): Partial<HiveState> {
  return s.fileDialog?.worktree === worktree ? { fileDialog: null, modal: null } : {};
}

/** The only way service data enters the store. */
export function apply(message: ServiceMessage): void {
  useHive.setState((s) => reduce(s, message));
}

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
/** Opening the inbox marks every alert read. */
export const markInboxRead = () => useHive.setState((s) => ({ inboxSeen: s.inbox[0]?.id ?? 0 }));
export const setNotice = (notice: string | null) => useHive.setState({ notice });
export const clearAddProjectError = () => useHive.setState({ addProjectError: null });
export const setRightPanel = (rightPanel: RightPanel) => useHive.setState({ rightPanel });
export const setPanelView = (panelView: PanelView) => useHive.setState({ panelView });
/**
 * Opens a file in its tab and shows it (null closes it), as editable text when `editing`,
 * dropping the previous file's edit buffer. The file already open stays as it is.
 */
export const setOpenFile = (openFile: OpenFile | null, editing = false, line?: number) =>
  useHive.setState((s) => opened(s, openFile, editing, line));

function opened(
  s: HiveState,
  openFile: OpenFile | null,
  editing: boolean,
  line?: number,
): Partial<HiveState> {
  const gotoLine = openFile && line ? { ...openFile, line } : null;
  return openFile && s.openFile && isFor(openFile, s.openFile)
    ? { fileShown: true, gotoLine, transcriptShown: null }
    : {
        openFile,
        fileShown: openFile !== null,
        transcriptShown: openFile ? null : s.transcriptShown,
        editing,
        edit: null,
        editorNotice: null,
        gotoLine,
      };
}
/** The line asked for was shown. */
export const clearGotoLine = () => useHive.setState({ gotoLine: null });
export const showFile = () => useHive.setState({ fileShown: true, transcriptShown: null });
/** Shows the open file as editable text (its buffer starts from the last answer) or not. */
export const setEditing = (editing: boolean) =>
  useHive.setState((s) => ({ editing, edit: editing ? editFor({ ...s, editing }, s.file) : null }));
export const setEdit = (edit: EditBuffer | null) => useHive.setState({ edit });
export const setEditorNotice = (editorNotice: string | null) => useHive.setState({ editorNotice });
export const setSelectedLines = (selectedLines: Lines | null) =>
  useHive.setState({ selectedLines });
/**
 * Selects a project, worktree or agent. The tab bar then shows that place's tabs: the shown
 * terminal stays when it is one of them, else the last of them is shown (none when it has
 * none), and the open file stays shown only when it belongs there.
 */
export const select = (selection: string | null) =>
  useHive.setState((s) => {
    const next = { ...s, selection };
    const tabs = visibleTabs(next);
    const keep = tabs.some((t) => t.id === s.activeTab);
    return {
      selection,
      activeTab: keep ? s.activeTab : (tabs.at(-1)?.id ?? null),
      fileShown: s.fileShown && fileVisible(next),
      transcriptShown: null,
    };
  });
/**
 * Shows a subagent's conversation in place of the terminals (6.10): its agent is selected, and
 * the agent's terminal is the one "Back to terminal" returns to.
 */
export const showTranscript = (agent: string, subagent: string) => {
  select(agent);
  useHive.setState((s) => {
    const tab = s.tabs.find((t) => t.id === s.agents[agent]?.terminal);
    return {
      transcriptShown: { agent, subagent },
      fileShown: false,
      activeTab: tab?.id ?? s.activeTab,
    };
  });
};
/** Back to the terminal from a subagent's conversation. */
export const hideTranscript = () => useHive.setState({ transcriptShown: null });
export const setFocused = (focused: boolean) => useHive.setState({ focused });
export const toggleCollapsed = (id: string) =>
  useHive.setState((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } }));

/** A terminal (or a chat) just opened in `cwd`: its tab is shown and its worktree selected. */
export const addTab = (id: number, cwd: string, kind?: "chat") =>
  useHive.setState((s) => ({
    tabs: [...s.tabs, kind ? { id, cwd, kind } : { id, cwd }],
    activeTab: id,
    fileShown: false,
    transcriptShown: null,
    selection: cwd,
  }));
export const activateTab = (tab: Tab) =>
  useHive.setState((s) => ({
    activeTab: tab.id,
    fileShown: false,
    transcriptShown: null,
    selection: tabPlace(s, tab.cwd),
  }));
/**
 * Removes the tab; when it was shown, the other pane of its split is, else its right neighbour
 * among the shown place's tabs. Closing either pane ends the split.
 */
export const removeTab = (id: number) =>
  useHive.setState((s) => {
    const shown = visibleTabs(s);
    const i = shown.findIndex((t) => t.id === id);
    const rest = shown.filter((t) => t.id !== id);
    const split = s.split && [s.split.left, s.split.right].includes(id) ? s.split : null;
    const other = split && (split.left === id ? split.right : split.left);
    const next = other ?? rest[Math.min(i, rest.length - 1)]?.id ?? null;
    return {
      tabs: s.tabs.filter((t) => t.id !== id),
      activeTab: s.activeTab === id ? next : s.activeTab,
      split: split ? null : s.split,
    };
  });

/** The split shown: the stored one while the active tab is one of its panes, else none. */
export function shownSplit(s: HiveState): Split | null {
  const split = s.split;
  return split && (s.activeTab === split.left || s.activeTab === split.right) ? split : null;
}

/** The terminals the terminal area shows, left to right. */
export function shownTerminals(s: HiveState): number[] {
  const split = shownSplit(s);
  if (split) return [split.left, split.right];
  return s.activeTab === null ? [] : [s.activeTab];
}

/** Shows `left` and `right` side by side, `right` focused; null ends the split. */
export const setSplit = (split: Split | null) =>
  useHive.setState((s) => ({
    split,
    activeTab: split ? split.right : s.activeTab,
    fileShown: split ? false : s.fileShown,
    transcriptShown: split ? null : s.transcriptShown,
  }));

/** Keeps chat `id`'s data from its tab's start (`cwd`), or drops it (null) when the tab closes. */
export const setChat = (id: number, cwd: string | null) =>
  useHive.setState((s) => {
    if (cwd !== null) return patchChat(s, id, () => ({ cwd }));
    const { [id]: _, ...chats } = s.chats;
    return { chats };
  });

/** A click in a shown pane focuses it: it becomes the active tab, the one "in view". */
export const focusPane = (id: number) =>
  useHive.setState((s) => {
    const split = shownSplit(s);
    return split && (split.left === id || split.right === id) ? { activeTab: id } : {};
  });

/**
 * The worktree a tab (or the open file) at `path` belongs to: the deepest followed worktree
 * holding it (a Claude worktree lies inside its main one). A gone worktree's path falls to its
 * project's main worktree, whose id is the project's; a path outside every project is itself.
 */
export function tabPlace(s: HiveState, path: string): string {
  const inside = Object.values(s.projects ?? {})
    .flatMap((p) => p.worktrees)
    .filter((w) => path === w.path || path.startsWith(`${w.path}/`));
  return inside.sort((a, b) => b.path.length - a.path.length)[0]?.id ?? path;
}

/**
 * Whose tabs the tab bar shows: the selected worktree (a selected project stands for its main
 * worktree); a selected agent's terminal's worktree. Null (nothing selected) shows every tab.
 */
export function tabsPlace(s: HiveState): string | null {
  const agent = s.agents[s.selection ?? ""];
  if (!agent) return s.selection;
  const tab = s.tabs.find((t) => t.id === agent.terminal);
  return tab ? tabPlace(s, tab.cwd) : agent.worktree;
}

/** The terminal tabs of the place the tab bar shows, in the order they opened. */
export function visibleTabs(s: HiveState): Tab[] {
  const place = tabsPlace(s);
  return place === null ? s.tabs : s.tabs.filter((t) => tabPlace(s, t.cwd) === place);
}

/** Whether the open file's tab belongs to the place the tab bar shows. */
export function fileVisible(s: HiveState): boolean {
  const place = tabsPlace(s);
  return !!s.openFile && (place === null || tabPlace(s, s.openFile.worktree) === place);
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
  return find(selectedPlace(s)) ?? find(tab?.cwd) ?? null;
}

/** The space holding the project `id`. */
export const spaceOf = (s: HiveState, project: string | null): Space | undefined =>
  s.spaces?.find((space) => space.projects.includes(project ?? ""));

/** The current space. */
export const currentSpace = (s: HiveState): Space | undefined =>
  s.spaces?.find((space) => space.id === s.currentSpace);

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
const WRITING: AgentState[] = [
  "working",
  "with_subagents",
  "waiting_permission",
  "waiting_plan",
  "waiting_answer",
];

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
