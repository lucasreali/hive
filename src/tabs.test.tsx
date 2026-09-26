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
import { App } from "./App";
import {
  addTab,
  apply,
  barItems,
  dropFile,
  type HiveState,
  initialState,
  moveTab,
  removeTab,
  savedTabOrder,
  saveTabOrder,
  select,
  setEdit,
  setOpenFile,
  useHive,
  visibleTabs,
} from "./store";
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
  setOpenFile(file("a.ts"), true);
  answer("a.ts", "a\n");
  type("mine a\n");
  setOpenFile(file("b.ts"));
  expect(s()).toMatchObject({ openFile: file("b.ts"), editing: false, edit: null });
  answer("b.ts", "b\n");
  expect(s().edit).toBeNull(); // Not editing b.ts: its answers keep no buffer.
  setOpenFile(file("a.ts"));
  expect(s().editing).toBe(true);
  expect(s().edit?.doc.toString()).toBe("mine a\n");
  // A save answered while the file is in the background still reaches its tab.
  const sending = { ...(s().edit as EditBuffer), saving: toText("mine a\n") };
  setEdit(sending);
  setOpenFile(file("b.ts"));
  apply({ type: "file_saved", ...file("a.ts"), version: "v2" });
  expect(s().openFiles[0]?.edit?.version).toBe("v2");
  apply({ type: "save_failed", ...file("a.ts"), error: "io", message: "no" });
  expect(s().openFiles[0]?.edit?.error).toBe("no");
  // Opening a file already open shows its tab; nothing is added.
  setOpenFile(file("b.ts"), true, 3);
  expect([bar(), s().gotoLine]).toEqual([["a.ts", "b.ts"], { ...file("b.ts"), line: 3 }]);
  expect(s().editing).toBe(false);
});

test("new tabs go last in opening order, terminals and files mixed; a drag reorders them", () => {
  addTab(1, W);
  setOpenFile(file("a.ts"));
  addTab(2, W);
  setOpenFile(file("b.ts"));
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
  setOpenFile(file("c.ts", checkout.path));
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
  setOpenFile(file("a.ts"));
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
  setOpenFile(file("a.ts"));
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
  const setItem = spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("full");
  });
  addTab(1, W);
  setOpenFile(file("a.ts"));
  moveTab(`file:${W}\na.ts`, "tab:1", false);
  expect(bar()).toEqual(["a.ts", 1]);
  setItem.mockRestore();
  // Unchanged, nothing is written.
  const write = spyOn(Storage.prototype, "setItem");
  const state = s();
  saveTabOrder({ ...state } as HiveState, state);
  expect(write).not.toHaveBeenCalled();
  write.mockRestore();
});

test("closing a tab shows its neighbour in the bar, a file or a terminal", () => {
  addTab(1, W);
  setOpenFile(file("a.ts"));
  addTab(2, W);
  setOpenFile(file("b.ts"));
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
  setOpenFile(file("c.ts"));
  setOpenFile(file("d.ts"));
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
  expect(s().split).toEqual({ left: 1, right: 3 });
});

test("a file moved into another folder (8.3) keeps its tab, its place and its unsaved edits", () => {
  setOpenFile(file("a.ts"), true);
  answer("a.ts", "a\n");
  type("mine\n");
  addTab(1, W);
  setOpenFile(file("b.ts"));
  // Moved while in the background, then while shown.
  apply({ type: "file_renamed", worktree: W, path: "a.ts", to: "lib/a.ts" });
  expect(bar()).toEqual(["lib/a.ts", 1, "b.ts"]);
  setOpenFile(file("lib/a.ts"));
  expect(s().edit?.doc.toString()).toBe("mine\n");
  expect(s().edit?.path).toBe("lib/a.ts");
  apply({ type: "file_renamed", worktree: W, path: "lib/a.ts", to: "src/lib/a.ts" });
  expect([s().openFile, s().edit?.path]).toEqual([file("src/lib/a.ts"), "src/lib/a.ts"]);
  expect(s().edit?.doc.toString()).toBe("mine\n");
  expect(s().tabOrder).toEqual([`file:${W}\nsrc/lib/a.ts`, "tab:1", `file:${W}\nb.ts`]);
});

test("a renamed or moved file keeps its tab and its place; a deleted one's tab closes", () => {
  const listing = (files: string[], truncated = false) =>
    apply({ type: "files", path: W, files, truncated });
  setOpenFile(file("a.ts"), true);
  answer("a.ts", "a\n");
  addTab(1, W);
  setOpenFile(file("b.ts"));
  setOpenFile(file("c.ts"));
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
  setOpenFile(file("src/a.ts"));
  type("mine\n");
  setOpenFile(file("c.ts"));
  listing([]);
  expect(bar()).toEqual(["src/a.ts", 1]); // c.ts closed; src/a.ts asks.
  expect(s().question?.text).toBe("src/a.ts was deleted. Your unsaved changes to it will be lost.");
  act(() => s().question?.run());
  expect(bar()).toEqual([1]);
  // Another worktree's listing, or the first one, closes nothing.
  setOpenFile(file("d.ts"));
  apply({ type: "files", path: checkout.path, files: [], truncated: false });
  listing([]);
  expect(bar()).toEqual([1, "d.ts"]);
});

test("the bar shows every tab in its order; a drag shows a drop line; the × asks when dirty", () => {
  render(<App />);
  act(() => {
    apply({ type: "projects", projects: [shop] });
    select(fixLogin.id);
    setOpenFile(file("a.ts"), true);
    answer("a.ts", "a\n");
    type("mine\n");
    setOpenFile(file("b.ts"));
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
