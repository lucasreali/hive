import type { PullDetail, Pulls } from "./pulls";
import type { JobLog, RunDetail, Runs } from "./runs";

// The service's messages to the app and what they carry: `hive_protocol::Control` as the app
// reads it. `protocol.test.ts` checks them against the samples the Rust side writes
// (`crates/hive-protocol/app-messages.json`).

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
  // `protocol`/`version` are the service's; `app_*` and `bundled` are added by the app side (Rust).
  // `bundled`: the app runs the `hive` its installer brought (not a development build).
  | {
      type: "version_mismatch";
      protocol: number;
      version: string;
      app_protocol: number;
      app_version: string;
      bundled: boolean;
    }
  // `worktree`: the followed worktree the service placed the terminal's cwd in, null outside.
  | { type: "terminal_opened"; channel: number; worktree: string | null }
  | { type: "terminal_exited"; channel: number; code: number | null }
  | { type: "unhooked_agent"; channel: number }
  // `hive badge` in that terminal; empty clears it.
  | { type: "badge"; channel: number; text: string }
  | ({ type: "agent_detected"; channel: number } & Omit<Agent, "terminal">)
  | { type: "agent_removed"; channel: number; id: string }
  | { type: "agent_title"; channel: number; id: string; title: string }
  | ({ type: "agent_state"; id: string } & AgentStatus)
  | ({ type: "agent_usage"; id: string } & AgentUsage)
  | { type: "subagent_worktrees"; worktrees: string[] }
  | { type: "projects"; projects: Project[] }
  | { type: "project_added"; project: Project }
  | { type: "add_project_failed"; path: string; error: ProjectError; message: string }
  | { type: "project_removed"; id: string }
  | { type: "remove_project_failed"; id: string; message: string }
  | { type: "spaces"; spaces: Space[]; current: string }
  | { type: "space_failed"; message: string }
  | ({ type: "gh_accounts" } & GhAccounts)
  | { type: "notice"; message: string }
  | ({ type: "pulls" } & Pulls)
  | { type: "pull"; project: string; number: number; pull: PullDetail | null; error: string | null }
  | { type: "pull_done"; project: string; number: number; message: string }
  | { type: "pull_failed"; project: string; number: number | null; message: string }
  | ({ type: "runs" } & Runs)
  | { type: "run"; project: string; run: number; detail: RunDetail | null; error: string | null }
  | ({ type: "job_log" } & JobLog)
  | { type: "run_done"; project: string; run: number; message: string }
  | { type: "run_failed"; project: string; run: number; message: string }
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
  | { type: "sessions"; sessions: Session[]; error: string | null; truncated: boolean }
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
  | { type: "file_deleted"; worktree: string; path: string }
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
  // Sent by the app side (Rust) when the bridge exits or its output closes.
  | { type: "disconnected"; reason: string; bundled: boolean };

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
 * Mirrors `hive_protocol::WorktreeStatus`: files changed and their lines added and removed
 * against HEAD, commits ahead of and behind the main worktree's branch (null for the main
 * worktree), all merged there, and the last commit's time.
 */
export type WorktreeStatus = {
  changes: number;
  added: number;
  removed: number;
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
  /** The `gh` account whose token its terminals get (9.30); null: `gh`'s active account. */
  gh_account: GhAccount | null;
};

/** Mirrors `hive_protocol::GhAccount`: a `gh` login on a host. */
export type GhAccount = { host: string; login: string };
/** Mirrors `hive_protocol::GhLogin`: an account as `gh auth status` lists it. */
export type GhLogin = GhAccount & { active: boolean; logged_in: boolean };
/** The accounts of `gh` in a config folder, logins only; `problem` says what went wrong. */
export type GhAccounts = {
  gh_config_dir: string | null;
  accounts: GhLogin[];
  problem: string | null;
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

/**
 * What a worktree's changes are compared with (9.11): its HEAD, or the merge-base with the main
 * worktree's branch (the service finds it).
 */
export type DiffBase = "head" | "branch";

/** A worktree's changes against its base, sorted by path, with the service's totals. */
export type Changes = {
  path: string;
  /** The base used: the one asked, or "head" when `base_error` says why not. */
  base: DiffBase;
  /** The main worktree's branch it can be compared with; null for the main worktree. */
  branch: string | null;
  base_error: string | null;
  files: ChangedFile[];
  added: number;
  removed: number;
  error: string | null;
};

/** Mirrors `hive_protocol::OpenSession`: a session that ran in a Hive terminal. */
export type OpenSession = { id: string; cwd: string };

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
  /** A `claude` is known to run it: in a Hive terminal (see `agents`), or outside Hive. */
  running: boolean;
};
export type SessionTarget = "log" | "folder";

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
  /** Its state may be writing files, in its own worktree. */
  writing: boolean;
} & Doing;

/** What an agent or subagent is doing (its current tool call) and since when (ms since the epoch) its state lasts. */
export type Doing = { activity: string | null; since_ms: number };

/** Mirrors `hive_protocol::Alert`: why a new state alerts (`notify`). */
export type Alert = "finished" | "waiting";

/**
 * What `agent_state` says about an agent, stored by its session id. `urgency` (higher wins),
 * `pending` (needs the user), `alert` and `writing` are the service's, so the app keeps no
 * table of its own.
 */
export type AgentStatus = {
  state: AgentState;
  urgency: number;
  pending: boolean;
  /** It waits for you because the user interrupted it: nothing alerts. */
  interrupted: boolean;
  /** Set only on the message whose state changed; never on the snapshot after `welcome`. */
  alert: Alert | null;
  /** Its state may be writing files, in its worktree: "Agent working here". */
  writing: boolean;
  subagents: Subagent[];
} & Doing;

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
  projects: Record<string, { scripts: ProjectScripts }>;
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

/** What the settings' About section shows (the service's `diagnostics`). */
export type Diagnostics = {
  settings_file: string;
  /** The `claude` wrapper Hive terminals run first. */
  wrapper: string;
  /** The `claude` it runs, as found on the service's `PATH`; null when none is. */
  claude: string | null;
};
