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
  followSpacePlaces,
  type HiveState,
  initialState,
  select,
  setRightPanel,
  toggleSidebar,
  useHive,
} from "./store";
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
