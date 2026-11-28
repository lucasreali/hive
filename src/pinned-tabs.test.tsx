import { afterEach, beforeEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { stubStorage } from "../test/storage";
import { App } from "./App";
import { NO_HOST_PLACES, savedHostPlaces, savedPlaces, savedTabOrder } from "./persist";
import { apply } from "./reduce";
import { paletteCommands } from "./shell/Palette";
import { closeActiveTab, pinShownTab } from "./shortcuts";
import {
  addTab,
  followPins,
  hostView,
  initialState,
  keepFile,
  moveTab,
  type OpenFile,
  removeTab,
  select,
  setEdit,
  setOpenFile,
  setSplit,
  showHost,
  startHosts,
  togglePin,
  useHive,
} from "./store";
import { barItems, dropFile, fileKey, isPinned, pinsFollowed, shownTab } from "./tabs";
import { closeTerminal } from "./terminals";
import { MOCK_REPOS } from "./transport/mock";
import { type EditBuffer, startEdit, toText } from "./viewer/buffer";

// 29.2 (docs/spike/pinned-tabs.md): any tab can be pinned; a pinned tab has no ×, refuses the
// user's ways to close it (⌘W, the palette, its menu) with a toast, keeps its place, and is
// remembered between runs beside the tab order (a chat by its session, a file by its path).

const [shop, api] = MOCK_REPOS;
const [, fixLogin] = shop.worktrees;
const W = fixLogin.path;
const s = () => useHive.getState();
const file = (path: string): OpenFile => ({ worktree: W, path });
const tabOf = (id: number) => s().tabs.find((t) => t.id === id) as (typeof initialState.tabs)[0];
const detected = (channel: number, id: string) =>
  apply({ type: "agent_detected", channel, id, project: shop.id, worktree: W, cwd: W });
const TEXT = {
  content: "a",
  base: null,
  version: "1",
  binary: false,
  too_large: false,
  error: null,
};
const toasts = () => s().notices.map((n) => n.text);

/** A restart: the store as a new run starts it from what the last run saved. */
function restart() {
  const local = startHosts();
  useHive.setState(
    { ...initialState, ...local, tabOrder: savedTabOrder(), ...savedPlaces() },
    true,
  );
  apply({ type: "projects", projects: [shop, api] });
}

beforeEach(() => {
  startHosts({});
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop, api] });
  select(fixLogin.id);
});
afterEach(() => {
  for (const tab of s().tabs) closeTerminal(tab.id);
  cleanup();
  startHosts({});
  useHive.setState(initialState, true);
});

test("a tab is pinned and unpinned in place: the bar's order does not move", () => {
  addTab(1, W);
  addTab(2, W);
  const order = s().tabOrder;
  togglePin(tabOf(1));
  expect(isPinned(s(), tabOf(1))).toBe(true);
  expect(isPinned(s(), tabOf(2))).toBe(false);
  expect(s().tabOrder).toBe(order);
  togglePin(tabOf(1));
  expect(isPinned(s(), tabOf(1))).toBe(false);
  expect(s().pinnedTabs).toEqual([]);
});

test("a terminal's pin follows its sessions: one starting in it, and `claude` exiting", () => {
  addTab(7, W);
  togglePin(tabOf(7));
  expect(s().pinnedTabs).toEqual(["tab:7"]);
  detected(7, "s1");
  expect(s().pinnedTabs).toEqual(["tab:7", "session:s1"]);
  apply({ type: "agent_removed", channel: 7, id: "s1" });
  // Its ended session is no longer a pin: the chat stays pinned by its channel for the run.
  expect(s().pinnedTabs).toEqual(["tab:7"]);
  expect(isPinned(s(), tabOf(7))).toBe(true);
  // A session replacing another in the terminal (`/clear`: one removed, the next detected)
  // takes over its pin.
  detected(7, "s3");
  apply({ type: "agent_removed", channel: 7, id: "s3" });
  detected(7, "s4");
  expect(s().pinnedTabs).toEqual(["tab:7", "session:s4"]);
  // A chat pinned by its session alone (resumed after a restart) gains its channel.
  addTab(8, W);
  useHive.setState({ pinnedTabs: [...s().pinnedTabs, "session:s2"] });
  detected(8, "s2");
  expect(s().pinnedTabs).toContain("tab:8");
  apply({ type: "agent_removed", channel: 8, id: "s2" });
  expect(isPinned(s(), tabOf(8))).toBe(true);
  // Unpinned and closed after its `claude` ended, nothing of it stays, saved or not.
  togglePin(tabOf(8));
  removeTab(8);
  expect(s().pinnedTabs).toEqual(["tab:7", "session:s4"]);
  expect(savedHostPlaces()[""]?.pins).toEqual(["session:s4"]);
  // Nothing missing: the same list.
  expect(pinsFollowed(s())).toBe(s().pinnedTabs);
  const before = s();
  followPins({ ...before, notices: [] }, before);
  expect(s()).toBe(before);
  // Full, no pin pushes another out: the missing key is left out and the list settles.
  const full = Array.from({ length: 499 }, (_, i) => `session:x${i}`);
  useHive.setState({ pinnedTabs: [...full, "tab:7"] });
  apply({ type: "agent_detected", channel: 7, id: "s5", project: shop.id, worktree: W, cwd: W });
  expect(s().pinnedTabs).toEqual([...full, "tab:7"]);
  expect(pinsFollowed(s())).toBe(s().pinnedTabs);
});

test("pinning a preview file keeps it: it is no longer replaced (11.1)", () => {
  setOpenFile(file("a.ts"));
  expect(s().openFiles[0]?.preview).toBe(true);
  togglePin(s().openFiles[0] as (typeof initialState.openFiles)[0]);
  expect(s().openFiles[0]?.preview).toBe(false);
  setOpenFile(file("b.ts"));
  expect(s().openFiles.map((f) => f.path)).toEqual(["a.ts", "b.ts"]);
  expect(s().pinnedTabs).toEqual([fileKey(file("a.ts"))]);
});

test("⌘W and the palette's Close leave a pinned tab open with a toast; unpinned, it closes", () => {
  addTab(1, W);
  togglePin(tabOf(1));
  closeActiveTab();
  expect(s().tabs.map((t) => t.id)).toEqual([1]);
  expect(toasts()).toEqual(["fix-login is pinned. Unpin it to close it."]);
  // The palette's "Close the tab" runs the same command.
  const close = paletteCommands(s()).find((c) => c.label === "Close the tab");
  close?.run();
  expect(s().tabs.map((t) => t.id)).toEqual([1]);
  togglePin(tabOf(1));
  closeActiveTab();
  expect(s().tabs).toEqual([]);

  setOpenFile(file("a.ts"));
  keepFile(file("a.ts"));
  togglePin(s().openFiles[0] as (typeof initialState.openFiles)[0]);
  closeActiveTab();
  expect(s().openFiles.map((f) => f.path)).toEqual(["a.ts"]);
  expect(toasts().at(-1)).toBe("a.ts is pinned. Unpin it to close it.");
  // A file shown of another place: nothing would close, so no toast either.
  addTab(3, W);
  togglePin(tabOf(3));
  useHive.setState({ fileShown: true, openFile: file("a.ts"), selection: api.id });
  const count = toasts().length;
  closeActiveTab();
  expect(toasts()).toHaveLength(count);
  expect(s().tabs.map((t) => t.id)).toEqual([3]);
});

test("the palette pins and unpins the tab shown, the focused pane's in a pair", () => {
  const label = () =>
    paletteCommands(s()).find((c) => c.label === "Pin the tab" || c.label === "Unpin the tab");
  // Nothing shown: nothing to pin, and the palette does not offer it.
  pinShownTab();
  expect(s().pinnedTabs).toEqual([]);
  expect(label()).toBeUndefined();
  addTab(1, W);
  expect(label()?.label).toBe("Pin the tab");
  addTab(2, W);
  setSplit({ left: 1, right: 2 }, 1);
  expect(shownTab(s())).toEqual(tabOf(1));
  label()?.run();
  expect(isPinned(s(), tabOf(1))).toBe(true);
  expect(isPinned(s(), tabOf(2))).toBe(false);
  expect(label()?.label).toBe("Unpin the tab");
  pinShownTab();
  expect(s().pinnedTabs).toEqual([]);
});

test("a tab closed another way, or a file renamed or its project removed, takes its pin along", () => {
  addTab(1, W);
  detected(1, "s1");
  togglePin(tabOf(1));
  removeTab(1);
  expect(s().pinnedTabs).toEqual([]);

  setOpenFile(file("a.ts"));
  togglePin(s().openFiles[0] as (typeof initialState.openFiles)[0]);
  apply({ type: "file_renamed", worktree: W, path: "a.ts", to: "c.ts" });
  expect(s().pinnedTabs).toEqual([fileKey(file("c.ts"))]);
  useHive.setState(dropFile(s(), file("c.ts")));
  expect(s().pinnedTabs).toEqual([]);

  setOpenFile(file("b.ts"));
  togglePin(s().openFiles[0] as (typeof initialState.openFiles)[0]);
  useHive.setState({ pinnedTabs: [...s().pinnedTabs, "session:gone", "session:kept"] });
  apply({ type: "agent_detected", channel: 5, id: "gone", project: shop.id, worktree: W, cwd: W });
  apply({ type: "project_removed", id: shop.id });
  expect(s().pinnedTabs).toEqual(["session:kept"]);
});

test("pins of files and chats come back after a restart; a plain terminal's and a closed file's do not", () => {
  setOpenFile(file("a.ts"));
  keepFile(file("a.ts"));
  setOpenFile(file("b.ts"));
  keepFile(file("b.ts"));
  togglePin(s().openFiles[0] as (typeof initialState.openFiles)[0]);
  addTab(1, W);
  detected(1, "s1");
  togglePin(tabOf(1));
  // The chat dragged before b.ts (8.21), so the order holds its session.
  moveTab("session:s1", fileKey(file("b.ts")), false);
  addTab(2, W);
  togglePin(tabOf(2));
  expect(savedHostPlaces()[""]?.pins).toEqual([fileKey(file("a.ts")), "session:s1"]);
  restart();
  expect(s().pinnedTabs).toEqual([fileKey(file("a.ts")), "session:s1"]);
  expect(s().openFiles.map((f) => f.path)).toEqual(["a.ts", "b.ts"]);
  // The chat resumed in a new terminal is pinned, in its place.
  addTab(9, W);
  detected(9, "s1");
  expect(isPinned(s(), tabOf(9))).toBe(true);
  const bar = barItems(s()).map((t) => ("path" in t ? t.path : t.id));
  expect(bar).toEqual(["a.ts", 9, "b.ts"]);
  // A file not reopened is pinned no longer.
  const places = { ...NO_HOST_PLACES, files: [file("b.ts")], pins: [fileKey(file("a.ts"))] };
  expect(startHosts({ "": places }).pinnedTabs).toEqual([]);
});

test("saved pins unreadable, or a storage that throws, give no pins; a full storage keeps them", () => {
  const read = (pins: unknown) => {
    localStorage.setItem("hive.hostPlaces", JSON.stringify({ "": { pins } }));
    return savedHostPlaces()[""]?.pins;
  };
  expect(read("session:a")).toEqual([]);
  expect(read([1, null, "tab:3", "browser:x", "session:a", "session:a", "file:/w\na"])).toEqual([
    "session:a",
    "file:/w\na",
  ]);
  expect(read(Array.from({ length: 600 }, (_, i) => `session:${i}`))).toHaveLength(500);
  localStorage.clear();
  expect(startHosts().pinnedTabs).toEqual([]);
  expect(
    savedHostPlaces({
      getItem: () => {
        throw new Error("blocked");
      },
    }),
  ).toEqual({});
  const full = stubStorage(true);
  addTab(1, W);
  detected(1, "s1");
  togglePin(tabOf(1));
  expect(full).toHaveBeenCalled();
  expect(isPinned(s(), tabOf(1))).toBe(true);
});

test("each host keeps its own pins: the same path on another host is not pinned", () => {
  setOpenFile(file("a.ts"));
  togglePin(s().openFiles[0] as (typeof initialState.openFiles)[0]);
  apply({ type: "host_link", host: "gpu", state: "connected" });
  expect(hostView(s(), "gpu").pinnedTabs).toEqual([]);
  showHost("gpu");
  expect(s().pinnedTabs).toEqual([]);
  showHost(null);
  expect(s().pinnedTabs).toEqual([fileKey(file("a.ts"))]);
  expect(savedHostPlaces().gpu?.pins).toEqual([]);
});

// The tab bar and the menus.

const bar = () => within(screen.getByRole("tablist", { name: "Open terminals and files" }));
const tabEl = (name: string) =>
  bar()
    .getByRole("tab", { name: new RegExp(`^${name}`) })
    .closest(".tab") as HTMLElement;
const menuItems = () =>
  within(screen.getByRole("menu"))
    .getAllByRole("menuitem")
    .map((i) => i.textContent);

test("a pinned tab shows a pin, not a ×; its menu unpins and disables Close", () => {
  render(<App />);
  act(() => addTab(1, W));
  const label = () => bar().getByRole("tab", { name: /^fix-login/ });
  fireEvent.contextMenu(label());
  expect(menuItems()).toEqual(["Split right", "Pin tab", "Close terminal"]);
  fireEvent.click(screen.getByRole("menuitem", { name: "Pin tab" }));
  expect(screen.queryByRole("menu")).toBeNull();
  const tab = tabEl("fix-login");
  expect(within(tab).queryByRole("button", { name: /^Close terminal/ })).toBeNull();
  expect(tab.querySelector(".tab-pin svg")).not.toBeNull();
  expect(tab.getAttribute("title")).toBe(`${W}\nPinned`);
  expect(tab.dataset.pinned).toBe("true");
  expect(label().getAttribute("aria-description")).toBe("Pinned");
  // A click on the pin does nothing.
  fireEvent.click(tab.querySelector(".tab-pin") as HTMLElement);
  expect(s().tabs).toHaveLength(1);

  fireEvent.contextMenu(label());
  expect(menuItems()).toEqual(["Split right", "Unpin tab", "Close terminalUnpin to close"]);
  const close = screen.getByRole("menuitem", { name: /^Close terminal/ }) as HTMLButtonElement;
  expect(close.disabled).toBe(true);
  fireEvent.click(close);
  expect(s().tabs).toHaveLength(1);
  fireEvent.click(screen.getByRole("menuitem", { name: "Unpin tab" }));
  expect(isPinned(s(), tabOf(1))).toBe(false);
  fireEvent.contextMenu(label());
  fireEvent.click(screen.getByRole("menuitem", { name: "Close terminal" }));
  expect(s().tabs).toEqual([]);
});

test("a pinned file's menu pins and closes it; with unsaved edits its dot shows, not the pin", () => {
  render(<App />);
  act(() => {
    setOpenFile(file("a.ts"));
    keepFile(file("a.ts"));
  });
  const label = () => bar().getByRole("tab", { name: /^a\.ts/ });
  fireEvent.contextMenu(label());
  fireEvent.click(screen.getByRole("menuitem", { name: "Pin tab" }));
  expect(tabEl("a.ts").querySelector(".tab-pin svg")).not.toBeNull();
  const text = { ...TEXT, worktree: W, path: "a.ts" };
  const buffer = startEdit(text) as EditBuffer;
  act(() => setEdit({ ...buffer, doc: toText("changed") }));
  const tab = tabEl("a.ts");
  expect(tab.querySelector(".tab-pin .dirty")).not.toBeNull();
  expect(tab.querySelector(".tab-pin svg")).toBeNull();
  expect(label().getAttribute("aria-description")).toBe("Pinned. Unsaved changes");
  fireEvent.contextMenu(label());
  fireEvent.click(screen.getByRole("menuitem", { name: "Unpin tab" }));
  act(() => setEdit(null));
  fireEvent.contextMenu(label());
  fireEvent.click(screen.getByRole("menuitem", { name: "Close file" }));
  expect(s().openFiles).toEqual([]);
});
