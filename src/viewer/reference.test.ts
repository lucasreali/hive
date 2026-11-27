import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { type HiveState, initialState, useHive } from "../store";
import { transport } from "../transport";
import { noSelection, reference, referenceTarget, sendReference } from "./reference";
import { savePick } from "./target";

afterEach(() => {
  mock.restore();
  window.localStorage.clear();
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
const agent = (id: string, terminal: number) => ({
  id,
  terminal,
  project: wt,
  worktree: wt,
  cwd: wt,
});
// Tab 2, active, is a plain terminal of the worktree (a dev server); 1 and 3 run its agents.
const ready: Partial<HiveState> = {
  openFile: { worktree: wt, path: "src/a.ts" },
  selectedLines: { from: 3, to: 5 },
  tabs: [1, 2, 3].map((id) => ({ id, cwd: wt })),
  activeTab: 2,
  agents: { a: agent("a", 1) },
};
const target = (patch: Partial<HiveState>, chosen?: number) =>
  referenceTarget({ ...initialState, ...ready, ...patch }, chosen);
const text = "@src/a.ts (lines 3–5) ";

test("the target is an agent of the file's worktree, never the active plain terminal", () => {
  expect(target({})).toEqual({ terminal: 1, text });
  const two = { a: agent("a", 1), b: agent("b", 3) };
  expect(target({ agents: two })).toEqual({ terminal: 1, text });
  expect(target({ agents: two }, 3)).toEqual({ terminal: 3, text });
  savePick(wt, "b");
  expect(target({ agents: two })).toEqual({ terminal: 3, text });
});

test("without selected lines or an agent in that worktree, it says why", () => {
  expect(target({ selectedLines: null })).toEqual({ why: "Select lines to send their reference" });
  expect(target({ openFile: null })).toEqual({ why: "Select lines to send their reference" });
  expect(noSelection({ openFile: null, selectedLines: null })).toBe(
    "Select lines to send their reference",
  );
  expect(noSelection({ ...initialState, ...ready })).toBeNull();
  expect(target({ agents: {} })).toEqual({ why: "No agent in this worktree" });
  expect(target({}, 2)).toEqual({ why: "That agent has ended" });
});

test("sending writes the reference as input, without Enter, and shows the agent's tab", () => {
  const write = spyOn(transport, "writeTerminal").mockImplementation(async () => {});
  sendReference();
  expect(write).not.toHaveBeenCalled();
  useHive.setState({ ...ready, fileShown: true });
  sendReference();
  expect(write).toHaveBeenCalledWith(1, text);
  expect(useHive.getState()).toMatchObject({ fileShown: false, activeTab: 1 });
});
