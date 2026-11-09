import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
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
  askBufferTable,
  editOpenTable,
  type HiveState,
  initialState,
  pinFile,
  select,
  setDiffBase,
  setEdit,
  setFocused,
  setOpenFile,
  setRightPanel,
  setTableShown,
  setVisible,
  useHive,
} from "./store";
import { transport as realTransport, type Transport } from "./transport";
import { MOCK_FOLDER, MOCK_REPOS } from "./transport/mock";
import { resolve, toText } from "./viewer/buffer";
import { saveOpenFile } from "./viewer/save";

beforeEach(() => useHive.setState(initialState, true));
afterEach(() => mock.restore());

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
const fileAnswer = (content: string, version = "v", at = CSV) =>
  apply({
    type: "file",
    ...at,
    content,
    base: content,
    version,
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
  // `main.tsx`, imported by another test, follows the real transport too.
  spyOn(realTransport, "editTable").mockImplementation(async () => {});
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

test("an edit after a save starts from the saved text, not the last answer from disk", () => {
  const { editTable, stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  tableEdited("x\na\n");
  useHive.setState((s) => ({ edit: s.edit && { ...s.edit, saving: s.edit.doc } }));
  apply({ type: "file_saved", ...CSV, version: "w" });
  // The file's answer for the save has not come: the last one still holds the old text.
  const second = { op: "remove_row", at: 0 } as const;
  editOpenTable(second);
  expect(editTable.mock.calls.at(-1)).toEqual(["/w", "a.csv", "x\na\n", second]);
  expect(useHive.getState().edit?.version).toBe("w");
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
  useHive.setState((s) => ({ edit: s.edit && { ...s.edit, saving: s.edit.doc } }));
  apply({ type: "file_saved", ...CSV, version: "w" });
  expect(openTable).toHaveBeenCalledTimes(2);
  stop();
});

test("what is asked goes with the connection; a save that waited for it too", () => {
  const { editTable, stop } = editedTable();
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  saveOpenFile();
  expect(useHive.getState().saveAfterAsks).toEqual(CSV);
  expect(editTable).toHaveBeenCalledTimes(1);
  apply({ type: "disconnected", reason: "gone", bundled: false });
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(useHive.getState().saveAfterAsks).toBeNull();
  expect(useHive.getState().tableSentFrom).toBeNull();
  // Once it is back, a new edit is sent, from the buffer as it is.
  welcome();
  editOpenTable(edit);
  expect(editTable).toHaveBeenCalledTimes(2);
  expect(editTable.mock.calls[1]).toEqual(["/w", "a.csv", "a\n", edit]);
  stop();
});

test("an edit that cannot be sent says so, drops those of its file and the save on them", async () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { editTable, stop } = editedTable();
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  tableEdited("x\na\n");
  editTable.mockImplementationOnce(async () => {
    throw new Error("too large");
  });
  editOpenTable(edit);
  editOpenTable(edit);
  saveOpenFile();
  expect(useHive.getState().saveAfterAsks).toEqual(CSV);
  await Bun.sleep(0);
  const state = useHive.getState();
  expect(state.tableAsks).toEqual([]);
  expect(state.tableSentFrom).toBeNull();
  expect(state.saveAfterAsks).toBeNull();
  expect(state.edit?.error).toBe("Error: too large");
  expect(state.edit?.doc.toString()).toBe("x\na\n");
  expect(save).not.toHaveBeenCalled();
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

test("a save while its file's edits are on their way waits for them, and saves them", () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { stop } = editedTable();
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  tableEdited("x\na\n");
  // Dirty, so without the wait it would go now, without the next edit.
  editOpenTable(edit);
  saveOpenFile();
  expect(save).not.toHaveBeenCalled();
  expect(useHive.getState().saveAfterAsks).toEqual(CSV);
  tableEdited("y\nx\na\n");
  expect(save.mock.calls).toEqual([["/w", "a.csv", "y\nx\na\n", "v"]]);
  expect(useHive.getState().saveAfterAsks).toBeNull();
  stop();
});

test("a save waits for its own file's edits only, and saves that file's buffer", () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { stop } = editedTable();
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  tableEdited("x\na\n");
  editOpenTable(edit);
  saveOpenFile();
  // On to another file with unsaved edits: its own draft is not what was saved.
  const MD = { worktree: "/w", path: "b.md" };
  setOpenFile(MD, true);
  fileAnswer("draft\n", "m", MD);
  const buffer = useHive.getState().edit as NonNullable<HiveState["edit"]>;
  setEdit({ ...buffer, doc: toText("mine\n") });
  tableEdited("y\nx\na\n");
  expect(save.mock.calls).toEqual([["/w", "a.csv", "y\nx\na\n", "v"]]);
  // The tab's buffer is the one saving; the open one is untouched.
  const tab = useHive.getState().openFiles.find((f) => f.path === "a.csv");
  expect(tab?.edit?.saving?.toString()).toBe("y\nx\na\n");
  expect(useHive.getState().edit?.saving).toBeNull();
  apply({ type: "file_saved", ...CSV, version: "w" });
  // Back on a.csv, another edit's wait does not hold b.md's save.
  save.mockClear();
  setOpenFile(CSV, true);
  editOpenTable(edit);
  expect(useHive.getState().tableAsks).toHaveLength(1);
  setOpenFile(MD, true);
  saveOpenFile();
  expect(save.mock.calls).toEqual([["/w", "b.md", "mine\n", "m"]]);
  expect(useHive.getState().saveAfterAsks).toBeNull();
  stop();
});

test("a waiting save goes once its file's edits are answered, whatever other files wait", () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { stop } = editedTable();
  const other = { worktree: "/w", path: "c.csv" };
  setOpenFile(other, true);
  fileAnswer("c\n", "c", other);
  setOpenFile(CSV, true);
  fileAnswer("a\n");
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  askBufferTable(other);
  saveOpenFile();
  expect(save).not.toHaveBeenCalled();
  tableEdited("x\na\n");
  // c.csv's is on its way now.
  expect(useHive.getState().tableAsks).toEqual([{ ...other, edit: null }]);
  expect(save.mock.calls).toEqual([["/w", "a.csv", "x\na\n", "v"]]);
  stop();
});

test("a save that could not be sent says why in the file's own buffer", async () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {
    throw new Error("no service");
  });
  save.mockClear();
  const { stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  tableEdited("x\na\n");
  saveOpenFile();
  await Bun.sleep(0);
  expect(useHive.getState().edit?.error).toBe("Error: no service");
  expect(useHive.getState().edit?.saving).toBeNull();
  stop();
});

test("a refused edit drops those of its file and the save on them; the error stays", () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { editTable, stop } = editedTable();
  const other = { worktree: "/w", path: "c.csv" };
  setOpenFile(other, true);
  fileAnswer("c\n", "c", other);
  setOpenFile(CSV, true);
  fileAnswer("a\n");
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  tableEdited("x\na\n");
  editOpenTable(edit);
  editOpenTable(edit);
  askBufferTable(other);
  editOpenTable(edit);
  saveOpenFile();
  editTable.mockClear();
  apply({ type: "table_edited", ...CSV, content: null, sheet: null, error: "no" });
  const state = useHive.getState();
  // Only the other file's is left, and it goes on.
  expect(state.tableAsks).toEqual([{ ...other, edit: null }]);
  expect(editTable.mock.calls).toEqual([["/w", "c.csv", "c\n", null]]);
  expect(state.saveAfterAsks).toBeNull();
  expect(state.edit?.error).toBe("no");
  expect(state.edit?.doc.toString()).toBe("x\na\n");
  expect(save).not.toHaveBeenCalled();
  stop();
});

const lastNotice = () => useHive.getState().notices.at(-1)?.text;

test("the file changes on disk with an edit on its way: the banner, and the save still checks", () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  // Clean, but its edit is coming: no reload, or the agent's write would be lost below.
  fileAnswer("agent\n", "w");
  const before = useHive.getState().edit;
  expect(before?.doc.toString()).toBe("a\n");
  expect(before?.version).toBe("v");
  expect(before?.conflict).toEqual({ content: "agent\n", version: "w" });
  tableEdited("x\na\n");
  const after = useHive.getState().edit;
  expect(after?.doc.toString()).toBe("x\na\n");
  expect(after?.conflict).toEqual({ content: "agent\n", version: "w" });
  // Against the version it was read at, so the service refuses it.
  saveOpenFile();
  expect(save.mock.calls).toEqual([["/w", "a.csv", "x\na\n", "v"]]);
  stop();
});

test("Reload with an edit on its way: its answer and the rest of the line are dropped", () => {
  const save = spyOn(realTransport, "saveFile").mockImplementation(async () => {});
  save.mockClear();
  const { editTable, stop } = editedTable();
  welcome();
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  editOpenTable(edit);
  saveOpenFile();
  fileAnswer("agent\n", "w");
  const buffer = useHive.getState().edit as NonNullable<HiveState["edit"]>;
  setEdit(resolve(buffer, false));
  tableEdited("x\na\n");
  const state = useHive.getState();
  expect(state.edit?.doc.toString()).toBe("agent\n");
  expect(state.edit?.version).toBe("w");
  expect(state.tableAsks).toEqual([]);
  expect(state.tableSentFrom).toBeNull();
  expect(state.saveAfterAsks).toBeNull();
  expect(lastNotice()).toBe("a.csv changed: the table edit was dropped");
  expect(editTable).toHaveBeenCalledTimes(1);
  expect(save).not.toHaveBeenCalled();
  stop();
});

test("a table asked from the buffer does not take back what was typed after", () => {
  const { stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  tableEdited("x\na\n");
  setTableShown(false);
  setTableShown(true);
  expect(useHive.getState().tableAsks).toHaveLength(1);
  const buffer = useHive.getState().edit as NonNullable<HiveState["edit"]>;
  setEdit({ ...buffer, doc: toText("typed\n") });
  tableEdited("x\na\n");
  expect(useHive.getState().edit?.doc.toString()).toBe("typed\n");
  expect(useHive.getState().tableAsks).toEqual([]);
  stop();
});

test("the same text, typed back, takes the answer", () => {
  const { stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  const buffer = useHive.getState().edit as NonNullable<HiveState["edit"]>;
  setEdit({ ...buffer, doc: toText("a\n") });
  tableEdited("x\na\n");
  expect(useHive.getState().edit?.doc.toString()).toBe("x\na\n");
  stop();
});

test("an answer for a tab closed meanwhile only takes its place in the line", () => {
  const { stop } = editedTable();
  welcome();
  editOpenTable({ op: "insert_row", at: 0 });
  useHive.setState({ edit: null, editing: false });
  const notices = useHive.getState().notices.length;
  tableEdited("x\na\n");
  expect(useHive.getState().tableAsks).toEqual([]);
  expect(useHive.getState().notices).toHaveLength(notices);
  stop();
});

test("an answer for nothing on its way changes nothing", () => {
  const { stop } = editedTable();
  welcome();
  const other = { worktree: "/w", path: "b.csv" };
  const answered = {
    type: "table_edited" as const,
    content: "b\n",
    sheet: { name: "", rows: [["b"]], total: 1, partial: false },
    error: null,
  };
  const shown = useHive.getState().table;
  apply({ ...answered, ...other });
  apply({ ...answered, ...CSV });
  expect(useHive.getState().table).toBe(shown);
  // Nor one out of its turn, which takes no one's place in the line.
  const edit = { op: "insert_row", at: 0 } as const;
  editOpenTable(edit);
  const before = useHive.getState().edit;
  apply({ ...answered, ...other });
  apply({ type: "table_edited", ...other, content: null, sheet: null, error: "no" });
  const state = useHive.getState();
  expect(state.tableAsks).toEqual([{ ...CSV, edit }]);
  expect(state.edit?.doc).toBe(before?.doc as NonNullable<HiveState["edit"]>["doc"]);
  expect(state.edit?.error).toBeNull();
  stop();
});
