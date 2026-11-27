import { afterEach, beforeEach, expect, test } from "bun:test";
import { stubStorage } from "../test/storage";
import {
  PLACES_LIMIT,
  savedLayout,
  savedSpacePlaces,
  savedUnseenOnly,
  saveLayout,
  saveSpacePlaces,
  saveUnseenOnly,
} from "./persist";
import type { Space } from "./protocol";
import { apply } from "./reduce";
import {
  addTab,
  followQuietTabs,
  followSpacePlaces,
  type HiveState,
  initialState,
  type OpenFile,
  select,
  setRightPanel,
  toggleSidebar,
  useHive,
} from "./store";
import { fileKey, placeKey, shownPane } from "./tabs";
import { MOCK_REPOS } from "./transport/mock";

// 28.27: the picks of 28.19's audit (docs/review/last-place.md) kept between runs, each read
// back with a fallback when the storage is empty, unreadable or throws.

const [shop, api] = MOCK_REPOS;
const [, fixLogin] = shop.worktrees;
const s = () => useHive.getState();
/** A storage whose reading throws. */
const blocked = {
  getItem: () => {
    throw new Error("blocked");
  },
};

beforeEach(() => {
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop, api] });
  select(fixLogin.id);
});
afterEach(() => useHive.setState(initialState, true));

test("the right panel closed and the sidebar hidden are kept with the widths (row 17)", () => {
  expect(savedLayout()).toEqual({ rightPanel: "files", sidebarShown: true });
  setRightPanel(null);
  toggleSidebar();
  expect(savedLayout()).toEqual({ rightPanel: null, sidebarShown: false });
  // The widths stay beside them.
  expect(JSON.parse(localStorage.getItem("hive.widths") ?? "")).toMatchObject({
    sidebarWidth: 264,
    rightPanel: null,
    sidebarShown: false,
  });
  setRightPanel("files");
  toggleSidebar();
  expect(savedLayout()).toEqual({ rightPanel: "files", sidebarShown: true });
  // Unreadable, absent or throwing: both open.
  const open = { rightPanel: "files", sidebarShown: true } as const;
  localStorage.setItem("hive.widths", "not json");
  expect(savedLayout()).toEqual(open);
  localStorage.setItem("hive.widths", "null");
  expect(savedLayout()).toEqual(open);
  expect(savedLayout(null)).toEqual(open);
  expect(savedLayout(blocked)).toEqual(open);
  // A failing storage keeps them for this run; nothing else changed writes nothing.
  const full = stubStorage(true);
  toggleSidebar();
  expect(full).toHaveBeenCalledTimes(1);
  expect(s().sidebarShown).toBe(false);
  const write = stubStorage();
  saveLayout({ ...s(), notices: [] } as HiveState, s());
  expect(write).not.toHaveBeenCalled();
});

test("Unseen only is kept between runs (row 8)", () => {
  expect(savedUnseenOnly()).toBe(false);
  useHive.setState({ unseenOnly: true });
  expect(savedUnseenOnly()).toBe(true);
  useHive.setState({ unseenOnly: false });
  expect(savedUnseenOnly()).toBe(false);
  expect(savedUnseenOnly(null)).toBe(false);
  expect(savedUnseenOnly(blocked)).toBe(false);
  const full = stubStorage(true);
  useHive.setState({ unseenOnly: true });
  expect(full).toHaveBeenCalledWith("hive.unseenOnly", "true");
  expect(s().unseenOnly).toBe(true);
  const write = stubStorage();
  saveUnseenOnly({ ...s() }, s());
  expect(write).not.toHaveBeenCalled();
});

const NO_ENV = { git_name: null, git_email: null, gh_account: null, claude_account: null };
const home: Space = { id: "default", name: "Home", projects: [shop.id], env: NO_ENV };
const work: Space = { id: "w", name: "Work", projects: [api.id], env: NO_ENV };
const spaces = (current: string) => apply({ type: "spaces", spaces: [home, work], current });
/** A restart: the store as a new run starts it, with what the last run saved. */
const restart = () => useHive.setState({ ...initialState, spacePlaces: savedSpacePlaces() }, true);

test("the launch selects the place the current space was left on, others' kept too (rows 1, 2)", () => {
  spaces("default");
  expect(s().placesRestored).toBe(true);
  select(fixLogin.id);
  spaces("w");
  expect(s().selection).toBe(api.id);
  const inWork = api.worktrees[1]?.id ?? api.id;
  select(inWork);
  spaces("default");
  expect(s().selection).toBe(fixLogin.id);
  restart();
  // The spaces may come before the projects: the place is selected once both are there.
  spaces("default");
  expect(s().selection).toBeNull();
  apply({ type: "projects", projects: [shop, api] });
  expect(s().selection).toBe(fixLogin.id);
  // Another space shows the place it was left on, not its first project.
  spaces("w");
  expect(s().selection).toBe(inWork);
  // The current space's place is saved as it changes: a restart in Work selects it again.
  restart();
  apply({ type: "projects", projects: [shop, api] });
  spaces("w");
  expect(s().selection).toBe(inWork);
});

test("a space left with nothing selected, or never seen, selects nothing at launch", () => {
  spaces("default");
  select(null);
  restart();
  apply({ type: "projects", projects: [shop, api] });
  spaces("default");
  expect([s().selection, s().placesRestored]).toEqual([null, true]);
  localStorage.clear();
  restart();
  apply({ type: "projects", projects: [shop, api] });
  spaces("w");
  expect(s().selection).toBeNull();
});

test("the spaces' places unreadable are none; capped; a failing storage keeps them", () => {
  const read = (value: string) => {
    localStorage.setItem("hive.spacePlaces", value);
    return savedSpacePlaces();
  };
  expect(read("not json")).toEqual({});
  expect(read("null")).toEqual({});
  expect(read("3")).toEqual({});
  expect(savedSpacePlaces(null)).toEqual({});
  expect(savedSpacePlaces(blocked)).toEqual({});
  const place = { place: "/w", project: "p" };
  const mixed = {
    a: place,
    b: null,
    c: { place: "/w", project: null },
    d: { place: 3, project: "p" },
    e: { place: "/w", project: 4 },
    f: "x",
  };
  expect(read(JSON.stringify(mixed))).toEqual({ a: place, b: null, c: mixed.c });
  const many = Object.fromEntries(
    Array.from({ length: PLACES_LIMIT + 5 }, (_, i) => [`s${i}`, null]),
  );
  const kept = read(JSON.stringify(many));
  expect([Object.keys(kept).length, "s0" in kept]).toEqual([PLACES_LIMIT, false]);
  saveSpacePlaces(many);
  expect(Object.keys(savedSpacePlaces())).toHaveLength(PLACES_LIMIT);
  // A storage that fails on writing keeps the place for this run.
  const full = stubStorage(true);
  spaces("default");
  select(fixLogin.id);
  expect(full).toHaveBeenCalled();
  spaces("w");
  spaces("default");
  expect(s().selection).toBe(fixLogin.id);
  // Another change writes nothing.
  const write = stubStorage();
  followSpacePlaces({ ...s(), notices: [] }, s());
  expect(write).not.toHaveBeenCalled();
});

const W = fixLogin.path;
const detected = (channel: number, id: string) =>
  apply({ type: "agent_detected", channel, id, project: shop.id, worktree: W, cwd: W });
/** A launch whose last run left fix-login with `history`, the most recent last, and `files`. */
function launch(history: string[], files: OpenFile[] = []) {
  const tabHistory = { [placeKey({ host: null }, W)]: history };
  const openFiles = files.map((f) => ({ ...f, editing: false, edit: null, view: null }));
  useHive.setState({ ...initialState, tabHistory, lastRunHistory: tabHistory, openFiles }, true);
  apply({ type: "projects", projects: [shop, api] });
  select(fixLogin.id);
}
/** A chat resumed at launch, or a terminal reattached: its tab takes no focus. */
const reopen = (id: number, host?: string) => addTab(id, W, host, false);

test("chats resumed at launch take no focus: the one shown last in the last run does (row 3)", () => {
  launch(["session:s1", "session:s2"]);
  expect(shownPane(s())).toBeNull();
  // The first shows, as nothing does; the second only once Hive knows it is s2.
  reopen(7);
  expect(shownPane(s())).toBe(7);
  detected(7, "s1");
  reopen(8);
  expect(shownPane(s())).toBe(7);
  detected(8, "s2");
  expect(shownPane(s())).toBe(8);
  // A terminal reattached later (no history names a channel) stays behind.
  reopen(9);
  expect([shownPane(s()), s().quietTabs]).toEqual([8, [7, 8, 9]]);
});

test("a tab the user went to meanwhile stays (row 3)", () => {
  launch(["session:s1", "session:s2"]);
  reopen(7);
  detected(7, "s1");
  // A terminal the user opens takes the focus, as always.
  addTab(5, W);
  expect(shownPane(s())).toBe(5);
  reopen(8);
  detected(8, "s2");
  expect(shownPane(s())).toBe(5);
});

test("a reopened file stands in until the chat shown after it comes (row 3)", () => {
  const a: OpenFile = { worktree: W, path: "a.ts" };
  launch([fileKey(a), "session:s1"], [a]);
  // Selecting the worktree showed its file, the best there was.
  expect(shownPane(s())).toBe(fileKey(a));
  reopen(7);
  expect([shownPane(s()), s().fileShown]).toEqual([fileKey(a), true]);
  detected(7, "s1");
  expect([shownPane(s()), s().fileShown]).toEqual([7, false]);
});

test("a stand-in the history never named gives way; nothing is picked elsewhere (row 3)", () => {
  launch(["session:s2"]);
  reopen(7);
  reopen(8);
  expect(shownPane(s())).toBe(7);
  detected(8, "s2");
  expect(shownPane(s())).toBe(8);
  // Another host's reattached terminal waits there.
  reopen(10, "gpu");
  expect([shownPane(s()), s().quietTabs.at(-1)]).toEqual([8, 10]);
  // With nothing selected (every tab shown) or another place, the history picks nothing.
  const before = s();
  followQuietTabs({ ...before, selection: null, tabs: [...before.tabs] }, before);
  followQuietTabs({ ...before, tabs: [...before.tabs] }, { ...before, selection: api.id });
  expect(s()).toBe(before);
});
