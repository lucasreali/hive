import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { asMac } from "../../test/mac";
import { App } from "../App";
import {
  apply,
  DEFAULT_SETTINGS,
  initialState,
  NO_SCRIPTS,
  openModal,
  type Project,
  select,
  useHive,
} from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [main, login] = shop.worktrees;
// A linked worktree outside `.claude/worktrees`, and one detached.
const outside = { ...login, id: "/elsewhere", path: "/elsewhere", name: "out", claude: false };
const detached = { ...login, id: "/d", path: "/d", name: "d", branch: null };
const project: Project = { ...shop, worktrees: [main, login, outside, detached] };

function show() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [project] }));
}

const row = (name: string) =>
  within(screen.getByRole("navigation", { name: "Projects" }))
    .getAllByRole("button", { name: new RegExp(`^${name}`) })
    .find((b) => b.classList.contains("row-main")) as HTMLElement;
const menu = () => screen.queryByRole("menu");
const item = (name: string) => screen.getByRole("menuitem", { name }) as HTMLButtonElement;
const rightClick = (name: string) => fireEvent.contextMenu(row(name), { clientX: 40, clientY: 60 });

test("a worktree's right click opens its menu at the pointer; main and foreign ones are limited", () => {
  show();
  rightClick("fix-login");
  expect(screen.getByRole("menu", { name: "Worktree fix-login" })).toBeTruthy();
  const items = screen
    .getAllByRole("menuitem")
    .map((i) => [i.textContent, i.hasAttribute("disabled")]);
  expect(items).toEqual([
    ["New terminal here", false],
    ["Copy path", false],
    ["Open in Explorer", false],
    ["Rename…", false],
    ["Delete…", false],
  ]);
  expect(document.activeElement).toBe(item("New terminal here"));
  expect((menu() as HTMLElement).style.left).toBe("40px");
  expect((menu() as HTMLElement).style.top).toBe("60px");

  rightClick("main");
  expect(item("Rename…").disabled).toBe(true);
  expect(item("Rename…").title).toBe("The main worktree cannot be renamed");
  expect(item("Delete…").disabled).toBe(true);
  expect(item("Delete…").title).toBe("The main worktree cannot be deleted");

  rightClick("out");
  expect(item("Rename…").title).toBe("Only worktrees under .claude/worktrees can be renamed");
  expect(item("Delete…").disabled).toBe(false);
});

test("the menu key opens it under the row", () => {
  show();
  fireEvent.contextMenu(row("fix-login"), { clientX: 0, clientY: 0 });
  // happy-dom has no layout: the row's box is all zeros.
  expect((menu() as HTMLElement).style.left).toBe("24px");
  expect(useHive.getState().menu).toEqual({ worktree: login.id, x: 24, y: 0 });
});

test("arrows move between enabled items; Esc, Tab, a click outside, scrolling and blur close it", () => {
  show();
  rightClick("main");
  const m = menu() as HTMLElement;
  fireEvent.keyDown(m, { key: "ArrowDown" });
  expect(document.activeElement).toBe(item("Copy path"));
  fireEvent.keyDown(m, { key: "ArrowDown" });
  fireEvent.keyDown(m, { key: "ArrowDown" });
  // Rename and Delete are disabled on main: it wraps to the first item.
  expect(document.activeElement).toBe(item("New terminal here"));
  fireEvent.keyDown(m, { key: "ArrowUp" });
  expect(document.activeElement).toBe(item("Open in Explorer"));
  fireEvent.keyDown(m, { key: "x" });
  expect(menu()).toBeTruthy();
  fireEvent.keyDown(m, { key: "Escape" });
  expect(menu()).toBeNull();

  rightClick("main");
  fireEvent.keyDown(menu() as HTMLElement, { key: "Tab" });
  expect(menu()).toBeNull();

  rightClick("main");
  fireEvent.pointerDown(item("Copy path"));
  expect(menu()).toBeTruthy();
  fireEvent.pointerDown(document.body);
  expect(menu()).toBeNull();

  rightClick("main");
  fireEvent.scroll(document);
  expect(menu()).toBeNull();
  rightClick("main");
  fireEvent.blur(window);
  expect(menu()).toBeNull();
  rightClick("main");
  fireEvent(window, new Event("resize"));
  expect(menu()).toBeNull();

  // A worktree that went away takes its menu with it.
  rightClick("fix-login");
  act(() => apply({ type: "projects", projects: [] }));
  expect(menu()).toBeNull();
});

test("new terminal, copy path and Explorer act on the worktree", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(7);
  const explorer = spyOn(transport, "openInEditor").mockResolvedValue();
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  show();
  rightClick("fix-login");
  fireEvent.click(item("New terminal here"));
  expect(menu()).toBeNull();
  expect(open.mock.calls[0]?.[0]).toBe(login.path);

  rightClick("fix-login");
  fireEvent.click(item("Copy path"));
  expect(writeText).toHaveBeenCalledWith(login.path);
  const notice = await screen.findByTitle("Dismiss");
  expect(notice.textContent).toBe(`Copied ${login.path}`);
  fireEvent.click(notice);
  expect(screen.queryByTitle("Dismiss")).toBeNull();

  writeText.mockRejectedValue("denied");
  rightClick("fix-login");
  fireEvent.click(item("Copy path"));
  expect((await screen.findByTitle("Dismiss")).textContent).toBe("Cannot copy the path: denied");

  rightClick("fix-login");
  fireEvent.click(item("Open in Explorer"));
  expect(explorer).toHaveBeenCalledWith(login.path, "");
  open.mockRestore();
  explorer.mockRestore();
  writeText.mockRestore();
});

test("each run script of the project is a menu item typing it into a new terminal", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  show();
  const settings = structuredClone(DEFAULT_SETTINGS);
  const run = [
    { name: "dev", command: "bun dev --port $HIVE_PORT" },
    { name: "test", command: "bun test" },
  ];
  settings.projects[shop.id] = { scripts: { ...NO_SCRIPTS, run } };
  act(() => apply({ type: "settings", settings }));
  rightClick("fix-login");
  const items = screen.getAllByRole("menuitem").map((i) => i.textContent);
  expect(items.slice(0, 4)).toEqual(["New terminal here", "Run: dev", "Run: test", "Copy path"]);
  expect(item("Run: dev").title).toBe("bun dev --port $HIVE_PORT");
  fireEvent.click(item("Run: dev"));
  expect(menu()).toBeNull();
  expect(open.mock.calls[0]?.[0]).toBe(login.path);
  await waitFor(() => expect(write).toHaveBeenCalledWith(8, "bun dev --port $HIVE_PORT\r"));
  open.mockRestore();
  write.mockRestore();
});

test("on macOS the folder is revealed in the Finder", () => {
  const finder = spyOn(transport, "openInEditor").mockResolvedValue();
  asMac();
  show();
  rightClick("fix-login");
  expect(screen.queryByRole("menuitem", { name: "Open in Explorer" })).toBeNull();
  fireEvent.click(item("Reveal in Finder"));
  expect(finder).toHaveBeenCalledWith(login.path, "");
  finder.mockRestore();
});

test("delete asks first; a refusal shows why and offers to force it", () => {
  const remove = spyOn(transport, "removeWorktree").mockResolvedValue();
  show();
  rightClick("fix-login");
  fireEvent.click(item("Delete…"));
  const dialog = screen.getByRole("dialog", { name: "Delete worktree" }) as HTMLDialogElement;
  expect(dialog.open).toBe(true);
  expect(dialog.textContent).toContain(`Delete fix-login and its folder ${login.path}?`);
  expect(dialog.textContent).toContain(`The branch ${login.branch} is kept.`);
  const submit = within(dialog).getByRole("button", { name: "Delete Enter" });
  expect(document.activeElement).toBe(submit);
  fireEvent.click(submit);
  expect(remove).toHaveBeenLastCalledWith(login.path, false);

  // A refusal for another worktree, or a rename's, is not this one's.
  act(() => apply({ type: "remove_worktree_failed", path: "/other", message: "x" }));
  const rename = {
    type: "rename_worktree_failed",
    path: login.path,
    name: "y",
    message: "x",
  } as const;
  act(() => apply(rename));
  expect(within(dialog).queryByRole("alert")).toBeNull();
  const message = "contains modified or untracked files, use --force to delete it";
  act(() => apply({ type: "remove_worktree_failed", path: login.path, message }));
  expect(within(dialog).getByRole("alert").textContent).toBe(message);
  fireEvent.click(within(dialog).getByRole("button", { name: "Delete anyway Enter" }));
  expect(remove).toHaveBeenLastCalledWith(login.path, true);

  select(login.id);
  const updated = { ...project, worktrees: [main, outside, detached] };
  act(() => apply({ type: "worktree_removed", project: updated, path: login.path }));
  expect(screen.queryByRole("dialog", { name: "Delete worktree" })).toBeNull();
  expect(useHive.getState().selection).toBe(project.id);

  // A detached worktree has no branch to keep; Cancel closes.
  act(() => openModal("remove-worktree", null, detached.id));
  const again = screen.getByRole("dialog", { name: "Delete worktree" });
  expect(again.textContent).not.toContain("is kept");
  fireEvent.click(within(again).getByRole("button", { name: "Cancel Esc" }));
  expect(useHive.getState().modal).toBeNull();
  remove.mockRestore();
});

test("rename checks the name with the service and moves the selection along", () => {
  const validate = spyOn(transport, "validateWorktreeName").mockResolvedValue();
  const rename = spyOn(transport, "renameWorktree").mockResolvedValue();
  show();
  select(login.id);
  rightClick("fix-login");
  fireEvent.click(item("Rename…"));
  const dialog = screen.getByRole("dialog", { name: "Rename worktree" });
  const field = within(dialog).getByLabelText("New name") as HTMLInputElement;
  const submit = within(dialog).getByRole("button", { name: "Rename Enter" }) as HTMLButtonElement;
  expect(document.activeElement).toBe(field);
  expect(field.value).toBe("fix-login");
  expect(validate).toHaveBeenLastCalledWith(shop.id, "fix-login");
  // The current name "exists", but that is no error here.
  const check = (name: string, error: string | null) =>
    act(() =>
      apply({
        type: "worktree_name_validated",
        project: shop.id,
        name,
        folder: `.claude/worktrees/${name}/`,
        branch: `worktree-${name}`,
        error,
      }),
    );
  check("fix-login", 'worktree "fix-login" already exists');
  expect(within(dialog).queryByRole("alert")).toBeNull();
  expect(submit.disabled).toBe(true);

  fireEvent.change(field, { target: { value: "Bad" } });
  check("Bad", "invalid worktree name");
  expect(within(dialog).getByRole("alert").textContent).toBe("invalid worktree name");
  fireEvent.submit(submit);
  expect(rename).not.toHaveBeenCalled();

  fireEvent.change(field, { target: { value: "auth" } });
  check("auth", null);
  expect(dialog.querySelector(".plan")?.textContent).toBe(
    "Folder: .claude/worktrees/auth/Branch: worktree-auth",
  );
  expect(submit.disabled).toBe(false);
  fireEvent.click(submit);
  expect(rename).toHaveBeenCalledWith(login.path, "auth");
  const failure = { type: "rename_worktree_failed", path: login.path, name: "auth" } as const;
  act(() => apply({ ...failure, message: "in use by fish (3): close its terminals first" }));
  expect(within(dialog).getByRole("alert").textContent).toContain("in use by fish (3)");
  expect(submit.disabled).toBe(true);

  const path = `${shop.path}/.claude/worktrees/auth`;
  const renamed = { ...login, id: path, path, name: "auth", branch: "worktree-auth" };
  const updated = { ...project, worktrees: [main, renamed, outside, detached] };
  act(() => apply({ type: "worktree_renamed", project: updated, from: login.path, path }));
  expect(screen.queryByRole("dialog", { name: "Rename worktree" })).toBeNull();
  expect(useHive.getState().selection).toBe(path);
  expect(row("auth")).toBeTruthy();

  // Another worktree's rename leaves the selection and an open dialog alone.
  act(() => openModal("rename-worktree", null, detached.id));
  const again = screen.getByRole("dialog", { name: "Rename worktree" });
  expect(again.querySelector(".plan")?.textContent).toContain("Branch: detached (unchanged)");
  act(() => apply({ type: "worktree_renamed", project: updated, from: "/x", path: "/y" }));
  expect(useHive.getState().selection).toBe(path);
  expect(useHive.getState().modal).toBe("rename-worktree");
  fireEvent.click(within(again).getByTitle("Close (Esc)"));
  expect(useHive.getState().modal).toBeNull();

  // A worktree on another branch keeps it.
  act(() => openModal("rename-worktree", null, outside.id));
  expect(document.querySelector(".plan")?.textContent).toContain(`${outside.branch} (unchanged)`);
  validate.mockRestore();
  rename.mockRestore();
});

test("a dialog for a worktree that is gone shows nothing", () => {
  show();
  act(() => openModal("remove-worktree", null, "/gone"));
  expect(screen.queryByRole("dialog")).toBeNull();
  act(() => openModal("rename-worktree", null, "/gone"));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("the project menu removes merged worktrees without changes, each with its result", () => {
  const remove = spyOn(transport, "removeWorktree").mockResolvedValue();
  const health = (merged: boolean, changes: number) => ({
    changes,
    ahead: merged ? 0 : 1,
    behind: 0,
    merged,
    last_commit_ms: 0,
  });
  const wt = (name: string, status: ReturnType<typeof health> | null) => ({
    ...login,
    id: `/r/${name}`,
    path: `/r/${name}`,
    name,
    status,
  });
  const [a, b] = [wt("a", health(true, 0)), wt("b", health(true, 0))];
  const others = [wt("dirty", health(true, 1)), wt("ahead", health(false, 0)), wt("x", null)];
  const merged = { ...shop, worktrees: [{ ...main, status: health(false, 0) }, a, b, ...others] };
  render(<App />);
  act(() => apply({ type: "projects", projects: [merged] }));
  fireEvent.contextMenu(row("shop"), { clientX: 10, clientY: 20 });
  expect(useHive.getState().projectMenu).toEqual({ project: shop.id, x: 10, y: 20 });
  expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual([
    "New worktree… Ctrl+Shift+N",
    "Remove merged worktrees…",
  ]);
  fireEvent.keyDown(screen.getByRole("menu", { name: "Project" }), { key: "Escape" });
  expect(menu()).toBeNull();
  fireEvent.contextMenu(row("shop"), { clientX: 10, clientY: 20 });
  fireEvent.click(item("Remove merged worktrees…"));
  expect(menu()).toBeNull();
  expect(useHive.getState().modalProject).toBe(shop.id);

  const dialog = screen.getByRole("dialog", { name: "Remove merged worktrees" });
  const box = (name: string) =>
    within(dialog).getByRole("checkbox", { name: new RegExp(`^${name}`) });
  const submit = () => within(dialog).getByRole("button", { name: /^Remove/ }) as HTMLButtonElement;
  expect(within(dialog).getAllByRole("checkbox")).toHaveLength(2);
  expect([box("a"), box("b")].map((c) => (c as HTMLInputElement).checked)).toEqual([true, true]);
  expect(submit().textContent).toBe("Remove (2) Enter");
  fireEvent.click(box("b"));
  fireEvent.click(box("a"));
  fireEvent.click(box("a"));
  expect(submit().textContent).toBe("Remove (1) Enter");
  fireEvent.click(submit());
  expect(remove.mock.calls).toEqual([[a.path, false]]);
  expect((box("a") as HTMLInputElement).disabled).toBe(true);
  expect(within(dialog).getByRole("status").textContent).toBe("Removing…");

  // Each answer shows next to its worktree; the dialog stays open.
  const without = { ...merged, worktrees: merged.worktrees.filter((w) => w.path !== a.path) };
  act(() => apply({ type: "worktree_removed", project: without, path: a.path }));
  expect(within(dialog).getByRole("status").textContent).toBe("Removed");
  fireEvent.click(box("b"));
  fireEvent.click(submit());
  expect(remove.mock.calls).toEqual([
    [a.path, false],
    [b.path, false],
  ]);
  act(() => apply({ type: "remove_worktree_failed", path: b.path, message: "in use by fish (1)" }));
  const results = within(dialog).getAllByRole("status");
  expect(results.map((r) => [r.textContent, r.className])).toEqual([
    ["Removed", "field-help"],
    ["in use by fish (1)", "field-error"],
  ]);
  expect(submit().disabled).toBe(true);
  fireEvent.click(within(dialog).getByRole("button", { name: /^Close Esc/ }));
  expect(useHive.getState().modal).toBeNull();
});

test("with no merged worktree the dialog says so", () => {
  show();
  act(() => openModal("remove-merged", shop.id));
  const dialog = screen.getByRole("dialog", { name: "Remove merged worktrees" });
  expect(dialog.textContent).toContain("No merged worktrees without changes.");
  expect(
    (within(dialog).getByRole("button", { name: /^Remove/ }) as HTMLButtonElement).disabled,
  ).toBe(true);
});
