import { beforeEach, expect, mock, test } from "bun:test";
import { followOpenFile, followPanel } from "./follow";
import {
  apply,
  initialState,
  select,
  setOpenFile,
  setRightPanel,
  setSidebarView,
  useHive,
} from "./store";
import type { Transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

beforeEach(() => useHive.setState(initialState, true));

function recorder() {
  const calls: (string | null)[] = [];
  const transport = {
    watchWorktree: async (path: string) => void calls.push(path),
    unwatchWorktree: async () => void calls.push(null),
  } as Transport;
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
  apply({ type: "files", path: shop.id, files: [], truncated: false });
  select(api.id);
  setRightPanel(null);
  expect(calls).toEqual([shop.id, api.id, null]);
  setRightPanel("files");
  expect(calls).toEqual([shop.id, api.id, null, api.id]);
  // A new connection has no watch: nothing to stop, and it is sent again.
  apply({ type: "disconnected", reason: "gone" });
  apply({ type: "welcome", version: "0.1.0", distro: null });
  expect(calls).toEqual([shop.id, api.id, null, api.id, api.id]);
  // The sidebar's Files watches too, with the Changes panel closed.
  setRightPanel(null);
  setSidebarView("files");
  setSidebarView("worktrees");
  expect(calls.slice(5)).toEqual([null, api.id, null]);
  stop();
  select(shop.id);
  expect(calls).toHaveLength(8);
});

const welcome = () => apply({ type: "welcome", version: "1", distro: null });
const changes = (path: string) =>
  apply({ type: "changes", path, files: [], added: 0, removed: 0, error: null });

test("asks for the open file when it opens, its worktree changes, or the service is new", () => {
  const openFile = mock(async (_worktree: string, _path: string) => {});
  const stop = followOpenFile({ openFile } as unknown as Transport);
  setOpenFile({ worktree: "/w", path: "a.ts" });
  expect(openFile).not.toHaveBeenCalled(); // Not connected yet.

  welcome();
  expect(openFile.mock.calls).toEqual([["/w", "a.ts"]]);
  changes("/other");
  apply({ type: "agent_removed", channel: 1, id: "x" });
  expect(openFile).toHaveBeenCalledTimes(1);
  changes("/w");
  setOpenFile({ worktree: "/w", path: "b.ts" });
  welcome();
  expect(openFile.mock.calls.slice(1)).toEqual([
    ["/w", "a.ts"],
    ["/w", "b.ts"],
    ["/w", "b.ts"],
  ]);

  setOpenFile(null);
  apply({ type: "disconnected", reason: "gone" });
  stop();
  setOpenFile({ worktree: "/w", path: "c.ts" });
  expect(openFile).toHaveBeenCalledTimes(4);
});

test("asks again when a save found a newer version on disk", () => {
  const openFile = mock(async (_worktree: string, _path: string) => {});
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
