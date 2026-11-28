import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import type { Space } from "../protocol";
import { apply } from "../reduce";
import { addTab, endRestore, initialState, resumeAtLaunch, useHive } from "../store";
import { placeKey, shownPane } from "../tabs";
import { MOCK_REPOS } from "../transport/mock";

// 28.27 review: a press or a key in the terminal area is the user's, even on the tab the restore
// showed: the launch restore ends, and the last run's pick named afterwards takes no focus.

const [shop] = MOCK_REPOS;
const [, fixLogin] = shop.worktrees;
const W = fixLogin.path;
const NO_ENV = { git_name: null, git_email: null, gh_account: null, claude_account: null };
const home: Space = { id: "default", name: "Home", projects: [shop.id], env: NO_ENV };
const s = () => useHive.getState();
const detected = (channel: number, id: string) =>
  apply({ type: "agent_detected", channel, id, project: shop.id, worktree: W, cwd: W });

afterEach(() => {
  cleanup();
  endRestore();
  useHive.setState(initialState, true);
});

/** The launch of 28.27's review: s1 and s2 named, s3 (the last run's pick) not yet; 8 shown. */
function launch() {
  const tabHistory = { [placeKey({ host: null }, W)]: ["session:s1", "session:s2", "session:s3"] };
  const spacePlaces = { default: { place: fixLogin.id, project: shop.id } };
  render(<App />);
  act(() => {
    useHive.setState(initialState, true);
    useHive.setState({ tabHistory, lastRunHistory: tabHistory, spacePlaces, restoring: true });
    apply({ type: "projects", projects: [shop] });
    apply({ type: "spaces", spaces: [home], current: "default" });
    resumeAtLaunch(["s1", "s2", "s3"]);
    for (const id of [7, 8, 9]) addTab(id, W, undefined, false);
    detected(7, "s1");
    detected(8, "s2");
  });
  expect([shownPane(s()), s().restoring]).toEqual([8, true]);
}

test("a press in the terminal area ends the restore: the pick named afterwards stays behind", () => {
  launch();
  fireEvent.pointerDown(screen.getByRole("region", { name: "Terminals" }));
  expect(s().restoring).toBe(false);
  act(() => detected(9, "s3"));
  expect(shownPane(s())).toBe(8);
});

test("a key typed in the terminal area ends the restore too", () => {
  launch();
  fireEvent.keyDown(screen.getByRole("region", { name: "Terminals" }), { key: "a" });
  expect(s().restoring).toBe(false);
  act(() => detected(9, "s3"));
  expect(shownPane(s())).toBe(8);
});
