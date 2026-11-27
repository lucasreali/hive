import { afterEach, beforeEach, expect, test } from "bun:test";
import { stubStorage } from "../test/storage";
import {
  BASES_LIMIT,
  COLLAPSED_LIMIT,
  FILES_LIMIT,
  HOSTS_LIMIT,
  type HostPlaces,
  NO_HOST_PLACES,
  PAIRS_LIMIT,
  savedHostPlaces,
  savedPlaces,
  savedTabOrder,
  saveHostPlaces,
} from "./persist";
import type { Changes, ServiceMessage } from "./protocol";
import { apply } from "./reduce";
import {
  addTab,
  diffBase,
  followPairs,
  hostStart,
  hostView,
  initialState,
  type OpenFile,
  pinFile,
  placesOf,
  restoredHost,
  saveHosts,
  select,
  setDiffBase,
  setOpenFile,
  setSplit,
  showHost,
  startHosts,
  toggleCollapsed,
  useHive,
} from "./store";
import { fileKey } from "./tabs";
import { MOCK_REPOS } from "./transport/mock";

// 28.27 (rows 4, 5, 6, 9 and 10 of docs/review/last-place.md): each host's open files, pairs,
// diff bases and trees are kept between runs (`hive.hostPlaces`).

const [shop, api] = MOCK_REPOS;
const [, fixLogin, checkout] = shop.worktrees;
const W = fixLogin.path;
const s = () => useHive.getState();
const file = (path: string, worktree = W): OpenFile => ({ worktree, path });
const open = (f: OpenFile) => {
  setOpenFile(f);
  pinFile(f);
};
/** A restart: the store as a new run starts it from what the last run saved. */
function restart() {
  const local = startHosts();
  useHive.setState(
    { ...initialState, ...local, tabOrder: savedTabOrder(), ...savedPlaces() },
    true,
  );
  apply({ type: "projects", projects: [shop, api] });
}
const detected = (channel: number, id: string, worktree = W) =>
  apply({ type: "agent_detected", channel, id, project: shop.id, worktree, cwd: worktree });

beforeEach(() => {
  startHosts({});
  useHive.setState(initialState, true);
  apply({ type: "projects", projects: [shop, api] });
  select(fixLogin.id);
});
afterEach(() => {
  startHosts({});
  useHive.setState(initialState, true);
});

test("open files reopen after a restart, without their edits; the one shown last shows (rows 5, 7)", () => {
  open(file("a.ts"));
  open(file("b.md"));
  setOpenFile(file("c.ts"));
  useHive.setState((x) => ({
    openFiles: x.openFiles.map((f) =>
      f.path === "b.md" ? { ...f, rendered: true, asText: false } : f,
    ),
  }));
  // Unsaved edits are this run's.
  useHive.setState((x) => ({
    openFiles: x.openFiles.map((f) => (f.path === "a.ts" ? { ...f, editing: true } : f)),
  }));
  open(file("b.md"));
  select(checkout.id);
  restart();
  expect(s().openFiles).toEqual([
    { worktree: W, path: "a.ts", editing: false, edit: null, view: null },
    { worktree: W, path: "b.md", rendered: true, editing: false, edit: null, view: null },
    { worktree: W, path: "c.ts", preview: true, editing: false, edit: null, view: null },
  ]);
  expect([s().openFile, s().fileShown]).toEqual([null, false]);
  select(fixLogin.id);
  expect([s().openFile, s().fileShown]).toEqual([file("b.md"), true]);
});

test("pairs of files come back; a pair with a chat once it is resumed; a plain terminal's not (row 4)", () => {
  open(file("a.ts"));
  open(file("b.ts"));
  setSplit({ left: fileKey(file("a.ts")), right: fileKey(file("b.ts")) });
  addTab(1, W);
  detected(1, "s1");
  open(file("c.ts"));
  setSplit({ left: 1, right: fileKey(file("c.ts")) }, 1);
  addTab(2, W);
  addTab(3, W);
  setSplit({ left: 2, right: 3 });
  expect(s().splits).toHaveLength(3);
  restart();
  expect(s().splits).toEqual([{ left: fileKey(file("a.ts")), right: fileKey(file("b.ts")) }]);
  expect(s().pendingSplits).toEqual([["session:s1", fileKey(file("c.ts"))]]);
  // Still waiting, it is saved again.
  restart();
  expect(s().pendingSplits).toEqual([["session:s1", fileKey(file("c.ts"))]]);
  // The chat resumed in terminal 7: once Hive knows its session, the pair is made.
  useHive.setState({ tabs: [{ id: 7, cwd: W }] });
  expect(s().pendingSplits).toHaveLength(1);
  detected(7, "s1");
  expect(s().pendingSplits).toEqual([]);
  expect(s().splits).toContainEqual({ left: 7, right: fileKey(file("c.ts")) });
});

test("a waiting pair whose file closed, or in another pair or worktree, is dropped (row 4)", () => {
  const c = fileKey(file("c.ts"));
  const d = fileKey(file("d.ts", checkout.path));
  const places: HostPlaces = {
    ...NO_HOST_PLACES,
    files: [file("c.ts"), file("e.ts"), file("d.ts", checkout.path)],
    pairs: [
      ["session:s1", c],
      ["session:s2", fileKey(file("e.ts"))],
      ["session:s3", d],
      // A file in two pairs, a pair of one tab, a file not reopened: left out.
      ["session:s4", c],
      ["session:s5", "session:s5"],
      ["session:s6", fileKey(file("gone.ts"))],
    ],
  };
  const restored = restoredHost(places);
  expect(restored.pendingSplits).toEqual([
    ["session:s1", c],
    ["session:s2", fileKey(file("e.ts"))],
    ["session:s3", d],
  ]);
  useHive.setState(restored);
  // e.ts closes: its pair goes.
  useHive.setState((x) => ({ openFiles: x.openFiles.filter((f) => f.path !== "e.ts") }));
  expect(s().pendingSplits).toHaveLength(2);
  // s1's file is paired meanwhile: s1 comes, no pair; s3's is another worktree's.
  addTab(1, W);
  setSplit({ left: 1, right: c }, 1);
  addTab(5, W);
  detected(5, "s1");
  addTab(6, W);
  detected(6, "s3");
  expect(s().pendingSplits).toEqual([]);
  expect(s().splits).toEqual([{ left: 1, right: c }]);
  // Nothing waiting, or nothing it reads changed: nothing happens.
  const before = s();
  followPairs({ ...before, notices: [] }, before);
  useHive.setState({ pendingSplits: [["session:x", "session:y"]] });
  const waiting = s();
  followPairs({ ...waiting, notices: [] }, waiting);
  expect(s()).toBe(waiting);
});

test("each worktree's diff base is kept; one the service could not use is not (row 6)", () => {
  const commit = { commit: { id: "abc1234" } };
  setDiffBase(W, commit);
  setDiffBase(checkout.path, { turn: { session: "s1" } });
  setDiffBase(api.path, "head");
  restart();
  expect(diffBase(s(), W)).toEqual(commit);
  expect(diffBase(s(), api.path)).toBe("head");
  // The turn is gone: the service compares with HEAD and says why; it falls back for good.
  const answer: Changes = {
    path: checkout.path,
    base: "head",
    branch: "main",
    base_error: "no snapshot of that turn",
    files: [],
    added: 0,
    removed: 0,
    error: null,
  };
  apply({ type: "changes", ...answer } as ServiceMessage);
  restart();
  expect(diffBase(s(), checkout.path)).toBe("branch");
  expect(diffBase(s(), W)).toEqual(commit);
});

test("collapsed projects and worktrees and open folders are kept (rows 9, 10)", () => {
  toggleCollapsed(shop.id);
  toggleCollapsed(`worktree:${fixLogin.id}`);
  useHive.setState((x) => ({ collapsed: { ...x.collapsed, [`files:${W}/src`]: false } }));
  restart();
  expect(s().collapsed).toEqual({
    [shop.id]: true,
    [`worktree:${fixLogin.id}`]: true,
    [`files:${W}/src`]: false,
  });
});

test("another host's places come back when its data is first made; a host removed forgets them", () => {
  const gpu: HostPlaces = {
    ...NO_HOST_PLACES,
    files: [{ worktree: "/srv/w", path: "x.ts" }],
    diffBases: { "/srv/w": "head" },
  };
  startHosts({ gpu });
  expect(hostStart("gpu")).toBe(hostStart("gpu"));
  expect(hostView(s(), "gpu").openFiles.map((f) => f.path)).toEqual(["x.ts"]);
  apply({ type: "host_link", host: "gpu", state: "connected" });
  showHost("gpu");
  expect(s().openFiles.map((f) => f.path)).toEqual(["x.ts"]);
  expect(s().diffBases).toEqual({ "/srv/w": "head" });
  // Kept while this Mac shows, from its stash.
  showHost(null);
  toggleCollapsed(shop.id);
  expect(savedHostPlaces().gpu?.files).toEqual([{ worktree: "/srv/w", path: "x.ts" }]);
  // A host linked later in this run keeps its saved places meanwhile.
  startHosts({ gpu, later: { ...NO_HOST_PLACES, collapsed: { p: true } } });
  toggleCollapsed(shop.id);
  expect(savedHostPlaces().later?.collapsed).toEqual({ p: true });
  apply({ type: "host_link", host: "later", state: "connected" });
  apply({ type: "host_link", host: "later", state: "removed" });
  expect(hostStart("later").collapsed).toEqual({});
  toggleCollapsed(shop.id);
  expect(savedHostPlaces().later).toBeUndefined();
});

test("saved host places unreadable are none; each list is capped; a failing storage keeps them", () => {
  const read = (value: string) => {
    localStorage.setItem("hive.hostPlaces", value);
    return savedHostPlaces();
  };
  expect(read("not json")).toEqual({});
  expect(read("null")).toEqual({});
  expect(read("[1]")).toEqual({});
  expect(savedHostPlaces(null)).toEqual({});
  expect(
    savedHostPlaces({
      getItem: () => {
        throw new Error("blocked");
      },
    }),
  ).toEqual({});
  expect(
    read(
      JSON.stringify({
        a: 3,
        b: {
          files: [
            { worktree: "/w", path: "x", preview: true, rendered: "yes", asText: true },
            { worktree: 1, path: "y" },
            "z",
          ],
          pairs: [["file:/w\nx", "session:s"], ["tab:1", "file:/w\nx"], ["file:/w\nx"], "p"],
          diffBases: {
            "/a": "head",
            "/b": "branch",
            "/c": { turn: { session: "s" } },
            "/d": { commit: { id: "c" } },
            "/e": "tail",
            "/f": { turn: { session: 1 } },
            "/g": { commit: {} },
            "/h": null,
          },
          collapsed: { p: true, q: false, r: "no" },
        },
      }),
    ),
  ).toEqual({
    a: NO_HOST_PLACES,
    b: {
      files: [{ worktree: "/w", path: "x", preview: true, asText: true }],
      pairs: [["file:/w\nx", "session:s"]],
      diffBases: {
        "/a": "head",
        "/b": "branch",
        "/c": { turn: { session: "s" } },
        "/d": { commit: { id: "c" } },
      },
      collapsed: { p: true, q: false },
    },
  });
  const range = (n: number) => Array.from({ length: n }, (_, i) => i);
  const big: HostPlaces = {
    files: range(FILES_LIMIT + 1).map((i) => ({ worktree: "/w", path: `${i}` })),
    pairs: range(PAIRS_LIMIT + 1).map((i) => [`file:${i}`, `session:${i}`]),
    diffBases: Object.fromEntries(range(BASES_LIMIT + 1).map((i) => [`/${i}`, "head"])),
    collapsed: Object.fromEntries(range(COLLAPSED_LIMIT + 1).map((i) => [`${i}`, true])),
  };
  const hosts = Object.fromEntries(range(HOSTS_LIMIT + 1).map((i) => [`h${i}`, big]));
  saveHostPlaces(hosts);
  const kept = savedHostPlaces();
  const last = kept[`h${HOSTS_LIMIT}`] as HostPlaces;
  expect([
    Object.keys(kept).length,
    "h0" in kept,
    last.files.length,
    last.files[0]?.path,
    last.pairs.length,
    Object.keys(last.diffBases).length,
    Object.keys(last.collapsed).length,
    "0" in last.collapsed,
  ]).toEqual([
    HOSTS_LIMIT,
    false,
    FILES_LIMIT,
    "1",
    PAIRS_LIMIT,
    BASES_LIMIT,
    COLLAPSED_LIMIT,
    false,
  ]);
  // A storage that fails on writing keeps them for this run; the same places write nothing.
  const full = stubStorage(true);
  toggleCollapsed(shop.id);
  expect(full).toHaveBeenCalledTimes(1);
  expect(s().collapsed[shop.id]).toBe(true);
  const write = stubStorage();
  const state = s();
  saveHosts({ ...state, notices: [] }, state);
  expect(write).not.toHaveBeenCalled();
  // Another object with the same places writes nothing either; a change does.
  saveHosts({ ...state, collapsed: { ...state.collapsed } }, state);
  expect(write).not.toHaveBeenCalled();
  saveHosts({ ...state, collapsed: { ...state.collapsed, x: true } }, state);
  expect(write).toHaveBeenCalledTimes(1);
  // The places of a view, as they are saved.
  expect(placesOf(s()).collapsed).toEqual({ [shop.id]: true });
});
