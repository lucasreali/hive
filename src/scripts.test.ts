import { afterEach, expect, spyOn, test } from "bun:test";
import type { Script, WorktreeScripts } from "./protocol";
import { apply } from "./reduce";
import { runningScripts, runScript, stopScript, worktreeScripts } from "./scripts";
import { DEFAULT_SETTINGS, initialState, NO_SCRIPTS, removeTab, useHive } from "./store";
import { transport } from "./transport";

afterEach(() => useHive.setState(initialState, true));

const dev: Script = { name: "dev", command: "pnpm run dev", source: "package_json" };
const listed: WorktreeScripts = { path: "/r", scripts: [dev], problems: [], port: 20_000 };

test("a worktree's scripts are the service's list, else the project's run scripts", () => {
  const settings = structuredClone(DEFAULT_SETTINGS);
  const run = [{ name: "seed", command: "./seed.sh" }];
  settings.projects["/r"] = { scripts: { ...NO_SCRIPTS, run } };
  expect(worktreeScripts(listed, settings, "/r")).toEqual([dev]);
  const own = [{ name: "seed", command: "./seed.sh", source: "settings" as const }];
  expect(worktreeScripts(undefined, settings, "/r")).toEqual(own);
  expect(worktreeScripts(undefined, settings, undefined)).toEqual([]);
});

test("a run script opens a terminal, types it, and runs there until stopped or closed", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(7);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  expect(await runScript("/r", dev)).toBe(7);
  expect(open.mock.calls[0]?.[0]).toBe("/r");
  expect(write).toHaveBeenCalledWith(7, "pnpm run dev\r");
  const running = () => runningScripts(useHive.getState(), "/r");
  // The terminal's tab is open: running in its worktree only.
  useHive.setState({ tabs: [{ id: 7, cwd: "/r" }] });
  expect(running()).toEqual([{ id: 7, name: "dev", command: "pnpm run dev" }]);
  expect(runningScripts(useHive.getState(), "/other")).toEqual([]);
  // Stopped: Ctrl+C, and no longer listed (the tab stays).
  stopScript(7);
  expect(write).toHaveBeenLastCalledWith(7, "\x03");
  expect(running()).toEqual([]);
  expect(useHive.getState().tabs).toHaveLength(1);
  open.mockRestore();
  write.mockRestore();
});

test("a script's run ends with its terminal: exited, or its tab closed", () => {
  const run = { worktree: "/r", name: "dev", command: "pnpm run dev" };
  useHive.setState({
    tabs: [
      { id: 1, cwd: "/r" },
      { id: 2, cwd: "/r" },
    ],
    scriptRuns: { 1: run, 2: run, 3: run },
  });
  const ids = () => runningScripts(useHive.getState(), "/r").map((r) => r.id);
  // 3 has no tab.
  expect(ids()).toEqual([1, 2]);
  apply({ type: "terminal_exited", channel: 1, code: 0 });
  expect(ids()).toEqual([2]);
  removeTab(2);
  expect(ids()).toEqual([]);
  expect(Object.keys(useHive.getState().scriptRuns)).toEqual(["1", "3"]);
});
