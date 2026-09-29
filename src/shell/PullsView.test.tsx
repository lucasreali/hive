import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { notice } from "../../test/notice";
import { App } from "../App";
import { PULLS_INTERVAL_MS, type PullDetail, type Pulls } from "../pulls";
import { apply } from "../reduce";
import {
  DEFAULT_SETTINGS,
  initialState,
  NO_SCRIPTS,
  select,
  setPanelView,
  useHive,
} from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { MOCK_PULLS } from "../transport/mockPulls";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [, login, checkout] = shop.worktrees;
const mine = MOCK_PULLS[shop.id]?.mine ?? [];
const review = MOCK_PULLS[shop.id]?.review ?? [];
const pulls: Pulls = {
  project: shop.id,
  repo: {
    name: "user/shop",
    url: "https://github.com/user/shop",
    merge_methods: ["merge", "squash"],
    default_merge: "squash",
  },
  mine,
  review,
  fetched_ms: Date.now(),
  error: null,
};
const HEAD = "c".repeat(40);

const detail = (number: number, extra: Partial<PullDetail> = {}): PullDetail => ({
  summary: [...mine, ...review].find((p) => p.number === number) ?? mine[0],
  body: "Fixes [it](javascript:alert(1)). ![shot](https://example.com/a.png)",
  head: HEAD,
  additions: 12,
  deletions: 3,
  conflicts: true,
  notes: [
    { author: "octo", body: "**Nice**", at: "2026-09-27T09:00:00Z", review: "approved" },
    { author: "octo-2", body: "", at: "bad date", review: null },
  ],
  checks: [
    { name: "test", workflow: "CI", state: "failing", url: "https://github.com/x/1", run: null },
    { name: "ci/legacy", workflow: null, state: "passing", url: null, run: null },
    {
      name: "build",
      workflow: "CI",
      state: "passing",
      url: "https://github.com/x/actions/runs/5/job/6",
      run: 5,
    },
  ],
  files: [{ path: "src/login.ts", additions: 12, deletions: 3 }],
  ...extra,
});

/** The app with shop, `worktree` selected and the right panel on Pull requests. */
function show(worktree = login.id) {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  act(() => {
    select(worktree);
    setPanelView("pulls");
  });
}

const view = () => screen.getByRole("region", { name: "PRs" });

test("the list asks when shown and every two minutes while it shows, never faster", () => {
  const list = spyOn(transport, "listPulls").mockResolvedValue();
  const every = spyOn(globalThis, "setInterval");
  const stop = spyOn(globalThis, "clearInterval");
  show();
  expect(list.mock.calls).toEqual([[shop.id, false]]);
  expect(within(view()).getByText("Loading…")).toBeTruthy();
  const timer = every.mock.calls.find(([, ms]) => ms === PULLS_INTERVAL_MS);
  const tick = timer?.[0] as () => void;
  tick();
  expect(list).toHaveBeenCalledTimes(2);
  expect(list).toHaveBeenLastCalledWith(shop.id, false);
  // On demand: GitHub is asked now.
  act(() => apply({ type: "pulls", ...pulls }));
  fireEvent.click(within(view()).getByTitle("Refresh"));
  expect(list).toHaveBeenLastCalledWith(shop.id, true);
  // Hidden: the timer stops.
  const id = every.mock.results[every.mock.calls.indexOf(timer as never)]?.value;
  act(() => setPanelView("files"));
  expect(stop).toHaveBeenCalledWith(id);
  list.mockRestore();
  every.mockRestore();
  stop.mockRestore();
});

test("each pull request shows its state, checks and review; the worktree's is marked", () => {
  const list = spyOn(transport, "listPulls").mockResolvedValue();
  show();
  act(() => apply({ type: "pulls", ...pulls }));
  const yours = within(view()).getByRole("region", { name: "Yours" });
  const rows = within(yours).getAllByRole("listitem");
  expect(rows.map((r) => r.querySelector(".label")?.textContent)).toEqual(mine.map((p) => p.title));
  expect(rows[0]?.dataset.here).toBe("true");
  expect(rows[1]?.dataset.here).toBe("false");
  expect(rows[0]?.querySelector(".pull-state")?.textContent).toBe("Open");
  expect(rows[1]?.querySelector(".pull-state")?.textContent).toBe("Draft");
  expect(rows[0]?.querySelector(".pull-checks")?.getAttribute("title")).toBe("Checks passing");
  expect(rows[1]?.querySelector(".pull-checks")?.textContent).toBe("●");
  expect(rows[0]?.querySelector(".session-meta")?.textContent).toMatch(/^#12 · Approved · /);
  const asked = within(view()).getByRole("region", { name: "Review requested" });
  expect(asked.querySelector(".session-meta")?.textContent).toMatch(
    /^#14 · octo-reviewer · Review required · /,
  );
  expect(view().querySelector(".pulls-updated")?.textContent).toBe("Updated now");
  // Its own pull request: nothing to create.
  expect(within(view()).queryByText(/Create pull request/)).toBeNull();
  // The repository opens on GitHub (only in the app: here the status bar says so).
  fireEvent.click(within(view()).getByRole("button", { name: "user/shop" }));
  expect(notice()).toBe("Only the Hive app opens links: https://github.com/user/shop");

  // The service's refusal is shown as is.
  const error = "No remote of this repository is on github.com";
  act(() => apply({ type: "pulls", ...pulls, repo: null, mine: [], review: [], error }));
  expect(within(view()).getByRole("alert").textContent).toBe(error);
  expect(within(view()).queryByRole("region", { name: "Yours" })).toBeNull();
  expect(view().querySelector(".pulls-bar")?.textContent).toContain("shop");
  act(() => apply({ type: "pulls", ...pulls, mine: [], review: [], fetched_ms: 0 }));
  expect(within(view()).getAllByText("None")).toHaveLength(2);
  expect(view().querySelector(".pulls-updated")?.textContent).toBe("");
  list.mockRestore();
});

test("a worktree without a pull request offers to create one from its branch", () => {
  const list = spyOn(transport, "listPulls").mockResolvedValue();
  show(checkout.id);
  act(() => apply({ type: "pulls", ...pulls }));
  useHive.setState({ pullError: { project: shop.id, number: null, message: "old" } });
  const label = `Create pull request from ${checkout.branch}`;
  const create = within(view()).getByRole("button", { name: label });
  // 12.3: only the branch is cut with an ellipsis; the tooltip keeps the whole label.
  expect(create.title).toBe(label);
  expect(create.querySelector(".pulls-create-branch")?.textContent).toBe(`${checkout.branch}`);
  fireEvent.click(create);
  const s = useHive.getState();
  expect([s.modal, s.modalWorktree, s.pullError]).toEqual(["new-pull", checkout.id, null]);
  expect(screen.getByRole("dialog", { name: "Create pull request" })).toBeTruthy();
  list.mockRestore();
});

test("the sidebar shows a worktree's pull request; a click shows its details", () => {
  const list = spyOn(transport, "listPulls").mockResolvedValue();
  const open = spyOn(transport, "openPull").mockResolvedValue();
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  // Asked for once connected, for every project of the sidebar.
  expect(list).not.toHaveBeenCalled();
  act(() => apply({ type: "welcome", version: "0", distro: null, windows: false }));
  expect(list.mock.calls).toEqual([[shop.id, false]]);
  act(() => apply({ type: "pulls", ...pulls }));
  const badges = document.querySelectorAll(".pull-badge");
  expect(badges).toHaveLength(1);
  const badge = badges[0] as HTMLElement;
  expect([badge.textContent, badge.title, badge.dataset.state]).toEqual([
    "#12",
    "Pull request #12: Open",
    "open",
  ]);
  fireEvent.click(badge);
  const s = useHive.getState();
  expect([s.selection, s.rightPanel, s.panelView]).toEqual([login.id, "files", "pulls"]);
  expect(open).toHaveBeenCalledWith(shop.id, 12);
  expect(within(view()).getByText("Loading #12…")).toBeTruthy();
  list.mockRestore();
  open.mockRestore();
});

test("details: merging and closing ask first; a refusal shows gh's message", () => {
  const list = spyOn(transport, "listPulls").mockResolvedValue();
  const open = spyOn(transport, "openPull").mockResolvedValue();
  const act_ = spyOn(transport, "actOnPull").mockResolvedValue();
  show();
  act(() => apply({ type: "pulls", ...pulls }));
  fireEvent.click(within(view()).getByRole("button", { name: /Fix the login redirect/ }));
  expect(open).toHaveBeenLastCalledWith(shop.id, 12);
  // The refresh asks again; an error shows in place of the details.
  fireEvent.click(within(view()).getByTitle("Refresh"));
  expect(open).toHaveBeenCalledTimes(2);
  const answer = { type: "pull", project: shop.id, number: 12 } as const;
  act(() => apply({ ...answer, pull: null, error: "gh pr view failed: HTTP 502" }));
  expect(within(view()).getByRole("alert").textContent).toBe("gh pr view failed: HTTP 502");
  act(() => apply({ ...answer, pull: detail(12), error: null }));

  const body = view().querySelector(".pull-body") as HTMLElement;
  expect(body.querySelector(".pull-heading")?.textContent).toBe("Fix the login redirect #12");
  expect(body.textContent).toContain("Conflicts with the base");
  // Untrusted Markdown: no script link, no image loaded.
  const markdown = body.querySelector(".markdown") as HTMLElement;
  expect(markdown.querySelector("a")).toBeNull();
  expect(markdown.querySelector("img")).toBeNull();
  expect(markdown.textContent).toBe("Fixes it. shot");
  const checks = within(body).getByRole("region", { name: "Checks (3)" });
  expect(checks.querySelectorAll(".pull-check")).toHaveLength(3);
  fireEvent.click(within(checks).getByRole("button", { name: "test" }));
  expect(notice()).toBe("Only the Hive app opens links: https://github.com/x/1");
  const notes = within(body).getByRole("region", { name: "Reviews and comments (2)" });
  expect(notes.querySelectorAll(".pull-note")[0]?.textContent).toMatch(/^octo approved · .*Nice$/);
  expect(notes.querySelectorAll(".pull-note")[1]?.textContent).toBe("octo-2 · ");
  expect(within(body).getByRole("region", { name: "Files (1)" }).textContent).toBe(
    "Files (1)src/login.ts+12−3",
  );
  fireEvent.click(within(body).getByRole("button", { name: "Open on GitHub" }));
  expect(notice()).toContain(mine[0]?.url);

  // Merge: the repository's default method, asked first.
  fireEvent.click(within(body).getByRole("button", { name: "Merge" }));
  const confirm = screen.getByRole("dialog", { name: "Merge pull request?" });
  expect(confirm.textContent).toContain(
    'Merge #12 "Fix the login redirect" into main (squash and merge)?',
  );
  expect(act_).not.toHaveBeenCalled();
  fireEvent.click(within(confirm).getByRole("button", { name: "Merge" }));
  expect(act_).toHaveBeenLastCalledWith(shop.id, 12, {
    kind: "merge",
    method: "squash",
    head: HEAD,
  });
  const buttons = () => within(body).getAllByRole("button", { name: /^(Merge|Close)$/ });
  expect(buttons().every((b) => (b as HTMLButtonElement).disabled)).toBe(true);
  const message = "gh pr merge failed: GraphQL: Pull request is not mergeable";
  act(() => apply({ type: "pull_failed", project: shop.id, number: 7, message: "not this" }));
  expect(within(body).queryByRole("alert")).toBeNull();
  act(() => apply({ type: "pull_failed", project: shop.id, number: 12, message }));
  expect(within(body).getByRole("alert").textContent).toBe(message);
  expect(buttons().some((b) => (b as HTMLButtonElement).disabled)).toBe(false);
  // Another method, picked.
  fireEvent.mouseDown(within(body).getByRole("combobox", { name: "Merge method" }));
  fireEvent.click(screen.getByRole("option", { name: "Create a merge commit" }));
  fireEvent.click(within(body).getByRole("button", { name: "Merge" }));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Merge" }));
  expect(act_).toHaveBeenLastCalledWith(shop.id, 12, {
    kind: "merge",
    method: "merge",
    head: HEAD,
  });

  // Close: asked first; Cancel sends nothing.
  act(() => apply({ type: "pull_done", project: shop.id, number: 12, message: "x" }));
  const calls = act_.mock.calls.length;
  fireEvent.click(within(body).getByRole("button", { name: "Close" }));
  const closing = screen.getByRole("dialog", { name: "Close pull request?" });
  expect(closing.textContent).toContain('Close #12 "Fix the login redirect" without merging it?');
  fireEvent.click(within(closing).getByRole("button", { name: "Cancel" }));
  expect(act_).toHaveBeenCalledTimes(calls);
  fireEvent.click(within(body).getByRole("button", { name: "Close" }));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
  expect(act_).toHaveBeenLastCalledWith(shop.id, 12, { kind: "close" });

  // An Actions job's check shows its run in the Actions view (9.32).
  const run = spyOn(transport, "openRun").mockResolvedValue();
  const runs = spyOn(transport, "listRuns").mockResolvedValue();
  fireEvent.click(within(body).getByRole("button", { name: "build" }));
  expect(run).toHaveBeenCalledWith(shop.id, 5);
  expect(screen.getByRole("region", { name: "Actions" })).toBeTruthy();
  act(() => setPanelView("pulls"));
  run.mockRestore();
  runs.mockRestore();

  // Its worktree is shown from here; back to the list.
  act(() => select(shop.id));
  fireEvent.click(within(view()).getByRole("button", { name: "Show its worktree" }));
  expect(useHive.getState().selection).toBe(login.id);
  fireEvent.click(within(view()).getByRole("button", { name: "Pull requests" }));
  expect(within(view()).getByRole("region", { name: "Yours" })).toBeTruthy();
  list.mockRestore();
  open.mockRestore();
  act_.mockRestore();
});

test("a draft is made ready; a checkout runs the setup script and selects the new worktree", async () => {
  const list = spyOn(transport, "listPulls").mockResolvedValue();
  const open = spyOn(transport, "openPull").mockResolvedValue();
  const act_ = spyOn(transport, "actOnPull").mockResolvedValue();
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  const opened = spyOn(transport, "openTerminal").mockResolvedValue(42);
  show();
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.projects[shop.id] = { scripts: { ...NO_SCRIPTS, setup: "bun install" } };
  act(() => apply({ type: "settings", settings }));
  act(() => apply({ type: "pulls", ...pulls }));
  fireEvent.click(within(view()).getByRole("button", { name: /New checkout flow/ }));
  const draft = detail(9, { conflicts: false, notes: [], checks: [], files: [], body: "" });
  act(() => apply({ type: "pull", project: shop.id, number: 9, pull: draft, error: null }));
  const body = view().querySelector(".pull-body") as HTMLElement;
  expect(body.textContent).not.toContain("Conflicts");
  expect(body.querySelector(".markdown")?.textContent).toBe("No description.");
  expect(within(body).getAllByText("None")).toHaveLength(3);
  // A draft is not merged from here.
  expect(within(body).queryByRole("button", { name: "Merge" })).toBeNull();
  fireEvent.click(within(body).getByRole("button", { name: "Ready for review" }));
  expect(act_).toHaveBeenLastCalledWith(shop.id, 9, { kind: "ready" });
  act(() => apply({ type: "pull_done", project: shop.id, number: 9, message: "ready" }));

  fireEvent.click(within(body).getByRole("button", { name: "Check out" }));
  expect(act_).toHaveBeenLastCalledWith(shop.id, 9, { kind: "checkout" });
  // Another worktree created meanwhile is not this checkout's.
  const path = `${shop.path}/.claude/worktrees/pr-9`;
  const made = { ...login, id: path, path, name: "pr-9", branch: "feature-9" };
  const project = { ...shop, worktrees: [...shop.worktrees, made] };
  act(() =>
    apply({ type: "worktree_created", project: { ...project, id: "/api" }, path, notes: [] }),
  );
  expect(useHive.getState().selection).toBe(login.id);
  act(() => apply({ type: "worktree_created", project, path, notes: [] }));
  expect(useHive.getState().selection).toBe(path);
  expect(useHive.getState().pullBusy).toBeNull();
  await waitFor(() => expect(useHive.getState().tabs).toHaveLength(1));
  const tab = useHive.getState().tabs[0];
  await waitFor(() => expect(write).toHaveBeenCalledWith(tab?.id, "bun install\r"));
  closeTerminal(tab?.id ?? 0);
  // A merged one has no actions but opening it.
  const merged = detail(5);
  act(() => apply({ type: "pull", project: shop.id, number: 9, pull: merged, error: null }));
  const actions = view().querySelector(".pull-actions") as HTMLElement;
  expect(
    within(actions)
      .getAllByRole("button")
      .map((b) => b.textContent),
  ).toEqual(["Open on GitHub"]);
  list.mockRestore();
  open.mockRestore();
  act_.mockRestore();
  write.mockRestore();
  opened.mockRestore();
});
