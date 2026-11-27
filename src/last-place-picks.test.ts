import { afterEach, beforeEach, expect, test } from "bun:test";
import { stubStorage } from "../test/storage";
import { savedLayout, savedUnseenOnly, saveLayout, saveUnseenOnly } from "./persist";
import { apply } from "./reduce";
import {
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
