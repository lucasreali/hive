import { afterEach, beforeEach, expect, test } from "bun:test";
import { alertText, notify, TONE_GAP_MS } from "./notify";
import type { AgentState, Alert } from "./protocol";
import { apply } from "./reduce";
import {
  addToInbox,
  DEFAULT_SETTINGS,
  type HiveState,
  INBOX_LIMIT,
  initialState,
  useHive,
} from "./store";
import { agentStatus } from "./transport/mock";

const g = globalThis as Record<string, unknown>;
let tones = 0;
/** The gain each tone started at. */
let gains: number[] = [];
let shown: { title: string; body?: string }[] = [];
// Advances past the rate limit between tests; each test moves its own clock from here.
let clock = 1_000_000;

class FakeAudio {
  currentTime = 0;
  destination = {};
  resume = () => Promise.resolve();
  createGain = () => ({
    gain: { setValueAtTime: (v: number) => gains.push(v), exponentialRampToValueAtTime() {} },
    connect: (d: unknown) => d,
  });
  createOscillator = () => ({
    frequency: { value: 0 },
    connect: (n: unknown) => n,
    start: () => tones++,
    stop() {},
  });
}

class FakeNotification {
  static permission = "granted";
  constructor(title: string, options: { body?: string }) {
    shown.push({ title, body: options.body });
  }
}

beforeEach(() => {
  tones = 0;
  gains = [];
  shown = [];
  clock += 10 * TONE_GAP_MS;
  g.AudioContext = FakeAudio;
  g.isTauri = true;
  (window as unknown as Record<string, unknown>).Notification = FakeNotification;
});

afterEach(() => {
  useHive.setState(initialState, true);
  delete g.isTauri;
  delete (window as unknown as Record<string, unknown>).Notification;
});

const state = (id: string, s: AgentState, alert: Alert | null = null) => ({
  type: "agent_state" as const,
  id,
  ...agentStatus(s, null, 0, alert),
  subagents: [],
});

/** Feeds a message the way main.tsx does, at `clock + at`. */
function feed(id: string, s: AgentState, alert: Alert | null, at = 0) {
  const m = state(id, s, alert);
  notify(m, useHive.getState(), clock + at);
  apply(m);
}

const settle = () => new Promise((r) => setTimeout(r, 0));

// Which change alerts is the service's (`states::Agent`, tested there); here, only what each
// alert shows.

test("a tone for each alert; a message without one raises nothing", async () => {
  for (const s of ["waiting_plan", "waiting_answer", "waiting_permission", "error"] as const) {
    feed("a", s, "waiting", tones * TONE_GAP_MS * 2);
  }
  expect(tones).toBe(4);
  // An agent's first state, a state that needs nobody, the snapshot after welcome, an interrupt.
  feed("a", "working", null, 10 * TONE_GAP_MS);
  feed("b", "waiting_you", null, 20 * TONE_GAP_MS);
  const interrupted = { ...state("a", "waiting_you"), pending: false, interrupted: true };
  notify(interrupted, useHive.getState(), clock + 30 * TONE_GAP_MS);
  await settle();
  // Only the four alerts notify.
  expect([tones, useHive.getState().inbox.length, shown.length]).toEqual([4, 4, 4]);
});

test("the tone follows the volume setting; 0 plays none", () => {
  feed("a", "error", "waiting");
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications.volume = 40;
  apply({ type: "settings", settings });
  feed("a", "error", "waiting", TONE_GAP_MS);
  settings.notifications.volume = 0;
  apply({ type: "settings", settings: structuredClone(settings) });
  feed("a", "error", "waiting", 2 * TONE_GAP_MS);
  expect([tones, gains]).toEqual([2, [0.15, 0.06]]);
});

test("several agents changing at once play one tone", () => {
  feed("a", "waiting_you", "finished");
  feed("b", "error", "waiting", 10);
  feed("c", "waiting_permission", "waiting", TONE_GAP_MS - 1);
  expect(tones).toBe(1);
  feed("a", "error", "waiting", TONE_GAP_MS);
  expect(tones).toBe(2);
});

test("every alert the service marks notifies with the agent's name, what happened and where", async () => {
  useHive.setState({
    projects: {
      p: {
        id: "p",
        name: "shop",
        path: "/r/shop",
        error: null,
        worktrees: [
          {
            id: "w",
            name: "feat",
            path: "/r/shop/w",
            branch: null,
            main: false,
            claude: true,
            status: null,
          },
        ],
      },
    },
  });
  apply({ type: "agent_detected", channel: 1, id: "a", project: "p", worktree: "w", cwd: null });
  apply({ type: "agent_detected", channel: 2, id: "b", project: "p", worktree: null, cwd: null });
  apply({ type: "agent_title", channel: 1, id: "a", title: "fix login" });
  feed("a", "waiting_you", "finished");
  feed("a", "waiting_permission", "waiting");
  feed("b", "waiting_plan", "waiting");
  feed("b", "waiting_answer", "waiting");
  feed("c", "error", "waiting");
  // Waiting for you without having finished (e.g. from idle).
  feed("c", "waiting_you", "waiting");
  await settle();
  expect(shown).toEqual([
    { title: "fix login finished", body: "shop · feat" },
    { title: "fix login is waiting for permission", body: "shop · feat" },
    { title: "Claude is waiting for plan approval", body: "shop" },
    { title: "Claude is waiting for your answer", body: "shop" },
    { title: "Claude failed", body: "" },
    { title: "Claude is waiting for you", body: "" },
  ]);
  const message = state("a", "waiting_you", "finished");
  expect(alertText(useHive.getState(), message)).toEqual({
    agent: "a",
    title: "fix login finished",
    body: "shop · feat",
  });
});

test("an alert the service does not mark (in view of the focused window) is not notified", async () => {
  for (const s of ["waiting_you", "waiting_permission"] as const) {
    const watched = { ...state("a", s, "waiting"), notify: false };
    notify(watched, useHive.getState(), clock + tones * 2 * TONE_GAP_MS);
  }
  await settle();
  expect([tones, useHive.getState().inbox.length, shown]).toEqual([2, 2, []]);
});

test("other messages are ignored", () => {
  notify({ type: "welcome", version: "1", distro: null }, useHive.getState() as HiveState, clock);
  expect(tones).toBe(0);
});

test("every alert is kept in the inbox, the newest first, at most INBOX_LIMIT", () => {
  apply({ type: "agent_title", channel: 1, id: "a", title: "fix login" });
  feed("a", "waiting_you", "finished");
  feed("a", "waiting_permission", "waiting");
  feed("b", "waiting_you", "waiting");
  feed("b", "error", "waiting");
  // Muted: no tone, but still an alert.
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications.volume = 0;
  apply({ type: "settings", settings });
  notify(state("b", "waiting_permission", "waiting"), useHive.getState(), clock, 1234);
  const { inbox } = useHive.getState();
  expect(inbox.map((i) => [i.id, i.agent, i.state, i.text])).toEqual([
    [5, "b", "waiting_permission", "Claude is waiting for permission"],
    [4, "b", "error", "Claude failed"],
    [3, "b", "waiting_you", "Claude is waiting for you"],
    [2, "a", "waiting_permission", "fix login is waiting for permission"],
    [1, "a", "waiting_you", "fix login finished"],
  ]);
  expect(inbox[0]?.at).toBe(1234);
  feed("a", "waiting_plan", "waiting");
  feed("a", "waiting_answer", "waiting");
  const texts = useHive.getState().inbox.map((i) => i.text);
  expect(texts.slice(0, 2)).toEqual([
    "fix login is waiting for your answer",
    "fix login is waiting for plan approval",
  ]);

  useHive.setState({ inbox: [] });
  for (let at = 0; at < INBOX_LIMIT + 5; at++) {
    addToInbox({ agent: "a", state: "error", at, text: "" });
  }
  const kept = useHive.getState().inbox;
  expect([kept.length, kept[0]?.at, kept.at(-1)?.at]).toEqual([INBOX_LIMIT, INBOX_LIMIT + 4, 5]);
});
