import { afterEach, beforeEach, expect, jest, spyOn, test } from "bun:test";
import { savedHostPlaces, savedSpacePlaces } from "./persist";
import type { ServiceMessage, Space } from "./protocol";
import { apply } from "./reduce";
import { goToAgent } from "./shortcuts";
import {
  activateTab,
  addTab,
  endRestore,
  followQuietTabs,
  type HiveState,
  hostStart,
  initialState,
  leaveSpace,
  type OpenFile,
  placesOf,
  RESTORE_LIMIT_MS,
  reattachDone,
  reattachStarted,
  restoredHost,
  resumeAtLaunch,
  resumeFailed,
  setDiffBase,
  setOpenFile,
  startHosts,
  toggleCollapsed,
  useHive,
  withCollapsed,
} from "./store";
import * as tabsModule from "./tabs";
import { fileKey, placeKey, shownPane } from "./tabs";
import { MOCK_REPOS } from "./transport/mock";

// 28.27 review: the launch restore (row 3's pick of the reopened tabs, the last run's pairs
// waiting for their chats, an agent left selected) lasts until the user's first move, until the
// chats it resumes are named, or until a link drops; never later.

const [shop, api] = MOCK_REPOS;
const [, fixLogin] = shop.worktrees;
const W = fixLogin.path;
const s = () => useHive.getState();
const NO_ENV = { git_name: null, git_email: null, gh_account: null, claude_account: null };
const home: Space = { id: "default", name: "Home", projects: [shop.id, api.id], env: NO_ENV };
const a: OpenFile = { worktree: W, path: "a.ts" };
const detected = (channel: number, id: string) =>
  apply({ type: "agent_detected", channel, id, project: shop.id, worktree: W, cwd: W });
/** A chat resumed at launch, or a terminal reattached: its tab takes no focus. */
const reopen = (id: number, host?: string) => addTab(id, W, host, false);
const connected = { status: "connected", version: "1", distro: null } as const;

/**
 * A launch whose last run left the Home space on `place` and fix-login with `history` (the most
 * recent last), with `files` reopened; the projects and the spaces then come.
 */
function launch(history: string[], files: OpenFile[] = [], place: string = fixLogin.id) {
  const tabHistory = { [placeKey({ host: null }, W)]: history };
  const openFiles = files.map((f) => ({ ...f, editing: false, edit: null, view: null }));
  // A new run: nothing of the run before.
  useHive.setState(initialState, true);
  const spacePlaces = { default: { place, project: shop.id } };
  useHive.setState(
    {
      ...initialState,
      tabHistory,
      lastRunHistory: tabHistory,
      openFiles,
      spacePlaces,
      restoring: true,
    },
    true,
  );
  apply({ type: "projects", projects: [shop, api] });
  apply({ type: "spaces", spaces: [home], current: "default" });
}

beforeEach(() => {
  // No restore timer of another test's left.
  endRestore();
  startHosts({});
});
afterEach(() => {
  jest.useRealTimers();
  endRestore();
  startHosts({});
  useHive.setState(initialState, true);
});

test("the last run's pick shows as the chats come; once all are named the restore ends", () => {
  launch(["session:s1", "session:s2"]);
  expect([s().selection, shownPane(s())]).toEqual([fixLogin.id, null]);
  resumeAtLaunch(["s1", "s2"]);
  reopen(7);
  expect(shownPane(s())).toBe(7);
  detected(7, "s1");
  reopen(8);
  expect([shownPane(s()), s().restoring]).toEqual([7, true]);
  detected(8, "s2");
  expect([shownPane(s()), s().restoring, s().quietTabs]).toEqual([8, false, []]);
  // A terminal reattached later (a reconnect), even one the last run showed, moves nothing.
  reopen(9);
  detected(9, "s1");
  expect([shownPane(s()), s().quietTabs]).toEqual([8, []]);
});

test("a reopened chat the user clicks before the last run's pick is named keeps the focus", () => {
  launch(["session:s1", "session:s2", "session:s3"]);
  resumeAtLaunch(["s1", "s2", "s3"]);
  reopen(7);
  reopen(8);
  reopen(9);
  detected(7, "s1");
  detected(8, "s2");
  expect(shownPane(s())).toBe(8);
  // The user goes to s1's tab and types; s3, the last run's pick, comes after.
  activateTab({ id: 7, cwd: W });
  expect(s().restoring).toBe(false);
  detected(9, "s3");
  expect(shownPane(s())).toBe(7);
});

test("a reopened file the restore showed gives way to the chat shown after it in the last run", () => {
  launch([fileKey(a), "session:s1"], [a]);
  reopen(7);
  expect([shownPane(s()), s().fileShown]).toEqual([fileKey(a), true]);
  detected(7, "s1");
  expect([shownPane(s()), s().fileShown]).toEqual([7, false]);
});

test("a click on the reopened file the restore showed keeps it", () => {
  launch([fileKey(a), "session:s1"], [a]);
  expect([shownPane(s()), s().fileShown]).toEqual([fileKey(a), true]);
  // The same file clicked: nothing changes on screen, but it is the user's.
  setOpenFile(a);
  expect(s().restoring).toBe(false);
  reopen(7);
  detected(7, "s1");
  expect([shownPane(s()), s().fileShown]).toEqual([fileKey(a), true]);
});

test("a link that drops ends the restore: a reconnect's terminals never move the focus", () => {
  launch(["session:s1"]);
  apply({ type: "host_link", host: "gpu", state: "connected" } as ServiceMessage);
  reopen(7);
  expect([shownPane(s()), s().restoring]).toEqual([7, true]);
  apply({
    type: "host_link",
    host: "gpu",
    state: "reconnecting",
    reason: "timeout",
    retry_secs: 5,
  } as ServiceMessage);
  expect(s().restoring).toBe(false);
  reopen(8);
  detected(8, "s1");
  expect(shownPane(s())).toBe(7);
  // So does the service's own connection dropping.
  launch(["session:s1"]);
  useHive.setState({ connection: { status: "connected", version: "1", distro: null } });
  expect(s().restoring).toBe(true);
  useHive.setState({ connection: { status: "connecting" } });
  expect(s().restoring).toBe(false);
});

test("a chat that cannot be resumed is waited for no more; a user's terminal ends the restore", () => {
  launch(["session:s1"]);
  resumeAtLaunch(["s1", "s2"]);
  resumeFailed("s2");
  reopen(7);
  expect(s().restoring).toBe(true);
  detected(7, "s1");
  expect(s().restoring).toBe(false);
  // Nothing more to wait for: these change nothing.
  resumeAtLaunch(["s3"]);
  resumeFailed("s3");
  expect(s().resuming).toBeNull();
  launch(["session:s1"]);
  addTab(5, W);
  expect([shownPane(s()), s().restoring]).toEqual([5, false]);
});

test("the last run's pairs waiting for a chat expire when the restore ends", () => {
  const c = fileKey({ worktree: W, path: "c.ts" });
  const places = {
    files: [{ worktree: W, path: "c.ts" }],
    pairs: [["session:s1", c]] as [string, string][],
    diffBases: {},
    collapsed: {},
  };
  startHosts({ "": places, gpu: places });
  launch(["session:s1"], [{ worktree: W, path: "c.ts" }]);
  useHive.setState({
    ...restoredHost(places),
    stash: { gpu: { ...hostStart("gpu") } } as HiveState["stash"],
  });
  expect([s().pendingSplits, s().stash.gpu?.pendingSplits]).toEqual([
    [["session:s1", c]],
    [["session:s1", c]],
  ]);
  endRestore();
  expect([s().pendingSplits, s().stash.gpu?.pendingSplits]).toEqual([[], []]);
  expect(hostStart("other").pendingSplits).toEqual([]);
  expect(restoredHost(places, false).pendingSplits).toEqual([]);
  // Never paired later, nor saved again.
  addTab(7, W);
  detected(7, "s1");
  expect(s().splits).toEqual([]);
  expect(savedHostPlaces()[""]?.pairs).toEqual([]);
  // Ended once, ending again changes nothing.
  const before = s();
  endRestore();
  expect(s()).toBe(before);
});

test("an agent the space was left on, not known yet, is selected once it comes", () => {
  launch(["session:s9"], [], "s9");
  // Its project for now; the place saved stays the agent.
  expect([s().selection, s().placeAwaited]).toEqual([shop.id, "s9"]);
  useHive.setState({ panelView: "changes" });
  expect(savedSpacePlaces().default?.place).toBe("s9");
  addTab(7, fixLogin.path, undefined, false);
  apply({
    type: "agent_detected",
    channel: 7,
    id: "s9",
    project: shop.id,
    worktree: W,
    cwd: W,
  });
  expect([s().selection, s().placeAwaited]).toEqual(["s9", null]);
  // Once the restore ended, an agent coming late selects nothing.
  launch(["session:s9"], [], "s8");
  expect(s().placeAwaited).toBe("s8");
  setOpenFile(null);
  expect([s().restoring, s().placeAwaited]).toEqual([false, null]);
  addTab(8, W);
  apply({ type: "agent_detected", channel: 8, id: "s8", project: shop.id, worktree: W, cwd: W });
  expect(s().selection).not.toBe("s8");
});

test("the caps forget the least recently changed; nodes as by default are not saved", () => {
  // A node changed again moves last.
  const collapsed = withCollapsed({ p: true, q: true, r: true }, { p: false });
  expect(Object.keys(collapsed)).toEqual(["q", "r", "p"]);
  useHive.setState({
    ...initialState,
    collapsed: { "files:/w/a": false, "files:/w/b": true, p: false, q: true },
  });
  expect(placesOf(s()).collapsed).toEqual({ "files:/w/a": false, q: true });
  setDiffBase("/a", "head");
  setDiffBase("/b", "head");
  setDiffBase("/a", "branch");
  expect(Object.keys(s().diffBases)).toEqual(["/b", "/a"]);
  // A space left again moves last.
  const places = { x: null, default: null, y: null };
  const left = leaveSpace({ ...s(), currentSpace: "default", spacePlaces: places });
  expect(Object.keys(left.spacePlaces ?? {})).toEqual(["x", "y", "default"]);
});

test("with nothing selected, or another host's terminal, the restore picks nothing", () => {
  launch(["session:s2"]);
  reopen(7);
  reopen(8);
  expect(shownPane(s())).toBe(7);
  detected(8, "s2");
  expect(shownPane(s())).toBe(8);
  // Another host's reattached terminal waits there, never taking the focus.
  reopen(10, "gpu");
  expect([shownPane(s()), s().quietTabs.at(-1)]).toEqual([8, 10]);
  // With nothing selected (every tab shown) or another place, the history picks nothing: with
  // nothing selected before either, the history is not even looked at.
  const before = s();
  const looked = spyOn(tabsModule, "lastShown");
  const none = { ...before, selection: null };
  followQuietTabs({ ...none, tabs: [...before.tabs] }, none);
  expect(looked).not.toHaveBeenCalled();
  looked.mockRestore();
  followQuietTabs({ ...before, tabs: [...before.tabs] }, { ...before, selection: api.id });
  expect(s()).toBe(before);
});

test("no chat resumed: the restore ends once the kept terminals are reattached", () => {
  launch(["session:s1"]);
  reattachStarted();
  reattachStarted();
  reopen(7);
  expect([s().reattaching, s().restoring]).toEqual([2, true]);
  reattachDone();
  expect(s().restoring).toBe(true);
  reattachDone();
  expect([s().restoring, s().reattaching]).toEqual([false, null]);
  // After the restore, these change nothing.
  reattachStarted();
  reattachDone();
  expect(s().reattaching).toBeNull();
});

test("the restore ends at the latest 30 s after the service connects, even with a chat never named", () => {
  jest.useFakeTimers();
  launch(["session:s1"]);
  resumeAtLaunch(["s1"]);
  useHive.setState({ connection: connected });
  jest.advanceTimersByTime(RESTORE_LIMIT_MS - 1);
  expect(s().restoring).toBe(true);
  jest.advanceTimersByTime(1);
  expect(s().restoring).toBe(false);
  // Nothing resumed nor kept: it ends then too.
  launch(["session:s1"]);
  useHive.setState({ connection: { status: "connecting" } });
  useHive.setState({ connection: connected });
  jest.advanceTimersByTime(RESTORE_LIMIT_MS);
  expect(s().restoring).toBe(false);
});

test("once the restore ended, the space's place is not selected; going to an agent ends it", () => {
  useHive.setState(initialState, true);
  useHive.setState({ spacePlaces: { default: { place: fixLogin.id, project: shop.id } } });
  apply({ type: "projects", projects: [shop, api] });
  apply({ type: "spaces", spaces: [home], current: "default" });
  expect([s().placesRestored, s().selection]).toEqual([true, null]);
  launch(["session:s1"]);
  addTab(7, W, undefined, false);
  detected(7, "s1");
  const agent = s().agents.s1;
  if (!agent) throw new Error("no agent");
  goToAgent(agent);
  expect(s().restoring).toBe(false);
});

test("a host not linked in this run keeps its saved places and pairs untouched", () => {
  const c = fileKey({ worktree: "/srv/w", path: "c.ts" });
  const off = {
    files: [{ worktree: "/srv/w", path: "c.ts" }],
    pairs: [["session:s1", c]] as [string, string][],
    diffBases: { "/srv/w": "head" as const },
    collapsed: { "files:/srv/w/src": false },
  };
  startHosts({ off });
  launch(["session:s1"]);
  endRestore();
  toggleCollapsed(shop.id);
  expect(savedHostPlaces().off).toEqual(off);
  // Linked after the restore, its waiting pairs expire.
  expect(hostStart("off").pendingSplits).toEqual([]);
});
