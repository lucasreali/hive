import { afterEach, beforeAll, expect, mock, spyOn, test } from "bun:test";
import { EditorView } from "@codemirror/view";
import {
  DefaultFileIcon,
  DefaultFolderIcon,
  DefaultFolderOpenedIcon,
  getIconForFile,
  getIconForFolder,
} from "@react-symbols/icons/utils";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  apply,
  type ChangedFile,
  type Changes,
  type FileText,
  initialState,
  type SearchResults,
  select,
  setOpenFile,
  setPanelView,
  setRightPanel,
  useHive,
} from "../store";
import { transport } from "../transport";
import { MOCK_CHANGES, MOCK_REPOS } from "../transport/mock";
import { allFiles, FilesView, fileRows, NAME_LIMIT, RightPanel } from "./RightPanel";
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
  return { path, files, added: sum("added"), removed: sum("removed"), error };
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
    fileRows("/w", files, collapsed).map((r) =>
      r.kind === "folder" ? [r.depth, `${r.name}/`, r.status, r.open] : [r.depth, r.name],
    );
  // Every folder starts collapsed.
  expect(shape({})).toEqual([
    [0, "src/", "deleted", false],
    [0, "test/", "renamed", false],
    [0, "README.md"],
  ]);
  const open = { "folder:/w/src": false, "folder:/w/src/b": false, "folder:/w/test": false };
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
  expect(shape({ ...open, "folder:/w/src/b": true, "folder:/other/test": true })).toEqual([
    [0, "src/", "deleted", true],
    [1, "b/", "deleted", false],
    [1, "a.ts"],
    [1, "z.ts"],
    [0, "test/", "renamed", true],
    [1, "t.ts"],
    [1, "u.ts"],
    [0, "README.md"],
  ]);
  const key = fileRows("/w", files, open)[1].key;
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
  // Not its project (5.8): the panel always shows a worktree of the project you are in.
  expect(screen.queryByText("api")).toBeNull();
  // Nothing is said before the service answers.
  expect(screen.queryByText(/changed|No changes/)).toBeNull();
  act(() => apply({ type: "changes", ...changes(refactor.path, MOCK_CHANGES[refactor.path]) }));
  expect(document.querySelector(".files-summary")?.textContent).toBe("5 files changed+24−49");
  expect(rows()).toEqual([
    ["assets", "M"],
    ["src", "D"],
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

  fireEvent.click(screen.getByRole("treeitem", { name: "auth" }));
  expect(screen.queryByRole("treeitem", { name: /token\.ts/ })).toBeNull();
  expect(
    screen.getByRole("treeitem", { name: "auth" }).querySelector(".status-dot"),
  ).not.toBeNull();

  fireEvent.click(screen.getByRole("button", { name: "Close file token.ts" }));
  expect(useHive.getState().openFile).toBeNull();
  expect(screen.queryByRole("region", { name: "src/auth/token.ts" })).toBeNull();
  const tabs = screen.getByRole("tablist", { name: "Open terminals and files" });
  expect(tabs.querySelector("[role=tab]")).toBeNull();
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
  const icon = (name: string | RegExp) =>
    screen.getByRole("treeitem", { name }).querySelector(".tree-icon")?.outerHTML;
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

test("the Changes panel's header button closes it", () => {
  panel();
  act(() => setRightPanel("files"));
  expect(screen.getByRole("complementary", { name: "Side panel" })).toBeDefined();
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

test("Files shows the watched worktree's files, the Changes panel only the changes", () => {
  const asked = spyOn(transport, "listChanges").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  const view = render(<FilesView worktree={fixLogin.path} />);
  act(() => select(fixLogin.id));
  const listed = ["README.md", "src/auth/session.ts", "src/main.ts"];
  // Another worktree's list is not this one's.
  act(() => apply({ type: "files", path: refactor.path, files: ["x"], truncated: false }));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("src/auth/session.ts")]) }));
  expand();
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
  expect(asked).not.toHaveBeenCalled();
  view.unmount();
  setPanelView("changes");
  render(<RightPanel />);
  expect(rows()).toEqual([
    ["src", "M"],
    ["auth", "M"],
    ["session.ts+1M", "M"],
  ]);
  expect(screen.queryByText(/cut short/)).toBeNull();
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
  act(() => apply({ type: "files", path: fixLogin.path, files: listed, truncated: false }));
  act(() => apply({ type: "changes", ...changes(fixLogin.path, [file("src/auth/session.ts")]) }));
  return asked;
}

const find = (value: string) =>
  fireEvent.change(screen.getByRole("searchbox", { name: "Find files" }), { target: { value } });

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
  fireEvent.click(screen.getByTitle("src/auth/session.ts"));
  expect(useHive.getState().openFile).toEqual({
    worktree: fixLogin.path,
    path: "src/auth/session.ts",
  });
  expect(useHive.getState().editing).toBe(false);
  fireEvent.click(screen.getByTitle("docs/session-notes.md"));
  expect(useHive.getState().editing).toBe(true);
  find("nothing-like-it");
  expect(found.textContent).toBe("No file name holds “nothing-like-it”.");
  // Clearing the search shows the tree again.
  find(" ");
  expect(screen.getByRole("tree", { name: "Files" })).toBeDefined();
});

test("Names shows at most its cap and says how many there are", () => {
  filesView();
  const many = Array.from({ length: NAME_LIMIT + 3 }, (_, i) => `f/${i}.ts`);
  act(() => apply({ type: "files", path: fixLogin.path, files: many, truncated: false }));
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
  // The changed file opens as its diff at the line; one git no longer lists opens editable.
  fireEvent.click(screen.getByTitle("src/auth/session.ts:9"));
  expect(useHive.getState().gotoLine).toEqual({
    worktree: fixLogin.path,
    path: "src/auth/session.ts",
    line: 9,
  });
  expect(useHive.getState().editing).toBe(false);
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
