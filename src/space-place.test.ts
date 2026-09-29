import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Space } from "./protocol";
import { apply } from "./reduce";
import { goToAgent } from "./shortcuts";
import { addTab, initialState, select, useHive } from "./store";
import { visibleTabs } from "./tabs";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 11.5: switching space selects the place last selected there.

const [shop, api, dotfiles] = MOCK_REPOS;
const NO_ENV = {
  git_name: null,
  git_email: null,
  gh_config_dir: null,
  gh_account: null,
};
const home: Space = { id: "default", name: "Home", projects: [shop.id], env: NO_ENV };
const work: Space = { id: "w", name: "Work", projects: [dotfiles.id, api.id], env: NO_ENV };
const empty: Space = { id: "e", name: "Empty", projects: [], env: NO_ENV };

/** The service switched to space `current`. */
const to = (current: string) => apply({ type: "spaces", spaces: [home, work, empty], current });
const selection = () => useHive.getState().selection;
const shown = () => visibleTabs(useHive.getState()).map((t) => t.id);

beforeEach(() => {
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop, api, dotfiles] });
  to("default");
  addTab(1, shop.worktrees[1].path);
  addTab(2, api.path);
  addTab(3, "/outside");
});
afterEach(() => useHive.setState(initialState, true));

test("a space never selected shows its first project; coming back shows the place left", () => {
  select(shop.worktrees[1].id);
  to("w");
  // The sidebar's first project (the service's order, not the space's), with its tabs.
  expect([selection(), shown()]).toEqual([api.id, [2]]);
  select(dotfiles.id);
  expect(shown()).toEqual([]);
  to("default");
  expect([selection(), shown()]).toEqual([shop.worktrees[1].id, [1]]);
  to("w");
  expect([selection(), shown()]).toEqual([dotfiles.id, []]);
  // The same space again (a rename, a new project) changes no selection.
  select(api.id);
  to("w");
  expect(selection()).toBe(api.id);
});

test("an empty space selects nothing and shows no tabs", () => {
  select(shop.id);
  to("e");
  expect([selection(), shown(), useHive.getState().activeTab]).toEqual([null, [], null]);
  to("default");
  expect(selection()).toBe(shop.id);
  // Elsewhere nothing selected still shows every tab.
  select(null);
  expect(shown()).toEqual([1, 2, 3]);
});

test("a remembered place that is gone falls back to its project, then to the first project", () => {
  // A worktree removed while its space was not shown.
  select(shop.worktrees[1].id);
  to("w");
  apply({
    type: "projects",
    projects: [{ ...shop, worktrees: [shop.worktrees[0]] }, api, dotfiles],
  });
  to("default");
  expect(selection()).toBe(shop.id);

  // An agent that ended.
  const agent = { id: "a", project: api.id, worktree: api.id, cwd: api.path };
  apply({ type: "agent_detected", channel: 2, ...agent });
  to("w");
  select("a");
  to("default");
  apply({ type: "agent_removed", channel: 2, id: "a" });
  to("w");
  expect(selection()).toBe(api.id);

  // A project no longer in its space, or nothing selected when the space was left.
  to("default");
  apply({ type: "spaces", spaces: [home, { ...work, projects: [dotfiles.id] }], current: "w" });
  expect(selection()).toBe(dotfiles.id);
  select(null);
  to("default");
  to("w");
  expect(selection()).toBe(api.id);
  // A place outside every project.
  select("/outside");
  to("default");
  to("w");
  expect(selection()).toBe(api.id);
});

test("going to an agent of another space selects it, and its space keeps the place left", () => {
  const selectSpace = spyOn(transport, "selectSpace").mockResolvedValue();
  const agent = { id: "a", project: api.id, worktree: api.id, cwd: api.path };
  apply({ type: "agent_detected", channel: 2, ...agent });
  select(shop.worktrees[1].id);
  goToAgent(useHive.getState().agents.a as never);
  expect(selectSpace).toHaveBeenCalledWith("w");
  // The service confirms the switch: the agent stays selected, not the space's first project.
  to("w");
  expect([selection(), shown()]).toEqual(["a", [2]]);
  to("default");
  expect(selection()).toBe(shop.worktrees[1].id);
  to("w");
  expect(selection()).toBe("a");
  selectSpace.mockRestore();
});

test("the first spaces, at start, select nothing", () => {
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop, api, dotfiles] });
  to("w");
  expect(selection()).toBeNull();
});
