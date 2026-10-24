import { afterAll, expect, spyOn, test } from "bun:test";
import type { Terminal } from "@xterm/xterm";
import { notice } from "../test/notice";
import { connect } from "./connect";
import type { ServiceMessage } from "./protocol";
import { initialState, useHive } from "./store";
import { closeTerminal, terminal } from "./terminals";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 18.21: after a WebContent crash the app keeps the terminals and tells the reloaded page about
// each one (`terminal_kept`), which reattaches to it.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};
const screen = (term: Terminal) =>
  new Promise<string>((resolve) =>
    term.write("", () => resolve(term.buffer.active.getLine(0)?.translateToString(true) ?? "")),
  );

afterAll(() => useHive.setState(initialState, true));

test("a kept terminal gets its tab and its output back in the reloaded page", async () => {
  await connect();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  // Opened by the page that crashed: its output went there.
  const kept = await transport.openTerminal(shop.path, 80, 24, () => {});
  await settle();
  useHive.setState(initialState, true);
  let onMessage: (message: ServiceMessage) => void = () => {};
  const connected = spyOn(transport, "connect").mockImplementation(async (handler) => {
    onMessage = handler;
  });
  await connect();
  connected.mockRestore();
  const resized = spyOn(transport, "resizeTerminal");

  onMessage({ type: "terminal_kept", channel: kept, cwd: shop.path, worktree: shop.path });
  await settle();
  const s = useHive.getState();
  expect(s.tabs).toEqual([{ id: kept, cwd: shop.path }]);
  expect(s.activeTab).toBe(kept);
  expect(s.terminals[kept]).toMatchObject({ exited: false, worktree: shop.path });
  // Resized from 80×24, so a full-screen program draws again once the pane is fitted.
  expect(resized).toHaveBeenCalledWith(kept, 80, 24);
  resized.mockRestore();
  // Its output reaches the new Terminal (the mock echoes what is typed).
  await transport.writeTerminal(kept, "x");
  await settle();
  expect(await screen(terminal(kept) as Terminal)).toContain("x");

  // One the app no longer has is shown as a failure.
  onMessage({ type: "terminal_kept", channel: 999, cwd: shop.path, worktree: null });
  await settle();
  expect(notice()).toContain("Cannot open a terminal: Error: no such terminal");
  expect(terminal(999)).toBeUndefined();
  closeTerminal(kept);
});
