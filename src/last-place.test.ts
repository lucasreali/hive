import { afterEach, beforeEach, expect, test } from "bun:test";
import { stubStorage } from "../test/storage";
import { HISTORY_LIMIT, PLACES_LIMIT, remembered, savedPlaces, savePlaces } from "./persist";
import type { Space } from "./protocol";
import { apply } from "./reduce";
import {
  activateTab,
  addTab,
  followPlaces,
  type HiveState,
  initialState,
  type OpenFile,
  pinFile,
  removeTab,
  select,
  setOpenFile,
  setPanelView,
  setSplit,
  showHost,
  showPanelView,
  useHive,
} from "./store";
import { placeKey, shownPane, shownTerminals } from "./tabs";
import { MOCK_REPOS } from "./transport/mock";

// 28.19: closing a tab shows the one used before it, and coming back to a worktree, a space or a
// host shows the tab and the panel view it was left with; kept between runs.

const [shop, api] = MOCK_REPOS;
const [main, fixLogin, checkout] = shop.worktrees;
const W = fixLogin.path;
const s = () => useHive.getState();
const file = (path: string, worktree = W): OpenFile => ({ worktree, path });
/** Opens `f` in a tab of its own (kept, 11.1). */
const open = (f: OpenFile) => {
  setOpenFile(f);
  pinFile(f);
};
/** The shown tab: a terminal by its id, a file by its path. */
const shown = () => {
  const pane = shownPane(s());
  return typeof pane === "string" ? pane.split("\n")[1] : pane;
};

beforeEach(() => {
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop, api] });
  select(fixLogin.id);
});
afterEach(() => useHive.setState(initialState, true));

test("closing the shown tab shows the one shown before it, not its neighbour", () => {
  addTab(1, W);
  addTab(2, W);
  addTab(3, W);
  open(file("a.ts"));
  // Shown in the order 3, 1, a.ts: the bar is 1, 2, 3, a.ts.
  activateTab({ id: 1, cwd: W });
  open(file("a.ts"));
  // The file closes: terminal 1 (shown before it), not 3 (its neighbour).
  setOpenFile(null);
  expect(shown()).toBe(1);
  // Terminal 1 closes: 3, shown before it; 2 was never shown.
  removeTab(1);
  expect(shown()).toBe(3);
  // A file shown before the terminal closed shows again.
  open(file("b.ts"));
  activateTab({ id: 3, cwd: W });
  removeTab(3);
  expect([shown(), s().fileShown]).toEqual(["b.ts", true]);
  // With none of its history left open, its neighbour.
  useHive.setState({ tabHistory: {} });
  setOpenFile(null);
  expect(shown()).toBe(2);
});

test("a terminal closed under a shown file leaves the file shown, the terminal used before active", () => {
  addTab(1, W);
  addTab(2, W);
  addTab(3, W);
  activateTab({ id: 1, cwd: W });
  activateTab({ id: 3, cwd: W });
  open(file("a.ts"));
  removeTab(3);
  expect([shown(), s().activeTab]).toEqual(["a.ts", 1]);
});

test("a closed tab of a pair still shows the other pane", () => {
  addTab(1, W);
  addTab(2, W);
  addTab(3, W);
  setSplit({ left: 1, right: 3 }, 3);
  removeTab(3);
  expect([shown(), shownTerminals(s())]).toEqual([1, [1]]);
});

test("coming back to a worktree shows the tab it showed, a file, or a pair with its focus", () => {
  addTab(1, W);
  addTab(2, W);
  addTab(3, W);
  addTab(5, checkout.path);
  // fix-login left on its first terminal.
  activateTab({ id: 1, cwd: W });
  select(checkout.id);
  expect(shown()).toBe(5);
  select(fixLogin.id);
  expect(shown()).toBe(1);
  // Left on a file: the file.
  open(file("a.ts"));
  select(checkout.id);
  expect(s().fileShown).toBe(false);
  select(fixLogin.id);
  expect([shown(), s().fileShown]).toEqual(["a.ts", true]);
  // Left on a pair, its left pane focused: the pair, that pane focused.
  setSplit({ left: 2, right: 3 }, 2);
  select(checkout.id);
  select(fixLogin.id);
  expect([shownTerminals(s()), shown()]).toEqual([[2, 3], 2]);
  // A worktree that never showed a tab shows its last terminal, as before.
  addTab(6, main.path);
  addTab(7, main.path);
  useHive.setState({ tabHistory: {} });
  select(main.id);
  expect(shown()).toBe(7);
});

test("coming back to a space shows the place, the tab and the view it was left with", () => {
  const env = { git_name: null, git_email: null, gh_account: null, claude_account: null };
  const home: Space = { id: "default", name: "Home", projects: [shop.id], env };
  const work: Space = { id: "w", name: "Work", projects: [api.id], env };
  const to = (current: string) => apply({ type: "spaces", spaces: [home, work], current });
  to("default");
  select(fixLogin.id);
  addTab(1, W);
  addTab(2, W);
  addTab(3, api.path);
  addTab(4, api.path);
  activateTab({ id: 1, cwd: W });
  showPanelView("changes");
  to("w");
  activateTab({ id: 3, cwd: api.path });
  setPanelView("sessions");
  to("default");
  expect([s().selection, shown(), s().panelView]).toEqual([fixLogin.id, 1, "changes"]);
  to("w");
  expect([s().selection, shown(), s().panelView]).toEqual([api.id, 3, "sessions"]);
});

test("coming back to a host shows the tab its worktree showed", () => {
  const HOST = "gpu-box";
  addTab(1, W);
  addTab(2, W);
  activateTab({ id: 1, cwd: W });
  addTab(3, "/remote", HOST);
  addTab(4, "/remote", HOST);
  activateTab({ id: 3, cwd: "/remote", host: HOST });
  expect(shown()).toBe(3);
  showHost(null);
  select(fixLogin.id);
  expect(shown()).toBe(1);
  // The remote's place is its own, though a path of this Mac's could be the same.
  expect(Object.keys(s().tabHistory)).toEqual([placeKey({ host: null }, W), `${HOST}\n/remote`]);
});

test("the panel's view is each worktree's; a view picked with the move is the new one's", () => {
  setPanelView("changes");
  select(checkout.id);
  // A worktree never shown takes the view shown.
  expect(s().panelView).toBe("changes");
  setPanelView("comments");
  select(fixLogin.id);
  expect(s().panelView).toBe("changes");
  select(checkout.id);
  expect(s().panelView).toBe("comments");
  useHive.setState({ selection: fixLogin.id, panelView: "files" });
  expect(s().panelViews[placeKey(s(), W)]).toBe("files");
  // Nothing selected and no terminal: no worktree shows, nothing is remembered.
  const views = s().panelViews;
  useHive.setState({ selection: null, panelView: "scripts" });
  expect(s().panelViews).toBe(views);
});

test("the places are kept for a reload: files' and sessions' tabs, and the views", () => {
  addTab(1, W);
  apply({ type: "agent_detected", channel: 1, id: "s1", project: shop.id, worktree: W, cwd: W });
  addTab(2, W);
  open(file("a.ts"));
  activateTab({ id: 1, cwd: W });
  setPanelView("changes");
  select(checkout.id);
  const saved = savedPlaces();
  // A plain terminal's channel is this run's only.
  expect(saved.tabHistory[placeKey(s(), W)]).toEqual([`file:${W}\na.ts`, "session:s1"]);
  expect(saved.panelViews[placeKey(s(), W)]).toBe("changes");
  // After a reload, with the session resumed in a terminal not shown yet, selecting its
  // worktree shows it (not the worktree's last terminal), in the view it was left with.
  useHive.setState({ ...initialState, ...saved }, true);
  apply({ type: "projects", projects: [shop, api] });
  useHive.setState({ tabs: [7, 8].map((id) => ({ id, cwd: W })) });
  apply({ type: "agent_detected", channel: 7, id: "s1", project: shop.id, worktree: W, cwd: W });
  select(fixLogin.id);
  expect([shown(), s().panelView]).toEqual([7, "changes"]);
});

test("saved places that cannot be read are none; each is capped; a failing storage keeps them", () => {
  const read = (value: string) => {
    localStorage.setItem("hive.places", value);
    return savedPlaces();
  };
  const none = { tabHistory: {}, panelViews: {} };
  expect(savedPlaces(null)).toEqual(none);
  expect(read("not json")).toEqual(none);
  expect(read("null")).toEqual(none);
  expect(read(JSON.stringify({ tabHistory: 3, panelViews: null }))).toEqual(none);
  expect(
    read(
      JSON.stringify({
        tabHistory: { a: ["file:x", 2, "tab:3", "browser:/w", "session:s"], b: "file:y", c: [] },
        panelViews: { a: "changes", b: "nowhere", c: 4 },
      }),
    ),
  ).toEqual({ tabHistory: { a: ["file:x", "session:s"] }, panelViews: { a: "changes" } });
  const many = Array.from({ length: HISTORY_LIMIT + 5 }, (_, i) => `file:${i}`);
  expect(read(JSON.stringify({ tabHistory: { a: many } })).tabHistory.a).toEqual(
    many.slice(-HISTORY_LIMIT),
  );
  const views = Object.fromEntries(
    Array.from({ length: PLACES_LIMIT + 5 }, (_, i) => [`p${i}`, "files"]),
  );
  expect(Object.keys(read(JSON.stringify({ panelViews: views })).panelViews)).toHaveLength(
    PLACES_LIMIT,
  );
  // A storage that throws on reading is none.
  expect(
    savedPlaces({
      getItem: () => {
        throw new Error("blocked");
      },
    }),
  ).toEqual(none);
  // One that fails on writing keeps them for this run.
  const full = stubStorage(true);
  addTab(1, W);
  expect(full).toHaveBeenLastCalledWith(
    "hive.places",
    JSON.stringify({ tabHistory: s().tabHistory, panelViews: s().panelViews }),
  );
  expect(s().tabHistory[placeKey(s(), W)]).toEqual(["tab:1"]);
  // Unchanged, nothing is written.
  const write = stubStorage();
  const state = s();
  savePlaces({ ...state } as HiveState, state);
  expect(write).not.toHaveBeenCalled();
});

test("a worktree's history keeps its last tabs, and the oldest worktrees are forgotten", () => {
  for (let id = 1; id <= HISTORY_LIMIT + 3; id++) addTab(id, W);
  const list = s().tabHistory[placeKey(s(), W)] ?? [];
  expect(list).toHaveLength(HISTORY_LIMIT);
  expect(list.at(-1)).toBe(`tab:${HISTORY_LIMIT + 3}`);
  // A tab shown again moves last, once.
  activateTab({ id: 10, cwd: W });
  expect((s().tabHistory[placeKey(s(), W)] ?? []).filter((k) => k === "tab:10")).toHaveLength(1);
  const many = Object.fromEntries(Array.from({ length: PLACES_LIMIT }, (_, i) => [`p${i}`, i]));
  const kept = remembered(many, "new", -1);
  expect([Object.keys(kept).length, "p0" in kept, kept.new]).toEqual([PLACES_LIMIT, false, -1]);
  // Another change of the store leaves the places alone.
  const before = s();
  followPlaces({ ...before, notices: [] }, before);
  expect(s()).toBe(before);
});
