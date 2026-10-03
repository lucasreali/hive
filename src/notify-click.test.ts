import { afterAll, expect, test } from "bun:test";
import { connect } from "./connect";
import { DEFAULT_SETTINGS, initialState, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 13.5: a click on an OS notification goes to its agent, as the mock transport sends the click
// (`notification-click` typed in a terminal stands in for it).

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};
const type = (id: number, line: string) => transport.writeTerminal(id, `${line}\r`);
const NO_ENV = { git_name: null, git_email: null, gh_config_dir: null, gh_account: null };

afterAll(() => useHive.setState(initialState, true));

test("clicking an agent's notification goes to it, in its space; a gone agent changes nothing", async () => {
  await connect();
  // Muted: the test has no Web Audio.
  await transport.setSettings({ ...DEFAULT_SETTINGS, notifications: { volume: 0 } });
  await settle();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const agent = await transport.openTerminal(shop.path, 80, 24, () => {});
  const shell = await transport.openTerminal(shop.path, 80, 24, () => {});
  // The agent finishes: the service notifies (every mock alert does), so the app shows one.
  for (const line of ["claude", "state working", "state waiting_you"]) await type(agent, line);
  await settle();
  const id = `mock-session-${agent}`;
  expect(useHive.getState().agents[id]).toBeDefined();
  // The user goes to another space, then clicks the notification.
  await transport.createSpace("Work", NO_ENV);
  await settle();
  expect([useHive.getState().currentSpace, useHive.getState().selection]).not.toContain(id);
  await type(shell, "notification-click");
  await settle();
  expect([useHive.getState().currentSpace, useHive.getState().selection]).toEqual(["default", id]);
  // The agent is gone: the click (the app side brought the window up) selects nothing else.
  await type(agent, "exit");
  await settle();
  useHive.setState({ selection: shop.id });
  await type(shell, "notification-click");
  await settle();
  expect(useHive.getState().agents[id]).toBeUndefined();
  expect(useHive.getState().selection).toBe(shop.id);
});
