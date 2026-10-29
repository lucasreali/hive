import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { stubStorage } from "../test/storage";
import { App } from "./App";
import { savedTabOrder, saveTabOrder } from "./persist";
import { apply } from "./reduce";
import {
  activateTab,
  addTab,
  dropOnPane,
  type HiveState,
  initialState,
  moveTab,
  type OpenFile,
  pinFile,
  removeTab,
  renderedShown,
  replacePane,
  select,
  setEdit,
  setEditing,
  setOpenFile,
  setRendered,
  setSplit,
  swapSides,
  unsplit,
  useHive,
} from "./store";
import { barItems, dropFile, shownSplit, shownTerminals, visibleTabs } from "./tabs";
import { closeTerminal, splitTerminal } from "./terminals";
import { MOCK_REPOS } from "./transport/mock";
import { type EditBuffer, toText } from "./viewer/buffer";
import { createEditor, restoreView, snapshot } from "./viewer/editor";

// 8.21: files open in tabs of their own, and the tab bar keeps the order the user gives it.

const [shop] = MOCK_REPOS;
const [, fixLogin, checkout] = shop.worktrees;
const W = fixLogin.path;
const s = () => useHive.getState();
/** The bar's tabs: a file by its path, a terminal by its id. */
const bar = () => barItems(s()).map((t) => ("path" in t ? t.path : t.id));
const file = (path: string, worktree = W) => ({ worktree, path });
/** Opens `f` and keeps its tab (a double click, 11.1), so the next file gets a tab of its own. */
const keepOpen = (f: OpenFile, editing?: boolean, line?: number) => {
  setOpenFile(f, editing, line);
  pinFile(f);
};

beforeEach(() => {
  useHive.setState(initialState, true);
  localStorage.clear();
  apply({ type: "projects", projects: [shop] });
  select(fixLogin.id);
});
afterEach(() => {
  for (const tab of s().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
  localStorage.clear();
});

const answer = (path: string, content: string) =>
  apply({
    type: "file",
    worktree: W,
    path,
    content,
    base: null,
    version: `v:${content}`,
    binary: false,
    too_large: false,
    error: null,
  });

/** Types `text` into the open file's buffer. */
const type = (text: string) => setEdit({ ...(s().edit as EditBuffer), doc: toText(text) });

test("two files open in two tabs, each keeping its own edits, editing mode and view", () => {
  keepOpen(file("a.ts"), true);
  answer("a.ts", "a\n");
  type("mine a\n");
  keepOpen(file("b.ts"));
  expect(s()).toMatchObject({ openFile: file("b.ts"), editing: false, edit: null });
  answer("b.ts", "b\n");
  expect(s().edit).toBeNull(); // Not editing b.ts: its answers keep no buffer.
  keepOpen(file("a.ts"));
  expect(s().editing).toBe(true);
  expect(s().edit?.doc.toString()).toBe("mine a\n");
  // A save answered while the file is in the background still reaches its tab.
  const sending = { ...(s().edit as EditBuffer), saving: toText("mine a\n") };
  setEdit(sending);
  keepOpen(file("b.ts"));
  apply({ type: "file_saved", ...file("a.ts"), version: "v2" });
  expect(s().openFiles[0]?.edit?.version).toBe("v2");
  apply({ type: "save_failed", ...file("a.ts"), error: "io", message: "no" });
  expect(s().openFiles[0]?.edit?.error).toBe("no");
  // Opening a file already open shows its tab; nothing is added.
  keepOpen(file("b.ts"), true, 3);
  expect([bar(), s().gotoLine]).toEqual([["a.ts", "b.ts"], { ...file("b.ts"), line: 3 }]);
  expect(s().editing).toBe(false);
});

test("a file opens in the preview tab, which the next file replaces in place (11.1)", () => {
  const preview = () =>
    s()
      .openFiles.filter((f) => f.preview)
      .map((f) => f.path);
  addTab(1, W);
  setOpenFile(file("a.ts"));
  setRendered(true);
  expect(renderedShown(s())).toBe(true);
  addTab(2, W);
  expect([bar(), preview()]).toEqual([[1, "a.ts", 2], ["a.ts"]]);
  // b.ts takes a.ts's place, not the end of the bar.
  setOpenFile(file("b.ts"), true, 4);
  expect(renderedShown(s())).toBe(false); // a.ts was shown rendered (11.2): b.ts is not.
  expect([bar(), preview(), s().openFile, s().editing]).toEqual([
    [1, "b.ts", 2],
    ["b.ts"],
    file("b.ts"),
    true,
  ]);
  expect(s().tabOrder).toEqual(["tab:1", `file:${W}\nb.ts`, "tab:2"]);
  // Opening a file already open only shows it: b.ts stays the preview.
  activateTab({ id: 2, cwd: W });
  setOpenFile(file("b.ts"));
  expect([bar(), preview(), s().fileShown]).toEqual([[1, "b.ts", 2], ["b.ts"], true]);
  // Pinned, it stays; the next file is the preview, last. At most one preview.
  pinFile(file("b.ts"));
  const kept = s().openFiles;
  pinFile(file("b.ts"));
  expect(s().openFiles).toBe(kept); // Already kept: nothing changes.
  setOpenFile(file("c.ts"));
  setOpenFile(file("d.ts"));
  expect([bar(), preview()]).toEqual([[1, "b.ts", 2, "d.ts"], ["d.ts"]]);
  // Unsaved edits keep it, and it stays kept once saved.
  setEditing(true); // As leaveFile does for a file already open.
  answer("d.ts", "d\n");
  expect(preview()).toEqual(["d.ts"]); // Its answer is no edit.
  type("mine\n");
  expect(preview()).toEqual([]);
  apply({ type: "file_saved", ...file("d.ts"), version: "v2" });
  setOpenFile(file("e.ts"));
  expect([bar(), preview()]).toEqual([[1, "b.ts", 2, "d.ts", "e.ts"], ["e.ts"]]);
  // A preview of another worktree closes: the new one goes last in its own bar.
  select(checkout.id);
  setOpenFile(file("f.ts", checkout.path));
  select(null);
  expect([bar(), preview()]).toEqual([[1, "b.ts", 2, "d.ts", "f.ts"], ["f.ts"]]);
  // A preview with unsaved edits (whatever its flag says) is never replaced.
  const at = { saved: null, version: null, conflict: null, saving: null, error: null, recheck: 0 };
  useHive.setState({ edit: { ...file("f.ts", checkout.path), doc: toText("x\n"), ...at } });
  setOpenFile(file("g.ts", checkout.path));
  expect(preview()).toEqual(["f.ts", "g.ts"]);
});

test("new tabs go last in opening order, terminals and files mixed; a drag reorders them", () => {
  addTab(1, W);
  keepOpen(file("a.ts"));
  addTab(2, W);
  keepOpen(file("b.ts"));
  expect(bar()).toEqual([1, "a.ts", 2, "b.ts"]);
  expect(s().tabOrder).toEqual(["tab:1", `file:${W}\na.ts`, "tab:2", `file:${W}\nb.ts`]);
  moveTab(`file:${W}\nb.ts`, "tab:1", false);
  expect(bar()).toEqual(["b.ts", 1, "a.ts", 2]);
  moveTab("tab:1", "tab:2", true);
  expect(bar()).toEqual(["b.ts", "a.ts", 2, 1]);
  // Next to itself, or an unknown tab: nothing moves.
  const order = s().tabOrder;
  moveTab("tab:1", "tab:1", true);
  moveTab("tab:9", "tab:1", true);
  expect(s().tabOrder).toBe(order);
  // Another worktree's tabs keep their places; its bar has its own order.
  select(checkout.id);
  addTab(3, checkout.path);
  keepOpen(file("c.ts", checkout.path));
  moveTab(`file:${checkout.path}\nc.ts`, "tab:3", false);
  expect(bar()).toEqual(["c.ts", 3]);
  select(fixLogin.id);
  expect(bar()).toEqual(["b.ts", "a.ts", 2, 1]);
  // Every place at once (nothing selected): all of them.
  select(null);
  expect(bar()).toEqual(["b.ts", "a.ts", 2, 1, "c.ts", 3]);
});

test("the order is remembered for a reload: files and sessions, not a run's channels", () => {
  addTab(1, W);
  apply({ type: "agent_detected", channel: 1, id: "s1", project: shop.id, worktree: W, cwd: W });
  addTab(2, W);
  keepOpen(file("a.ts"));
  moveTab(`file:${W}\na.ts`, "session:s1", false);
  expect(bar()).toEqual(["a.ts", 1, 2]);
  expect(savedTabOrder()).toEqual([`file:${W}\na.ts`, "session:s1"]);
  // After a reload, a resumed session and the reopened file take their places back, whatever
  // order they come back in; a new terminal goes last.
  useHive.setState({ ...initialState, tabOrder: savedTabOrder() }, true);
  apply({ type: "projects", projects: [shop] });
  select(fixLogin.id);
  addTab(7, W);
  addTab(8, W);
  apply({ type: "agent_detected", channel: 8, id: "s1", project: shop.id, worktree: W, cwd: W });
  keepOpen(file("a.ts"));
  expect(bar()).toEqual(["a.ts", 8, 7]);
  // Closing tabs forgets them.
  removeTab(8);
  expect(s().tabOrder).toEqual([`file:${W}\na.ts`, "tab:7"]);
  setOpenFile(null);
  expect(s().tabOrder).toEqual(["tab:7"]);
});

test("a saved order that cannot be read is empty; a failing storage keeps the move", () => {
  localStorage.setItem("hive.tabOrder", "not json");
  expect(savedTabOrder()).toEqual([]);
  localStorage.setItem("hive.tabOrder", JSON.stringify({ a: 1 }));
  expect(savedTabOrder()).toEqual([]);
  localStorage.setItem("hive.tabOrder", JSON.stringify(["file:x", 2, "tab:3", "session:s"]));
  expect(savedTabOrder()).toEqual(["file:x", "session:s"]);
  expect(savedTabOrder(null)).toEqual([]);
  const full = stubStorage(true);
  addTab(1, W);
  keepOpen(file("a.ts"));
  moveTab(`file:${W}\na.ts`, "tab:1", false);
  expect(bar()).toEqual(["a.ts", 1]);
  expect(full).toHaveBeenLastCalledWith("hive.tabOrder", JSON.stringify(s().tabOrder));
  // Unchanged, nothing is written.
  const write = stubStorage();
  const state = s();
  saveTabOrder({ ...state } as HiveState, state);
  expect(write).not.toHaveBeenCalled();
});

test("closing a tab shows its neighbour in the bar, a file or a terminal", () => {
  addTab(1, W);
  keepOpen(file("a.ts"));
  addTab(2, W);
  keepOpen(file("b.ts"));
  // The shown file's tab closes: the one on its left (it was last) shows.
  setOpenFile(null);
  expect([bar(), s().fileShown, s().activeTab]).toEqual([[1, "a.ts", 2], false, 2]);
  // The shown terminal closes: its right neighbour is gone, so the file on its left shows.
  removeTab(2);
  expect([s().fileShown, s().openFile]).toEqual([true, file("a.ts")]);
  // The shown file closes: the terminal on its left shows.
  setOpenFile(null);
  expect([s().fileShown, s().activeTab, s().openFile]).toEqual([false, 1, null]);
  // A file not shown closes alone; a place with files but no terminal shows its last file.
  keepOpen(file("c.ts"));
  keepOpen(file("d.ts"));
  useHive.setState({ fileShown: false });
  useHive.setState((x) => dropFile(x, file("c.ts")));
  expect([bar(), s().fileShown, s().openFile]).toEqual([[1, "d.ts"], false, file("d.ts")]);
  removeTab(1);
  select(checkout.id);
  select(fixLogin.id);
  expect([s().fileShown, s().openFile]).toEqual([true, file("d.ts")]);
});

test("Ctrl+Shift+D and closing follow the bar's order", async () => {
  addTab(1, W);
  addTab(2, W);
  addTab(3, W);
  moveTab("tab:3", "tab:1", true); // 1, 3, 2
  expect(visibleTabs(s()).map((t) => t.id)).toEqual([1, 3, 2]);
  await splitTerminal(1);
  expect(s().splits).toEqual([{ left: 1, right: 3 }]);
});

test("pairs (20.5): many, a tab in one at most, each shown from either tab, its tabs side by side", () => {
  for (const id of [1, 2, 3, 4, 5]) addTab(id, W);
  setSplit({ left: 1, right: 4 });
  // The right tab moves next to the left one; it is focused.
  expect(bar()).toEqual([1, 4, 2, 3, 5]);
  expect(shownTerminals(s())).toEqual([1, 4]);
  expect(s().activeTab).toBe(4);
  setSplit({ left: 5, right: 2 }, 5);
  expect(bar()).toEqual([1, 4, 3, 5, 2]);
  expect(s().splits).toEqual([
    { left: 1, right: 4 },
    { left: 5, right: 2 },
  ]);
  // Each tab shows its own pair, or itself alone.
  const shown = (id: number) => {
    activateTab({ id, cwd: W });
    return [shownSplit(s()), shownTerminals(s())];
  };
  expect(shown(1)).toEqual([{ left: 1, right: 4 }, [1, 4]]);
  expect(shown(2)).toEqual([{ left: 5, right: 2 }, [5, 2]]);
  expect(shown(3)).toEqual([null, [3]]);
  // A tab joining another pair leaves its own, which ends.
  setSplit({ left: 3, right: 4 });
  expect(s().splits).toEqual([
    { left: 5, right: 2 },
    { left: 3, right: 4 },
  ]);
  expect(bar()).toEqual([1, 3, 4, 5, 2]);
  // A pair of one tab is none.
  setSplit({ left: 1, right: 1 });
  expect(s().splits).toHaveLength(2);

  // Replacing a pane: the tab replaced stays alone; one of another pair leaves it.
  replacePane(4, 2);
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  expect([s().activeTab, bar()]).toEqual([2, [1, 3, 2, 4, 5]]);
  replacePane(3, 3);
  replacePane(1, 3);
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  // The partner put in a pane swaps the sides; so does "Swap sides", the focus kept.
  replacePane(3, 2);
  expect([s().splits, s().activeTab, bar()]).toEqual([[{ left: 2, right: 3 }], 2, [1, 2, 3, 4, 5]]);
  // Swapped while hidden, the pair is shown from the tab swapped.
  activateTab({ id: 5, cwd: W });
  swapSides(3);
  expect([s().splits, s().activeTab, bar()]).toEqual([[{ left: 3, right: 2 }], 3, [1, 3, 2, 4, 5]]);
  swapSides(5);
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  setSplit({ left: 4, right: 5 });
  // Unsplit and closing a tab end only their own pair.
  unsplit(5);
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  setSplit({ left: 4, right: 5 });
  removeTab(4);
  expect([s().splits, s().activeTab]).toEqual([[{ left: 3, right: 2 }], 5]);
  setSplit({ left: 1, right: 5 }, 1);
  removeTab(2);
  expect([s().splits, s().activeTab]).toEqual([[{ left: 1, right: 5 }], 1]);
});

test("dragging a paired tab moves its pair; a tab dropped inside a pair lands after it (20.5)", () => {
  for (const id of [1, 2, 3, 4]) addTab(id, W);
  setSplit({ left: 2, right: 3 });
  expect(bar()).toEqual([1, 2, 3, 4]);
  moveTab("tab:2", "tab:4", true);
  expect(bar()).toEqual([1, 4, 2, 3]);
  moveTab("tab:3", "tab:1", false);
  expect(bar()).toEqual([2, 3, 1, 4]);
  moveTab("tab:4", "tab:2", true);
  expect(bar()).toEqual([2, 3, 4, 1]);
  // Within the pair, the order stays left first.
  moveTab("tab:3", "tab:2", false);
  expect(bar()).toEqual([2, 3, 4, 1]);
});

test("a tab dropped on the terminals takes a pane or makes a pair on its side (20.5)", () => {
  for (const id of [1, 2, 3]) addTab(id, W);
  addTab(9, checkout.path);
  select(fixLogin.id);
  activateTab({ id: 1, cwd: W });
  // Onto the terminal alone: a pair, the tab dropped on the side it was dropped, focused.
  dropOnPane("tab:2", "left");
  expect([s().splits, s().activeTab, bar()]).toEqual([[{ left: 2, right: 1 }], 2, [2, 1, 3]]);
  // Onto a pane: it takes it.
  dropOnPane("tab:3", "right");
  expect([s().splits, s().activeTab]).toEqual([[{ left: 2, right: 3 }], 3]);
  // The other pane's tab swaps the sides; the pane's own tab does nothing.
  dropOnPane("tab:2", "right");
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  dropOnPane("tab:2", "right");
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  // Files, other worktrees' tabs and unknown keys do nothing; nor does a tab on itself.
  for (const key of [`file:${W}\na.ts`, "tab:9", "tab:99"]) dropOnPane(key, "left");
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
  activateTab({ id: 1, cwd: W });
  dropOnPane("tab:1", "left");
  expect([s().splits, shownTerminals(s())]).toEqual([[{ left: 3, right: 2 }], [1]]);
  // Over a file's view, nothing.
  keepOpen(file("a.ts"));
  dropOnPane("tab:3", "left");
  expect(s().splits).toEqual([{ left: 3, right: 2 }]);
});

test("Ctrl+Shift+D pairs with the next tab in no pair; on a paired tab it ends that pair (20.5)", async () => {
  for (const id of [1, 2, 3, 4]) addTab(id, W);
  setSplit({ left: 2, right: 3 });
  // From 1, the next tab in no pair is 4.
  await splitTerminal(1);
  expect(s().splits).toEqual([
    { left: 2, right: 3 },
    { left: 1, right: 4 },
  ]);
  expect(bar()).toEqual([1, 4, 2, 3]);
  await splitTerminal(3);
  expect(s().splits).toEqual([{ left: 1, right: 4 }]);
});

test("a file moved into another folder (8.3) keeps its tab, its place and its unsaved edits", () => {
  keepOpen(file("a.ts"), true);
  answer("a.ts", "a\n");
  type("mine\n");
  addTab(1, W);
  keepOpen(file("b.ts"));
  // Moved while in the background, then while shown.
  apply({ type: "file_renamed", worktree: W, path: "a.ts", to: "lib/a.ts" });
  expect(bar()).toEqual(["lib/a.ts", 1, "b.ts"]);
  keepOpen(file("lib/a.ts"));
  expect(s().edit?.doc.toString()).toBe("mine\n");
  expect(s().edit?.path).toBe("lib/a.ts");
  apply({ type: "file_renamed", worktree: W, path: "lib/a.ts", to: "src/lib/a.ts" });
  expect([s().openFile, s().edit?.path]).toEqual([file("src/lib/a.ts"), "src/lib/a.ts"]);
  expect(s().edit?.doc.toString()).toBe("mine\n");
  expect(s().tabOrder).toEqual([`file:${W}\nsrc/lib/a.ts`, "tab:1", `file:${W}\nb.ts`]);
});

test("a renamed or moved file keeps its tab and its place; a deleted one's tab closes", () => {
  const listing = (files: string[], truncated = false) =>
    apply({ type: "files", path: W, files, ignored: [], truncated });
  keepOpen(file("a.ts"), true);
  answer("a.ts", "a\n");
  addTab(1, W);
  keepOpen(file("b.ts"));
  keepOpen(file("c.ts"));
  apply({ type: "file_renamed", worktree: W, path: "a.ts", to: "src/a.ts" });
  expect(bar()).toEqual(["src/a.ts", 1, "b.ts", "c.ts"]);
  expect(s().openFiles[0]?.edit?.path).toBe("src/a.ts");
  // A deleted file: gone from the listing that held it. A cut-short listing proves nothing.
  listing(["src/a.ts", "b.ts", "c.ts"]);
  listing(["src/a.ts"], true);
  expect(bar()).toEqual(["src/a.ts", 1, "b.ts", "c.ts"]);
  listing(["src/a.ts", "b.ts", "c.ts"]);
  listing(["src/a.ts", "c.ts"]);
  expect(bar()).toEqual(["src/a.ts", 1, "c.ts"]);
  // With unsaved edits it asks first.
  keepOpen(file("src/a.ts"));
  type("mine\n");
  keepOpen(file("c.ts"));
  listing([]);
  expect(bar()).toEqual(["src/a.ts", 1]); // c.ts closed; src/a.ts asks.
  expect(s().question?.text).toBe("src/a.ts was deleted. Your unsaved changes to it will be lost.");
  act(() => s().question?.run());
  expect(bar()).toEqual([1]);
  // Another worktree's listing, or the first one, closes nothing.
  keepOpen(file("d.ts"));
  apply({ type: "files", path: checkout.path, files: [], ignored: [], truncated: false });
  listing([]);
  expect(bar()).toEqual([1, "d.ts"]);
});

test("an ignored file's tab closes when it is deleted, not when it moves or its folder closes", () => {
  const listing = (files: string[], ignored: string[]) =>
    apply({ type: "files", path: W, files, ignored, truncated: false });
  listing([], [".env", "notes.txt", "deps/", "deps/a.js"]);
  for (const path of [".env", "notes.txt", "deps/a.js"]) keepOpen(file(path));
  // No longer ignored, or its ignored folder closed: still there.
  listing(["notes.txt"], [".env", "deps/"]);
  expect(bar()).toEqual([".env", "notes.txt", "deps/a.js"]);
  listing([], [".env", "notes.txt", "deps/"]);
  expect(bar()).toEqual([".env", "notes.txt", "deps/a.js"]);
  listing([], ["notes.txt", "deps/"]);
  expect(bar()).toEqual(["notes.txt", "deps/a.js"]);
});

test("a deleted file or folder leaves the tree at once and its tabs close, asking once when dirty", () => {
  apply({
    type: "files",
    path: W,
    files: ["a.ts", "src/b.ts", "src/lib/c.ts", "srcx/d.ts"],
    ignored: [".env", "srcx/out/"],
    truncated: false,
  });
  useHive.setState({ newFolders: { [W]: ["src/empty", "other"] } });
  for (const path of ["src/b.ts", "src/lib/c.ts", "srcx/d.ts", "a.ts"]) {
    keepOpen(file(path), true);
    answer(path, "x\n");
    if (path !== "srcx/d.ts") type(`${path} mine\n`);
  }
  keepOpen(file("src/b.ts", checkout.path));
  // Every open file, another worktree's marked.
  const open = () => s().openFiles.map((f) => (f.worktree === W ? f.path : `other:${f.path}`));
  // Clean tabs under the folder close at once; the dirty ones ask, in one question.
  apply({ type: "file_deleted", worktree: W, path: "src" });
  expect(s().worktreeFiles?.files).toEqual(["a.ts", "srcx/d.ts"]);
  expect(s().newFolders).toEqual({ [W]: ["other"] });
  // An ignored folder goes the same way.
  apply({ type: "file_deleted", worktree: W, path: "srcx/out" });
  expect(s().worktreeFiles?.ignored).toEqual([".env"]);
  expect(open()).toEqual(["src/b.ts", "src/lib/c.ts", "srcx/d.ts", "a.ts", "other:src/b.ts"]);
  expect(s().question?.text).toBe(
    "src/b.ts, src/lib/c.ts were deleted. Your unsaved changes to them will be lost.",
  );
  act(() => s().question?.run());
  expect(open()).toEqual(["srcx/d.ts", "a.ts", "other:src/b.ts"]);
  // A clean file closes without asking; a dirty one asks, and Cancel keeps it.
  useHive.setState({ modal: null, question: null });
  apply({ type: "file_deleted", worktree: W, path: "srcx/d.ts" });
  expect([open(), s().question]).toEqual([["a.ts", "other:src/b.ts"], null]);
  // Another worktree's deletion touches neither this listing nor these tabs.
  apply({ type: "file_deleted", worktree: checkout.path, path: "a.ts" });
  expect([s().worktreeFiles?.files, open()]).toEqual([["a.ts"], ["a.ts", "other:src/b.ts"]]);
  apply({ type: "file_deleted", worktree: W, path: "a.ts" });
  expect(s().question?.text).toBe("a.ts was deleted. Your unsaved changes to it will be lost.");
  expect(open()).toEqual(["a.ts", "other:src/b.ts"]);
});

test("dirty files deleted one after another are asked about in one question, over the dialog open", () => {
  const paths = ["a.ts", "b.ts", "c.ts"];
  apply({ type: "files", path: W, files: [...paths, "d.ts"], ignored: [], truncated: false });
  for (const path of paths) {
    keepOpen(file(path), true);
    answer(path, "x\n");
    type(`${path} mine\n`);
  }
  useHive.setState({ modal: "settings" });
  // Each goes in its own message: the question grows instead of being replaced.
  apply({ type: "file_deleted", worktree: W, path: "a.ts" });
  apply({ type: "files", path: W, files: ["c.ts", "d.ts"], ignored: [], truncated: false });
  apply({ type: "file_deleted", worktree: W, path: "c.ts" });
  expect(s().modal).toBe("confirm");
  expect(s().question?.text).toBe(
    "a.ts, b.ts, c.ts were deleted. Your unsaved changes to them will be lost.",
  );
  expect(s().question?.back).toBe("settings");
  act(() => s().question?.run());
  expect(s().openFiles).toEqual([]);
});

test("the bar shows every tab in its order; a drag shows a drop line; the × asks when dirty", () => {
  render(<App />);
  act(() => {
    apply({ type: "projects", projects: [shop] });
    select(fixLogin.id);
    keepOpen(file("a.ts"), true);
    answer("a.ts", "a\n");
    type("mine\n");
    keepOpen(file("b.ts"));
  });
  const tabs = () =>
    within(screen.getByRole("tablist", { name: "Open terminals and files" }))
      .getAllByRole("tab")
      .map((t) => t.textContent);
  expect(tabs()).toEqual(["a.ts", "b.ts"]);
  const box = (name: string) => screen.getByRole("tab", { name }).parentElement as HTMLElement;
  expect(box("b.ts").dataset.active).toBe("true");
  // a.ts keeps its unsaved edits while b.ts shows.
  screen.getByRole("button", { name: "Close file a.ts (unsaved changes)" });
  // Dragged onto the right half of b.ts: after it.
  const data = { setData() {}, effectAllowed: "", dropEffect: "" };
  const drag = (type: "dragStart" | "dragOver" | "drop", el: HTMLElement) => {
    const event = createEvent[type](el);
    Object.assign(event, { dataTransfer: data, clientX: 100 });
    fireEvent(el, event);
  };
  spyOn(box("b.ts"), "getBoundingClientRect").mockReturnValue({ left: 0, width: 100 } as DOMRect);
  drag("dragStart", box("a.ts"));
  drag("dragOver", box("b.ts"));
  expect(box("b.ts").dataset.drop).toBe("after");
  drag("drop", box("b.ts"));
  expect(tabs()).toEqual(["b.ts", "a.ts"]);
  fireEvent.dragEnd(box("a.ts"));
  // Clicking a tab shows it; its × asks first, since its edits are unsaved.
  fireEvent.click(screen.getByRole("tab", { name: "a.ts" }));
  expect(s().openFile).toEqual(file("a.ts"));
  fireEvent.click(screen.getByRole("button", { name: "Close file a.ts (unsaved changes)" }));
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(tabs()).toEqual(["b.ts"]);
  fireEvent.click(screen.getByRole("button", { name: "Close file b.ts" }));
  expect(s().openFiles).toEqual([]);
});

test("a view's selection and scroll are kept and put back, within the text", () => {
  const on = { change() {}, save() {}, select() {} };
  const one = createEditor(document.body, "a.ts", toText("hello\nworld\n"), on);
  one.view.dispatch({ selection: { anchor: 2, head: 9 } });
  const kept = snapshot(one.view);
  one.destroy();
  const two = createEditor(document.body, "a.ts", toText("hello\nworld\n"), on);
  restoreView(two.view, kept);
  expect([two.view.state.selection.main.anchor, two.view.state.selection.main.head]).toEqual([
    2, 9,
  ]);
  const short = createEditor(document.body, "a.ts", toText("hi"), on);
  restoreView(short.view, kept);
  expect(short.view.state.selection.main.head).toBe(2);
  restoreView(short.view, null);
  expect(short.view.state.selection.main.head).toBe(2);
  for (const e of [two, short]) e.destroy();
  document.body.innerHTML = "";
});
