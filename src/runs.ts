import type { CheckState } from "./pulls";
import { type HiveState, useHive } from "./store";
import { transport } from "./transport";

// Mirror `hive_protocol`'s Actions run types (9.32).
export type RunSummary = {
  id: number;
  number: number;
  workflow: string;
  title: string;
  branch: string;
  event: string;
  state: CheckState;
  /** GitHub's conclusion once completed, else its status (`in_progress`, `cancelled`…). */
  status: string;
  url: string;
  /** ISO 8601. */
  created_at: string;
  started_at: string;
  updated_at: string;
  /** The project's worktree on its branch, if any. */
  worktree: string | null;
};
export type RunStep = { number: number; name: string; state: CheckState; status: string };
export type RunJob = {
  id: number;
  name: string;
  state: CheckState;
  status: string;
  url: string;
  started_at: string;
  completed_at: string;
  steps: RunStep[];
};
export type RunDetail = { summary: RunSummary; jobs: RunJob[] };
export type RunAction = { kind: "rerun"; failed: boolean } | { kind: "cancel" };
/** The `runs` message: a project's latest runs (on `branch` only, when not null). */
export type Runs = {
  project: string;
  branch: string | null;
  runs: RunSummary[];
  fetched_ms: number;
  error: string | null;
};
/** The run whose jobs show, with them once they arrive. */
export type OpenRun = {
  project: string;
  run: number;
  detail: RunDetail | null;
  error: string | null;
};
/** The job whose log tail shows, with it once it arrives. */
export type JobLog = { project: string; job: number; log: string | null; error: string | null };
/** An action on `run` sent and not answered yet. */
export type RunBusy = { project: string; run: number };
/** Why the last action on `run` failed (`gh`'s message). */
export type RunError = { project: string; run: number; message: string };

/**
 * How often a running run's jobs are asked for while they show (9.32); the list refreshes as
 * the pull requests' does (`PULLS_INTERVAL_MS`).
 */
export const RUN_INTERVAL_MS = 30_000;

/**
 * How long from `start` to `end` (now while null), as "42s", "3m 12s" or "1h 5m"; nothing when
 * either cannot be read or it has not started (GitHub's zero time is year 1).
 */
export function duration(start: string, end: string | null, now = Date.now()): string | null {
  const from = Date.parse(start);
  const to = end === null ? now : Date.parse(end);
  const s = Math.round((to - from) / 1000);
  if (!(s >= 0) || from < 0) return null;
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

/** GitHub's `in_progress` as "in progress". */
export const words = (status: string) => status.replaceAll("_", " ");

/** Shows run `run`'s jobs and asks for them. */
export function showRun(project: string, run: number): void {
  useHive.setState({ openRun: { project, run, detail: null, error: null }, runError: null });
  void transport.openRun(project, run);
}

/** Back to the list. */
export const hideRun = () => useHive.setState({ openRun: null, runError: null, jobLog: null });

/** Shows the end of job `job`'s log and asks for it. */
export function showJobLog(project: string, job: number): void {
  useHive.setState({ jobLog: { project, job, log: null, error: null } });
  void transport.openJobLog(project, job);
}

export const hideJobLog = () => useHive.setState({ jobLog: null });

/** Re-runs or cancels `run`; the `branch` list comes again after it. */
export function actOnRun(
  project: string,
  run: number,
  action: RunAction,
  branch: string | null,
): void {
  useHive.setState({ runBusy: { project, run }, runError: null });
  void transport.actOnRun(project, run, action, branch);
}

/**
 * The latest run of the worktree `id`'s branch in its project's list of all branches, with its
 * project (select it with `useShallow`).
 */
export function runOf(s: HiveState, id: string): [string, RunSummary] | [] {
  for (const list of Object.values(s.runs)) {
    const found = list.branch === null && list.runs.find((r) => r.worktree === id);
    if (found) return [list.project, found];
  }
  return [];
}
