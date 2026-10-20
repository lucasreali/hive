import { afterAll, expect, spyOn, test } from "bun:test";
import { connect } from "./connect";
import { initialState, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 18.1: files dropped from Finder reach the UI as `files_dropped` from the app side; the mock
// transport sends it for `drop-files <x> <y> <text>` typed in a terminal.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};

afterAll(() => useHive.setState(initialState, true));

test("a drop from the app side looks for the terminal at its point", async () => {
  const at = spyOn(document, "elementFromPoint").mockReturnValue(null);
  await connect();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const shell = await transport.openTerminal(shop.path, 80, 24, () => {});
  await transport.writeTerminal(shell, "drop-files 120.5 64 /Users/me/My\\ Shots/a.png \r");
  await settle();
  expect(at.mock.calls).toEqual([[120.5, 64]]);
  // Not a drop: nothing looked for.
  await transport.writeTerminal(shell, "drop-files 1\r");
  await settle();
  expect(at).toHaveBeenCalledTimes(1);
  at.mockRestore();
});
