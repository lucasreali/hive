import { afterAll, afterEach, expect, test } from "bun:test";
import { connect } from "./connect";
import { DEFAULT_SETTINGS, initialState, openModal, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 17.7 (D17.3): ⌘Q, the Dock's Quit and logging out reach the UI as `quit_requested` while
// agents would end; the mock transport sends it for `quit-app` typed in a terminal.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};
const type = (id: number, line: string) => transport.writeTerminal(id, `${line}\r`);
const closed = () => document.documentElement.dataset.closed !== undefined;

afterEach(() => {
  openModal(null);
  delete document.documentElement.dataset.closed;
});
afterAll(() => useHive.setState(initialState, true));

test("a quit request asks through the close dialog while an agent works, else quits", async () => {
  await connect();
  // Muted: the test has no Web Audio.
  await transport.setSettings({ ...DEFAULT_SETTINGS, notifications: { ...DEFAULT_SETTINGS.notifications, volume: 0 } });
  await settle();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const shell = await transport.openTerminal(shop.path, 80, 24, () => {});
  // No agent: it quits at once.
  await type(shell, "quit-app");
  await settle();
  expect(useHive.getState().modal).toBeNull();
  expect(closed()).toBe(true);
  delete document.documentElement.dataset.closed;
  // A working agent: the close dialog asks, nothing closes.
  const agent = await transport.openTerminal(shop.path, 80, 24, () => {});
  for (const line of ["claude", "state working"]) await type(agent, line);
  await type(shell, "quit-app");
  await settle();
  expect(useHive.getState().modal).toBe("close-app");
  expect(closed()).toBe(false);
});
