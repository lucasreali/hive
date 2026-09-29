import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { notice } from "../../test/notice";
import { App } from "../App";
import { PULLS_INTERVAL_MS } from "../pulls";
import { apply } from "../reduce";
import { RUN_INTERVAL_MS, type RunDetail, type RunSummary, type Runs } from "../runs";
import { initialState, select, setPanelView, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { MOCK_LOG, MOCK_RUNS } from "../transport/mockRuns";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [main, login, checkout] = shop.worktrees;
const all = MOCK_RUNS[shop.id] ?? [];
const [running, failed] = all as [RunSummary, RunSummary];
const runs = (branch: string | null, list: RunSummary[]): Runs => ({
  project: shop.id,
  branch,
  runs: list,
  fetched_ms: Date.now(),
  error: null,
});

const detail = (summary: RunSummary): RunDetail => ({
  summary,
  jobs: [
    {
      id: 11,
      name: "lint",
      state: "passing",
      status: "success",
      url: "",
      started_at: "2026-09-27T09:00:05Z",
      completed_at: "2026-09-27T09:00:47Z",
      steps: [{ number: 1, name: "Set up job", state: "passing", status: "success" }],
    },
    {
      id: 12,
      name: "test",
      state: summary.state,
      status: summary.status,
      url: "",
      started_at: "2026-09-27T09:00:05Z",
      completed_at: "0001-01-01T00:00:00Z",
      steps: [{ number: 2, name: "Run bun test", state: summary.state, status: summary.status }],
    },
  ],
});

/** The app with shop, `worktree` selected and the right panel on Actions. */
function show(worktree = login.id) {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  act(() => {
    select(worktree);
    setPanelView("actions");
  });
}

const view = () => screen.getByRole("region", { name: "Actions" });

test("the list asks for the worktree's branch when shown and every two minutes, never faster", () => {
  const list = spyOn(transport, "listRuns").mockResolvedValue();
  const every = spyOn(globalThis, "setInterval");
  const stop = spyOn(globalThis, "clearInterval");
  show();
  expect(list.mock.calls).toEqual([[shop.id, login.branch, false]]);
  expect(within(view()).getByText("Loading…")).toBeTruthy();
  const timer = every.mock.calls.find(([, ms]) => ms === PULLS_INTERVAL_MS);
  const tick = timer?.[0] as () => void;
  tick();
  expect(list).toHaveBeenCalledTimes(2);
  // On demand: GitHub is asked now.
  act(() => apply({ type: "runs", ...runs(login.branch, [failed]) }));
  fireEvent.click(within(view()).getByTitle("Refresh"));
  expect(list).toHaveBeenLastCalledWith(shop.id, login.branch, true);
  // Another branch, or all: its own list.
  fireEvent.mouseDown(within(view()).getByRole("combobox", { name: "Branch" }));
  const options = screen.getAllByRole("option").map((o) => o.textContent);
  expect(options).toEqual(["worktree-fix-login", "main", "worktree-feat-checkout", "All branches"]);
  fireEvent.click(screen.getByRole("option", { name: "All branches" }));
  expect(list).toHaveBeenLastCalledWith(shop.id, null, false);
  // Hidden: the timer stops.
  const id = every.mock.results[every.mock.calls.indexOf(timer as never)]?.value;
  act(() => setPanelView("files"));
  expect(stop).toHaveBeenCalledWith(id);
  list.mockRestore();
  every.mockRestore();
  stop.mockRestore();
});

test("each run shows its state, workflow, branch, event and time; errors and none show", () => {
  const list = spyOn(transport, "listRuns").mockResolvedValue();
  show(main.id);
  act(() => apply({ type: "runs", ...runs(main.branch, []) }));
  expect(within(view()).getByText("No runs")).toBeTruthy();
  fireEvent.mouseDown(within(view()).getByRole("combobox", { name: "Branch" }));
  fireEvent.click(screen.getByRole("option", { name: "All branches" }));
  act(() => apply({ type: "runs", ...runs(null, all) }));
  const rows = within(view()).getAllByRole("listitem");
  expect(rows.map((r) => r.querySelector(".label")?.textContent)).toEqual(all.map((r) => r.title));
  const meta = rows.map((r) => r.querySelector(".session-meta")?.textContent);
  expect(meta[1]).toMatch(/^CI #101 · worktree-fix-login · pull_request · failure · 4m 30s · /);
  expect(meta[0]).toMatch(/^CI #102 · worktree-feat-checkout · pull_request · in progress · /);
  const mark = rows[1]?.querySelector(".pull-checks") as HTMLElement;
  expect([mark.textContent, mark.title, mark.dataset.state]).toEqual(["✗", "failure", "failing"]);
  expect(view().querySelector(".pulls-updated")?.textContent).toBe("Updated now");
  const error = "No remote of this repository is on github.com";
  act(() => apply({ type: "runs", ...runs(null, []), error }));
  expect(within(view()).getByRole("alert").textContent).toBe(error);
  expect(within(view()).queryByText("No runs")).toBeNull();
  list.mockRestore();
});

test("a running run's jobs refresh every 30 s while shown; cancelling asks first", () => {
  const list = spyOn(transport, "listRuns").mockResolvedValue();
  const open = spyOn(transport, "openRun").mockResolvedValue();
  const act_ = spyOn(transport, "actOnRun").mockResolvedValue();
  const every = spyOn(globalThis, "setInterval");
  const stop = spyOn(globalThis, "clearInterval");
  show(checkout.id);
  act(() => apply({ type: "runs", ...runs(checkout.branch, [running]) }));
  fireEvent.click(within(view()).getByRole("button", { name: /New checkout flow/ }));
  expect(open).toHaveBeenLastCalledWith(shop.id, running.id);
  expect(within(view()).getByText("Loading the run…")).toBeTruthy();
  act(() =>
    apply({ type: "run", project: shop.id, run: running.id, detail: detail(running), error: null }),
  );
  const timer = every.mock.calls.find(([, ms]) => ms === RUN_INTERVAL_MS);
  const tick = timer?.[0] as () => void;
  tick();
  expect(open).toHaveBeenCalledTimes(2);
  // Its running job is open; a job not finished has no end yet.
  const jobs = within(view()).getByRole("region", { name: "Jobs (2)" });
  const [lint, test_] = within(jobs)
    .getAllByRole("listitem", { hidden: true })
    .filter((li) => li.classList.contains("run-job")) as HTMLElement[];
  expect(lint?.querySelector("details")?.open).toBe(false);
  expect(lint?.querySelector("summary")?.textContent).toBe("✓lint42s");
  expect(test_?.querySelector("details")?.open).toBe(true);
  // Running: no re-run, a cancel asked first.
  expect(within(view()).queryByRole("button", { name: /Re-run/ })).toBeNull();
  fireEvent.click(within(view()).getByRole("button", { name: "Cancel" }));
  const confirm = screen.getByRole("dialog", { name: "Cancel run?" });
  expect(confirm.textContent).toContain(`Cancel CI #102 on ${checkout.branch}?`);
  expect(act_).not.toHaveBeenCalled();
  fireEvent.click(within(confirm).getByRole("button", { name: "Cancel run" }));
  expect(act_).toHaveBeenLastCalledWith(shop.id, running.id, { kind: "cancel" }, checkout.branch);
  expect(
    (within(view()).getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  // Refused: gh's message.
  const message = "gh run cancel failed: nope";
  act(() => apply({ type: "run_failed", project: shop.id, run: running.id, message }));
  expect(within(view()).getByRole("alert").textContent).toBe(message);
  // Finished: no more asking.
  const id = every.mock.results[every.mock.calls.indexOf(timer as never)]?.value;
  const done = { ...running, state: "skipped", status: "cancelled" } as const;
  act(() =>
    apply({ type: "run", project: shop.id, run: running.id, detail: detail(done), error: null }),
  );
  expect(stop).toHaveBeenCalledWith(id);
  // Back to the list.
  fireEvent.click(within(view()).getByRole("button", { name: "Runs" }));
  expect(useHive.getState().openRun).toBeNull();
  for (const spy of [list, open, act_, every, stop]) spy.mockRestore();
});

test("a failed run re-runs all or its failed jobs and shows its failed job's log", () => {
  const list = spyOn(transport, "listRuns").mockResolvedValue();
  const open = spyOn(transport, "openRun").mockResolvedValue();
  const act_ = spyOn(transport, "actOnRun").mockResolvedValue();
  const log = spyOn(transport, "openJobLog").mockResolvedValue();
  show();
  act(() => apply({ type: "runs", ...runs(login.branch, [failed]) }));
  fireEvent.click(within(view()).getByRole("button", { name: /Fix the login redirect/ }));
  act(() => apply({ type: "run", project: shop.id, run: failed.id, error: "gone", detail: null }));
  expect(within(view()).getByRole("alert").textContent).toBe("gone");
  fireEvent.click(within(view()).getByTitle("Refresh"));
  expect(open).toHaveBeenLastCalledWith(shop.id, failed.id);
  act(() =>
    apply({ type: "run", project: shop.id, run: failed.id, detail: detail(failed), error: null }),
  );
  const body = view().querySelector(".pull-body") as HTMLElement;
  expect(body.querySelector(".pull-heading")?.textContent).toBe("Fix the login redirect #101");
  expect(body.querySelector(".session-meta")?.textContent).toBe(
    "✗ CI · worktree-fix-login · pull_request · failure · 4m 30s",
  );
  fireEvent.click(within(body).getByRole("button", { name: "Re-run failed jobs" }));
  expect(act_).toHaveBeenLastCalledWith(
    shop.id,
    failed.id,
    { kind: "rerun", failed: true },
    login.branch,
  );
  act(() => apply({ type: "run_done", project: shop.id, run: failed.id, message: "ok" }));
  fireEvent.click(within(body).getByRole("button", { name: "Re-run all jobs" }));
  expect(act_).toHaveBeenLastCalledWith(
    shop.id,
    failed.id,
    { kind: "rerun", failed: false },
    login.branch,
  );
  expect(within(body).queryByRole("button", { name: "Cancel" })).toBeNull();
  fireEvent.click(within(body).getByRole("button", { name: "Open on GitHub" }));
  expect(notice()).toContain(failed.url);

  // The failed job's log: asked for, then its end in a monospace block.
  fireEvent.click(within(body).getByRole("button", { name: "Show the end of its log" }));
  expect(log).toHaveBeenLastCalledWith(shop.id, 12);
  expect(body.querySelector(".run-log")?.textContent).toBe("Loading the log…");
  act(() => apply({ type: "job_log", project: shop.id, job: 12, log: MOCK_LOG, error: null }));
  expect(body.querySelector(".run-log")?.textContent).toBe(MOCK_LOG);
  act(() => apply({ type: "job_log", project: shop.id, job: 12, log: null, error: "HTTP 410" }));
  expect(within(body).getByRole("alert").textContent).toBe("HTTP 410");
  fireEvent.click(within(body).getByRole("button", { name: "Hide the log" }));
  expect(useHive.getState().jobLog).toBeNull();
  // No jobs at all.
  act(() =>
    apply({
      type: "run",
      project: shop.id,
      run: failed.id,
      detail: { summary: { ...failed, url: "" }, jobs: [] },
      error: null,
    }),
  );
  expect(within(body).getByText("None")).toBeTruthy();
  expect(within(body).queryByRole("button", { name: "Open on GitHub" })).toBeNull();
  for (const spy of [list, open, act_, log]) spy.mockRestore();
});

test("the sidebar shows a worktree's latest run; a click shows it", () => {
  const list = spyOn(transport, "listRuns").mockResolvedValue();
  const open = spyOn(transport, "openRun").mockResolvedValue();
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  // Asked for once connected, on every branch, for every project of the sidebar.
  expect(list).not.toHaveBeenCalled();
  act(() => apply({ type: "welcome", version: "0", distro: null }));
  expect(list.mock.calls).toEqual([[shop.id, null, false]]);
  act(() => apply({ type: "runs", ...runs(null, all) }));
  const badges = [...document.querySelectorAll(".run-badge")] as HTMLElement[];
  expect(badges.map((b) => [b.textContent, b.title, b.dataset.state])).toEqual([
    ["✓", "CI #100: success", "passing"],
    ["✗", "CI #101: failure", "failing"],
    ["●", "CI #102: in progress", "running"],
  ]);
  fireEvent.click(badges[1] as HTMLElement);
  const s = useHive.getState();
  expect([s.selection, s.rightPanel, s.panelView]).toEqual([login.id, "files", "actions"]);
  expect(open).toHaveBeenCalledWith(shop.id, failed.id);
  expect(within(view()).getByText("Loading the run…")).toBeTruthy();
  list.mockRestore();
  open.mockRestore();
});
