import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, openModal, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

// Tests run outside Tauri, so the mock service answers every request once connected. It keeps
// its state for the whole run, so each test creates a worktree with its own name.
beforeAll(async () => {
  // happy-dom has no layout: give the branch list its CSS size so the virtualizer shows rows.
  for (const [key, size] of [
    ["offsetHeight", 156],
    ["offsetWidth", 486],
  ] as const) {
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, key)?.get;
    Object.defineProperty(HTMLElement.prototype, key, {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains("branch-list") ? size : original?.call(this);
      },
    });
  }
  await transport.connect(apply);
  await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;

function open(project: string | null = shop.id) {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => apply({ type: "projects", projects: [shop, api] }));
  act(() => openModal("new-worktree", project));
  return screen.getByRole("dialog", { name: "New worktree" }) as HTMLDialogElement;
}

const nameField = () => screen.getByLabelText("Worktree name") as HTMLInputElement;
const filter = () => screen.getByLabelText("Base branch") as HTMLInputElement;
const create = () =>
  screen.getByRole("button", { name: "Create worktree Enter" }) as HTMLButtonElement;
const picked = () =>
  document.querySelector(".branch-row[aria-pressed=true] .branch-name")?.textContent;
const branchNames = () => [...document.querySelectorAll(".branch-name")].map((b) => b.textContent);
const plan = () => document.querySelector(".plan")?.textContent;
const type = (value: string) => fireEvent.change(nameField(), { target: { value } });

test("the project row opens the dialog for that project, with its branches", async () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  fireEvent.click(screen.getAllByTitle("New worktree (Ctrl+Shift+N)")[1]);
  const dialog = screen.getByRole("dialog", { name: "New worktree" }) as HTMLDialogElement;
  expect(dialog.open).toBe(true);
  expect(document.activeElement).toBe(nameField());
  expect((screen.getByLabelText("Project") as HTMLSelectElement).value).toBe(api.id);
  expect(create().disabled).toBe(true);
  await waitFor(() => expect(picked()).toBe("main"));
  expect(branchNames()).toEqual([
    "main",
    "refactor-auth",
    "origin/main",
    "origin/v2-legacy",
    "origin/feat-webhooks",
  ]);
  expect(document.querySelector(".branch-row .badge")?.textContent).toBe("default");
  expect([...document.querySelectorAll(".branch-head")].map((h) => h.textContent)).toEqual([
    "Local",
    "Remote",
  ]);
  // The empty name gets no error, only the service's placeholder preview.
  await waitFor(() =>
    expect(plan()).toBe("Folder: .claude/worktrees/<name>/Branch: worktree-<name> (from main)"),
  );
  expect(screen.queryByRole("alert")).toBeNull();
});

test("the service's verdict on the name is shown as the user types", async () => {
  open();
  type("Fix");
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toStartWith('invalid worktree name "Fix": use lowercase letters');
  expect(nameField().getAttribute("aria-invalid")).toBe("true");
  expect(create().disabled).toBe(true);
  type("fix-login");
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toStartWith(
      'worktree "fix-login" already exists',
    ),
  );
  type("fix-cart");
  await waitFor(() => expect(create().disabled).toBe(false));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(plan()).toBe("Folder: .claude/worktrees/fix-cart/Branch: worktree-fix-cart (from main)");
});

test("an answer for an older name, arriving late, is ignored", async () => {
  open();
  type("fix-cart");
  await waitFor(() => expect(create().disabled).toBe(false));
  act(() =>
    apply({
      type: "worktree_name_validated",
      project: shop.id,
      name: "old",
      folder: "f",
      branch: "b",
      error: "stale",
    }),
  );
  expect(screen.queryByRole("alert")).toBeNull();
  expect(create().disabled).toBe(false);
});

test("the filter narrows local and remote branches and picks the first one shown", async () => {
  open();
  await waitFor(() => expect(picked()).toBe("main"));
  // Long remote lists are virtualized: only the rows in view are in the DOM.
  expect(branchNames().length).toBeLessThan(30);
  fireEvent.change(filter(), { target: { value: "RELEASE" } });
  expect(branchNames()).toEqual(["origin/release/2.4"]);
  expect(picked()).toBe("origin/release/2.4");
  expect(plan()).toEndWith("(from origin/release/2.4)");
  fireEvent.change(filter(), { target: { value: "nothing-like-this" } });
  expect(document.querySelector(".branch-empty")?.textContent).toBe("No branches found");
  fireEvent.keyDown(filter(), { key: "ArrowDown" });
  // With nothing shown, the base is the user's pick, or the default.
  expect(plan()).toEndWith("(from main)");
  fireEvent.change(filter(), { target: { value: "dependency-19" } });
  expect(branchNames()).toEqual([
    "origin/renovate/dependency-19",
    "origin/renovate/dependency-190",
    "origin/renovate/dependency-191",
    "origin/renovate/dependency-192",
    "origin/renovate/dependency-193",
    "origin/renovate/dependency-194",
    "origin/renovate/dependency-195",
    "origin/renovate/dependency-196",
    "origin/renovate/dependency-197",
    "origin/renovate/dependency-198",
    "origin/renovate/dependency-199",
  ]);
});

test("arrow keys and clicks pick the base branch", async () => {
  open();
  await waitFor(() => expect(picked()).toBe("main"));
  fireEvent.keyDown(filter(), { key: "ArrowUp" });
  expect(picked()).toBe("main");
  fireEvent.keyDown(filter(), { key: "ArrowDown" });
  fireEvent.keyDown(filter(), { key: "ArrowDown" });
  expect(picked()).toBe("fix-login");
  fireEvent.keyDown(filter(), { key: "a" });
  expect(picked()).toBe("fix-login");
  fireEvent.click(screen.getByRole("button", { name: "origin/main" }));
  expect(picked()).toBe("origin/main");
});

test("create asks the service, opens a terminal in the new worktree and updates the tree", async () => {
  const createWorktree = spyOn(transport, "createWorktree");
  const openTerminal = spyOn(transport, "openTerminal");
  const write = spyOn(transport, "writeTerminal");
  open();
  await waitFor(() => expect(picked()).toBe("main"));
  fireEvent.click(screen.getByRole("button", { name: "develop" }));
  type("fix-cart");
  await waitFor(() => expect(create().disabled).toBe(false));
  fireEvent.click(create());
  expect(createWorktree).toHaveBeenCalledWith(shop.id, "fix-cart", "develop");
  await waitFor(() => expect(useHive.getState().modal).toBeNull());
  const path = `${shop.path}/.claude/worktrees/fix-cart`;
  expect(openTerminal).toHaveBeenCalledTimes(1);
  expect(openTerminal.mock.calls[0].slice(0, 3)).toEqual([path, 80, 24]);
  expect(useHive.getState().selection).toBe(path);
  expect(screen.getByRole("button", { name: "fix-cart" })).toBeDefined();
  // Its tab is shown, with the worktree's name (after the agent's state, once claude runs).
  await waitFor(() => expect(screen.getByRole("tab", { name: /fix-cart$/ })).toBeDefined());
  expect(useHive.getState().tabs.map((t) => t.cwd)).toEqual([path]);
  // Claude is started in it, typed as the user would.
  const id = useHive.getState().tabs[0].id;
  await waitFor(() => expect(write).toHaveBeenCalledWith(id, "claude\r"));
  closeTerminal(id);
  createWorktree.mockRestore();
  openTerminal.mockRestore();
  write.mockRestore();
});

test("unticking Start claude opens a plain terminal; it needs the terminal option", async () => {
  const write = spyOn(transport, "writeTerminal");
  open();
  const claude = screen.getByLabelText("Start claude in the terminal") as HTMLInputElement;
  expect(claude.checked).toBe(true);
  fireEvent.click(screen.getByLabelText("Open a terminal in the new worktree"));
  expect([claude.checked, claude.disabled]).toEqual([false, true]);
  fireEvent.click(screen.getByLabelText("Open a terminal in the new worktree"));
  fireEvent.click(claude);
  expect([claude.checked, claude.disabled]).toEqual([false, false]);
  type("plain-shell");
  await waitFor(() => expect(create().disabled).toBe(false));
  fireEvent.click(create());
  await waitFor(() => expect(useHive.getState().tabs).toHaveLength(1));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(write).not.toHaveBeenCalled();
  closeTerminal(useHive.getState().tabs[0].id);
  write.mockRestore();
});

test("without the terminal option no terminal opens", async () => {
  const openTerminal = spyOn(transport, "openTerminal");
  open();
  fireEvent.click(screen.getByLabelText("Open a terminal in the new worktree"));
  type("no-terminal");
  await waitFor(() => expect(create().disabled).toBe(false));
  fireEvent.submit(create());
  await waitFor(() => expect(useHive.getState().modal).toBeNull());
  expect(openTerminal).not.toHaveBeenCalled();
  openTerminal.mockRestore();
});

test("the service's notes keep the dialog open until it is closed", async () => {
  open();
  const path = `${shop.path}/.claude/worktrees/x`;
  const notes = [
    "warning: .claude/settings.json defines its own WorktreeCreate hook; it will compete with Hive's",
  ];
  act(() => apply({ type: "worktree_created", project: shop, path, notes }));
  expect(document.querySelector(".created")?.textContent).toBe(`Created ${path}`);
  expect(document.querySelector(".notes")?.textContent).toBe(notes[0]);
  expect(useHive.getState().modal).toBe("new-worktree");
  fireEvent.click(screen.getByRole("button", { name: "Close Enter" }));
  expect(useHive.getState().modal).toBeNull();
});

test("a refused create is explained under the name until it changes", async () => {
  open();
  type("refused");
  await waitFor(() => expect(create().disabled).toBe(false));
  const message =
    "git worktree add -b worktree-refused failed: fatal: a branch named 'worktree-refused' already exists";
  act(() => apply({ type: "create_worktree_failed", project: shop.id, name: "refused", message }));
  expect(screen.getByRole("alert").textContent).toBe(message);
  expect(create().disabled).toBe(true);
  // Submitting while refused sends nothing.
  const createWorktree = spyOn(transport, "createWorktree");
  fireEvent.submit(create());
  expect(createWorktree).not.toHaveBeenCalled();
  createWorktree.mockRestore();
  type("refused2");
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

test("another project asks for its branches; a failure to list them is shown", async () => {
  const listBranches = spyOn(transport, "listBranches");
  open(null);
  const project = screen.getByLabelText("Project") as HTMLSelectElement;
  expect(project.value).toBe(shop.id);
  fireEvent.change(project, { target: { value: api.id } });
  expect(listBranches).toHaveBeenLastCalledWith(api.id);
  listBranches.mockRestore();
  const error = "git for-each-ref failed";
  act(() =>
    apply({ type: "branches", project: api.id, local: [], remote: [], current: null, error }),
  );
  expect(document.querySelector(".branch-empty")?.textContent).toBe(error);
  expect(plan()).not.toContain("(from");
});

test("close, cancel and Esc close the dialog; it needs a project and a connection", () => {
  for (const how of ["Close (Esc)", "Cancel Esc", "Escape"]) {
    const dialog = open();
    if (how === "Escape") fireEvent(dialog, new Event("close"));
    else if (how === "Cancel Esc") fireEvent.click(screen.getByRole("button", { name: how }));
    else fireEvent.click(screen.getByTitle(how));
    expect(useHive.getState().modal).toBeNull();
    cleanup();
  }
  open();
  act(() => apply({ type: "disconnected", reason: "gone" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  cleanup();
  useHive.setState(initialState, true);
  render(<App />);
  act(() => openModal("new-worktree"));
  expect(screen.queryByRole("dialog")).toBeNull();
});
