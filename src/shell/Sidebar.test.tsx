import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import { type AgentState, apply, initialState, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";
import { STATE_LABEL } from "./icons";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const tree = () => screen.getByRole("navigation", { name: "Projects" });

test("projects show their worktrees with the service's names and paths", () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  const rows = [...tree().querySelectorAll(".tree-row")].map((r) => [
    r.className,
    r.textContent,
    r.getAttribute("title"),
  ]);
  expect(rows).toEqual([
    ["tree-row project", "shopNew worktree", "/home/user/projects/shop"],
    ["tree-row worktree", "main", "/home/user/projects/shop"],
    ["tree-row worktree", "fix-login", "/home/user/projects/shop/.claude/worktrees/fix-login"],
    [
      "tree-row worktree",
      "feat-checkout",
      "/home/user/projects/shop/.claude/worktrees/feat-checkout",
    ],
    ["tree-row project", "apiNew worktree", "/home/user/projects/api"],
    ["tree-row worktree", "main", "/home/user/projects/api"],
    [
      "tree-row worktree",
      "refactor-auth",
      "/home/user/projects/api/.claude/worktrees/refactor-auth",
    ],
  ]);
  expect(tree().textContent).not.toContain("No projects");
});

test("a project collapses and expands", () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  const chevron = screen.getByRole("button", { name: "Collapse shop" });
  expect(chevron.getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(chevron);
  expect(screen.queryByRole("button", { name: "fix-login" })).toBeNull();
  const expand = screen.getByRole("button", { name: "Expand shop" });
  expect(expand.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(expand);
  expect(screen.getByRole("button", { name: "fix-login" })).toBeDefined();
});

test("clicking a row selects it", () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  const worktree = screen.getByRole("button", { name: "fix-login" });
  fireEvent.click(worktree);
  expect(useHive.getState().selection).toBe(shop.worktrees[1].id);
  expect(worktree.getAttribute("aria-current")).toBe("true");
  expect(worktree.parentElement?.dataset.selected).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: "shop" }));
  expect(useHive.getState().selection).toBe(shop.id);
  expect(worktree.getAttribute("aria-current")).toBe("false");
});

test("a project whose worktrees cannot be listed says why", () => {
  render(<App />);
  const error = "git worktree list --porcelain -z failed: fatal: cannot change to '/gone'";
  act(() => apply({ type: "projects", projects: [{ ...shop, worktrees: [], error }] }));
  expect(tree().querySelector(".tree-error")?.textContent).toBe(error);
});

test("refresh asks the service for the projects again", () => {
  const list = spyOn(transport, "listProjects");
  render(<App />);
  fireEvent.click(screen.getByTitle("Refresh worktrees"));
  expect(list).toHaveBeenCalledTimes(1);
  list.mockRestore();
});

test("an agent shows under the worktree it was placed in and shows its tab when clicked", () => {
  render(<App />);
  const fixLogin = shop.worktrees[1];
  act(() => {
    apply({ type: "projects", projects: [shop] });
    // The terminal was opened in main; the service placed the agent by its own cwd.
    useHive.setState({
      tabs: [
        { id: 1, cwd: shop.path },
        { id: 2, cwd: shop.path },
      ],
      activeTab: 2,
    });
    const placed = { project: shop.id, worktree: fixLogin.id, cwd: `${fixLogin.path}/src` };
    apply({ type: "agent_detected", channel: 1, id: "s1", ...placed });
    apply({ type: "agent_detected", channel: 9, id: "s2", ...placed });
    apply({
      type: "agent_detected",
      channel: 2,
      id: "s3",
      project: null,
      worktree: null,
      cwd: "/tmp",
    });
  });
  const rows = [...tree().querySelectorAll(".tree-row")].map((r) => [r.className, r.textContent]);
  expect(rows).toEqual([
    ["tree-row project", "shopNew worktree"],
    ["tree-row worktree", "main"],
    ["tree-row worktree", "fix-login"],
    ["tree-row agent", "idleClaudeidle"],
    ["tree-row agent", "idleClaudeidle"],
    ["tree-row worktree", "feat-checkout"],
  ]);
  const [agent, orphan] = screen.getAllByRole("button", { name: "idle Claude" });
  expect(agent.parentElement?.getAttribute("title")).toBe(`${fixLogin.path}/src`);
  expect(agent.getAttribute("aria-current")).toBe("false");
  fireEvent.click(agent);
  expect(useHive.getState().activeTab).toBe(1);
  expect(agent.getAttribute("aria-current")).toBe("true");
  expect(agent.parentElement?.dataset.selected).toBe("true");
  // An agent whose tab is gone does nothing.
  fireEvent.click(orphan);
  expect(useHive.getState().activeTab).toBe(1);
  act(() => apply({ type: "agent_removed", channel: 1, id: "s1" }));
  expect(screen.getAllByRole("button", { name: "idle Claude" })).toHaveLength(1);
});

test("agents and their subagents show the state the service sent, named for screen readers", () => {
  render(<App />);
  const [main, fixLogin] = shop.worktrees;
  const at = (w: typeof main) => ({ project: shop.id, worktree: w.id, cwd: w.path });
  act(() => {
    apply({ type: "projects", projects: [shop] });
    useHive.setState({ tabs: [{ id: 1, cwd: shop.path }], activeTab: null });
    apply({ type: "agent_detected", channel: 1, id: "s1", ...at(fixLogin) });
    apply({ type: "agent_detected", channel: 2, id: "s2", ...at(main) });
    apply({
      type: "agent_state",
      id: "s1",
      ...agentStatus("waiting_permission"),
      subagents: [
        { id: "a1", agent_type: "Explore", state: "working" },
        { id: "a2", agent_type: null, state: "waiting_permission" },
      ],
    });
    apply({ type: "agent_state", id: "s2", ...agentStatus("ended"), subagents: [] });
  });
  const rows = [...tree().querySelectorAll(".tree-row.agent, .tree-row.subagent")].map((r) => [
    r.className,
    r.querySelector(".state-icon")?.getAttribute("aria-label"),
    r.querySelector(".state-label")?.textContent,
    r.querySelector(".label")?.textContent,
  ]);
  expect(rows).toEqual([
    ["tree-row agent", "ended", "ended", "Claude"],
    ["tree-row agent", "waiting for permission", "waiting for permission", "Claude"],
    ["tree-row subagent", "working", "working", "subagent: Explore"],
    ["tree-row subagent", "waiting for permission", "waiting for permission", "subagent: unknown"],
  ]);
  // Every state has its own shape.
  const shapes = new Set<string>();
  for (const state of Object.keys(STATE_LABEL) as AgentState[]) {
    act(() => apply({ type: "agent_state", id: "s2", ...agentStatus(state), subagents: [] }));
    // s2 is the first agent (main comes before fix-login).
    const icon = within(tree().querySelector(".tree-row.agent") as HTMLElement).getByRole("img", {
      name: STATE_LABEL[state],
    });
    expect(icon.getAttribute("data-state")).toBe(state);
    shapes.add(icon.innerHTML.replace(/<title>.*<\/title>/, ""));
  }
  expect(shapes.size).toBe(7);
  // A subagent row shows its agent's terminal.
  fireEvent.click(screen.getByRole("button", { name: /subagent: Explore/ }));
  expect(useHive.getState().activeTab).toBe(1);
  // Subagents that ended leave the tree.
  act(() => apply({ type: "agent_state", id: "s1", ...agentStatus("idle"), subagents: [] }));
  expect(tree().querySelector(".tree-row.subagent")).toBeNull();
});

test("a collapsed node shows the most urgent state inside; the counter counts pending agents", () => {
  render(<App />);
  const [main, fixLogin] = shop.worktrees;
  const agent = (id: string, worktree: (typeof main)["id"] | null, state?: AgentState) => {
    const project = MOCK_REPOS.find((p) => p.worktrees.some((w) => w.id === worktree));
    const placed = { project: project?.id ?? null, worktree, cwd: worktree };
    apply({ type: "agent_detected", channel: 1, id, ...placed });
    if (state) apply({ type: "agent_state", id, ...agentStatus(state), subagents: [] });
  };
  const counter = () => tree().querySelector(".bar > :first-child") as HTMLElement;
  act(() => apply({ type: "projects", projects: [shop, api] }));
  expect(counter().textContent).toBe("Nothing pending");
  act(() => {
    agent("s1", main.id, "working");
    agent("s2", fixLogin.id, "idle");
    agent("s3", api.worktrees[0].id, "error");
    agent("s4", api.worktrees[1].id);
  });
  expect(counter().textContent).toBe("1 pendingF8");
  act(() => {
    agent("s2", fixLogin.id, "waiting_you");
    agent("s5", null, "waiting_permission");
  });
  // An agent outside every project still counts.
  expect(counter().textContent).toBe("3 pendingF8");
  expect(counter().title).toBe("Go to the next pending agent (F8)");

  const rollup = (name: string) =>
    screen.getByRole("button", { name: new RegExp(`^${name}\\b`) }).querySelector(".state-icon");
  // Expanded nodes show nothing; a worktree without agents has nothing to collapse.
  expect(rollup("shop")).toBeNull();
  expect(screen.queryByRole("button", { name: "Collapse feat-checkout" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Collapse fix-login" }));
  expect(rollup("fix-login")?.getAttribute("data-state")).toBe("waiting_you");
  expect(screen.getAllByRole("button", { name: /Claude/ })).toHaveLength(3);
  // A main worktree collapses apart from its project (they share an id).
  fireEvent.click(screen.getAllByRole("button", { name: "Collapse main" })[1]);
  expect(screen.getByRole("button", { name: "main error" })).toBeDefined();
  expect(screen.getByRole("button", { name: "refactor-auth" })).toBeDefined();
  // No state yet: nothing to show.
  fireEvent.click(screen.getByRole("button", { name: "Collapse refactor-auth" }));
  expect(rollup("refactor-auth")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Collapse shop" }));
  fireEvent.click(screen.getByRole("button", { name: "Collapse api" }));
  expect(rollup("shop")?.getAttribute("aria-label")).toBe("waiting for you");
  expect(rollup("api")?.getAttribute("data-state")).toBe("error");
  // The service's urgency decides, not the order the agents came in.
  act(() => agent("s1", main.id, "waiting_permission"));
  expect(rollup("shop")?.getAttribute("data-state")).toBe("waiting_permission");

  // The counter is F8.
  fireEvent.click(counter());
  expect(useHive.getState().selection).toBe("s1");
  expect(screen.getByRole("button", { name: /^shop/ }).querySelector(".state-icon")).toBeNull();
});

test("the agent F8 picks is selected and scrolled into view", () => {
  const scroll = spyOn(HTMLElement.prototype, "scrollIntoView");
  render(<App />);
  const placed = { project: shop.id, worktree: shop.worktrees[1].id, cwd: null };
  act(() => {
    apply({ type: "projects", projects: [shop] });
    apply({ type: "agent_detected", channel: 7, id: "s1", ...placed });
  });
  const row = tree().querySelector(".tree-row.agent") as HTMLElement;
  expect(row.dataset.selected).toBe("false");
  expect(scroll).not.toHaveBeenCalled();
  act(() => useHive.setState({ selection: "s1" }));
  expect(row.dataset.selected).toBe("true");
  expect(scroll).toHaveBeenCalledWith({ block: "nearest" });
  scroll.mockRestore();
});

test("arrow keys move in the tree and collapse or expand a project", () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  const row = (name: string, i = 0) => screen.getAllByRole("button", { name })[i];
  const key = (key: string) => fireEvent.keyDown(document.activeElement as Element, { key });
  // Keys outside the rows are not the tree's.
  screen.getByTitle("Refresh worktrees").focus();
  key("ArrowDown");
  expect(document.activeElement).toBe(screen.getByTitle("Refresh worktrees"));

  row("shop").focus();
  key("ArrowUp");
  expect(document.activeElement).toBe(row("shop"));
  key("ArrowDown");
  expect(document.activeElement).toBe(row("main"));
  // A worktree has nothing to collapse.
  key("ArrowLeft");
  expect(row("fix-login")).toBeDefined();
  key("ArrowUp");
  key("ArrowRight");
  expect(row("fix-login")).toBeDefined();
  key("ArrowLeft");
  expect(screen.queryByRole("button", { name: "fix-login" })).toBeNull();
  key("ArrowLeft");
  expect(screen.queryByRole("button", { name: "fix-login" })).toBeNull();
  key("ArrowDown");
  expect(document.activeElement).toBe(row("api"));
  key("ArrowUp");
  key("ArrowRight");
  expect(row("fix-login")).toBeDefined();
  // Other keys (Enter clicks the row, which selects it) are left alone.
  key("Tab");
  expect(document.activeElement).toBe(row("shop"));
});
