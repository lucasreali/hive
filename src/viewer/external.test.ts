import { afterEach, expect, test } from "bun:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { asMac } from "../../test/mac";
import { initialState, setOpenFile, useHive } from "../store";
import { openExternal, openFolder } from "./external";

afterEach(() => {
  clearMocks();
  useHive.setState(initialState, true);
});

const target = (windows_path: string | null, error: string | null = null, path = "a.ts") => ({
  type: "editor_target" as const,
  worktree: "/w",
  path,
  windows_path,
  error,
});
const notice = () => useHive.getState().editorNotice;
const unc = "\\\\wsl.localhost\\Ubuntu\\w\\a.ts";

test("the Windows path opens with its default app inside Tauri", async () => {
  const calls: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
  });
  setOpenFile({ worktree: "/w", path: "a.ts" });
  await openExternal(target(unc), true);
  expect(calls).toEqual([["plugin:opener|open_path", { path: unc }]]);
  expect(notice()).toBeNull();

  mockIPC(() => {
    throw new Error("no default app");
  });
  await openExternal(target(unc), true);
  expect(notice()).toContain("no default app");
});

test("the service's refusal, the browser, and a file no longer open", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  await openExternal(target(unc), true); // Nothing is open.
  setOpenFile({ worktree: "/w", path: "a.ts" });
  await openExternal(target(unc, null, "b.ts"), true);
  expect([calls, notice()]).toEqual([[], null]);

  await openExternal(target(null, "Windows would run a .bat file"), true);
  expect(notice()).toBe("Windows would run a .bat file");
  await openExternal(target(unc), false);
  expect(notice()).toBe(`Only the Hive app opens an external editor: ${unc}`);
  await openExternal(target(unc));
  expect(calls).toEqual([]);
});

test("a worktree's folder opens in the Explorer, or the status bar says why not", async () => {
  const calls: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
  });
  const folder = "\\\\wsl.localhost\\Ubuntu\\w\\";
  await openFolder(target(folder, null, ""), true);
  expect(calls).toEqual([["plugin:opener|open_path", { path: folder }]]);
  const status = () => useHive.getState().notice;
  expect(status()).toBeNull();
  await openFolder(target(null, "wslpath failed: x", ""), true);
  expect(status()).toBe("wslpath failed: x");
  await openFolder(target(folder, null, ""), false);
  expect(status()).toBe(`Only the Hive app opens the Explorer: ${folder}`);
  mockIPC(() => {
    throw new Error("no Explorer");
  });
  await openFolder(target(folder, null, ""), true);
  expect(status()).toContain("no Explorer");
  await openFolder(target(folder, null, ""));
});

test("on macOS the folder opens in the Finder", async () => {
  asMac();
  await openFolder(target("/Users/you/w", null, ""), false);
  expect(useHive.getState().notice).toBe("Only the Hive app opens the Finder: /Users/you/w");
});
