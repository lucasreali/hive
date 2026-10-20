import { afterEach, beforeEach, expect, test } from "bun:test";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { asMac } from "../test/mac";
import { bounceFor, followDock, shouldBounce } from "./dock";
import type { AgentState, Alert, ServiceMessage } from "./protocol";
import { apply } from "./reduce";
import { DEFAULT_SETTINGS, type HiveState, initialState, markInboxRead, useHive } from "./store";
import { agentStatus } from "./transport/mock";

const g = globalThis as { isTauri?: boolean };

beforeEach(() => useHive.setState(initialState, true));
afterEach(() => {
  clearMocks();
  delete g.isTauri;
  // The next test file starts from a clean store too.
  useHive.setState(initialState, true);
});

const message = (state: AgentState, alert: Alert | null = "action", pending?: boolean) => {
  const status = agentStatus(state, null, 0, alert);
  return {
    type: "agent_state" as const,
    id: "a",
    ...status,
    pending: pending ?? status.pending,
    subagents: [],
  };
};
const at = (focused: boolean, bounce = true): HiveState => ({
  ...useHive.getState(),
  focused,
  settings: { ...DEFAULT_SETTINGS, notifications: { volume: 100, bounce, menu_bar: true } },
});

test("a new 🟡 or 🔴 bounces only while Hive is not focused", () => {
  for (const s of ["waiting_permission", "waiting_plan", "waiting_answer", "error"] as const) {
    expect(shouldBounce(message(s), at(false))).toBe(true);
    expect(shouldBounce(message(s), at(true))).toBe(false);
  }
});

test("a 🟠, seen or not, never bounces: only the service's action alert does", () => {
  expect(shouldBounce(message("waiting_you", "finished"), at(false))).toBe(false);
  expect(shouldBounce(message("waiting_you", "waiting"), at(false))).toBe(false);
  expect(shouldBounce(message("waiting_you", "finished", false), at(false))).toBe(false);
  // The service's classification is read as it is: no table of states here.
  expect(shouldBounce(message("error", "waiting"), at(false))).toBe(false);
});

test("no bounce without the service's alert, when not pending, or with the setting off", () => {
  // The snapshot after `welcome` carries no alert.
  expect(shouldBounce(message("error", null), at(false))).toBe(false);
  expect(shouldBounce(message("error", "action", false), at(false))).toBe(false);
  expect(shouldBounce(message("error"), at(false, false))).toBe(false);
  const other: ServiceMessage = { type: "agent_title", channel: 1, id: "a", title: "x" };
  expect(shouldBounce(other, at(false))).toBe(false);
});

test("bounceFor asks the macOS app's window for one informational bounce", () => {
  g.isTauri = true;
  mockWindows("main");
  asMac();
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, (args as { value?: unknown }).value]);
  });
  useHive.setState({ focused: true });
  bounceFor(message("error"));
  expect(calls).toEqual([]);
  useHive.setState({ focused: false });
  bounceFor(message("error"));
  expect(calls).toEqual([["plugin:window|request_user_attention", { type: "Informational" }]]);
});

test("the Dock badge follows the bell's count, sent only when it changes", () => {
  const badges: number[] = [];
  const stop = followDock((n) => badges.push(n));
  const detect = (id: string, channel: number) =>
    apply({ type: "agent_detected", channel, id, project: null, worktree: null, cwd: null });
  const state = (id: string, s: AgentState, pending?: boolean) =>
    apply({ ...message(s, null, pending), id });
  detect("a", 1);
  detect("b", 2);
  state("a", "working");
  expect(badges).toEqual([]);
  state("a", "waiting_permission");
  state("b", "waiting_you");
  expect(badges).toEqual([1, 2]);
  // 15.6: a seen 🟠 leaves the count.
  state("b", "waiting_you", false);
  expect(badges).toEqual([1, 2, 1]);
  // Opening the bell clears it, as it clears the bell's.
  markInboxRead();
  expect(badges).toEqual([1, 2, 1, 0]);
  stop();
  state("b", "error");
  expect(badges).toEqual([1, 2, 1, 0]);
});
