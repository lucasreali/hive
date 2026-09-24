import { afterEach, beforeAll, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  apply,
  type ChangedFile,
  type Changes,
  type FileText,
  initialState,
  select,
  setOpenFile,
  setRightPanel,
  useHive,
} from "../store";
import { transport } from "../transport";
import { MOCK_CHANGES, MOCK_REPOS } from "../transport/mock";
import { allFiles, fileRows, RightPanel } from "./RightPanel";

beforeAll(() => {
  // happy-dom has no layout: give the tree its CSS size so the virtualizer shows rows.
  for (const [key, size] of [
    ["offsetHeight", 400],
    ["offsetWidth", 380],
  ] as const) {
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, key)?.get;
    Object.defineProperty(HTMLElement.prototype, key, {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains("files-tree") ? size : original?.call(this);
      },
    });
  }
});

afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const refactor = api.worktrees[1];
const fixLogin = shop.worktrees[1];

const file = (path: string, status: ChangedFile["status"] = "modified"): ChangedFile => ({
  path,
  status,
  old_path: null,
  added: 1,
  removed: 0,
});

function changes(path: string, files: ChangedFile[], error: string | null = null): Changes {
  const sum = (key: "added" | "removed") => files.reduce((n, f) => n + (f[key] ?? 0), 0);
  return { path, files, added: sum("added"), removed: sum("removed"), error };
}

function panel() {
  const asked = spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  render(<RightPanel />);
  return asked;
}

const rows = () => screen.queryAllByRole("treeitem").map((r) => [r.textContent, r.dataset.status]);
const tree = () => screen.getByRole("tree", { name: "Files" });

test("rows group paths into folders, folders first, with the strongest status inside", () => {
  const files = [
    file("README.md"),
    file("src/a.ts", "added"),
    file("src/b/c.ts", "deleted"),
    file("src/b/d.ts", "renamed"),
    file("src/z.ts", "untracked"),
    file("test/t.ts", "renamed"),
    file("test/u.ts"),
  ];
  const shape = (collapsed: Record<string, boolean>) =>
    fileRows("/w", files, collapsed).map((r) =>
      r.kind === "folder" ? [r.depth, `${r.name}/`, r.status, r.open] : [r.depth, r.name],
    );
  expect(shape({})).toEqual([
    [0, "src/", "deleted", true],
    [1, "b/", "deleted", true],
    [2, "c.ts"],
    [2, "d.ts"],
    [1, "a.ts"],
    [1, "z.ts"],
    [0, "test/", "renamed", true],
    [1, "t.ts"],
    [1, "u.ts"],
    [0, "README.md"],
  ]);
  expect(shape({ "folder:/w/src/b": true, "folder:/other/test": true })).toEqual([
    [0, "src/", "deleted", true],
    [1, "b/", "deleted", false],
    [1, "a.ts"],
    [1, "z.ts"],
    [0, "test/", "renamed", true],
    [1, "t.ts"],
    [1, "u.ts"],
    [0, "README.md"],
  ]);
  const key = fileRows("/w", files, {})[1].key;
  expect(key).toBe("folder:/w/src/b");
});

test("without a worktree the panel says what to select", () => {
  const asked = panel();
  expect(screen.getByText("Select a project or agent to see its files.")).toBeDefined();
  expect(asked).not.toHaveBeenCalled();
});

test("the selected worktree's changes: summary, totals, letters and counts", () => {
  const asked = panel();
  act(() => select(refactor.id));
  expect(asked).toHaveBeenCalledWith(refactor.path);
  expect(screen.getByText("refactor-auth")).toBeDefined();
  expect(screen.getByText("api")).toBeDefined();
  // Nothing is said before the service answers.
  expect(screen.queryByText(/changed|No changes/)).toBeNull();
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  expect(document.querySelector(".files-summary")?.textContent).toBe("5 files changed+24−49");
  expect(rows()).toEqual([
    ["assets", "M"],
    ["logo.pngM", "M"],
    ["src", "D"],
    ["auth", "R"],
    ["token.ts+2−1R", "R"],
    ["legacy", "D"],
    ["jwt.ts−30D", "D"],
    ["middleware", "M"],
    ["auth.ts+21−17M", "M"],
    ["package.json+1−1M", "M"],
  ]);
  const deleted = screen.getByRole("treeitem", { name: /jwt\.ts/ });
  expect(deleted.dataset.deleted).toBe("true");
  expect(deleted.getAttribute("title")).toBe("src/legacy/jwt.ts");
});

test("one file, no changes, and the service's error", () => {
  panel();
  act(() => select(fixLogin.id));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("a.ts", "untracked")]) }));
  expect(document.querySelector(".files-summary")?.textContent).toBe("1 file changed+1");
  expect(rows()).toEqual([["a.ts+1A", "A"]]);
  act(() => apply({ type: "changes", ...changes(fixLogin.path, []) }));
  expect(document.querySelector(".files-summary")?.textContent).toBe("No changes");
  expect(screen.getByText("No changes in this worktree.")).toBeDefined();
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [], "git status failed") }));
  expect(screen.getByText("git status failed")).toBeDefined();
});

test("the panel follows the selected agent, else the shown terminal", () => {
  const asked = panel();
  act(() => useHive.setState({ tabs: [{ id: 3, cwd: fixLogin.path }], activeTab: 3 }));
  expect(screen.getByText("fix-login")).toBeDefined();
  act(() =>
    apply({
      type: "agent_detected",
      channel: 3,
      id: "s1",
      project: api.id,
      worktree: refactor.id,
      cwd: refactor.path,
    }),
  );
  act(() => select("s1"));
  expect(screen.getByText("refactor-auth")).toBeDefined();
  // A selected project shows its main worktree.
  act(() => select(shop.id));
  expect(screen.getByText("main")).toBeDefined();
  expect(asked.mock.calls.map(([p]) => p)).toEqual([fixLogin.path, refactor.path, shop.path]);
});

test("clicking a file opens it under the tree; folders collapse; Close diff closes it", () => {
  panel();
  act(() => select(refactor.id));
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  fireEvent.click(screen.getByRole("treeitem", { name: /token\.ts/ }));
  expect(useHive.getState().openFile).toEqual({
    worktree: refactor.path,
    path: "src/auth/token.ts",
  });
  expect(screen.getByRole("treeitem", { name: /token\.ts/ }).getAttribute("aria-selected")).toBe(
    "true",
  );
  const view = screen.getByRole("region", { name: "src/auth/token.ts" });
  expect(view.querySelector(".file-view-bar")?.textContent).toBe("Rsrc/auth/token.ts+2−1");
  expect(view.textContent).not.toContain("No changes in this file.");

  fireEvent.click(screen.getByRole("treeitem", { name: "auth" }));
  expect(screen.queryByRole("treeitem", { name: /token\.ts/ })).toBeNull();
  expect(
    screen.getByRole("treeitem", { name: "auth" }).querySelector(".status-dot"),
  ).not.toBeNull();

  fireEvent.click(screen.getByTitle("Close diff"));
  expect(useHive.getState().openFile).toBeNull();
  expect(screen.queryByRole("region")).toBeNull();
});

test("a file without changes says so, and another worktree's file is not shown", () => {
  panel();
  act(() => select(refactor.id));
  act(() => useHive.setState({ openFile: { worktree: refactor.path, path: "README.md" } }));
  const view = screen.getByRole("region", { name: "README.md" });
  expect(view.textContent).toContain("No changes in this file.");
  act(() => useHive.setState({ openFile: { worktree: fixLogin.path, path: "README.md" } }));
  expect(screen.queryByRole("region")).toBeNull();
});

test("the open file's text shows as a diff when changed, else as is, or why not", () => {
  panel();
  act(() => select(refactor.id));
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  const text = (path: string, patch: Partial<FileText> = {}): FileText => ({
    worktree: refactor.path,
    path,
    content: "new\n",
    base: "old\n",
    version: "v",
    binary: false,
    too_large: false,
    error: null,
    ...patch,
  });
  const body = () => document.querySelector(".file-view-body") as HTMLElement;
  const editor = () => body().querySelector(".cm-editor");
  act(() => setOpenFile({ worktree: refactor.path, path: "src/legacy/jwt.ts" }));
  expect(editor()).toBeNull(); // Nothing until the service answers.
  // An answer for another file is not this one's.
  act(() => apply({ type: "file", ...text("package.json") }));
  expect(editor()).toBeNull();

  act(() => apply({ type: "file", ...text("src/legacy/jwt.ts", { content: null }) }));
  const shown = editor();
  expect(shown?.classList.contains("cm-merge-b")).toBe(true);
  expect(body().querySelector(".cm-deletedChunk")?.textContent).toBe("old");
  // A new answer updates the same view.
  act(() => apply({ type: "file", ...text("src/legacy/jwt.ts", { content: "x\n" }) }));
  expect(editor()).toBe(shown);
  expect(body().querySelector(".cm-content > .cm-line")?.textContent).toBe("x");

  act(() => setOpenFile({ worktree: refactor.path, path: "README.md" }));
  act(() => apply({ type: "file", ...text("README.md", { base: "new\n" }) }));
  expect(body().textContent).toStartWith("No changes in this file.");
  expect(editor()?.classList.contains("cm-merge-b")).toBe(false);
  expect(body().querySelector(".cm-content")?.textContent).toBe("new");

  for (const [patch, why] of [
    [{ binary: true, content: null, base: null }, "Binary file not shown."],
    [{ too_large: true, content: null, base: null }, "File too large to show."],
    [{ error: "README.md does not exist" }, "README.md does not exist"],
  ] as const) {
    act(() => apply({ type: "file", ...text("README.md", patch) }));
    expect(body().textContent).toBe(`No changes in this file.${why}`);
    expect(editor()).toBeNull();
  }
});

test("the tree works from the keyboard", () => {
  panel();
  act(() => select(refactor.id));
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  const active = () => document.getElementById(tree().getAttribute("aria-activedescendant") ?? "");
  const key = (k: string) => fireEvent.keyDown(tree(), { key: k });
  expect(active()?.textContent).toBe("assets");
  key("ArrowUp");
  expect(active()?.textContent).toBe("assets");
  key("ArrowLeft");
  expect(rows()[1]).toEqual(["src", "D"]);
  key("ArrowLeft");
  expect(screen.getByRole("treeitem", { name: "assets" }).getAttribute("aria-expanded")).toBe(
    "false",
  );
  key("ArrowRight");
  key("ArrowRight");
  expect(rows()[1]?.[0]).toBe("logo.pngM");
  key("ArrowDown");
  key("Enter");
  expect(useHive.getState().openFile?.path).toBe("assets/logo.png");
  key("ArrowUp");
  key(" ");
  expect(screen.getByRole("treeitem", { name: "assets" }).getAttribute("aria-expanded")).toBe(
    "false",
  );
  // Other keys are left alone.
  expect(fireEvent.keyDown(tree(), { key: "a" })).toBe(true);
  for (let i = 0; i < 20; i++) key("ArrowDown");
  expect(active()?.textContent).toBe("package.json+1−1M");
});

test("an empty tree ignores keys", () => {
  panel();
  act(() => select(fixLogin.id));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, []) }));
  expect(tree().getAttribute("aria-activedescendant")).toBeNull();
  expect(fireEvent.keyDown(tree(), { key: "ArrowDown" })).toBe(true);
});

test("All and Changed switch the mode; the header button closes the panel", () => {
  panel();
  act(() => setRightPanel("files"));
  const all = screen.getByRole("button", { name: "All" });
  const changed = screen.getByRole("button", { name: "Changed" });
  expect([all.getAttribute("aria-pressed"), changed.getAttribute("aria-pressed")]).toEqual([
    "true",
    "false",
  ]);
  fireEvent.click(changed);
  expect(useHive.getState().changedOnly).toBe(true);
  expect(changed.getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(all);
  expect(useHive.getState().changedOnly).toBe(false);
  fireEvent.click(screen.getByTitle("Collapse (Ctrl+Shift+B)"));
  expect(useHive.getState().rightPanel).toBeNull();
});

test("All lists every file with the changes' statuses, deleted files included", () => {
  const changed = [file("b.ts", "added"), file("gone/x.ts", "deleted")];
  const all = allFiles(["a.ts", "b.ts", "src/c.ts"], changed);
  expect(all.map((f) => [f.path, f.status])).toEqual([
    ["a.ts", null],
    ["b.ts", "added"],
    ["gone/x.ts", "deleted"],
    ["src/c.ts", null],
  ]);
  // A folder with nothing changed inside has no status.
  const folders = fileRows("/w", all, {}).filter((r) => r.kind === "folder");
  expect(folders.map((r) => r.kind === "folder" && [r.name, r.status])).toEqual([
    ["gone", "deleted"],
    ["src", null],
  ]);
});

test("the tree shows the watched worktree's files in All, only the changes in Changed", () => {
  panel();
  act(() => select(fixLogin.id));
  const listed = ["README.md", "src/auth/session.ts", "src/main.ts"];
  // Another worktree's list is not this one's.
  act(() => apply({ type: "files", path: refactor.path, files: ["x"], truncated: false }));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("src/auth/session.ts")]) }));
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth", "M"],
    ["session.ts+1M", "M"],
  ]);
  act(() => apply({ type: "files", path: fixLogin.path, files: listed, truncated: false }));
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth", "M"],
    ["session.ts+1M", "M"],
    ["main.ts", undefined],
    ["README.md", undefined],
  ]);
  expect(screen.queryByText(/cut short/)).toBeNull();
  // A collapsed folder with nothing changed inside shows no dot.
  act(() => apply({ type: "files", path: fixLogin.path, files: ["lib/x.ts"], truncated: true }));
  fireEvent.click(screen.getByText("lib"));
  expect(document.querySelectorAll(".status-dot")).toHaveLength(0);
  expect(screen.getByText("Too many files: the list is cut short.")).toBeDefined();
  act(() => useHive.setState({ changedOnly: true }));
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth", "M"],
    ["session.ts+1M", "M"],
  ]);
  expect(screen.queryByText(/cut short/)).toBeNull();
});
