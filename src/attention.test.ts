import { beforeEach, expect, test } from "bun:test";
import { attention } from "./attention";
import type { AgentState } from "./protocol";
import { apply } from "./reduce";
import { initialState, markInboxRead, useHive } from "./store";
import { agentStatus } from "./transport/mock";

beforeEach(() => useHive.setState(initialState, true));

const detect = (id: string, channel: number) =>
  apply({ type: "agent_detected", channel, id, project: null, worktree: null, cwd: null });
const state = (id: string, s: AgentState, pending?: boolean) => {
  const status = agentStatus(s);
  apply({ type: "agent_state", id, ...status, pending: pending ?? status.pending, subagents: [] });
};

test("nothing needs the user without agents", () => {
  expect(attention(useHive.getState())).toEqual({ pending: [], unseen: 0, urgent: null });
});

test("counts the service's pending agents, not a seen 🟠, and names the most urgent", () => {
  for (const [i, id] of ["work", "ask", "fail", "seen", "done"].entries()) detect(id, i + 1);
  state("work", "working");
  state("ask", "waiting_permission");
  state("fail", "error");
  state("seen", "waiting_you", false); // 15.6: looked at, no longer pending
  state("done", "waiting_you");
  const a = attention(useHive.getState());
  expect(a.pending.map((x) => x.id)).toEqual(["ask", "fail", "done"]);
  expect(a.unseen).toBe(3);
  expect(a.urgent?.state).toBe("waiting_permission");
  // Opening the bell clears its count; the pending list stays until the states change.
  markInboxRead();
  expect(attention(useHive.getState()).unseen).toBe(0);
  expect(attention(useHive.getState()).pending).toHaveLength(3);
  // A new 🟡 counts again.
  state("fail", "waiting_answer");
  expect(attention(useHive.getState()).unseen).toBe(1);
});
