import { afterEach, expect, mock, test } from "bun:test";
import { apply, initialState, setOpenFile, useHive } from "../store";
import type { Transport } from "../transport";
import { followOpenFile } from "./follow";

afterEach(() => useHive.setState(initialState, true));

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
