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
