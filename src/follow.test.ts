import { beforeEach, expect, mock, test } from "bun:test";
import { followOpenFile, followPanel, followView } from "./follow";
import { apply } from "./reduce";
import {
  addTab,
  initialState,
  pinFile,
  select,
  setDiffBase,
  setFocused,
  setOpenFile,
  setRightPanel,
  useHive,
} from "./store";
import type { Transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

beforeEach(() => useHive.setState(initialState, true));

function recorder() {
  const calls: (string | null)[] = [];
  const transport = {
    // The main worktrees are watched against HEAD; any other base shows.
    watchWorktree: async (path: string, base: string) =>
      void calls.push(base === "head" ? path : `${base}:${path}`),
    unwatchWorktree: async () => void calls.push(null),
  } as unknown as Transport;
  return { calls, transport };
}

test("the service watches the worktree the open files panel shows, and nothing else", () => {
  const { calls, transport } = recorder();
  const [shop, api] = MOCK_REPOS as [(typeof MOCK_REPOS)[number], (typeof MOCK_REPOS)[number]];
  apply({ type: "projects", projects: MOCK_REPOS });
  setRightPanel("files");
  select(shop.id);
  const stop = followPanel(transport);
  // Not connected yet.
  expect(calls).toEqual([]);
  apply({ type: "welcome", version: "0.1.0", distro: null });
  expect(calls).toEqual([shop.id]);
  // Other changes do not send it again.
  apply({ type: "files", path: shop.id, files: [], ignored: [], truncated: false });
  select(api.id);
  setRightPanel(null);
  expect(calls).toEqual([shop.id, api.id, null]);
  setRightPanel("files");
  expect(calls).toEqual([shop.id, api.id, null, api.id]);
  // A new connection has no watch: nothing to stop, and it is sent again.
  apply({ type: "disconnected", reason: "gone", bundled: false });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  expect(calls).toEqual([shop.id, api.id, null, api.id, api.id]);
  stop();
  select(shop.id);
  expect(calls).toHaveLength(5);
});

test("the ignored folders open in the tree are sent when they change, and again after a new watch", () => {
  const sent: unknown[] = [];
  const transport = {
    watchWorktree: async (path: string) => void sent.push(`watch ${path}`),
    unwatchWorktree: async () => void sent.push("unwatch"),
    expandIgnored: async (path: string, folders: string[]) => void sent.push([path, folders]),
  } as unknown as Transport;
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  apply({ type: "projects", projects: MOCK_REPOS });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  setRightPanel("files");
  select(shop.id);
  const stop = followPanel(transport);
  const listing = (ignored: string[], path = shop.id) =>
    apply({ type: "files", path, files: ["a.ts"], ignored, truncated: false });
  const setOpen = (folder: string, open: boolean) =>
    useHive.setState((s) => ({
      collapsed: { ...s.collapsed, [`files:${shop.id}/${folder}`]: !open },
    }));
  // What was sent since the last look.
  const taken = () => sent.splice(0);
  listing([".env", "deps/", "out/"]);
  expect(taken()).toEqual([`watch ${shop.id}`]);
  // An open folder that is not ignored is not sent.
  setOpen("src", true);
  expect(taken()).toEqual([]);
  setOpen("deps", true);
  expect(taken()).toEqual([[shop.id, ["deps"]]]);
  // Another worktree's listing holds none of this one's.
  listing([".env", "deps/", "deps/a/", "out/"], "/elsewhere");
  listing([".env", "deps/", "deps/a/", "out/"]);
  expect(taken()).toEqual([
    [shop.id, []],
    [shop.id, ["deps"]],
  ]);
  // A folder inside counts once the listing names it; one closed is not sent.
  setOpen("deps/a", true);
  setOpen("out", false);
  expect(taken()).toEqual([[shop.id, ["deps", "deps/a"]]]);
  // Unchanged, nothing goes; closed, it goes.
  listing([".env", "deps/", "deps/a/", "deps/a/x.js", "out/"]);
  setOpen("deps", false);
  expect(taken()).toEqual([[shop.id, ["deps/a"]]]);
  // A new watch starts with none open: sent again once listed.
  apply({ type: "disconnected", reason: "gone", bundled: false });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  listing([".env", "deps/", "deps/a/", "out/"]);
  expect(taken()).toEqual([`watch ${shop.id}`, [shop.id, ["deps/a"]]]);
  // Closed panel: nothing.
  setRightPanel(null);
  setOpen("out", true);
  expect(taken()).toEqual(["unwatch"]);
  stop();
});

test("a Claude worktree is watched against its branch, and anew when its base is picked", () => {
  const { calls, transport } = recorder();
  const fix = (MOCK_REPOS[0] as (typeof MOCK_REPOS)[number]).worktrees[1] as { path: string };
  apply({ type: "projects", projects: MOCK_REPOS });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  setRightPanel("files");
  select(fix.path);
  const stop = followPanel(transport);
  expect(calls).toEqual([`branch:${fix.path}`]);
  setDiffBase(fix.path, "head");
  setDiffBase("/elsewhere", "branch");
  expect(calls).toEqual([`branch:${fix.path}`, fix.path]);
  stop();
});

const welcome = () => apply({ type: "welcome", version: "1", distro: null });
const none = { base: "head", branch: null, base_error: null } as const;
const changes = (path: string) =>
  apply({ type: "changes", path, ...none, files: [], added: 0, removed: 0, error: null });

test("asks for the open file when it opens, its worktree changes, or the service is new", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  const stop = followOpenFile({ openFile } as unknown as Transport);
  setOpenFile({ worktree: "/w", path: "a.ts" });
  pinFile({ worktree: "/w", path: "a.ts" }); // Its tab stays when b.ts opens (11.1).
  expect(openFile).not.toHaveBeenCalled(); // Not connected yet.

  welcome();
  expect(openFile.mock.calls).toEqual([["/w", "a.ts", "head"]]);
  changes("/other");
  apply({ type: "agent_removed", channel: 1, id: "x" });
  expect(openFile).toHaveBeenCalledTimes(1);
  changes("/w");
  setOpenFile({ worktree: "/w", path: "b.ts" });
  welcome();
  expect(openFile.mock.calls.slice(1)).toEqual([
    ["/w", "a.ts", "head"],
    ["/w", "b.ts", "head"],
    ["/w", "b.ts", "head"],
  ]);

  // Closing b.ts's tab shows a.ts's, beside it (8.21).
  setOpenFile(null);
  expect(openFile.mock.calls.at(-1)).toEqual(["/w", "a.ts", "head"]);
  apply({ type: "disconnected", reason: "gone", bundled: false });
  stop();
  setOpenFile({ worktree: "/w", path: "c.ts" });
  expect(openFile).toHaveBeenCalledTimes(5);
});

test("asks for the open file again at the base picked for its worktree", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  welcome();
  setOpenFile({ worktree: "/w", path: "a.ts" });
  const stop = followOpenFile({ openFile } as unknown as Transport);
  setDiffBase("/other", "branch");
  setDiffBase("/w", "branch");
  expect(openFile.mock.calls).toEqual([
    ["/w", "a.ts", "head"],
    ["/w", "a.ts", "branch"],
  ]);
  stop();
});

test("asks again when a save found a newer version on disk", () => {
  const openFile = mock(async (_worktree: string, _path: string, _base: string) => {});
  welcome();
  setOpenFile({ worktree: "/w", path: "a.ts" }, true);
  const stop = followOpenFile({ openFile } as unknown as Transport);
  const answer = { worktree: "/w", path: "a.ts", base: null, binary: false, too_large: false };
  apply({ type: "file", ...answer, content: "a\n", version: "v", error: null });
  expect(openFile).toHaveBeenCalledTimes(1); // A new buffer is no reason.
  const at = { worktree: "/w", path: "a.ts" };
  apply({ type: "save_failed", ...at, error: "io", message: "disk full" });
  expect(openFile).toHaveBeenCalledTimes(1);
  apply({ type: "save_failed", ...at, error: "conflict", message: "a.ts changed on disk" });
  expect(openFile).toHaveBeenCalledTimes(2);
  stop();
});

test("tells the service the terminal in view and the window focus, when either changes", () => {
  const setView = mock(async (_terminal: number | null, _focused: boolean) => {});
  addTab(1, "/w");
  const stop = followView({ setView } as unknown as Transport);
  setFocused(true);
  expect(setView).not.toHaveBeenCalled(); // Not connected yet.
  welcome();
  expect(setView.mock.calls).toEqual([[1, true]]);
  // Other changes do not send it again.
  setRightPanel("files");
  select("/w");
  expect(setView).toHaveBeenCalledTimes(1);
  setFocused(false);
  addTab(2, "/w");
  useHive.setState({ fileShown: true });
  expect(setView.mock.calls.slice(1)).toEqual([
    [1, false],
    [2, false],
    [null, false],
  ]);
  useHive.setState({ fileShown: false });
  expect(setView.mock.calls.at(-1)).toEqual([2, false]);
  useHive.setState({ fileShown: true });
  // A new service is told again once connected.
  apply({ type: "disconnected", reason: "gone", bundled: false });
  setFocused(true);
  expect(setView).toHaveBeenCalledTimes(6);
  welcome();
  expect(setView.mock.calls.at(-1)).toEqual([null, true]);
  stop();
  setFocused(false);
  expect(setView).toHaveBeenCalledTimes(7);
});
