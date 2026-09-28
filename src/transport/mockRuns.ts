import type { ServiceMessage } from "../protocol";
import type { RunAction, RunDetail, RunJob, RunSummary } from "../runs";

const SHOP = "/home/user/projects/shop";
const URL = "https://github.com/user/shop/actions/runs";

const run = (id: number, extra: Partial<RunSummary>): RunSummary => ({
  id,
  number: id - 200,
  workflow: "CI",
  title: "Fix the login redirect",
  branch: "main",
  event: "push",
  state: "passing",
  status: "success",
  url: `${URL}/${id}`,
  created_at: "2026-09-27T09:00:00Z",
  started_at: "2026-09-27T09:00:05Z",
  updated_at: "2026-09-27T09:04:35Z",
  worktree: null,
  ...extra,
});

/**
 * The fake service's Actions runs (9.32), newest first: shop's fix-login failed its tests,
 * feat-checkout's run is in progress, main passed. api has no GitHub repository.
 */
export const MOCK_RUNS: Record<string, RunSummary[]> = {
  [SHOP]: [
    run(302, {
      title: "New checkout flow",
      branch: "worktree-feat-checkout",
      event: "pull_request",
      state: "running",
      status: "in_progress",
      updated_at: "2026-09-27T09:10:00Z",
      worktree: `${SHOP}/.claude/worktrees/feat-checkout`,
    }),
    run(301, {
      branch: "worktree-fix-login",
      event: "pull_request",
      state: "failing",
      status: "failure",
      worktree: `${SHOP}/.claude/worktrees/fix-login`,
    }),
    run(300, { title: "Bump dependencies", worktree: SHOP }),
    run(299, { workflow: "Release", title: "v0.3.0", state: "skipped", status: "cancelled" }),
  ],
};

/** The job's log tail, as the service sends it (timestamps and escapes gone). */
export const MOCK_LOG = [
  "Run bun test",
  "bun test v1.2.0",
  "src/login.test.ts:",
  "✓ redirects home after login",
  "✗ keeps the page asked for [3.00ms]",
  "  expect(received).toBe(expected)",
  '  Expected: "/orders"',
  '  Received: "/"',
  " 1 pass",
  " 1 fail",
  "##[error]Process completed with exit code 1.",
].join("\n");

/** A run's jobs: lint passed; test is as the run is. */
const jobs = (r: RunSummary): RunJob[] => {
  const job = (id: number, name: string, state: RunSummary["state"], status: string) => ({
    id,
    name,
    state,
    status,
    url: `${r.url}/job/${id}`,
    started_at: r.started_at,
    completed_at: r.state === "running" ? "0001-01-01T00:00:00Z" : r.updated_at,
    steps: [
      { number: 1, name: "Set up job", state: "passing" as const, status: "success" },
      { number: 2, name: `Run bun ${name}`, state, status },
    ],
  });
  return [
    job(r.id * 10, "lint", "passing", "success"),
    job(r.id * 10 + 1, "test", r.state, r.status),
  ];
};

/** The Actions side of the mock transport, as the service answers (`later` sends a message). */
export function createMockRuns(later: (message: ServiceMessage) => void) {
  const lists = structuredClone(MOCK_RUNS);
  const find = (project: string, id: number) => lists[project]?.find((r) => r.id === id);
  const sendList = (project: string, branch: string | null) => {
    const list = lists[project];
    const runs = (list ?? []).filter((r) => branch === null || r.branch === branch);
    const error = list ? null : "No remote of this repository is on github.com";
    later({
      type: "runs",
      project,
      branch,
      runs: structuredClone(runs),
      fetched_ms: Date.now(),
      error,
    });
  };
  const sendRun = (project: string, id: number) => {
    const found = find(project, id);
    const detail: RunDetail | null = found
      ? { summary: structuredClone(found), jobs: jobs(found) }
      : null;
    later({ type: "run", project, run: id, detail, error: found ? null : `no run ${id}` });
  };
  return {
    async listRuns(project: string, branch: string | null, _force: boolean) {
      sendList(project, branch);
    },
    async openRun(project: string, id: number) {
      sendRun(project, id);
    },
    async openJobLog(project: string, job: number) {
      later({ type: "job_log", project, job, log: MOCK_LOG, error: null });
    },
    async actOnRun(project: string, id: number, action: RunAction, branch: string | null) {
      const found = find(project, id);
      if (!found || (action.kind === "cancel" && found.state !== "running")) {
        const message = `gh run ${action.kind} failed: Cannot ${action.kind} a workflow run that is completed`;
        return later({ type: "run_failed", project, run: id, message });
      }
      const running = action.kind === "rerun";
      found.state = running ? "running" : "skipped";
      found.status = running ? "queued" : "cancelled";
      const message = running
        ? action.failed
          ? "Re-running the failed jobs"
          : "Re-running every job"
        : "Cancelling the run";
      later({ type: "run_done", project, run: id, message });
      sendRun(project, id);
      sendList(project, branch);
    },
  };
}
