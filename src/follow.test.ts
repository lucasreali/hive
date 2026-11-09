import { beforeEach, expect, mock, test } from "bun:test";
import {
  followOpenFile,
  followOpenTable,
  followPanel,
  followTableEdits,
  followView,
} from "./follow";
import { apply } from "./reduce";
import {
  addTab,
  editOpenTable,
  initialState,
  pinFile,
  select,
  setDiffBase,
  setFocused,
  setOpenFile,
  setRightPanel,
  setTableShown,
  setVisible,
  useHive,
} from "./store";
import type { Transport } from "./transport";
import { MOCK_FOLDER, MOCK_REPOS } from "./transport/mock";
import { saveOpenFile } from "./viewer/save";

beforeEach(() => useHive.setState(initialState, true));

function recorder() {
  const calls: (string | null)[] = [];
  const transport = {
    // The main worktrees are watched against HEAD; any other base shows.
    watchWorktree: async (path: string, base: string) =>
      void calls.push(base === "head" ? path : `${base}:${path}`),
    unwatchWorktree: async () => void calls.push(null),
  } as unknown as Transport;
  return { calls, transport };
}

test("the service watches the worktree the open files panel shows, and nothing else", () => {
  const { calls, transport } = recorder();
  const [shop, api] = MOCK_REPOS as [(typeof MOCK_REPOS)[number], (typeof MOCK_REPOS)[number]];
  apply({ type: "projects", projects: MOCK_REPOS });
  setRightPanel("files");
  select(shop.id);
  const stop = followPanel(transport);
  // Not connected yet.
  expect(calls).toEqual([]);
  apply({ type: "welcome", version: "0.1.0", distro: null });
  expect(calls).toEqual([shop.id]);
  // Other changes do not send it again.
  apply({ type: "files", path: shop.id, files: [], ignored: [], truncated: false });
  select(api.id);
  setRightPanel(null);
  expect(calls).toEqual([shop.id, api.id, null]);
  setRightPanel("files");
  expect(calls).toEqual([shop.id, api.id, null, api.id]);
  // A new connection has no watch: nothing to stop, and it is sent again.
  apply({ type: "disconnected", reason: "gone", bundled: false });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  expect(calls).toEqual([shop.id, api.id, null, api.id, api.id]);
  stop();
  select(shop.id);
  expect(calls).toHaveLength(5);
});

test("the ignored folders open in the tree are sent when they change, and again after a new watch", () => {
  const sent: unknown[] = [];
  const transport = {
    watchWorktree: async (path: string) => void sent.push(`watch ${path}`),
    unwatchWorktree: async () => void sent.push("unwatch"),
    expandIgnored: async (path: string, folders: string[]) => void sent.push([path, folders]),
  } as unknown as Transport;
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  apply({ type: "projects", projects: MOCK_REPOS });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  setRightPanel("files");
  select(shop.id);
  const stop = followPanel(transport);
  const listing = (ignored: string[], path = shop.id) =>
    apply({ type: "files", path, files: ["a.ts"], ignored, truncated: false });
  const setOpen = (folder: string, open: boolean) =>
    useHive.setState((s) => ({
      collapsed: { ...s.collapsed, [`files:${shop.id}/${folder}`]: !open },
    }));
  // What was sent since the last look.
  const taken = () => sent.splice(0);
  listing([".env", "deps/", "out/"]);
  expect(taken()).toEqual([`watch ${shop.id}`]);
  // An open folder that is not ignored is not sent.
  setOpen("src", true);
  expect(taken()).toEqual([]);
  setOpen("deps", true);
  expect(taken()).toEqual([[shop.id, ["deps"]]]);
  // Another worktree's listing holds none of this one's.
  listing([".env", "deps/", "deps/a/", "out/"], "/elsewhere");
  listing([".env", "deps/", "deps/a/", "out/"]);
  expect(taken()).toEqual([
    [shop.id, []],
    [shop.id, ["deps"]],
  ]);
  // A folder inside counts once the listing names it; one closed is not sent.
  setOpen("deps/a", true);
  setOpen("out", false);
  expect(taken()).toEqual([[shop.id, ["deps", "deps/a"]]]);
  // Unchanged, nothing goes; closed, it goes.
  listing([".env", "deps/", "deps/a/", "deps/a/x.js", "out/"]);
  setOpen("deps", false);
  expect(taken()).toEqual([[shop.id, ["deps/a"]]]);
  // A new watch starts with none open: sent again once listed.
  apply({ type: "disconnected", reason: "gone", bundled: false });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  listing([".env", "deps/", "deps/a/", "out/"]);
  expect(taken()).toEqual([`watch ${shop.id}`, [shop.id, ["deps/a"]]]);
  // Closed panel: nothing.
  setRightPanel(null);
  setOpen("out", true);
  expect(taken()).toEqual(["unwatch"]);
  stop();
});

test("a Claude worktree is watched against its branch, and anew when its base is picked", () => {
  const { calls, transport } = recorder();
  const fix = (MOCK_REPOS[0] as (typeof MOCK_REPOS)[number]).worktrees[1] as { path: string };
  apply({ type: "projects", projects: MOCK_REPOS });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  setRightPanel("files");
  select(fix.path);
  const stop = followPanel(transport);
  expect(calls).toEqual([`branch:${fix.path}`]);
  setDiffBase(fix.path, "head");
  setDiffBase("/elsewhere", "branch");
  expect(calls).toEqual([`branch:${fix.path}`, fix.path]);
  stop();
});

const welcome = () => apply({ type: "welcome", version: "1", distro: null });
const none = { base: "head", branch: null, base_error: null } as const;
const changes = (path: string) =>
  apply({ type: "changes", path, ...none, files: [], added: 0, removed: 0, error: null });

test("asks for the open file when it opens, its worktree changes, or the service is new", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  const stop = followOpenFile({ openFile } as unknown as Transport);
  setOpenFile({ worktree: "/w", path: "a.ts" });
  pinFile({ worktree: "/w", path: "a.ts" }); // Its tab stays when b.ts opens (11.1).
  expect(openFile).not.toHaveBeenCalled(); // Not connected yet.

  welcome();
  expect(openFile.mock.calls).toEqual([["/w", "a.ts", "head"]]);
  changes("/other");
  apply({ type: "agent_removed", channel: 1, id: "x" });
  expect(openFile).toHaveBeenCalledTimes(1);
  changes("/w");
  // An edit that keeps the counts sends an equal list: the file is asked for again (19.13).
  const list = useHive.getState().changes["/w"];
  changes("/w");
  expect(useHive.getState().changes["/w"]).toBe(list);
  expect(openFile).toHaveBeenCalledTimes(3);
  setOpenFile({ worktree: "/w", path: "b.ts" });
  welcome();
  expect(openFile.mock.calls.slice(1)).toEqual([
    ["/w", "a.ts", "head"],
    ["/w", "a.ts", "head"],
    ["/w", "b.ts", "head"],
    ["/w", "b.ts", "head"],
  ]);

  // Closing b.ts's tab shows a.ts's, beside it (8.21).
  setOpenFile(null);
  expect(openFile.mock.calls.at(-1)).toEqual(["/w", "a.ts", "head"]);
  apply({ type: "disconnected", reason: "gone", bundled: false });
  stop();
  setOpenFile({ worktree: "/w", path: "c.ts" });
  expect(openFile).toHaveBeenCalledTimes(6);
});

test("a folder project's open file is asked for again with each listing of its folder", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  const folder = MOCK_FOLDER.path;
  welcome();
  apply({ type: "projects", projects: [MOCK_FOLDER, ...MOCK_REPOS] });
  setOpenFile({ worktree: folder, path: "todo.md" });
  const stop = followOpenFile({ openFile } as unknown as Transport);
  const listing = { files: ["todo.md"], ignored: [], truncated: false };
  apply({ type: "files", path: folder, ...listing });
  apply({ type: "files", path: folder, ...listing });
  expect(openFile).toHaveBeenCalledTimes(3);
  // A repository's open file waits for its changes, not its listing.
  const shop = (MOCK_REPOS[0] as (typeof MOCK_REPOS)[number]).path;
  setOpenFile({ worktree: shop, path: "a.ts" });
  apply({ type: "files", path: shop, ...listing });
  expect(openFile).toHaveBeenCalledTimes(4);
  changes(shop);
  expect(openFile).toHaveBeenCalledTimes(5);
  stop();
});

test("asks for the open file again at the base picked for its worktree", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  welcome();
  setOpenFile({ worktree: "/w", path: "a.ts" });
  const stop = followOpenFile({ openFile } as unknown as Transport);
  setDiffBase("/other", "branch");
  setDiffBase("/w", "branch");
  expect(openFile.mock.calls).toEqual([
    ["/w", "a.ts", "head"],
    ["/w", "a.ts", "branch"],
  ]);
  stop();
});

test("asks again when a save found a newer version on disk", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  welcome();
  setOpenFile({ worktree: "/w", path: "a.ts" }, true);
  const stop = followOpenFile({ openFile } as unknown as Transport);
  const answer = { worktree: "/w", path: "a.ts", base: null, binary: false, too_large: false };
  apply({ type: "file", ...answer, content: "a\n", version: "v", error: null });
  expect(openFile).toHaveBeenCalledTimes(1); // A new buffer is no reason.
  const at = { worktree: "/w", path: "a.ts" };
  apply({ type: "save_failed", ...at, error: "io", message: "disk full" });
  expect(openFile).toHaveBeenCalledTimes(1);
  apply({ type: "save_failed", ...at, error: "conflict", message: "a.ts changed on disk" });
  expect(openFile).toHaveBeenCalledTimes(2);
  stop();
});

test("tells the service the terminal in view, the window focus and visibility, on any change", () => {
  const setView = mock(
    async (_terminal: number | null, _focused: boolean, _visible: boolean) => {},
  );
  addTab(1, "/w");
  const stop = followView({ setView } as unknown as Transport);
  setFocused(true);
  expect(setView).not.toHaveBeenCalled(); // Not connected yet.
  welcome();
  expect(setView.mock.calls).toEqual([[1, true, true]]);
  // Other changes do not send it again.
  setRightPanel("files");
  select("/w");
  expect(setView).toHaveBeenCalledTimes(1);
  setFocused(false);
  addTab(2, "/w");
  useHive.setState({ fileShown: true });
  setVisible(false);
  expect(setView.mock.calls.slice(1)).toEqual([
    [1, false, true],
    [2, false, true],
    [null, false, true],
    [null, false, false],
  ]);
  setVisible(true);
  useHive.setState({ fileShown: false });
  expect(setView.mock.calls.at(-1)).toEqual([2, false, true]);
  useHive.setState({ fileShown: true });
  // A new service is told again once connected.
  apply({ type: "disconnected", reason: "gone", bundled: false });
  setFocused(true);
  expect(setView).toHaveBeenCalledTimes(8);
  welcome();
  expect(setView.mock.calls.at(-1)).toEqual([null, true, true]);
  stop();
  setFocused(false);
  expect(setView).toHaveBeenCalledTimes(9);
});

test("asks for the open table file's table while it shows as one", () => {
  const openTable = mock(async (_worktree: string, _path: string) => {});
  setOpenFile({ worktree: "/w", path: "a.csv" });
  pinFile({ worktree: "/w", path: "a.csv" });
  const stop = followOpenTable({ openTable } as unknown as Transport);
  expect(openTable).not.toHaveBeenCalled(); // Not connected yet.
  welcome();
  expect(openTable.mock.calls).toEqual([["/w", "a.csv"]]);
  changes("/other");
  expect(openTable).toHaveBeenCalledTimes(1);
  changes("/w");
  expect(openTable).toHaveBeenCalledTimes(2);
  // As text it is not asked for; as a table again, it is.
  setTableShown(false);
  changes("/w");
  expect(openTable).toHaveBeenCalledTimes(2);
  setTableShown(true);
  expect(openTable).toHaveBeenCalledTimes(3);
  // Any other file is no table.
  setOpenFile({ worktree: "/w", path: "b.ts" });
  welcome();
  expect(openTable).toHaveBeenCalledTimes(3);
  setOpenFile({ worktree: "/w", path: "a.csv" });
  expect(openTable.mock.calls.at(-1)).toEqual(["/w", "a.csv"]);
  expect(openTable).toHaveBeenCalledTimes(4);
  stop();
  welcome();
  expect(openTable).toHaveBeenCalledTimes(4);
});

// 25.4: a table with unsaved edits is its buffer's.

const CSV = { worktree: "/w", path: "a.csv" };
const fileAnswer = (content: string) =>
  apply({
    type: "file",
    ...CSV,
    content,
    base: content,
    version: "v",
    binary: false,
    too_large: false,
    error: null,
  });
const tableEdited = (content: string) =>
  apply({
    type: "table_edited",
    ...CSV,
    content,
    sheet: { name: "", rows: [[content]], total: 1, partial: false },
    error: null,
  });

function editedTable() {
  const openTable = mock(async (_worktree: string, _path: string) => {});
  const editTable = mock(async (..._args: unknown[]) => {});
  const fake = { openTable, editTable };
  const transport = fake as unknown as Transport;
  setOpenFile(CSV);
  pinFile(CSV);
  fileAnswer("a\n");
  const stops = [followOpenTable(transport), followTableEdits(transport)];
  const stop = () => {
    for (const each of stops) each();
  };
  return { ...fake, stop };
}

test("a table's edits go one at a time, each from the text the last one left", () => {
  const { editTable, stop } = editedTable();
  welcome();
  const first = { op: "insert_row", at: 0 } as const;
  const second = { op: "remove_row", at: 0 } as const;
  editOpenTable(first);
  editOpenTable(second);
  expect(editTable.mock.calls).toEqual([["/w", "a.csv", "a\n", first]]);
  tableEdited("x\na\n");
  expect(editTable.mock.calls[1]).toEqual(["/w", "a.csv", "x\na\n", second]);
  expect(editTable).toHaveBeenCalledTimes(2);
  tableEdited("a\n");
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(editTable).toHaveBeenCalledTimes(2);
  stop();
});

test("no buffer to start from: no edit is asked", () => {
  const { editTable, stop } = editedTable();
  welcome();
  apply({
    type: "file",
    ...CSV,
    content: null,
    base: null,
    version: null,
    binary: true,
    too_large: false,
    error: null,
  });
  editOpenTable({ op: "insert_row", at: 0 });
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(useHive.getState().editing).toBe(false);
  expect(editTable).not.toHaveBeenCalled();
  stop();
});

test("a table shown again with unsaved edits is asked from the buffer, not the disk", () => {
  const { openTable, editTable, stop } = editedTable();
  welcome();
  expect(openTable).toHaveBeenCalledTimes(1);
  editOpenTable({ op: "insert_row", at: 0 });
  tableEdited("x\na\n");
  editTable.mockClear();
  // The answers themselves ask nothing more, nor do new changes while dirty.
  changes("/w");
  expect(editTable).not.toHaveBeenCalled();
  expect(openTable).toHaveBeenCalledTimes(1);

  setTableShown(false);
  setTableShown(true);
  expect(editTable.mock.calls).toEqual([["/w", "a.csv", "x\na\n", null]]);
  // Asked once, however many times it shows before the answer.
  setTableShown(false);
  setTableShown(true);
  expect(useHive.getState().tableAsks).toHaveLength(1);
  expect(openTable).toHaveBeenCalledTimes(1);
  // Its answer changes no text, so it asks nothing more.
  const before = useHive.getState().edit?.doc;
  tableEdited("x\na\n");
  expect(useHive.getState().edit?.doc).toBe(before as NonNullable<typeof before>);
  expect(editTable).toHaveBeenCalledTimes(1);

  // Clean again (saved, or reloaded): the disk's.
  apply({ type: "file_saved", ...CSV, version: "w" });
  useHive.setState((s) => ({ edit: s.edit && { ...s.edit, saved: s.edit.doc } }));
  expect(openTable).toHaveBeenCalledTimes(2);
  stop();
});

test("what is asked goes with the connection; a save that waited for it too", () => {
  const { editTable, stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  useHive.setState({ saveAfterAsks: true });
  expect(editTable).toHaveBeenCalledTimes(1);
  apply({ type: "connection", status: "disconnected" } as never);
  useHive.setState({ connection: { status: "disconnected" } as never });
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(useHive.getState().saveAfterAsks).toBe(false);
  // After it comes back, the same edit is sent anew.
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  expect(editTable).toHaveBeenCalledTimes(2);
  stop();
});

test("an edit that cannot be sent drops those of its file", async () => {
  const { editTable, stop } = editedTable();
  welcome();
  editTable.mockImplementation(async () => {
    throw new Error("too large");
  });
  editOpenTable({ op: "insert_row", at: 0 });
  await Bun.sleep(0);
  expect(useHive.getState().tableAsks).toEqual([]);
  stop();
});

test("an edit of a file closed meanwhile is dropped", () => {
  const { editTable, stop } = editedTable();
  welcome();
  useHive.setState({ tableAsks: [{ worktree: "/w", path: "gone.csv", edit: null }] });
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(editTable).not.toHaveBeenCalled();
  stop();
});

test("a save while edits are on their way waits for them", () => {
  const { stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  saveOpenFile();
  expect(useHive.getState().saveAfterAsks).toBe(true);
  stop();
});

test("a table answer for a file not shown keeps the shown table and its buffer's edit", () => {
  const { stop } = editedTable();
  welcome();
  const other = { worktree: "/w", path: "b.csv" };
  const shown = useHive.getState().table;
  apply({
    type: "table_edited",
    ...other,
    content: "b\n",
    sheet: { name: "", rows: [["b"]], total: 1, partial: false },
    error: null,
  });
  expect(useHive.getState().table).toBe(shown);
  // An answer out of its turn takes no one's place in the line.
  editOpenTable({ op: "insert_row", at: 0 });
  apply({
    type: "table_edited",
    ...other,
    content: null,
    sheet: null,
    error: "no",
  });
  expect(useHive.getState().tableAsks).toHaveLength(1);
  stop();
});
