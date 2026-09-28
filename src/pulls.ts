import { type HiveState, openModal, useHive } from "./store";
import { transport } from "./transport";

// Mirror `hive_protocol`'s pull request types (9.31).
export type MergeMethod = "merge" | "squash" | "rebase";
export type PullState = "open" | "draft" | "merged" | "closed";
export type ReviewDecision = "approved" | "changes_requested" | "review_required";
export type ReviewState = "approved" | "changes_requested" | "commented" | "dismissed";
export type CheckState = "passing" | "failing" | "running" | "skipped";
export type PullRepo = {
  name: string;
  url: string;
  merge_methods: MergeMethod[];
  default_merge: MergeMethod | null;
};
export type PullSummary = {
  number: number;
  title: string;
  url: string;
  state: PullState;
  branch: string;
  base: string;
  author: string;
  review: ReviewDecision | null;
  checks: CheckState | null;
  /** ISO 8601. */
  updated_at: string;
  /** The project's worktree on its branch, if any. */
  worktree: string | null;
};
export type PullNote = {
  author: string;
  body: string;
  at: string;
  review: ReviewState | null;
};
export type PullCheck = {
  name: string;
  workflow: string | null;
  state: CheckState;
  url: string | null;
  /** Its Actions run, when its page is one (the Actions view shows it). */
  run: number | null;
};
export type PullFile = { path: string; additions: number; deletions: number };
export type PullDetail = {
  summary: PullSummary;
  body: string;
  head: string;
  additions: number;
  deletions: number;
  conflicts: boolean;
  notes: PullNote[];
  checks: PullCheck[];
  files: PullFile[];
};
export type PullAction =
  | { kind: "ready" }
  | { kind: "close" }
  | { kind: "merge"; method: MergeMethod; head: string }
  | { kind: "checkout" };
/** The `pulls` message: a project's pull requests, or why there are none. */
export type Pulls = {
  project: string;
  repo: PullRepo | null;
  mine: PullSummary[];
  review: PullSummary[];
  fetched_ms: number;
  error: string | null;
};
/** The pull request whose details show, with them once they arrive. */
export type OpenPull = {
  project: string;
  number: number;
  detail: PullDetail | null;
  error: string | null;
};
/** An action sent and not answered yet; `number` is null while creating one. */
export type PullBusy = {
  project: string;
  number: number | null;
  action: PullAction["kind"] | "create";
};
/** Why the last action failed (`gh`'s message). */
export type PullError = { project: string; number: number | null; message: string };

/**
 * How often the view asks for the list while it shows; the service never asks GitHub more
 * often, whoever asks (`hive::pulls::INTERVAL`).
 */
export const PULLS_INTERVAL_MS = 120_000;

/** Shows pull request `number`'s details and asks for them. */
export function showPull(project: string, number: number): void {
  useHive.setState({ openPull: { project, number, detail: null, error: null }, pullError: null });
  void transport.openPull(project, number);
}

/** The "Create pull request" dialog for the worktree `worktree` of `project`. */
export function newPull(project: string, worktree: string): void {
  openModal("new-pull", project, worktree);
  useHive.setState({ pullError: null });
}

/** Back to the list. */
export const hidePull = () => useHive.setState({ openPull: null, pullError: null });

export function actOnPull(project: string, number: number, action: PullAction): void {
  useHive.setState({ pullBusy: { project, number, action: action.kind }, pullError: null });
  void transport.actOnPull(project, number, action);
}

export const setPullBusy = (pullBusy: PullBusy | null) => useHive.setState({ pullBusy });

/**
 * The pull request of the worktree `id` in the lists last sent, if one names it, with its
 * project (select it with `useShallow`).
 */
export function pullOf(s: HiveState, id: string): [string, PullSummary] | [] {
  for (const p of Object.values(s.pulls)) {
    const found = [...p.mine, ...p.review].find((x) => x.worktree === id);
    if (found) return [p.project, found];
  }
  return [];
}
