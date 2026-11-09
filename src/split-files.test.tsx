import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { EditorView } from "@codemirror/view";
import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { followOpenFile, followOpenTable } from "./follow";
import { apply } from "./reduce";
import { TerminalArea } from "./shell/TerminalArea";
import { showBarItem } from "./shortcuts";
import {
  activateTab,
  addTab,
  dropOnPane,
  focusFilePane,
  focusPane,
  initialState,
  moveTab,
  pinFile,
  renderedShown,
  replacePane,
  select,
  selectIn,
  setEdit,
  setOpenFile,
  setRendered,
  setSplit,
  swapSides,
  tableShown,
  unsplit,
  useHive,
} from "./store";
import {
  asideFile,
  barItems,
  dropFile,
  droppable,
  fileKey,
  paneFile,
  paneIdOf,
  paneState,
  shownPane,
  shownPanes,
  shownSplit,
  shownTerminals,
} from "./tabs";
import { closeTerminal, splitTerminal } from "./terminals";
import type { Transport } from "./transport";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";
import { type EditBuffer, isDirty, startSave, toText } from "./viewer/buffer";

// 25.1: any two tabs of a worktree can be a pair, a file with a terminal or two files.

const [shop] = MOCK_REPOS;
const [, fixLogin, checkout] = shop.worktrees;
const W = fixLogin.path;
const s = () => useHive.getState();
const a = { worktree: W, path: "src/a.ts" };
const b = { worktree: W, path: "src/b.ts" };
const A = fileKey(a);
const B = fileKey(b);

beforeEach(() => {
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop] });
  select(fixLogin.id);
});
afterEach(() => {
  for (const tab of s().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

const answer = (path: string, content: string, version = `v:${content}`) =>
  apply({
    type: "file",
    worktree: W,
    path,
    content,
    base: null,
    version,
    binary: false,
    too_large: false,
    error: null,
  });
const table = (path: string) =>
  apply({
    type: "table",
    worktree: W,
    path,
    sheets: [],
    error: null,
  });

/** `a` and `b` open with tabs of their own, `b` the open file. */
function twoFiles(editing = false) {
  setOpenFile(a, editing);
  pinFile(a);
  setOpenFile(b, editing);
  pinFile(b);
}

test("a file and a terminal pair: both show, the focused one's tab active, in the bar's order", () => {
  addTab(1, W);
  setOpenFile(a);
  pinFile(a);
  addTab(2, W);
  setSplit({ left: 1, right: A });
  expect(s().splits).toEqual([{ left: 1, right: A }]);
  // The focus on the file, on the right: the terminal beside it is the active one.
  expect([shownPane(s()), shownPanes(s()), shownTerminals(s())]).toEqual([A, [1, A], [1]]);
  expect(s().fileShown).toBe(true);
  expect(s().activeTab).toBe(1);
  expect(barItems(s()).map((t) => ("path" in t ? t.path : t.id))).toEqual([1, a.path, 2]);
  // The terminal's tab shows the same pair, with the focus there.
  activateTab({ id: 1, cwd: W });
  expect([shownPane(s()), shownPanes(s()), s().fileShown]).toEqual([1, [1, A], false]);
  // The file's tab shows it again, from a terminal outside it too.
  activateTab({ id: 2, cwd: W });
  expect(shownPanes(s())).toEqual([2]);
  setOpenFile(a);
  expect([shownPanes(s()), s().activeTab]).toEqual([[1, A], 1]);
  // Swapped with the focus kept; a click in the terminal focuses it.
  swapSides(1);
  expect([s().splits, shownPane(s())]).toEqual([[{ left: A, right: 1 }], A]);
  focusPane(1);
  expect([shownPane(s()), s().fileShown]).toEqual([1, false]);
  // Ended, the file stays in the bar, alone.
  unsplit(A);
  expect(shownSplit(s())).toBeNull();
  expect(shownPanes(s())).toEqual([1]);
});

test("two files pair: the focused one is the open file, the other keeps its state in its tab", () => {
  twoFiles(true);
  answer(b.path, "bee\n");
  setSplit({ left: A, right: B }, B);
  expect([shownPanes(s()), shownTerminals(s())]).toEqual([[A, B], []]);
  expect(s().openFile).toEqual(b);
  expect(asideFile(s())?.path).toBe(a.path);

  // The other file's answer goes to its tab, and does not take the open file's place.
  answer(a.path, "ay\n");
  expect(s().file?.path).toBe(b.path);
  expect(s().edit?.path).toBe(b.path);
  const pane = paneState(s(), a);
  expect([pane.file?.content, pane.edit?.doc.toString(), pane.editing]).toEqual([
    "ay\n",
    "ay\n",
    true,
  ]);
  // An equal answer keeps its object, so the view is not rebuilt.
  const text = pane.file;
  answer(a.path, "ay\n");
  expect(paneState(s(), a).file).toBe(text);
  // A newer one reloads its clean buffer.
  answer(a.path, "aye\n");
  expect(paneState(s(), a).edit?.doc.toString()).toBe("aye\n");

  // A click in its pane makes it the open file, with what its tab kept; the other keeps its own.
  useHive.setState({ selectedLines: { from: 1, to: 1 } });
  const buffer = paneState(s(), a).edit;
  focusFilePane(a);
  expect(s().openFile).toEqual(a);
  expect(s().edit).toBe(buffer);
  expect(s().file?.content).toBe("aye\n");
  expect(s().selectedLines).toBeNull();
  expect(paneState(s(), b).file?.content).toBe("bee\n");
  expect(paneState(s(), b).edit?.doc.toString()).toBe("bee\n");
  // Already focused, nothing changes.
  useHive.setState({ gotoLine: { ...a, line: 3 } });
  focusFilePane(a);
  expect(s().gotoLine).toEqual({ ...a, line: 3 });
  // Without the focus, a buffer gets what its view reports only while it is the open file.
  selectIn(b)({ from: 2, to: 2 });
  expect(s().selectedLines).toBeNull();
  selectIn(a)({ from: 3, to: 3 });
  expect(s().selectedLines).toEqual({ from: 3, to: 3 });
});

test("a save's answer reaches the buffer of the pane that lost the focus meanwhile", () => {
  twoFiles(true);
  answer(b.path, "bee\n");
  setSplit({ left: A, right: B }, B);
  answer(a.path, "ay\n");
  const buffer = () => paneFile(s(), B)?.edit as EditBuffer;
  // `b` is sending its edits when the focus moves to `a`.
  setEdit(startSave({ ...(s().edit as EditBuffer), doc: toText("bees\n") }));
  focusFilePane(a);
  expect(buffer().saving?.toString()).toBe("bees\n");
  expect(isDirty(buffer())).toBe(true);

  apply({ type: "file_saved", worktree: W, path: b.path, version: "v2" });
  expect(buffer().saving).toBeNull();
  expect(buffer().version).toBe("v2");
  expect(buffer().saved?.toString()).toBe("bees\n");
  expect(isDirty(buffer())).toBe(false);
  // The open file's buffer is not touched.
  expect(s().edit?.version).toBe("v:ay\n");

  // A failure too.
  focusFilePane(b);
  setEdit(startSave({ ...(s().edit as EditBuffer), doc: toText("more\n") }));
  focusFilePane(a);
  apply({ type: "save_failed", worktree: W, path: b.path, error: "io", message: "disk full" });
  expect([buffer().saving, buffer().error]).toEqual([null, "disk full"]);
  expect(isDirty(buffer())).toBe(true);
});

test("a pane's answers and modes: a tab without a buffer yet, a read-only one, a table", () => {
  setOpenFile(a, true);
  pinFile(a);
  setOpenFile(b);
  pinFile(b);
  setSplit({ left: B, right: A }, B);
  // Editing, no buffer yet: the first answer starts it. Read-only: only the text is kept.
  answer(a.path, "ay\n");
  expect(paneState(s(), a).edit?.doc.toString()).toBe("ay\n");
  focusFilePane(a);
  answer(b.path, "bee\n");
  expect(paneState(s(), b).edit).toBeNull();
  expect(paneState(s(), b).file?.content).toBe("bee\n");
  // A table goes to its tab too, and comes back with the focus.
  table(b.path);
  expect(s().table).toBeNull();
  expect(paneState(s(), b).table?.path).toBe(b.path);
  focusFilePane(b);
  expect(s().table?.path).toBe(b.path);
  // Unknown to any tab: nothing kept, nothing shown.
  expect(paneState(s(), { worktree: W, path: "nope" }).file).toBeNull();
  expect(paneState(s(), { worktree: W, path: "nope" }).editing).toBe(false);
});

test("a preview tab that enters a pair is kept: opening another file does not replace a pane", () => {
  addTab(1, W);
  setOpenFile(a);
  expect(paneFile(s(), A)?.preview).toBe(true);
  setSplit({ left: 1, right: A }, 1);
  expect(paneFile(s(), A)?.preview).toBe(false);
  setOpenFile(b);
  expect(s().openFiles.map((f) => f.path)).toEqual([a.path, b.path]);
  expect(s().splits).toEqual([{ left: 1, right: A }]);
  // `b` shows alone: outside every pair.
  expect(shownPanes(s())).toEqual([B]);
  // The pair is still there: its pane without the focus is followed.
  activateTab({ id: 1, cwd: W });
  expect(asideFile(s())?.path).toBe(a.path);
});

test("closing a paired tab ends its pair and the other pane shows", () => {
  addTab(1, W);
  twoFiles();
  // The focused file closes: the terminal beside it shows.
  setSplit({ left: 1, right: A }, A);
  useHive.setState((x) => ({ ...x, ...dropFile(s(), a) }));
  expect([s().splits, s().fileShown, s().activeTab, s().openFile]).toEqual([[], false, 1, null]);
  // A file beside a file: the other file shows.
  setOpenFile(a);
  pinFile(a);
  setSplit({ left: A, right: B }, A);
  useHive.setState((x) => ({ ...x, ...dropFile(s(), a) }));
  expect([s().splits, s().fileShown, s().openFile]).toEqual([[], true, b]);
  // The terminal focused closes: the file beside it shows.
  setSplit({ left: 1, right: B }, 1);
  closeTerminal(1);
  expect([s().splits, s().tabs, s().fileShown, s().openFile]).toEqual([[], [], true, b]);
  // The terminal not focused closes: the file keeps the focus, and the active tab moves on.
  addTab(2, W);
  addTab(3, W);
  setSplit({ left: 3, right: B }, B);
  closeTerminal(3);
  expect([s().fileShown, s().openFile, s().activeTab]).toEqual([true, b, 2]);
  // A file without the focus closes: the terminal focused stays.
  setSplit({ left: 2, right: B }, 2);
  useHive.setState((x) => ({ ...x, ...dropFile(s(), b) }));
  expect([s().splits, s().fileShown, s().activeTab]).toEqual([[], false, 2]);
});

test("a file renamed keeps its pane in its pair", () => {
  addTab(1, W);
  setOpenFile(a);
  setSplit({ left: A, right: 1 });
  apply({ type: "file_renamed", worktree: W, path: a.path, to: "src/c.ts" });
  expect(s().splits).toEqual([{ left: fileKey({ worktree: W, path: "src/c.ts" }), right: 1 }]);
  setSplit({ left: 1, right: fileKey({ worktree: W, path: "src/c.ts" }) });
  apply({ type: "file_renamed", worktree: W, path: "src/c.ts", to: "src/d.ts" });
  expect(s().splits).toEqual([{ left: 1, right: fileKey({ worktree: W, path: "src/d.ts" }) }]);
});

test("a tab dropped: files and terminals of the worktree, onto panes and onto tabs alone", () => {
  addTab(1, W);
  addTab(2, W);
  addTab(9, shop.worktrees[0]?.path as string);
  select(fixLogin.id);
  activateTab({ id: 1, cwd: W });
  twoFiles();
  const other = fileKey({ worktree: "/elsewhere", path: "x" });
  useHive.setState((x) => ({
    openFiles: [
      ...x.openFiles,
      { worktree: "/elsewhere", path: "x", editing: false, edit: null, view: null },
    ],
  }));
  expect(droppable(s(), A)).toBe(A);
  expect(droppable(s(), "tab:1")).toBe(1);
  expect([droppable(s(), other), droppable(s(), "tab:9"), droppable(s(), "nope")]).toEqual([
    null,
    null,
    null,
  ]);
  // Onto file `b` alone: a pair, the dropped one on the side it was dropped, focused.
  dropOnPane(A, "left");
  expect([s().splits, s().openFile]).toEqual([[{ left: A, right: B }], a]);
  // Onto a pane: a terminal takes the file's place, which stays in the bar, alone.
  dropOnPane("tab:1", "right");
  expect([s().splits, s().activeTab, s().fileShown]).toEqual([[{ left: A, right: 1 }], 1, false]);
  // A file onto a terminal's pane.
  dropOnPane(B, "right");
  expect([s().splits, s().openFile, s().fileShown]).toEqual([[{ left: A, right: B }], b, true]);
  // Replaced by a terminal, with the file's own tab left alone.
  replacePane(A, 2);
  expect([s().splits, s().activeTab]).toEqual([[{ left: 2, right: B }], 2]);
  // On itself nothing changes, from a tab alone.
  unsplit(2);
  activateTab({ id: 2, cwd: W });
  dropOnPane("tab:2", "left");
  expect(s().splits).toEqual([]);
  // Nothing shown: nothing to drop on.
  useHive.setState({ activeTab: null, fileShown: false });
  expect(droppable(s(), A)).toBeNull();
});

test("a pane that is a tab of no known kind pairs without taking the focus (for later tab kinds)", () => {
  addTab(1, W);
  setSplit({ left: 1, right: "browser:x" });
  expect(s().splits).toEqual([{ left: 1, right: "browser:x" }]);
  expect(s().activeTab).toBe(1);
});

test('"Split right" on a file: the next tab of its worktree in no pair, else a new terminal', async () => {
  addTab(1, W);
  addTab(2, W);
  twoFiles();
  // The bar: 1, 2, a, b. After `a`, `b`.
  await splitTerminal(A);
  expect(s().splits).toEqual([{ left: A, right: B }]);
  // On a paired file: the pair ends.
  await splitTerminal(A);
  expect(s().splits).toEqual([]);
  // Wrapping past the bar's end: after `b`, terminal 1.
  await splitTerminal(B);
  expect(s().splits).toEqual([{ left: B, right: 1 }]);
  expect([s().activeTab, s().fileShown]).toEqual([1, false]);
});

test('"Split right" on a file skips a tab in a pair', async () => {
  for (const id of [1, 2, 3]) addTab(id, W);
  twoFiles();
  // `b`, the tab after `a`, goes in a pair with terminal 1: the bar is 2, 3, a, b, 1.
  setSplit({ left: B, right: 1 });
  expect(barItems(s()).map(paneIdOf)).toEqual([2, 3, A, B, 1]);
  await splitTerminal(A);
  // Past both, wrapping: terminal 2, and the other pair stays.
  expect(s().splits).toEqual([
    { left: B, right: 1 },
    { left: A, right: 2 },
  ]);
});

test('"Split right" on a file skips another worktree\'s tabs, files and terminals', async () => {
  setOpenFile(a);
  pinFile(a);
  // Right after `a` in the order: a file and a terminal of another worktree.
  const away = { worktree: checkout.path, path: "away.ts" };
  setOpenFile(away);
  pinFile(away);
  addTab(9, checkout.path);
  addTab(1, W);
  select(fixLogin.id);
  await splitTerminal(A);
  expect(s().splits).toEqual([{ left: A, right: 1 }]);
});

test('"Split right" on a file opens a new terminal in its worktree when no tab is free', async () => {
  // Ids the service will not give the new terminal.
  addTab(101, W);
  addTab(109, checkout.path);
  twoFiles();
  select(fixLogin.id);
  setSplit({ left: B, right: 101 }, B);
  const before = s().tabs.map((t) => t.id);
  await splitTerminal(A);
  const added = s().tabs.find((t) => !before.includes(t.id));
  expect(added?.cwd).toBe(W);
  expect(s().splits).toEqual([
    { left: B, right: 101 },
    { left: A, right: added?.id as number },
  ]);
  expect([s().activeTab, s().fileShown]).toEqual([added?.id as number, false]);
});

test("a pair's other file is followed as the open one is, its table too", () => {
  const asked = async (_worktree: string, _path: string, _base?: string) => {};
  const [openFile, openTable] = [mock(asked), mock(asked)];
  const follow = { openFile, openTable } as unknown as Transport;
  apply({ type: "welcome", version: "1", distro: null });
  const stops = [followOpenFile(follow), followOpenTable(follow)];
  const [c, d] = [
    { worktree: W, path: "c.csv" },
    { worktree: W, path: "d.csv" },
  ];
  setOpenFile(c);
  pinFile(c);
  setOpenFile(d);
  pinFile(d);
  setSplit({ left: fileKey(c), right: fileKey(d) }, fileKey(d));
  expect(tableShown(paneState(s(), c))).toBe(true);
  expect(openFile.mock.calls.at(-1)?.slice(0, 2)).toEqual([W, c.path]);
  expect(openTable.mock.calls.at(-1)).toEqual([W, c.path]);
  openFile.mockClear();
  openTable.mockClear();
  // Its worktree changes: both, once each.
  apply({
    type: "changes",
    path: W,
    base: "head",
    branch: null,
    base_error: null,
    files: [],
    added: 0,
    removed: 0,
    error: null,
  });
  expect(openFile.mock.calls.map((call) => call[1])).toEqual([d.path, c.path]);
  expect(openTable.mock.calls.map((call) => call[1])).toEqual([d.path, c.path]);
  // Something else: neither.
  apply({ type: "agent_removed", channel: 1, id: "x" });
  expect(openFile).toHaveBeenCalledTimes(2);
  // The pair ended: only the open file.
  openFile.mockClear();
  unsplit(fileKey(c));
  expect(asideFile(s())).toBeNull();
  expect(openFile).not.toHaveBeenCalled();
  for (const stop of stops) stop();
});

// The panes themselves.

const bar = () => within(screen.getByRole("tablist", { name: "Open terminals and files" }));
const pane = (f: typeof a) => screen.getByRole("region", { name: f.path });
const inPane = (f: typeof a) => within(pane(f));
const editorOf = (f: typeof a) =>
  EditorView.findFromDOM(pane(f).querySelector(".cm-editor") as HTMLElement);

test("two file panes side by side, each with its own Save: only the focused one saves", () => {
  const save = spyOn(transport, "saveFile").mockResolvedValue();
  twoFiles(true);
  render(<TerminalArea />);
  act(() => {
    answer(b.path, "bee\n");
    setSplit({ left: A, right: B }, B);
  });
  act(() => answer(a.path, "ay\n"));
  expect(pane(a).getAttribute("data-pane")).toBe("left");
  expect(pane(b).getAttribute("data-pane")).toBe("right");
  expect(screen.getByRole("separator", { name: "Resize the split terminals" })).toBeTruthy();
  expect(document.querySelector<HTMLElement>(".terminal-host")?.hidden).toBe(true);
  // Both editors are there, with their own text.
  expect(editorOf(a)?.state.doc.toString()).toBe("ay\n");
  expect(editorOf(b)?.state.doc.toString()).toBe("bee\n");

  // A click in the pane without the focus gives it the focus; its editor stays, its edits reach it.
  const view = editorOf(a);
  act(() => {
    fireEvent.pointerDown(pane(a));
  });
  expect(s().openFile).toEqual(a);
  expect(editorOf(a)).toBe(view);
  act(() => view?.dispatch({ changes: { from: 0, insert: "x" } }));
  expect(s().edit?.doc.toString()).toBe("xay\n");
  // The other pane's editor reloading does not touch the open file's buffer.
  act(() => answer(b.path, "bees\n"));
  expect(editorOf(b)?.state.doc.toString()).toBe("bees\n");
  expect(s().edit?.doc.toString()).toBe("xay\n");

  fireEvent.click(inPane(a).getByRole("button", { name: "Save" }));
  expect(save.mock.calls).toEqual([[W, a.path, "xay\n", "v:ay\n"]]);
  // `b` has nothing to save and its Save button says so.
  expect((inPane(b).getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  // The bar's tab of the other one moves the keyboard to its editor.
  act(() => setOpenFile(b));
  expect(pane(b).contains(document.activeElement)).toBe(true);
  // Review comments list once: under the focused pane.
  act(() => {
    useHive.setState({
      comments: { [W]: [{ path: b.path, from: 1, to: 1, text: "note" }] },
    });
  });
  expect(screen.getAllByRole("region", { name: "Review comments" }).length).toBe(1);
  expect(inPane(b).getByRole("region", { name: "Review comments" })).toBeTruthy();
  // A Comment button of the pane without the focus does not count the other's selection.
  act(() => useHive.setState({ selectedLines: { from: 1, to: 1 } }));
  expect((inPane(a).getByRole("button", { name: "Comment" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  expect((inPane(b).getByRole("button", { name: "Comment" }) as HTMLButtonElement).disabled).toBe(
    false,
  );
  save.mockRestore();
});

test("a rendered Markdown pane's ⌘S acts only when it has the focus", () => {
  const save = spyOn(transport, "saveFile").mockResolvedValue();
  const [m, n] = [
    { worktree: W, path: "m.md" },
    { worktree: W, path: "n.md" },
  ];
  addTab(1, W);
  setOpenFile(m, true);
  pinFile(m);
  setOpenFile(n, true);
  pinFile(n);
  render(<TerminalArea />);
  act(() => {
    answer(n.path, "# n\n");
    setSplit({ left: fileKey(m), right: fileKey(n) }, fileKey(n));
  });
  act(() => {
    answer(m.path, "# m\n");
    setRendered(true);
    focusFilePane(m);
    setRendered(true);
  });
  expect(renderedShown(s())).toBe(true);
  // Edits in the focused `m`; ⌘S saves it and shows its text; `n` stays rendered.
  act(() => {
    const edit = s().edit;
    if (edit) useHive.setState({ edit: { ...edit, doc: toText("# changed\n") } });
  });
  act(() => {
    fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  });
  expect(save.mock.calls.map((c) => c[1])).toEqual([m.path]);
  expect(renderedShown(s())).toBe(false);
  expect(renderedShown(paneState(s(), n))).toBe(true);
  // With the terminal focused nothing saves.
  save.mockClear();
  act(() => {
    setOpenFile(n);
    useHive.setState({ fileShown: false });
  });
  act(() => {
    fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  });
  expect(save).not.toHaveBeenCalled();
  expect(renderedShown(paneState(s(), n))).toBe(true);
  save.mockRestore();
});

test("a file beside a terminal: the host takes its side and keeps the keyboard until the file has it", () => {
  addTab(1, W);
  setOpenFile(a, true);
  pinFile(a);
  render(<TerminalArea />);
  act(() => setSplit({ left: 1, right: A }, 1));
  act(() => answer(a.path, "ay\n"));
  const host = document.querySelector(".terminal-host") as HTMLElement;
  expect([host.hidden, host.getAttribute("data-half")]).toEqual([false, "left"]);
  expect(host.hasAttribute("data-split")).toBe(false);
  expect(pane(a).getAttribute("data-pane")).toBe("right");
  expect(s().fileShown).toBe(false);
  expect(pane(a).contains(document.activeElement)).toBe(false);
  // Keyboard focus into the pane gives it the focus too.
  act(() => {
    fireEvent.focusIn(pane(a).querySelector(".file-view-bar button") as HTMLElement);
  });
  expect(s().fileShown).toBe(true);
  // Swapped, each takes the other's side.
  act(() => swapSides(1));
  expect(host.getAttribute("data-half")).toBe("right");
  expect(pane(a).getAttribute("data-pane")).toBe("left");
  // Alone, a pane has no side and takes no focus of its own.
  act(() => unsplit(A));
  expect(pane(a).hasAttribute("data-pane")).toBe(false);
  expect(pane(a).hasAttribute("tabindex")).toBe(false);
});

test("the file tab's and the pane header's menu split, replace and close; Replace with lists files", async () => {
  addTab(1, W);
  addTab(2, W);
  twoFiles();
  const c = { worktree: W, path: "src/c.ts" };
  setOpenFile(c);
  pinFile(c);
  render(<TerminalArea />);
  const fileTab = (name: string) => bar().getByRole("tab", { name: new RegExp(`^${name}`) });
  const items = () =>
    within(screen.getByRole("menu", { name: "File" }))
      .getAllByRole("menuitem")
      .map((i) => i.textContent);

  // Alone: Split right and Close.
  fireEvent.contextMenu(fileTab("a.ts"));
  expect(items()).toEqual(["Split right", "Close file"]);
  await act(async () => {
    fireEvent.click(screen.getByRole("menuitem", { name: "Split right" }));
  });
  expect(s().splits).toEqual([{ left: A, right: B }]);
  // In the bar: one group, side by side, saying so.
  const box = (name: string) => fileTab(name).closest(".tab") as HTMLElement;
  expect(box("a.ts").getAttribute("data-pair")).toBe("left");
  expect(box("b.ts").getAttribute("data-pair")).toBe("right");
  expect(box("a.ts").querySelector(".tab-split")).not.toBeNull();
  expect(box("a.ts").getAttribute("data-split")).toBe("view");
  expect(box("b.ts").getAttribute("data-split")).toBeNull();
  expect(fileTab("a.ts").getAttribute("aria-description")).toBe("Split with b.ts");
  expect(box("a.ts").title).toBe(`${a.path}\nSplit with b.ts`);
  // Not shown: just paired.
  act(() => activateTab({ id: 2, cwd: W }));
  expect(box("a.ts").getAttribute("data-split")).toBe("paired");
  act(() => setOpenFile(b));

  // The header of a pane has the same menu.
  fireEvent.contextMenu(pane(a).querySelector(".file-view-bar") as HTMLElement);
  expect(items()).toEqual(["Unsplit", "Replace with", "Swap sides", "Close file"]);
  // Anywhere else in the pane, not.
  fireEvent.click(screen.getByRole("menuitem", { name: "Swap sides" }));
  expect(s().splits).toEqual([{ left: B, right: A }]);
  fireEvent.contextMenu(pane(a).querySelector(".file-view-body") as HTMLElement);
  expect(screen.queryByRole("menu", { name: "File" })).toBeNull();

  // Replace with ▸ lists the worktree's other tabs, files too.
  fireEvent.contextMenu(fileTab("b.ts"));
  fireEvent.click(screen.getByRole("menuitem", { name: "Replace with" }));
  const sub = within(screen.getByRole("menu", { name: "Replace with" }));
  expect(sub.getAllByRole("menuitem").map((i) => i.textContent)).toEqual([
    `${fixLogin.name} (tab 1)`,
    `${fixLogin.name} (tab 2)`,
    "c.ts",
  ]);
  fireEvent.click(sub.getByRole("menuitem", { name: "c.ts" }));
  expect(s().splits).toEqual([{ left: fileKey(c), right: A }]);
  expect(s().openFile).toEqual(c);

  // A terminal's menu lists them too, and Close file closes.
  fireEvent.contextMenu(fileTab("a.ts"));
  fireEvent.click(screen.getByRole("menuitem", { name: "Close file" }));
  expect(s().openFiles.map((f) => f.path)).toEqual([b.path, c.path]);
  // A menu left open on a file closed elsewhere shows nothing.
  fireEvent.contextMenu(fileTab("c.ts"));
  act(() => useHive.setState((x) => ({ ...x, ...dropFile(x, c) })));
  expect(screen.queryByRole("menu", { name: "File" })).toBeNull();
});

test("Replace with follows the bar's order and says a tab's place in it, not its id", () => {
  for (const id of [1, 2, 3, 4]) addTab(id, W);
  setOpenFile(a);
  pinFile(a);
  // The bar: a.ts, 4, 3, 1, 2.
  moveTab("tab:3", "tab:1", false);
  moveTab("tab:4", "tab:3", false);
  moveTab(A, "tab:4", false);
  setSplit({ left: 1, right: 2 });
  expect(barItems(s()).map(paneIdOf)).toEqual([A, 4, 3, 1, 2]);
  render(<TerminalArea />);
  fireEvent.contextMenu(bar().getAllByRole("tab")[3] as HTMLElement);
  fireEvent.click(screen.getByRole("menuitem", { name: "Replace with" }));
  const sub = within(screen.getByRole("menu", { name: "Replace with" }));
  expect(sub.getAllByRole("menuitem").map((i) => i.textContent)).toEqual([
    "a.ts",
    `${fixLogin.name} (tab 2)`,
    `${fixLogin.name} (tab 3)`,
  ]);
  // The item at place 2 is terminal 4.
  fireEvent.click(sub.getByRole("menuitem", { name: `${fixLogin.name} (tab 2)` }));
  expect(s().splits).toEqual([{ left: 4, right: 2 }]);
});

test("Replace with says why it is off when the pair holds the worktree's only tabs", () => {
  addTab(1, W);
  setOpenFile(a);
  pinFile(a);
  render(<TerminalArea />);
  act(() => setSplit({ left: 1, right: A }));
  fireEvent.contextMenu(bar().getByRole("tab", { name: /^a\.ts/ }));
  const item = screen.getByRole("menuitem", { name: "Replace with" }) as HTMLButtonElement;
  expect(item.disabled).toBe(true);
  expect(item.title).toBe("No other tab in this worktree");
});

test("a tab dragged over a file pane is highlighted and dropped there, ahead of the editor", () => {
  addTab(1, W);
  twoFiles(true);
  render(<TerminalArea />);
  act(() => answer(b.path, "bee\n"));
  const body = document.querySelector(".terminal-body") as HTMLElement;
  spyOn(body, "getBoundingClientRect").mockReturnValue({ left: 0, width: 200 } as DOMRect);
  const data = { setData() {}, effectAllowed: "", dropEffect: "" };
  const drag = (type: "dragStart" | "dragOver" | "drop", el: HTMLElement, x = 0) => {
    const event = createEvent[type](el);
    Object.assign(event, { dataTransfer: data, clientX: x });
    fireEvent(el, event);
    return event;
  };
  const dragged = bar()
    .getByRole("tab", { name: /^a\.ts/ })
    .closest(".tab") as HTMLElement;
  drag("dragStart", dragged);
  const content = pane(b).querySelector(".cm-content") as HTMLElement;
  expect(drag("dragOver", content, 150).defaultPrevented).toBe(true);
  expect(data.dropEffect).toBe("move");
  expect(document.querySelector(".pane-drop")?.getAttribute("data-side")).toBe("right");
  drag("drop", content, 150);
  expect(document.querySelector(".pane-drop")).toBeNull();
  expect(s().splits).toEqual([{ left: B, right: A }]);
  fireEvent.dragEnd(dragged);
});

test("the bar's number keys show a paired file's pair", () => {
  addTab(1, W);
  setOpenFile(a);
  pinFile(a);
  setSplit({ left: 1, right: A }, 1);
  showBarItem(1);
  expect([shownPanes(s()), shownPane(s())]).toEqual([[1, A], A]);
  // A tab removed from the state under a pane shows nothing there, without failing.
  render(<TerminalArea />);
  act(() => {
    useHive.setState((x) => ({ openFiles: x.openFiles.filter((f) => f.path !== a.path) }));
  });
  expect(screen.queryByRole("region", { name: a.path })).toBeNull();
  // Without its tab, nothing is followed for it.
  expect(asideFile(s())).toBeNull();
  // And a pane without its partner's tab says no name.
  act(() => {
    useHive.setState({ fileShown: false, openFile: null });
  });
  expect(bar().getByRole("tab").getAttribute("aria-description")).toBe("Split with ");
});
