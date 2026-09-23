import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

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
    ["tree-row agent", "Claude"],
    ["tree-row agent", "Claude"],
    ["tree-row worktree", "feat-checkout"],
  ]);
  const [agent, orphan] = screen.getAllByRole("button", { name: "Claude" });
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
  expect(screen.getAllByRole("button", { name: "Claude" })).toHaveLength(1);
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
