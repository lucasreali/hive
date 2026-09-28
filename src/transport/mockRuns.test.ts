import { expect, test } from "bun:test";
import type { ServiceMessage } from "../protocol";
import { createMockTransport, MOCK_REPOS } from "./mock";
import { MOCK_LOG, MOCK_RUNS } from "./mockRuns";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const [shop, api] = MOCK_REPOS;
const [, login] = shop.worktrees;

async function connected() {
  const transport = createMockTransport();
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  messages.length = 0;
  return { transport, messages };
}

test("shop lists its runs, on one branch or all; api has no GitHub remote", async () => {
  const { transport, messages } = await connected();
  await transport.listRuns(shop.id, null, false);
  await transport.listRuns(shop.id, "worktree-fix-login", false);
  await transport.listRuns(api.id, null, true);
  await tick();
  const [all, one, none] = messages;
  expect(all).toMatchObject({ type: "runs", project: shop.id, branch: null, error: null });
  if (all?.type !== "runs" || one?.type !== "runs") throw new Error("expected runs");
  expect(all.runs).toEqual(MOCK_RUNS[shop.id] ?? []);
  expect(one.runs.map((r) => [r.id, r.worktree])).toEqual([[301, login.id]]);
  expect(none).toMatchObject({
    type: "runs",
    runs: [],
    error: "No remote of this repository is on github.com",
  });
});

test("a run's jobs, a job's log, re-runs and cancels", async () => {
  const { transport, messages } = await connected();
  await transport.openRun(shop.id, 301);
  await transport.openRun(shop.id, 1);
  await transport.openJobLog(shop.id, 3011);
  await tick();
  const [run, missing, log] = messages;
  if (run?.type !== "run") throw new Error("expected a run");
  expect(run.detail?.jobs.map((j) => [j.name, j.state])).toEqual([
    ["lint", "passing"],
    ["test", "failing"],
  ]);
  expect(missing).toMatchObject({ type: "run", run: 1, detail: null, error: "no run 1" });
  expect(log).toEqual({ type: "job_log", project: shop.id, job: 3011, log: MOCK_LOG, error: null });

  messages.length = 0;
  await transport.actOnRun(shop.id, 301, { kind: "cancel" }, null);
  await transport.actOnRun(shop.id, 301, { kind: "rerun", failed: true }, null);
  await transport.actOnRun(shop.id, 301, { kind: "cancel" }, "worktree-fix-login");
  await transport.actOnRun(shop.id, 302, { kind: "rerun", failed: false }, null);
  await tick();
  const types = messages.map((m) => m.type);
  expect(types).toEqual([
    "run_failed",
    "run_done",
    "run",
    "runs",
    "run_done",
    "run",
    "runs",
    "run_done",
    "run",
    "runs",
  ]);
  expect(messages.filter((m) => m.type === "run_done").map((m) => m.message)).toEqual([
    "Re-running the failed jobs",
    "Cancelling the run",
    "Re-running every job",
  ]);
  const cancelled = messages[5];
  expect(cancelled).toMatchObject({ type: "run", run: 301 });
  if (cancelled?.type !== "run") throw new Error("expected a run");
  expect(cancelled.detail?.summary.status).toBe("cancelled");
  expect(messages[6]).toMatchObject({ type: "runs", branch: "worktree-fix-login" });
});
