import { beforeEach, expect, test } from "bun:test";
import samples from "../crates/hive-protocol/tests/app-messages.json";
import type { ServiceMessage } from "./protocol";
import { apply } from "./reduce";
import { initialState, useHive } from "./store";

// The protocol's contract (9.25): the Rust side writes one sample of every message the app
// receives (`crates/hive-protocol/tests/app_messages.rs`); each must be a type the app knows,
// with exactly the fields its type says.

type Fields<M> = { [F in Exclude<keyof M, "type">]-?: true };
/** Every message type with its fields, as `ServiceMessage` says: the typecheck keeps it exact. */
const FIELDS: { [T in ServiceMessage["type"]]: Fields<Extract<ServiceMessage, { type: T }>> } = {
  welcome: { version: true, distro: true },
  settings: { settings: true },
  settings_failed: { message: true },
  diagnostics: { settings_file: true, wrapper: true, claude: true },
  update_ready: { version: true },
  update_failed: { error: true },
  version_mismatch: { protocol: true, version: true, app_protocol: true, app_version: true },
  terminal_opened: { channel: true, worktree: true },
  terminal_exited: { channel: true, code: true },
  unhooked_agent: { channel: true },
  badge: { channel: true, text: true },
  agent_detected: { channel: true, id: true, project: true, worktree: true, cwd: true },
  agent_removed: { channel: true, id: true },
  agent_title: { channel: true, id: true, title: true },
  agent_state: {
    id: true,
    state: true,
    urgency: true,
    pending: true,
    interrupted: true,
    alert: true,
    writing: true,
    subagents: true,
    activity: true,
    since_ms: true,
  },
  agent_usage: { id: true, context_tokens: true, context_limit: true, output_tokens: true },
  subagent_worktrees: { worktrees: true },
  projects: { projects: true },
  project_added: { project: true },
  add_project_failed: { path: true, error: true, message: true },
  project_removed: { id: true },
  remove_project_failed: { id: true, message: true },
  spaces: { spaces: true, current: true },
  space_failed: { message: true },
  gh_accounts: { gh_config_dir: true, accounts: true, problem: true },
  notice: { message: true },
  pulls: { project: true, repo: true, mine: true, review: true, fetched_ms: true, error: true },
  pull: { project: true, number: true, pull: true, error: true },
  pull_done: { project: true, number: true, message: true },
  pull_failed: { project: true, number: true, message: true },
  runs: { project: true, branch: true, runs: true, fetched_ms: true, error: true },
  run: { project: true, run: true, detail: true, error: true },
  job_log: { project: true, job: true, log: true, error: true },
  run_done: { project: true, run: true, message: true },
  run_failed: { project: true, run: true, message: true },
  branches: { project: true, local: true, remote: true, current: true, error: true },
  worktree_name_validated: { project: true, name: true, folder: true, branch: true, error: true },
  worktree_created: { project: true, path: true, notes: true },
  create_worktree_failed: { project: true, name: true, message: true },
  worktree_removed: { project: true, path: true },
  remove_worktree_failed: { path: true, message: true },
  worktree_renamed: { project: true, from: true, path: true },
  rename_worktree_failed: { path: true, name: true, message: true },
  worktree_status: { path: true, status: true },
  files: { path: true, files: true, truncated: true },
  error: { message: true },
  changes: {
    path: true,
    base: true,
    branch: true,
    base_error: true,
    files: true,
    added: true,
    removed: true,
    error: true,
  },
  file: {
    worktree: true,
    path: true,
    content: true,
    base: true,
    version: true,
    binary: true,
    too_large: true,
    error: true,
  },
  search_results: { worktree: true, query: true, matches: true, truncated: true, error: true },
  dirs: { path: true, windows: true, linux_path: true, parent: true, dirs: true, error: true },
  sessions: { sessions: true, error: true, truncated: true },
  session_located: { id: true, target: true, windows_path: true, error: true },
  session_deleted: { id: true },
  restore_sessions: { sessions: true },
  delete_session_failed: { id: true, message: true },
  file_saved: { worktree: true, path: true, version: true },
  save_failed: { worktree: true, path: true, error: true, message: true },
  file_created: { worktree: true, path: true },
  file_renamed: { worktree: true, path: true, to: true },
  folder_created: { worktree: true, path: true },
  file_deleted: { worktree: true, path: true },
  file_op_failed: { worktree: true, message: true },
  editor_target: { worktree: true, path: true, windows_path: true, error: true },
  disconnected: { reason: true },
};

/** Sent by the app's own Rust side, never by the service. */
const APP_SIDE = ["update_ready", "update_failed", "disconnected"];

/** A service message as the UI gets it: the Tauri pump (`src-tauri/src/lib.rs`) adds these. */
function fromPump(sample: { type: string }) {
  const versions = { app_protocol: 1, app_version: "0.4.0" };
  return { ...sample, channel: 1, ...(sample.type === "version_mismatch" ? versions : {}) };
}

/** A message's fields, sorted; `channel` is on every one the pump forwards. */
const fields = (m: object) =>
  Object.keys({ ...m, channel: true })
    .filter((k) => k !== "type")
    .sort();

beforeEach(() => useHive.setState(initialState, true));

test("every message the service sends is one the app knows, with the fields its type says", () => {
  const sent = Object.fromEntries(samples.map((m) => [m.type, fields(fromPump(m))]));
  const known = Object.fromEntries(
    Object.entries(FIELDS)
      .filter(([type]) => !APP_SIDE.includes(type))
      .map(([type, f]) => [type, fields(f)]),
  );
  expect(sent).toEqual(known);
});

test("the reducer takes every sample, and a type it does not know changes nothing", () => {
  for (const m of samples) apply(fromPump(m) as ServiceMessage);
  const s = useHive.getState();
  expect(s.connection.status).toBe("version_mismatch");
  expect(Object.keys(s.projects ?? {})).toEqual(["/r"]);
  expect(s.worktreeFiles).toMatchObject({ path: "/r", files: ["a.rs"], truncated: false });
  apply({ type: "from_a_newer_service" } as unknown as ServiceMessage);
  expect(useHive.getState()).toEqual(s);
});
