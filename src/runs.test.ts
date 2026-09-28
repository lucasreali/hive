import { afterEach, expect, spyOn, test } from "bun:test";
import { apply } from "./reduce";
import {
  actOnRun,
  duration,
  hideJobLog,
  hideRun,
  type Runs,
  runOf,
  showJobLog,
  showRun,
  words,
} from "./runs";
import { initialState, runsKey, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_RUNS } from "./transport/mockRuns";

afterEach(() => useHive.setState(initialState, true));

const SHOP = "/home/user/projects/shop";
const all: Runs = {
  project: SHOP,
  branch: null,
  runs: MOCK_RUNS[SHOP] ?? [],
  fetched_ms: 1,
  error: null,
};

test("lists are kept by project and branch; badges read the all-branches one", () => {
  const fixLogin = `${SHOP}/.claude/worktrees/fix-login`;
  // A mock transport of another test file may still answer after its reset: start empty.
  useHive.setState({ runs: {} });
  apply({ type: "runs", ...all, branch: "worktree-fix-login", runs: [] });
  // A branch's own list names no badge.
  expect(runOf(useHive.getState(), fixLogin)).toEqual([]);
  apply({ type: "runs", ...all });
  const s = useHive.getState();
  expect(s.runs[runsKey(SHOP, null)]).toEqual(all);
  expect(s.runs[runsKey(SHOP, "worktree-fix-login")]?.runs).toEqual([]);
  expect(runOf(s, fixLogin)).toEqual([SHOP, all.runs[1]]);
  expect(runOf(s, "/elsewhere")).toEqual([]);
});

test("jobs and logs show for the open run and job only; actions wait for their answer", () => {
  const open = spyOn(transport, "openRun").mockResolvedValue();
  const log = spyOn(transport, "openJobLog").mockResolvedValue();
  const act = spyOn(transport, "actOnRun").mockResolvedValue();
  useHive.setState({ runError: { project: SHOP, run: 1, message: "old" } });
  showRun(SHOP, 301);
  expect(open).toHaveBeenCalledWith(SHOP, 301);
  expect(useHive.getState().runError).toBeNull();
  const answer = { type: "run", project: SHOP, detail: null, error: "gone" } as const;
  apply({ ...answer, run: 300 });
  apply({ ...answer, project: "/api", run: 301 });
  expect(useHive.getState().openRun).toEqual({
    project: SHOP,
    run: 301,
    detail: null,
    error: null,
  });
  apply({ ...answer, run: 301 });
  expect(useHive.getState().openRun?.error).toBe("gone");

  showJobLog(SHOP, 3011);
  expect(log).toHaveBeenCalledWith(SHOP, 3011);
  const tail = { type: "job_log", project: SHOP, log: "end", error: null } as const;
  apply({ ...tail, job: 3010 });
  apply({ ...tail, project: "/api", job: 3011 });
  expect(useHive.getState().jobLog?.log).toBeNull();
  apply({ ...tail, job: 3011 });
  expect(useHive.getState().jobLog?.log).toBe("end");
  hideJobLog();
  expect(useHive.getState().jobLog).toBeNull();

  actOnRun(SHOP, 301, { kind: "cancel" }, "main");
  expect(act).toHaveBeenCalledWith(SHOP, 301, { kind: "cancel" }, "main");
  expect(useHive.getState().runBusy).toEqual({ project: SHOP, run: 301 });
  apply({ type: "run_failed", project: SHOP, run: 301, message: "no" });
  expect(useHive.getState().runBusy).toBeNull();
  expect(useHive.getState().runError).toEqual({ project: SHOP, run: 301, message: "no" });
  actOnRun(SHOP, 301, { kind: "rerun", failed: true }, null);
  expect(useHive.getState().runError).toBeNull();
  apply({ type: "run_done", project: SHOP, run: 301, message: "Re-running the failed jobs" });
  const s = useHive.getState();
  expect([s.runBusy, s.notices.at(-1)]).toMatchObject([
    null,
    { kind: "info", text: "Re-running the failed jobs" },
  ]);

  showJobLog(SHOP, 1);
  hideRun();
  expect([useHive.getState().openRun, useHive.getState().jobLog]).toEqual([null, null]);
  open.mockRestore();
  log.mockRestore();
  act.mockRestore();
});

test("durations and GitHub's words read as people say them", () => {
  const start = "2026-09-27T09:00:00Z";
  expect(duration(start, "2026-09-27T09:00:42Z")).toBe("42s");
  expect(duration(start, "2026-09-27T09:03:12Z")).toBe("3m 12s");
  expect(duration(start, "2026-09-27T10:05:30Z")).toBe("1h 5m");
  // Still running: until now.
  expect(duration(start, null, Date.parse("2026-09-27T09:01:00Z"))).toBe("1m 0s");
  // Not started, not finished (GitHub's zero time), or unreadable: nothing.
  expect(duration("0001-01-01T00:00:00Z", null)).toBeNull();
  expect(duration(start, "0001-01-01T00:00:00Z")).toBeNull();
  expect(duration("soon", null)).toBeNull();
  expect(words("in_progress")).toBe("in progress");
});
