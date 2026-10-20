import { afterEach, beforeEach, expect, test } from "bun:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import type { AgentState } from "./protocol";
import { apply } from "./reduce";
import { DEFAULT_SETTINGS, initialState, useHive } from "./store";
import { agentStatus } from "./transport/mock";
import { tauriTransport } from "./transport/tauri";
import { followTray, type Tray, trayOf, usageLines } from "./tray";

beforeEach(() => useHive.setState(initialState, true));
afterEach(() => {
  clearMocks();
  // The next test file starts from a clean store too.
  useHive.setState(initialState, true);
});

const detect = (id: string, channel: number) =>
  apply({ type: "agent_detected", channel, id, project: null, worktree: null, cwd: null });
const state = (id: string, s: AgentState, pending?: boolean) => {
  const status = agentStatus(s);
  apply({ type: "agent_state", id, ...status, pending: pending ?? status.pending, subagents: [] });
};
// 14:30 local time on a Monday, and the next Monday 09:00.
const resets = new Date(2026, 9, 19, 14, 30).getTime() / 1000;
const week = new Date(2026, 9, 26, 9, 0).getTime() / 1000;

test("the usage ring's lines: the session, then the week when sent; none without a session", () => {
  expect(usageLines(null, { used_percentage: 41, resets_at: week })).toEqual([]);
  expect(usageLines({ used_percentage: 42, resets_at: resets }, null)).toEqual([
    "Session 42% · resets 14:30",
  ]);
  expect(
    usageLines(
      { used_percentage: 42, resets_at: resets },
      { used_percentage: 41, resets_at: week },
    ),
  ).toEqual(["Session 42% · resets 14:30", "Week 41% · resets Mon 09:00"]);
});

test("nothing to show without agents, but the icon stays while the setting is on", () => {
  expect(trayOf(useHive.getState())).toEqual({ shown: true, urgent: null, agents: [], usage: [] });
});

test("the tray lists the pending agents in tree order with the most urgent state", () => {
  for (const [i, id] of ["work", "ask", "seen", "done"].entries()) detect(id, i + 1);
  state("work", "working");
  state("ask", "waiting_permission");
  state("seen", "waiting_you", false); // 15.6: looked at, no longer pending
  state("done", "waiting_you");
  apply({ type: "agent_title", channel: 2, id: "ask", title: "fix login" });
  apply({ type: "session_usage", usage: { used_percentage: 42, resets_at: resets }, week: null });
  expect(trayOf(useHive.getState())).toEqual({
    shown: true,
    urgent: ["waiting_permission", true],
    agents: [
      ["ask", "fix login", "waiting_permission", null],
      ["done", "Claude", "waiting_you", null],
    ],
    usage: ["Session 42% · resets 14:30"],
  });
});

test("a seen 🟠 is the most urgent but not pending; the setting off hides the icon", () => {
  detect("seen", 1);
  state("seen", "waiting_you", false);
  const off = { ...DEFAULT_SETTINGS.notifications, menu_bar: false };
  useHive.setState({ settings: { ...DEFAULT_SETTINGS, notifications: off } });
  expect(trayOf(useHive.getState())).toMatchObject({
    shown: false,
    urgent: ["waiting_you", false],
    agents: [],
  });
});

test("each pending agent names its space when there are several (6.14)", () => {
  apply({
    type: "spaces",
    current: "w",
    spaces: [
      { id: "default", name: "Default", projects: [], env: {} },
      { id: "w", name: "Work", projects: ["/r"], env: {} },
    ],
  } as never);
  apply({ type: "agent_detected", channel: 1, id: "a", project: "/r", worktree: null, cwd: null });
  state("a", "error");
  expect(trayOf(useHive.getState()).agents).toEqual([["a", "Claude", "error", "Work"]]);
});

test("the tray follows the store on macOS only, sent when it changes; a failure is dropped", async () => {
  const sent: Tray[] = [];
  const send = async (tray: Tray) => {
    sent.push(tray);
  };
  followTray(send, false)();
  detect("a", 1);
  expect(sent).toEqual([]);
  const stop = followTray(send, true);
  expect(sent).toHaveLength(1);
  expect(sent[0]?.urgent).toBeNull();
  // Another change of the store with the same tray sends nothing.
  useHive.setState({ focused: !useHive.getState().focused });
  expect(sent).toHaveLength(1);
  state("a", "error");
  expect(sent.map((t) => t.agents.length)).toEqual([0, 1]);
  stop();
  state("a", "working");
  expect(sent).toHaveLength(2);
  // A refusing app side changes nothing.
  const refused = followTray(() => Promise.reject(new Error("no status bar")), true);
  await Promise.resolve();
  refused();
});

test("the tray reaches the app side's set_tray command", async () => {
  const calls: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    return null;
  });
  const tray: Tray = {
    shown: true,
    urgent: ["error", true],
    agents: [["a", "Claude", "error", null]],
    usage: ["Session 42% · resets 14:30"],
  };
  await tauriTransport.setTray(tray);
  expect(calls).toEqual([["set_tray", tray]]);
});
