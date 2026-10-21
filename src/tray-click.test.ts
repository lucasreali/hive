import { afterAll, expect, test } from "bun:test";
import { connect } from "./connect";
import { DEFAULT_SETTINGS, initialState, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";
import { followTray } from "./tray";

// 17.22: choosing a pending agent in the menu bar icon's menu goes to it, as the mock transport
// sends the choice (`tray-click` typed in a terminal stands in for it: the first pending agent).

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};
const type = (id: number, line: string) => transport.writeTerminal(id, `${line}\r`);
const NO_ENV = { git_name: null, git_email: null, gh_config_dir: null, gh_account: null };

let stop = () => {};
afterAll(() => {
  stop();
  useHive.setState(initialState, true);
});

test("choosing a pending agent in the menu bar goes to it, in its space; a gone one changes nothing", async () => {
  await connect();
  stop = followTray(undefined, true);
  // Muted: the test has no Web Audio.
  const notifications = { ...DEFAULT_SETTINGS.notifications, volume: 0 };
  await transport.setSettings({ ...DEFAULT_SETTINGS, notifications });
  await settle();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const agent = await transport.openTerminal(shop.path, 80, 24, () => {});
  const shell = await transport.openTerminal(shop.path, 80, 24, () => {});
  // Nobody pending yet: choosing does nothing.
  await type(shell, "tray-click");
  await settle();
  expect(useHive.getState().selection).toBeNull();
  for (const line of ["claude", "state working", "state waiting_permission"]) {
    await type(agent, line);
  }
  await settle();
  const id = `mock-session-${agent}`;
  expect(useHive.getState().agentStates[id]?.pending).toBe(true);
  // The user goes to another space, then chooses the agent in the menu bar.
  await transport.createSpace("Work", NO_ENV);
  await settle();
  expect([useHive.getState().currentSpace, useHive.getState().selection]).not.toContain(id);
  await type(shell, "tray-click");
  await settle();
  expect([useHive.getState().currentSpace, useHive.getState().selection]).toEqual(["default", id]);
  // The agent is gone: its line leaves the menu, and the choice selects nothing else.
  await type(agent, "exit");
  await settle();
  useHive.setState({ selection: shop.id });
  await type(shell, "tray-click");
  await settle();
  expect(useHive.getState().agents[id]).toBeUndefined();
  expect(useHive.getState().selection).toBe(shop.id);
});
