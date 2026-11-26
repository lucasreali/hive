import type { HostMessage, ServiceMessage, Space } from "./protocol";

// Remote hosts in the UI (26.18, spike §5, Q1): a space belongs to one host. The store holds the
// shown host's service data and stashes the others' (`HOST_FIELDS`); this module names a host's
// messages, agents and spaces so that no two hosts' ids or paths ever meet.

/**
 * The messages the UI takes from a remote host's service (26.18), each applied to that host's
 * data only (`apply` in `reduce.ts`). It mirrors `REMOTE_TO_UI` in `src-tauri/src/lib.rs`, which
 * drops every other one before the app side acts on it; `hosts.test.ts` keeps the two equal.
 * Never the local-only ones: the settings, an uninstall, a path to open (`editor_target`,
 * `session_located`), the sessions to restore, or anything the app side itself sends.
 */
export const REMOTE_TYPES = [
  "terminal_opened",
  "terminal_exited",
  "error",
  "badge",
  "command_finished",
  "terminal_progress",
  "unhooked_agent",
  "terminal_notification",
  "agent_detected",
  "agent_removed",
  "agent_title",
  "agent_state",
  "agent_usage",
  "subagent_worktrees",
  "session_usage",
  "spaces",
  "space_failed",
  "projects",
  "project_added",
  "add_project_failed",
  "project_created",
  "project_removed",
  "remove_project_failed",
  "dirs",
  "dir_created",
  "create_dir_failed",
  "branches",
  "worktree_name_validated",
  "worktree_created",
  "create_worktree_failed",
  "worktree_removed",
  "remove_worktree_failed",
  "worktree_renamed",
  "rename_worktree_failed",
  "bring_back_plan",
  "brought_back",
  "bring_back_failed",
  "worktree_status",
  "sessions",
  "session_deleted",
  "delete_session_failed",
  "line_author",
  "session_files",
  "files",
  "changes",
  "commits",
  "scripts",
  "search_results",
  "file",
  "table",
  "table_edited",
  "file_saved",
  "save_failed",
  "file_created",
  "file_renamed",
  "folder_created",
  "file_deleted",
  "file_op_failed",
  "pulls",
  "pull",
  "pull_done",
  "pull_failed",
  "pull_ended",
  "runs",
  "run",
  "job_log",
  "run_done",
  "run_failed",
  "gh_accounts",
  "gh_identity",
  "gh_identity_failed",
] as const satisfies readonly ServiceMessage["type"][];

/** What the app side itself says of a remote link, tagged with its host (26.17). */
const LINK_TYPES: readonly ServiceMessage["type"][] = [
  "host_link",
  "terminal_kept",
  "terminal_reattached",
];

/**
 * Whether the UI takes `message`: any of the local service's, and of a remote's only
 * `REMOTE_TYPES` and what its link says. The app side already drops the rest; this keeps the
 * UI from acting on one, whatever let it through.
 */
export const taken = (message: HostMessage): boolean =>
  message.host === undefined ||
  (REMOTE_TYPES as readonly string[]).includes(message.type) ||
  LINK_TYPES.includes(message.type);

/** The messages naming an agent by `id`. */
const AGENT_TYPES: readonly ServiceMessage["type"][] = [
  "agent_detected",
  "agent_removed",
  "agent_title",
  "agent_state",
  "agent_usage",
];

/**
 * The store's key of agent `id` of `host` (26.18): a remote's is `<host> <id>`, so it never
 * meets a local agent's (a session id, which has no space) or another host's (an alias has
 * none either). The local service's agents keep their ids.
 */
export const agentKey = (host: string | null | undefined, id: string): string =>
  host ? `${host} ${id}` : id;

/** The host of the agent keyed `key` (`agentKey`): null for the local service's. */
export function agentHost(key: string): string | null {
  const space = key.indexOf(" ");
  return space < 0 ? null : key.slice(0, space);
}

/** `message` with a remote's agent ids as the store keys them (`agentKey`). */
export function keyed(message: HostMessage): HostMessage {
  if (message.host === undefined || !AGENT_TYPES.includes(message.type)) return message;
  const m = message as HostMessage & { id: string };
  return { ...m, id: agentKey(message.host, m.id) } as HostMessage;
}

/**
 * How a space is named where hosts mix (26.18): "Work" on this Mac, "Work · gpu-box" on a
 * remote, whose host is always named.
 */
export const spaceLabel = (space: Pick<Space, "name"> | undefined, host: string | null) =>
  [space?.name, host].filter(Boolean).join(" · ");
