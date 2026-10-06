import { afterEach, beforeAll, expect, jest, mock, spyOn, test } from "bun:test";
import { EditorView } from "@codemirror/view";
import {
  DefaultFileIcon,
  DefaultFolderIcon,
  DefaultFolderOpenedIcon,
  getIconForFile,
  getIconForFolder,
} from "@react-symbols/icons/utils";
import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expectClearButton } from "../../test/searchClear";
import type { ChangedFile, Changes, FileText, SearchResults } from "../protocol";
import { apply } from "../reduce";
import { initialState, select, setOpenFile, setPanelView, setRightPanel, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_CHANGES, MOCK_REPOS } from "../transport/mock";
import * as RightPanelModule from "./RightPanel";
import {
  allFiles,
  FilesView,
  fileRows,
  fileTarget,
  fileTree,
  HOVER_OPEN_MS,
  leaveFile,
  NAME_LIMIT,
  RightPanel,
} from "./RightPanel";
import { TerminalArea } from "./TerminalArea";

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
  const counts = { added: sum("added"), removed: sum("removed") };
  return { path, base: "head", branch: null, base_error: null, files, ...counts, error };
}

function panel() {
  const asked = spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  // Most of these check the changed files' tree.
  setPanelView("changes");
  // The open file shows in its tab of the terminal area.
  render(
    <>
      <RightPanel />
      <TerminalArea />
    </>,
  );
  return asked;
}

const rows = () => screen.queryAllByRole("treeitem").map((r) => [r.textContent, r.dataset.status]);
const tree = () => screen.getByRole("tree", { name: "Files" });
/** A row's name without its counts and letter: a closed folder's accessible name has its counts. */
const named = (name: string | RegExp) =>
  typeof name === "string"
    ? (_: string, row: Element | null) => row?.querySelector(".name")?.textContent === name
    : name;
const treeRow = (name: string | RegExp) => screen.getByRole("treeitem", { name: named(name) });

/** Opens every folder of the tree: they all start collapsed. */
function expand() {
  for (;;) {
    const closed = screen
      .queryAllByRole("treeitem")
      .find((r) => r.getAttribute("aria-expanded") === "false");
    if (!closed) return;
    fireEvent.click(closed);
  }
}

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
    fileRows("/w", fileTree(files), collapsed).map((r) =>
      r.kind === "folder" ? [r.depth, `${r.name}/`, r.status, r.open] : [r.depth, r.name],
    );
  // Every folder starts collapsed.
  expect(shape({})).toEqual([
    [0, "src/", "deleted", false],
    [0, "test/", "renamed", false],
    [0, "README.md"],
  ]);
  const open = { "files:/w/src": false, "files:/w/src/b": false, "files:/w/test": false };
  expect(shape(open)).toEqual([
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
  expect(shape({ ...open, "files:/w/src/b": true, "files:/other/test": true })).toEqual([
    [0, "src/", "deleted", true],
    [1, "b/", "deleted", false],
    [1, "a.ts"],
    [1, "z.ts"],
    [0, "test/", "renamed", true],
    [1, "t.ts"],
    [1, "u.ts"],
    [0, "README.md"],
  ]);
  const key = fileRows("/w", fileTree(files), open)[1].key;
  expect(key).toBe("files:/w/src/b");
  // The Diff tree keeps its own folders: the Files tree's open ones stay closed there.
  const diff = fileRows("/w", fileTree(files), open, "changes");
  expect(diff.map((r) => r.kind === "folder" && [r.key, r.open])).toEqual([
    ["changes:/w/src", false],
    ["changes:/w/test", false],
    false,
  ]);
});

test("without a worktree the panel says what to select", () => {
  const asked = panel();
  expect(screen.getByText("Select a project or agent to see its files.")).toBeDefined();
  expect(asked).not.toHaveBeenCalled();
});

test("the selected worktree's changes: summary, totals, letters and counts", () => {
  const asked = panel();
  act(() => select(refactor.id));
  expect(asked).toHaveBeenCalledWith(refactor.path, "branch");
  expect(screen.getByText("refactor-auth")).toBeDefined();
  // Not its project (5.8): the panel always shows a worktree of the project you are in.
  expect(screen.queryByText("api")).toBeNull();
  // Nothing is said before the service answers.
  expect(screen.queryByText(/changed|No changes/)).toBeNull();
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  expect(document.querySelector(".files-summary")?.textContent).toBe("5 files changed+24−49");
  // A closed folder sums its files' counts; a binary file (logo.png) adds none.
  expect(rows()).toEqual([
    ["assets", "M"],
    ["src+23−48", "D"],
    ["package.json+1−1M", "M"],
  ]);
  expand();
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

test("the changes compare with HEAD or the main branch, as picked, or say why not", () => {
  const asked = panel();
  act(() => select(fixLogin.id));
  const toggle = () => screen.queryByRole("group", { name: "Compare with" });
  const pressed = () =>
    screen.getAllByRole("button", { pressed: true }).filter((b) => toggle()?.contains(b));
  // Without a branch to compare with (the main worktree), no toggle.
  act(() => apply({ type: "changes", ...changes(fixLogin.path, []) }));
  expect(toggle()).toBeNull();

  const onBranch = { ...changes(fixLogin.path, []), base: "branch", branch: "main" } as const;
  act(() => apply({ type: "changes", ...onBranch }));
  expect(pressed().map((b) => b.textContent)).toEqual(["main"]);
  fireEvent.click(screen.getByRole("button", { name: "HEAD" }));
  expect(useHive.getState().diffBases).toEqual({ [fixLogin.path]: "head" });
  expect(asked.mock.calls).toEqual([
    [fixLogin.path, "branch"],
    [fixLogin.path, "head"],
  ]);
  act(() => apply({ type: "changes", ...onBranch, base: "head" }));
  expect(pressed().map((b) => b.textContent)).toEqual(["HEAD"]);
  fireEvent.click(screen.getByRole("button", { name: "main" }));
  expect(asked.mock.calls.at(-1)).toEqual([fixLogin.path, "branch"]);

  // No common base: HEAD, the branch disabled with the reason.
  const why = "No commit in common with main";
  act(() => apply({ type: "changes", ...onBranch, base: "head", base_error: why }));
  const branch = screen.getByRole("button", { name: "main" }) as HTMLButtonElement;
  expect([branch.disabled, branch.title]).toEqual([true, why]);
  // Without a branch to name, it is "Branch".
  const detached = { ...onBranch, base: "head", branch: null, base_error: why } as const;
  act(() => apply({ type: "changes", ...detached }));
  expect((screen.getByRole("button", { name: "Branch" }) as HTMLButtonElement).disabled).toBe(true);
});

test("the panel follows the selected agent, else the shown terminal", () => {
  const asked = panel();
  act(() => useHive.setState({ tabs: [{ id: 3, cwd: fixLogin.path }], activeTab: 3 }));
  // The terminal's tab has the same name: the panel's header is the one read.
  expect(document.querySelector(".files-worktree .name")?.textContent).toBe("fix-login");
  // Only the worktree: the project is the one you are in.
  expect(document.querySelector(".files-worktree")?.textContent).toBe("fix-login");
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

test("clicking a file opens it in a tab; folders collapse; the tab's close closes it", () => {
  panel();
  act(() => select(refactor.id));
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  expand();
  fireEvent.click(screen.getByRole("treeitem", { name: /token\.ts/ }));
  expect(useHive.getState().openFile).toEqual({
    worktree: refactor.path,
    path: "src/auth/token.ts",
  });
  expect(screen.getByRole("treeitem", { name: /token\.ts/ }).getAttribute("aria-selected")).toBe(
    "true",
  );
  expect(screen.getByRole("tab", { name: "token.ts" }).getAttribute("aria-selected")).toBe("true");
  const view = screen.getByRole("region", { name: "src/auth/token.ts" });
  expect(view.querySelector(".file-view-bar")?.textContent).toBe(
    "Rsrc/auth/token.ts+2−1EditComment",
  );
  expect(view.textContent).not.toContain("No changes in this file.");
  // Nothing selected yet: the reference cannot be sent, and the button says why.
  const send = screen.getByRole("button", { name: "Send to terminal" }) as HTMLButtonElement;
  expect(send.disabled).toBe(true);
  expect(send.title).toBe("Select lines to send their reference");
  act(() => useHive.setState({ selectedLines: { from: 1, to: 1 } }));
  expect(send.title).toBe("No terminal open");

  fireEvent.click(treeRow("auth"));
  expect(screen.queryByRole("treeitem", { name: /token\.ts/ })).toBeNull();
  expect(treeRow("auth").querySelector(".status-dot")).not.toBeNull();

  fireEvent.click(screen.getByRole("button", { name: "Close file token.ts" }));
  expect(useHive.getState().openFile).toBeNull();
  expect(screen.queryByRole("region", { name: "src/auth/token.ts" })).toBeNull();
  const tabs = screen.getByRole("tablist", { name: "Open terminals and files" });
  expect(tabs.querySelector("[role=tab]")).toBeNull();
});

test("a file opens in the preview tab; a double click on its row keeps it (11.1)", () => {
  panel();
  act(() => select(refactor.id));
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  expand();
  const names = () =>
    screen.getByRole("tablist", { name: "Open terminals and files" }).querySelectorAll(".tab-name");
  const shown = () => [...names()].map((n) => [n.textContent, n.hasAttribute("data-preview")]);
  fireEvent.click(screen.getByRole("treeitem", { name: /token\.ts/ }));
  expect(shown()).toEqual([["token.ts", true]]);
  // Another file takes the preview tab's place.
  fireEvent.click(screen.getByRole("treeitem", { name: /package\.json/ }));
  expect(shown()).toEqual([["package.json", true]]);
  // A double click (after its two clicks) keeps it; the next file gets a tab of its own.
  const row = screen.getByRole("treeitem", { name: /package\.json/ });
  fireEvent.click(row);
  fireEvent.click(row);
  fireEvent.doubleClick(row);
  fireEvent.click(screen.getByRole("treeitem", { name: /token\.ts/ }));
  expect(shown()).toEqual([
    ["package.json", false],
    ["token.ts", true],
  ]);
  // A folder's double click only toggles it (twice): no file is kept.
  const auth = treeRow("auth");
  fireEvent.click(auth);
  fireEvent.click(auth);
  fireEvent.doubleClick(auth);
  // Below the rows: nothing.
  fireEvent.doubleClick(screen.getByRole("tree", { name: "Files" }));
  expect(shown()).toEqual([
    ["package.json", false],
    ["token.ts", true],
  ]);
});

test("a file without changes says so", () => {
  panel();
  act(() => select(refactor.id));
  act(() => setOpenFile({ worktree: refactor.path, path: "README.md" }));
  const view = screen.getByRole("region", { name: "README.md" });
  expect(view.textContent).toContain("No changes in this file.");
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

test("the Files tree opens a changed file as editable text, the Diff tab as its diff", () => {
  spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  act(() => select(fixLogin.id));
  const withTabs = (panel: ReactElement) =>
    render(
      <>
        {panel}
        <TerminalArea />
      </>,
    );
  const files = withTabs(<FilesView worktree={fixLogin.path} />);
  act(() =>
    apply({
      type: "files",
      path: fixLogin.path,
      files: ["src/a.ts"],
      ignored: [],
      truncated: false,
    }),
  );
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("src/a.ts")]) }));
  const open = { worktree: fixLogin.path, path: "src/a.ts" };
  const answer = { ...open, content: "new\n", base: "old\n", version: "v" };
  const body = () => document.querySelector(".file-view-body") as HTMLElement;
  expand();
  fireEvent.click(screen.getByRole("treeitem", { name: /a\.ts/ }));
  act(() => apply({ type: "file", ...answer, binary: false, too_large: false, error: null }));
  expect(useHive.getState().editing).toBe(true);
  expect(body().querySelector(".cm-merge-b")).toBeNull();
  expect(body().querySelector(".cm-content")?.getAttribute("contenteditable")).toBe("true");
  files.unmount();

  // The same file, open as text, switches to its diff when picked in the Diff tab.
  setPanelView("changes");
  withTabs(<RightPanel />);
  expand();
  fireEvent.click(screen.getByRole("treeitem", { name: /a\.ts/ }));
  expect(useHive.getState().editing).toBe(false);
  expect(body().querySelector(".cm-merge-b")).not.toBeNull();

  // Back to text; unsaved edits stay in the editor whichever tree picks the file.
  act(() => leaveFile(open, true));
  expect(useHive.getState().editing).toBe(true);
  act(() => useHive.setState((s) => ({ edit: s.edit && { ...s.edit, saved: null } })));
  fireEvent.click(screen.getByRole("treeitem", { name: /a\.ts/ }));
  expect(useHive.getState().editing).toBe(true);
  expect(body().querySelector(".cm-merge-b")).toBeNull();
});

test("review comments on the open file mark its lines and are listed under it", () => {
  panel();
  act(() => select(refactor.id));
  act(() => setOpenFile({ worktree: refactor.path, path: "README.md" }));
  const content = "one\ntwo\nthree\n";
  act(() =>
    apply({
      type: "file",
      worktree: refactor.path,
      path: "README.md",
      content,
      base: content,
      version: "v",
      binary: false,
      too_large: false,
      error: null,
    }),
  );
  const view = screen.getByRole("region", { name: "README.md" });
  const marked = () =>
    [...view.querySelectorAll(".cm-line.cm-commented")].map((line) => line.textContent);
  expect(marked()).toEqual([]);
  expect(screen.queryByRole("region", { name: "Review comments" })).toBeNull();
  const comment = { from: 2, to: 3, text: "why?" };
  act(() =>
    useHive.setState({
      comments: {
        [refactor.path]: [
          { path: "README.md", ...comment },
          { path: "b.ts", ...comment },
        ],
      },
    }),
  );
  // Only this file's comment marks lines; the list has the worktree's.
  expect(marked()).toEqual(["two", "three"]);
  expect(screen.getByRole("button", { name: "Send review (2)" })).toBeTruthy();
});

test("rows show the library's icon for their name, its defaults for unknown ones, open or closed", async () => {
  panel();
  act(() => select(refactor.id));
  const paths = ["src/main.ts", "mystery/notes.qqq", "mystery/package.json"];
  act(() =>
    apply({
      type: "changes",
      ...changes(
        refactor.path,
        paths.map((p) => file(p)),
      ),
    }),
  );
  const props = { className: "tree-icon", width: 14, height: 14, "aria-hidden": true } as const;
  const svg = (el: ReactElement) => renderToStaticMarkup(el);
  const icon = (name: string | RegExp) => treeRow(name).querySelector(".tree-icon")?.outerHTML;
  // The named folder has its own icon; the unknown one the default, closed then open.
  const src = svg(getIconForFolder({ folderName: "src", ...props }));
  expect(src).not.toBe(svg(<DefaultFolderIcon {...props} />));
  // The library loads in its own chunk: until then each row keeps the icon's place.
  await waitFor(() => expect(icon("mystery")).toBe(svg(<DefaultFolderIcon {...props} />)));
  expect(icon("src")).toBe(src);
  expand();
  expect(icon("mystery")).toBe(svg(<DefaultFolderOpenedIcon {...props} />));
  expect(icon("src")).toBe(src);
  // Files by extension, by full name, and the default.
  expect(icon(/^main\.ts/)).toBe(svg(getIconForFile({ fileName: "main.ts", ...props })));
  expect(icon(/^main\.ts/)).not.toBe(svg(<DefaultFileIcon {...props} />));
  expect(icon(/^package\.json/)).toBe(
    svg(getIconForFile({ fileName: "package.json", autoAssign: true, ...props })),
  );
  expect(icon(/^package\.json/)).not.toBe(svg(getIconForFile({ fileName: "x.json", ...props })));
  expect(icon(/^notes\.qqq/)).toBe(svg(<DefaultFileIcon {...props} />));
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
  expect(rows()[1]).toEqual(["src+23−48", "D"]);
  key("ArrowLeft");
  expect(treeRow("assets").getAttribute("aria-expanded")).toBe("false");
  key("ArrowRight");
  key("ArrowRight");
  expect(rows()[1]?.[0]).toBe("logo.pngM");
  key("ArrowDown");
  key("Enter");
  expect(useHive.getState().openFile?.path).toBe("assets/logo.png");
  key("ArrowUp");
  key(" ");
  expect(treeRow("assets").getAttribute("aria-expanded")).toBe("false");
  // Other keys are left alone.
  expect(fireEvent.keyDown(tree(), { key: "a" })).toBe(true);
  for (let i = 0; i < 20; i++) key("ArrowDown");
  expect(active()?.textContent).toBe("package.json+1−1M");
  // Delete asks first, naming the entry.
  expect(useHive.getState().question).toBeNull();
  expect(key("Delete")).toBe(false);
  expect(useHive.getState()).toMatchObject({
    modal: "confirm",
    question: { title: "Delete file?", action: "Delete" },
  });
  expect(useHive.getState().question?.text).toStartWith("Delete package.json? ");
});

test("moving in the tree neither rebuilds nor walks it; opening a folder only walks it", () => {
  panel();
  act(() => select(refactor.id));
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  const build = spyOn(RightPanelModule, "fileTree");
  const walk = spyOn(RightPanelModule, "fileRows");
  const key = (k: string) => fireEvent.keyDown(tree(), { key: k });
  key("ArrowDown");
  key("ArrowDown");
  key("ArrowUp");
  expect([build.mock.calls.length, walk.mock.calls.length]).toEqual([0, 0]);
  key("ArrowRight");
  expect([build.mock.calls.length, walk.mock.calls.length]).toEqual([0, 1]);
  // A new listing is built once.
  act(() => apply({ type: "changes", ...changes(refactor.path, [file("x/y.ts")]) }));
  expect([build.mock.calls.length, walk.mock.calls.length]).toEqual([1, 2]);
});

test("an empty tree ignores keys", () => {
  panel();
  act(() => select(fixLogin.id));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, []) }));
  expect(tree().getAttribute("aria-activedescendant")).toBeNull();
  expect(fireEvent.keyDown(tree(), { key: "ArrowDown" })).toBe(true);
});

test("the Changes panel's header button closes it", () => {
  panel();
  act(() => setRightPanel("files"));
  expect(screen.getByRole("complementary", { name: "Side panel" })).toBeDefined();
  fireEvent.click(screen.getByTitle("Collapse (Ctrl+Shift+B)"));
  expect(useHive.getState().rightPanel).toBeNull();
});

// 10.4: a narrow panel shows the tabs' icons only (CSS), so each keeps its label as its name.
test("each panel tab keeps its label as its accessible name and title", () => {
  panel();
  act(() => setRightPanel("files"));
  const tabs = screen.getByRole("tablist", { name: "Panel" }).querySelectorAll('[role="tab"]');
  const labels = ["Files", "Diff", "Sessions", "PRs", "Actions"];
  expect([...tabs].map((t) => [t.getAttribute("aria-label"), t.getAttribute("title")])).toEqual(
    labels.map((l) => [l, l]),
  );
  for (const l of labels) expect(screen.getByRole("tab", { name: l })).toBeDefined();
});

test("a folder sums the line counts of every changed file below it; binary files add none", () => {
  const counted = (path: string, added: number | null, removed: number | null) => ({
    ...file(path),
    added,
    removed,
  });
  const root = fileTree(
    allFiles(
      ["a/x.ts", "a/b/y.ts", "a/b/c/z.ts", "a/bin/p.png", "clean/u.ts"],
      [
        counted("a/x.ts", 3, 1),
        { ...counted("a/b/y.ts", 5, 0), status: "added" },
        counted("a/b/c/z.ts", 2, 7),
        { ...counted("a/b/gone.ts", 0, 4), status: "deleted" },
        counted("a/bin/p.png", null, null),
      ],
    ),
  );
  const open = { "files:/w/a": false, "files:/w/a/b": false };
  const folders = fileRows("/w", root, open).flatMap((r) =>
    r.kind === "folder" ? [[r.path, r.status, r.added, r.removed]] : [],
  );
  expect(folders).toEqual([
    ["a", "deleted", 10, 12],
    ["a/b", "deleted", 7, 11],
    ["a/b/c", "modified", 2, 7],
    ["a/bin", "modified", 0, 0],
    ["clean", null, 0, 0],
  ]);
  expect([root.status, root.added, root.removed]).toEqual(["deleted", 10, 12]);
});

test("a closed folder shows its counts before its dot; an open or clean one shows nothing", () => {
  filesView();
  const binary = { ...file("art/logo.png"), added: null, removed: null };
  act(() =>
    apply({
      type: "changes",
      ...changes(fixLogin.path, [
        { ...file("src/auth/a.ts"), added: 12, removed: 4 },
        { ...file("src/b.ts", "deleted"), added: 0, removed: 3 },
        binary,
      ]),
    }),
  );
  act(() =>
    apply({
      type: "files",
      path: fixLogin.path,
      files: ["lib/x.ts"],
      ignored: [],
      truncated: false,
    }),
  );
  const parts = (name: string) =>
    [...treeRow(name).children].slice(2).map((e) => [e.className, e.textContent]);
  expect(parts("src")).toEqual([
    ["name", "src"],
    ["count-added", "+12"],
    ["count-removed", "−7"],
    ["status-dot", ""],
  ]);
  // Its accessible name has the counts, as a file's does.
  expect(screen.getByRole("treeitem", { name: "src +12 −7" })).toBe(treeRow("src"));
  // Only binary files: the dot only.
  expect(treeRow("art").textContent).toBe("art");
  expect(treeRow("art").querySelector(".status-dot")).not.toBeNull();
  // Nothing changed inside: nothing.
  expect(parts("lib")).toEqual([["name", "lib"]]);
  // Open: nothing; its closed folder inside sums its own files.
  fireEvent.click(treeRow("src"));
  expect(parts("src")).toEqual([["name", "src"]]);
  expect(treeRow("auth").textContent).toBe("auth+12−4");
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
  const folders = fileRows("/w", fileTree(all), {}).filter((r) => r.kind === "folder");
  expect(folders.map((r) => r.kind === "folder" && [r.name, r.status])).toEqual([
    ["gone", "deleted"],
    ["src", null],
  ]);
});

test("Files shows the watched worktree's files, the Changes panel only the changes", () => {
  const asked = spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  const view = render(<FilesView worktree={fixLogin.path} />);
  act(() => select(fixLogin.id));
  const listed = ["README.md", "src/auth/session.ts", "src/main.ts"];
  // Another worktree's list is not this one's.
  act(() =>
    apply({ type: "files", path: refactor.path, files: ["x"], ignored: [], truncated: false }),
  );
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("src/auth/session.ts")]) }));
  expand();
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth", "M"],
    ["session.ts+1M", "M"],
  ]);
  act(() =>
    apply({ type: "files", path: fixLogin.path, files: listed, ignored: [], truncated: false }),
  );
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth", "M"],
    ["session.ts+1M", "M"],
    ["main.ts", undefined],
    ["README.md", undefined],
  ]);
  expect(screen.queryByText(/cut short/)).toBeNull();
  // A collapsed folder with nothing changed inside shows no dot.
  act(() =>
    apply({
      type: "files",
      path: fixLogin.path,
      files: ["lib/x.ts"],
      ignored: [],
      truncated: true,
    }),
  );
  fireEvent.click(screen.getByText("lib"));
  expect(document.querySelectorAll(".status-dot")).toHaveLength(0);
  expect(screen.getByText("Too many files: the list is cut short.")).toBeDefined();
  expect(asked).not.toHaveBeenCalled();
  view.unmount();
  setPanelView("changes");
  const diff = render(<RightPanel />);
  // Folders opened in Files stay closed in Diff, and back.
  expect(rows()).toEqual([["src+1", "M"]]);
  expect(screen.queryByText(/cut short/)).toBeNull();
  expand();
  fireEvent.click(screen.getByText("auth"));
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth+1", "M"],
  ]);
  diff.unmount();
  render(<FilesView worktree={fixLogin.path} />);
  expect(rows().map(([name]) => name)).toEqual(["lib", "x.ts", "src", "auth", "session.ts+1M"]);
});

function filesView() {
  const asked = spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  act(() => select(fixLogin.id));
  render(
    <>
      <FilesView worktree={fixLogin.path} />
      <TerminalArea />
    </>,
  );
  const listed = ["README.md", "src/auth/session.ts", "src/main.ts", "docs/session-notes.md"];
  act(() =>
    apply({ type: "files", path: fixLogin.path, files: listed, ignored: [], truncated: false }),
  );
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("src/auth/session.ts")]) }));
  return asked;
}

const find = (value: string) =>
  fireEvent.change(screen.getByRole("searchbox", { name: "Find files" }), { target: { value } });

test("Find files has the app's clear button, which brings the tree back", () => {
  filesView();
  expectClearButton("Find files");
  expect(screen.getByRole("tree")).toBeDefined();
});

test("Names lists the files whose path holds the text; one opens as the tree opens it", () => {
  filesView();
  find("  SESSION ");
  const found = screen.getByRole("list", { name: "Matching files" });
  const names = () =>
    [...found.querySelectorAll<HTMLElement>(".result-file")].map((r) => [
      r.textContent,
      r.dataset.status,
    ]);
  expect(names()).toEqual([
    ["session-notes.mddocs", undefined],
    ["session.tssrc/authM", "M"],
  ]);
  expect(screen.queryByRole("tree")).toBeNull();
  // Changed or not, a file opens as editable text.
  fireEvent.click(screen.getByTitle("src/auth/session.ts"));
  expect(useHive.getState().openFile).toEqual({
    worktree: fixLogin.path,
    path: "src/auth/session.ts",
  });
  expect(useHive.getState().editing).toBe(true);
  fireEvent.click(screen.getByTitle("docs/session-notes.md"));
  expect(useHive.getState().editing).toBe(true);
  find("nothing-like-it");
  expect(found.textContent).toBe("No file name holds “nothing-like-it”.");
  // Clearing the search shows the tree again.
  find(" ");
  expect(screen.getByRole("tree", { name: "Files" })).toBeDefined();
});

test("what git ignores shows dimmed, an ignored folder closed until opened; Names leaves it out", () => {
  filesView();
  const listing = (ignored: string[]) =>
    act(() =>
      apply({
        type: "files",
        path: fixLogin.path,
        files: ["README.md"],
        ignored,
        truncated: false,
      }),
    );
  listing([".env", "node_modules/"]);
  const dim = () =>
    screen
      .queryAllByRole("treeitem")
      .filter((r) => r.dataset.ignored === "true")
      .map((r) => r.textContent);
  expect(dim()).toEqual(["node_modules", ".env"]);
  expect(treeRow("README.md").dataset.ignored).toBe("false");
  expect(treeRow("node_modules").getAttribute("aria-expanded")).toBe("false");
  // Opened, what the service then lists in it shows, dimmed too.
  fireEvent.click(screen.getByText("node_modules"));
  listing([".env", "node_modules/", "node_modules/react/", "node_modules/x.js"]);
  expect(dim()).toEqual(["node_modules", "react", "x.js", ".env"]);
  // An ignored file opens as any other.
  fireEvent.click(screen.getByTitle(".env"));
  expect(useHive.getState().openFile).toEqual({ worktree: fixLogin.path, path: ".env" });
  // Names finds what git lists, not what it ignores.
  find("env");
  expect(screen.getByRole("list", { name: "Matching files" }).textContent).toBe(
    "No file name holds “env”.",
  );
});

test("Names shows at most its cap and says how many there are", () => {
  filesView();
  const many = Array.from({ length: NAME_LIMIT + 3 }, (_, i) => `f/${i}.ts`);
  act(() =>
    apply({ type: "files", path: fixLogin.path, files: many, ignored: [], truncated: false }),
  );
  find(".ts");
  expect(document.querySelectorAll(".search-results .result-file")).toHaveLength(NAME_LIMIT);
  expect(
    screen.getByText(`Showing ${NAME_LIMIT} of ${NAME_LIMIT + 4} files: type more to narrow them.`),
  ).toBeDefined();
});

test("Contents asks the service once typing pauses and opens a line where it is", async () => {
  const search = spyOn(transport, "searchFiles").mockImplementation(async () => {});
  filesView();
  fireEvent.click(screen.getByRole("button", { name: "Contents" }));
  expect(screen.getByRole("searchbox").getAttribute("placeholder")).toBe("Search in files");
  find("tok");
  find("token");
  expect(screen.getByText("Searching…")).toBeDefined();
  await waitFor(() => expect(search).toHaveBeenCalledTimes(1));
  expect(search).toHaveBeenCalledWith(fixLogin.path, "token");

  const results = (patch: Partial<SearchResults>) =>
    act(() =>
      apply({
        type: "search_results",
        worktree: fixLogin.path,
        query: "token",
        matches: [],
        truncated: false,
        error: null,
        ...patch,
      }),
    );
  // An answer for an older query is not this one's.
  results({ query: "tok", matches: [{ path: "a", line: 1, text: "tok" }] });
  expect(screen.getByText("Searching…")).toBeDefined();
  results({});
  expect(screen.getByRole("list", { name: "Matching lines" }).textContent).toBe(
    "No file holds “token”.",
  );
  results({ error: "search for 1 to 256 bytes on one line" });
  expect(screen.getByText("search for 1 to 256 bytes on one line")).toBeDefined();
  results({
    matches: [
      { path: "src/auth/session.ts", line: 3, text: "a Token, then token" },
      { path: "src/auth/session.ts", line: 9, text: "token" },
      { path: "gone.ts", line: 1, text: "x token" },
    ],
    truncated: true,
  });
  const found = screen.getByRole("list", { name: "Matching lines" });
  expect([...found.querySelectorAll("mark")].map((m) => m.textContent)).toEqual([
    "Token",
    "token",
    "token",
    "token",
  ]);
  expect([...found.querySelectorAll(".result-count")].map((c) => c.textContent)).toEqual([
    "2",
    "1",
  ]);
  expect(found.textContent).toContain("Too many matches: only the first 3 show.");
  // A match opens its file as editable text at the line, changed or not.
  fireEvent.click(screen.getByTitle("src/auth/session.ts:9"));
  expect(useHive.getState().gotoLine).toEqual({
    worktree: fixLogin.path,
    path: "src/auth/session.ts",
    line: 9,
  });
  expect(useHive.getState().editing).toBe(true);
  fireEvent.click(screen.getByTitle("gone.ts:1"));
  expect(useHive.getState().editing).toBe(true);
});

test("the line asked for is shown once the file's text is there, in the diff or the editor", () => {
  panel();
  act(() => select(fixLogin.id));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("a.ts")]) }));
  const text = (path: string): FileText => ({
    worktree: fixLogin.path,
    path,
    content: "one\ntwo\nthree\n",
    base: "one\n",
    version: "v",
    binary: false,
    too_large: false,
    error: null,
  });
  const selected = () => {
    const view = EditorView.findFromDOM(document.querySelector(".cm-editor") as HTMLElement);
    const { from, to } = view?.state.selection.main ?? { from: 0, to: 0 };
    return view?.state.sliceDoc(from, to);
  };
  act(() => setOpenFile({ worktree: fixLogin.path, path: "a.ts" }, false, 2));
  expect(useHive.getState().gotoLine?.line).toBe(2);
  act(() => apply({ type: "file", ...text("a.ts") }));
  expect(useHive.getState().gotoLine).toBeNull();
  expect(selected()).toBe("two");

  act(() => setOpenFile({ worktree: fixLogin.path, path: "b.ts" }, true, 3));
  act(() => apply({ type: "file", ...text("b.ts") }));
  expect(useHive.getState().gotoLine).toBeNull();
  expect(selected()).toBe("three");
  // Another line of the open file moves there too.
  act(() => setOpenFile({ worktree: fixLogin.path, path: "b.ts" }, true, 1));
  expect(useHive.getState().gotoLine).toBeNull();
  expect(selected()).toBe("one");
});

test("the files menu targets a folder, a file's folder, or the root; a deleted file has no rename", () => {
  const files = [file("README.md"), file("src/a.ts"), file("src/gone.ts", "deleted")];
  const targets = fileRows("/w", fileTree(files), { "files:/w/src": false }).map((r) =>
    fileTarget("/w", r),
  );
  expect(targets).toEqual([
    { worktree: "/w", folder: "src", path: "src" },
    { worktree: "/w", folder: "src", path: "src/a.ts" },
    { worktree: "/w", folder: "src", path: null },
    { worktree: "/w", folder: "", path: "README.md" },
  ]);
  expect(fileTarget("/w", undefined)).toEqual({ worktree: "/w", folder: "", path: null });
});

test("a right click opens the files menu for its row, below the rows for the root", () => {
  panel();
  act(() => select(refactor.id));
  const listed = [file("README.md"), file("src/a.ts")];
  act(() => apply({ type: "changes", ...changes(refactor.path, listed) }));
  const menu = () => useHive.getState().fileMenu;
  const at = { worktree: refactor.path };
  const readme = () => screen.getByRole("treeitem", { name: /^README/ });
  fireEvent.contextMenu(readme(), { clientX: 5, clientY: 6 });
  expect(menu()).toEqual({ ...at, folder: "", path: "README.md", x: 5, y: 6 });
  expect(tree().getAttribute("aria-activedescendant")).toBe(readme().id);
  fireEvent.contextMenu(tree(), { clientX: 7, clientY: 8 });
  expect(menu()).toEqual({ ...at, folder: "", path: null, x: 7, y: 8 });
  // The Menu key (no pointer) on the tree: the active row, under it.
  readme().getBoundingClientRect = () => ({ left: 10, bottom: 30 }) as DOMRect;
  fireEvent.contextMenu(tree());
  expect(menu()).toEqual({ ...at, folder: "", path: "README.md", x: 10, y: 30 });
  // On a row without the pointer: under the row itself.
  fireEvent.contextMenu(treeRow("src"));
  expect(menu()).toMatchObject({ folder: "src", path: "src", x: 0, y: 0 });
});

test("folders created from the tree show even when git lists nothing in them", () => {
  const root = fileTree([file("a/x.ts")], ["new/inner", "a"]);
  const rows = fileRows("/w", root, { "files:/w/new": false }, "files");
  expect(rows.map((r) => [r.kind, r.name, r.depth])).toEqual([
    ["folder", "a", 0],
    ["folder", "new", 0],
    ["folder", "inner", 1],
  ]);
  filesView();
  act(() => apply({ type: "folder_created", worktree: fixLogin.path, path: "src/empty" }));
  expect(treeRow("empty").getAttribute("aria-expanded")).toBe("false");
  // Only the Files tree shows them, not the Diff one.
  cleanup();
  setPanelView("changes");
  render(<RightPanel />);
  expand();
  expect(screen.queryByRole("treeitem", { name: named("empty") })).toBeNull();
});

test("a file dragged onto a folder, a file or below the rows moves there", async () => {
  const moved = spyOn(transport, "moveFile").mockResolvedValue();
  filesView();
  const row = treeRow;
  const dataTransfer = {};
  // The events' own data transfer (the testing library copies the one passed).
  let last: DataTransfer | null = null;
  const seen = (e: Event) => {
    last = (e as DragEvent).dataTransfer;
  };
  for (const type of ["dragstart", "dragover"]) document.addEventListener(type, seen);
  const readme = row(/^README/);
  expect(readme.getAttribute("draggable")).toBe("true");
  // A drag from outside the tree (e.g. a file of the system) is not a move.
  fireEvent.dragOver(row("src"), { dataTransfer });
  fireEvent.drop(row("src"), { dataTransfer });
  expect(moved).not.toHaveBeenCalled();

  fireEvent.dragStart(readme, { dataTransfer });
  const started = last as unknown as DataTransfer;
  expect([started.getData("text/plain"), started.effectAllowed]).toEqual(["README.md", "move"]);
  fireEvent.dragOver(row("src"), { dataTransfer });
  expect([row("src").dataset.fileDrop, (last as unknown as DataTransfer).dropEffect]).toEqual([
    "true",
    "move",
  ]);
  // A closed folder hovered for a moment opens.
  await waitFor(() => expect(row("src").getAttribute("aria-expanded")).toBe("true"), {
    timeout: HOVER_OPEN_MS * 3,
  });
  fireEvent.drop(row("src"), { dataTransfer });
  expect(moved).toHaveBeenCalledWith(fixLogin.path, "README.md", "src");
  expect(row("src").dataset.fileDrop).toBe("false");

  // Onto a file: its folder. Its own folder does nothing.
  fireEvent.dragStart(row("main.ts"), { dataTransfer });
  fireEvent.dragOver(readme, { dataTransfer });
  const scroller = document.querySelector(".files-tree") as HTMLElement;
  expect(scroller.dataset.fileDrop).toBe("true");
  fireEvent.drop(readme, { dataTransfer });
  expect(moved).toHaveBeenLastCalledWith(fixLogin.path, "src/main.ts", "");
  fireEvent.dragStart(row("main.ts"), { dataTransfer });
  fireEvent.drop(row("auth"), { dataTransfer });
  expect(moved).toHaveBeenLastCalledWith(fixLogin.path, "src/main.ts", "src/auth");
  fireEvent.dragStart(row("main.ts"), { dataTransfer });
  fireEvent.drop(row("src"), { dataTransfer });
  expect(moved).toHaveBeenCalledTimes(3);
  // Below the rows: the root.
  fireEvent.dragStart(row("main.ts"), { dataTransfer });
  fireEvent.drop(tree(), { dataTransfer });
  expect(moved).toHaveBeenLastCalledWith(fixLogin.path, "src/main.ts", "");

  // Leaving the tree, or hovering another row, cancels a pending open; the drag's end clears it.
  fireEvent.dragStart(readme, { dataTransfer });
  fireEvent.dragOver(row("docs"), { dataTransfer });
  fireEvent.dragOver(row("docs"), { dataTransfer });
  // happy-dom's drag events carry no `relatedTarget`: set it.
  const leave = (from: Element, to: Element) => {
    const event = createEvent.dragLeave(from);
    Object.defineProperty(event, "relatedTarget", { value: to });
    fireEvent(from, event);
  };
  leave(row("docs"), readme);
  expect(row("docs").dataset.fileDrop).toBe("true");
  leave(scroller, document.body);
  expect(row("docs").dataset.fileDrop).toBe("false");
  fireEvent.dragOver(row("docs"), { dataTransfer });
  fireEvent.dragOver(readme, { dataTransfer });
  fireEvent.dragEnd(readme, { dataTransfer });
  await new Promise((done) => setTimeout(done, HOVER_OPEN_MS * 1.5));
  expect(row("docs").getAttribute("aria-expanded")).toBe("false");
  expect(scroller.dataset.fileDrop).toBe("false");
  // An open folder does not wait to open.
  fireEvent.dragStart(readme, { dataTransfer });
  fireEvent.dragOver(row("src"), { dataTransfer });
  fireEvent.dragEnd(readme, { dataTransfer });
  for (const type of ["dragstart", "dragover"]) document.removeEventListener(type, seen);
});

const isOpen = (name: string) => treeRow(name).getAttribute("aria-expanded") === "true";
const openFolders = (...folders: string[]) =>
  act(() =>
    useHive.setState((s) => ({
      collapsed: {
        ...s.collapsed,
        ...Object.fromEntries(folders.map((f) => [`files:${fixLogin.path}/${f}`, false])),
      },
    })),
  );

test("a folder the drag opened closes when the drag leaves it or ends without a drop", () => {
  const moved = spyOn(transport, "moveFile").mockResolvedValue();
  jest.useFakeTimers();
  try {
    filesView();
    const dataTransfer = {};
    const over = (name: string | RegExp) => fireEvent.dragOver(treeRow(name), { dataTransfer });
    const hold = (name: string) => {
      over(name);
      act(() => jest.advanceTimersByTime(HOVER_OPEN_MS));
    };
    const readme = treeRow(/^README/);
    // Opened before the drag: it stays open.
    openFolders("docs");
    fireEvent.dragStart(readme, { dataTransfer });
    hold("src");
    hold("auth");
    expect([isOpen("src"), isOpen("auth")]).toEqual([true, true]);
    // Inside them, both stay open; out of one, it closes.
    over(/^session\.ts/);
    expect([isOpen("src"), isOpen("auth")]).toEqual([true, true]);
    over(/^main/);
    expect([isOpen("src"), isOpen("auth")]).toEqual([true, false]);
    over("docs");
    expect([isOpen("src"), isOpen("docs")]).toEqual([false, true]);
    // A drag that ends without a drop closes what it opened.
    hold("src");
    expect(isOpen("src")).toBe(true);
    fireEvent.dragEnd(readme, { dataTransfer });
    expect([isOpen("src"), isOpen("docs")]).toEqual([false, true]);
    // So does leaving the tree.
    fireEvent.dragStart(readme, { dataTransfer });
    hold("src");
    const scroller = document.querySelector(".files-tree") as HTMLElement;
    const leave = createEvent.dragLeave(scroller);
    Object.defineProperty(leave, "relatedTarget", { value: document.body });
    fireEvent(scroller, leave);
    expect(isOpen("src")).toBe(false);
    fireEvent.dragEnd(readme, { dataTransfer });
    // A drop keeps the folders around it open, and opens the one it goes to.
    fireEvent.dragStart(readme, { dataTransfer });
    hold("src");
    over("auth");
    fireEvent.drop(treeRow("auth"), { dataTransfer });
    fireEvent.dragEnd(readme, { dataTransfer });
    expect(moved).toHaveBeenCalledWith(fixLogin.path, "README.md", "src/auth");
    expect([isOpen("src"), isOpen("auth")]).toEqual([true, true]);
  } finally {
    jest.useRealTimers();
  }
});

test("a folder is dragged like a file, never into itself", () => {
  const moved = spyOn(transport, "moveFile").mockResolvedValue();
  filesView();
  openFolders("src", "src/auth");
  const dataTransfer = {};
  expect(treeRow("auth").getAttribute("draggable")).toBe("true");
  fireEvent.dragStart(treeRow("auth"), { dataTransfer });
  // Onto itself or what it holds: no drop line, no drop.
  for (const name of ["auth", /^session\.ts/]) {
    const event = createEvent.dragOver(treeRow(name), { dataTransfer });
    fireEvent(treeRow(name), event);
    expect([event.defaultPrevented, treeRow("auth").dataset.fileDrop]).toEqual([false, "false"]);
    fireEvent.drop(treeRow(name), { dataTransfer });
  }
  // Onto its own folder: nothing moves.
  fireEvent.dragOver(treeRow(/^main/), { dataTransfer });
  expect(treeRow("src").dataset.fileDrop).toBe("true");
  fireEvent.drop(treeRow(/^main/), { dataTransfer });
  expect(moved).not.toHaveBeenCalled();
  fireEvent.dragStart(treeRow("auth"), { dataTransfer });
  fireEvent.drop(treeRow("docs"), { dataTransfer });
  expect(moved).toHaveBeenCalledWith(fixLogin.path, "src/auth", "docs");
  expect(isOpen("docs")).toBe(true);
  fireEvent.dragStart(treeRow("auth"), { dataTransfer });
  fireEvent.drop(tree(), { dataTransfer });
  expect(moved).toHaveBeenLastCalledWith(fixLogin.path, "src/auth", "");
});

test("the tree's active row follows a renamed or moved entry once it is listed", () => {
  filesView();
  act(() => apply({ type: "file_renamed", worktree: fixLogin.path, path: "docs", to: "zeta" }));
  expect(useHive.getState().movedRow).toEqual({ worktree: fixLogin.path, path: "zeta" });
  const listed = ["README.md", "src/main.ts", "zeta/session-notes.md"];
  act(() =>
    apply({ type: "files", path: fixLogin.path, files: listed, ignored: [], truncated: false }),
  );
  expect(tree().getAttribute("aria-activedescendant")).toBe(treeRow("zeta").id);
  expect(useHive.getState().movedRow).toBeNull();
  // A moved file too.
  act(() => apply({ type: "file_renamed", worktree: fixLogin.path, path: "a", to: "README.md" }));
  expect(tree().getAttribute("aria-activedescendant")).toBe(treeRow(/^README/).id);
});
