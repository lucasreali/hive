import { expect, test } from "bun:test";
import type { Agent, AgentStatus, Project } from "../protocol";
import { type HiveState, initialState, type Terminal } from "../store";
import {
  agentLabel,
  agentsIn,
  agentTarget,
  NO_AGENT,
  PICK_LIMIT,
  savedPick,
  savePick,
  terminalIn,
} from "./target";

const wt = "/home/u/shop";
const agent = (id: string, terminal: number, worktree: string | null = wt): Agent => ({
  id,
  terminal,
  project: wt,
  worktree,
  cwd: worktree,
});
const live = (id: number, patch: Partial<Terminal> = {}): Terminal => ({
  id,
  exited: false,
  code: null,
  unhooked: false,
  worktree: wt,
  account: null,
  ...patch,
});
const status = (state: AgentStatus["state"]) => ({ state }) as AgentStatus;

/** A storage holding `value` under the picks' key, recording what is written. */
function memory(value: string | null = null) {
  const store = {
    value,
    getItem: () => store.value,
    setItem: (_: string, v: string) => {
      store.value = v;
    },
  };
  return store;
}
const throwing = {
  getItem: (): string | null => {
    throw new Error("blocked");
  },
  setItem: () => {
    throw new Error("blocked");
  },
};

// Tab 1 a dev server (no agent), active; tabs 2 and 3 agents of the worktree; 4 one elsewhere.
const base: HiveState = {
  ...initialState,
  tabs: [1, 2, 3, 4].map((id) => ({ id, cwd: wt })),
  activeTab: 1,
  terminals: { 1: live(1), 2: live(2), 3: live(3), 4: live(4) },
};
const two = { a: agent("a", 2), b: agent("b", 3), c: agent("c", 4, "/home/u/other") };

test("no agent in the worktree: nothing to send to, the active plain terminal is never chosen", () => {
  expect(agentsIn(base, wt)).toEqual([]);
  expect(agentTarget(base, wt, memory())).toEqual({ why: NO_AGENT });
  expect(terminalIn(base, wt, undefined, memory())).toEqual({ why: NO_AGENT });
  // Even picked in a menu, a terminal no agent runs in is not a target.
  expect(terminalIn({ ...base, agents: { a: two.a } }, wt, 1, memory())).toEqual({
    why: "That agent has ended",
  });
});

test("one agent: it is the target, whatever tab is active", () => {
  const s = { ...base, agents: { a: two.a, c: two.c } };
  expect(agentsIn(s, wt)).toEqual([two.a]);
  expect(terminalIn(s, wt, undefined, memory('{"/home/u/shop":"c"}'))).toEqual({ terminal: 2 });
  expect(terminalIn(s, wt, 2, memory())).toEqual({ terminal: 2 });
});

test("several agents: in the user's order, the one last picked there, else the first", () => {
  const s = { ...base, agents: two, agentOrder: ["b", "a"] };
  expect(agentsIn(s, wt)).toEqual([two.b, two.a]);
  expect(agentTarget(s, wt, memory())).toEqual({ agents: [two.b, two.a], agent: two.b });
  expect(terminalIn(s, wt, undefined, memory('{"/home/u/shop":"a"}'))).toEqual({ terminal: 2 });
  // The one picked has gone: the first again.
  expect(terminalIn(s, wt, undefined, memory('{"/home/u/shop":"gone"}'))).toEqual({
    terminal: 3,
  });
  // A storage that is empty, unreadable or throws only loses the preference.
  for (const storage of [memory("not json"), memory("[1]"), memory("null"), throwing, null]) {
    expect(terminalIn(s, wt, undefined, storage)).toEqual({ terminal: 3 });
  }
  // A terminal picked in the menu wins while its agent lives.
  expect(terminalIn(s, wt, 2, memory())).toEqual({ terminal: 2 });
});

test("an exited, ended, closed or other host's agent is left out", () => {
  const s = { ...base, agents: two };
  expect(
    agentsIn({ ...s, terminals: { ...s.terminals, 2: live(2, { exited: true }) } }, wt),
  ).toEqual([two.b]);
  expect(agentsIn({ ...s, agentStates: { a: status("ended"), b: status("working") } }, wt)).toEqual(
    [two.b],
  );
  expect(agentsIn({ ...s, tabs: s.tabs.filter((t) => t.id !== 3) }, wt)).toEqual([two.a]);
  const remote = s.tabs.map((t) => (t.id === 2 ? { ...t, host: "box" } : t));
  expect(agentsIn({ ...s, tabs: remote }, wt)).toEqual([two.b]);
  // Not yet named by the service, a terminal counts as running.
  expect(agentsIn({ ...s, terminals: {} }, wt)).toEqual([two.a, two.b]);
});

test("the pick is remembered per worktree, the oldest forgotten first", () => {
  const storage = memory();
  expect(savedPick(wt, storage)).toBeNull();
  savePick(wt, "a", storage);
  savePick("/home/u/other", "c", storage);
  savePick(wt, "b", storage);
  expect(JSON.parse(storage.value ?? "")).toEqual({ "/home/u/other": "c", [wt]: "b" });
  expect(savedPick(wt, storage)).toBe("b");
  // Values that are not session ids are dropped.
  const odd = memory('{"/x":1,"/y":"y"}');
  savePick(wt, "a", odd);
  expect(JSON.parse(odd.value ?? "")).toEqual({ "/y": "y", [wt]: "a" });
  for (let i = 0; i < PICK_LIMIT + 5; i++) savePick(`/w${i}`, `s${i}`, storage);
  const kept = Object.keys(JSON.parse(storage.value ?? ""));
  expect(kept).toHaveLength(PICK_LIMIT);
  expect(kept[0]).toBe("/w5");
  expect(savedPick(wt, storage)).toBeNull();
  // A storage that throws, or none, keeps nothing and breaks nothing.
  expect(() => savePick(wt, "a", throwing)).not.toThrow();
  expect(() => savePick(wt, "a", null)).not.toThrow();
  expect(savedPick(wt, throwing)).toBeNull();
  expect(savedPick(wt, null)).toBeNull();
});

test("an agent is listed by its tab's title: the session's name, else the worktree's", () => {
  const project = { id: wt, worktrees: [{ id: wt, path: wt, name: "shop" }] } as unknown as Project;
  const s = { ...base, projects: { [wt]: project } };
  expect(agentLabel({ ...s, agentTitles: { a: "fix the login" } }, two.a)).toBe("fix the login");
  expect(agentLabel(s, two.a)).toBe("shop");
  expect(agentLabel(base, two.a)).toBe("Claude");
});
