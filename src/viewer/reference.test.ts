import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { type HiveState, initialState, useHive } from "../store";
import { transport } from "../transport";
import { reference, referenceTarget, sendReference } from "./reference";

afterEach(() => {
  mock.restore();
  useHive.setState(initialState, true);
});

test("a reference names the path and its lines, with a trailing space", () => {
  expect(reference("src/checkout/validators.ts", { from: 44, to: 46 })).toBe(
    "@src/checkout/validators.ts (lines 44–46) ",
  );
  expect(reference("a.ts", { from: 7, to: 7 })).toBe("@a.ts (line 7) ");
  expect(reference("docs/my notes.md", { from: 1, to: 2 })).toBe(
    '@"docs/my notes.md" (lines 1–2) ',
  );
});

const wt = "/home/u/shop";
const ready: Partial<HiveState> = {
  openFile: { worktree: wt, path: "src/a.ts" },
  selectedLines: { from: 3, to: 5 },
  tabs: [
    { id: 1, cwd: "/home/u/other" },
    { id: 2, cwd: wt },
  ],
  activeTab: 2,
};
const target = (patch: Partial<HiveState>) =>
  referenceTarget({ ...initialState, ...ready, ...patch });

test("the target is the active terminal of the open file's worktree", () => {
  expect(target({})).toEqual({ terminal: 2, text: "@src/a.ts (lines 3–5) " });
  const agent = { id: "s", terminal: 1, project: null, worktree: wt, cwd: null };
  // An agent placed by the service in the file's worktree counts, wherever its tab opened.
  expect(target({ activeTab: 1, agents: { s: agent } })).toEqual({
    terminal: 1,
    text: "@src/a.ts (lines 3–5) ",
  });
  expect(target({ agents: { s: { ...agent, terminal: 2, worktree: null } } })).toEqual({
    why: "The active terminal is in another worktree",
  });
});

test("without selected lines or a live terminal in that worktree, it says why", () => {
  expect(target({ selectedLines: null })).toEqual({ why: "Select lines to send their reference" });
  expect(target({ openFile: null })).toEqual({ why: "Select lines to send their reference" });
  expect(target({ activeTab: null })).toEqual({ why: "No terminal open" });
  expect(target({ terminals: { 2: { id: 2, exited: true, code: 0, unhooked: false } } })).toEqual({
    why: "The terminal has exited",
  });
  expect(target({ activeTab: 1 })).toEqual({ why: "The active terminal is in another worktree" });
});

test("sending writes the reference as input, without Enter, only when it can", () => {
  const write = spyOn(transport, "writeTerminal").mockImplementation(async () => {});
  sendReference();
  expect(write).not.toHaveBeenCalled();
  useHive.setState({ ...ready, fileShown: true });
  sendReference();
  expect(write).toHaveBeenCalledWith(2, "@src/a.ts (lines 3–5) ");
  expect(useHive.getState().fileShown).toBe(false);
});
