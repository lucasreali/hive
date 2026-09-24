import { afterEach, beforeEach, expect, test } from "bun:test";
import { notify, TONE_GAP_MS } from "./notify";
import { type AgentState, apply, type HiveState, initialState, useHive } from "./store";

const g = globalThis as Record<string, unknown>;
let tones = 0;
let shown: { title: string; body?: string }[] = [];
// Advances past the rate limit between tests; each test moves its own clock from here.
let clock = 1_000_000;

class FakeAudio {
  currentTime = 0;
  destination = {};
  resume = () => Promise.resolve();
  createGain = () => ({
    gain: { setValueAtTime() {}, exponentialRampToValueAtTime() {} },
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

const state = (id: string, s: AgentState) => ({
  type: "agent_state" as const,
  id,
  state: s,
  subagents: [],
});

/** Feeds a message the way main.tsx does, at `clock + at`. */
function feed(id: string, s: AgentState, at = 0) {
  const m = state(id, s);
  notify(m, useHive.getState(), clock + at);
  apply(m);
}

const settle = () => new Promise((r) => setTimeout(r, 0));

test("a tone only when entering an alerting state, not while staying in it", () => {
  feed("a", "working");
  for (const s of ["waiting_permission", "error"] as const) {
    feed("a", s, tones * TONE_GAP_MS * 2);
  }
  expect(tones).toBe(2);
  feed("a", "error", 10 * TONE_GAP_MS);
  feed("a", "idle", 20 * TONE_GAP_MS);
  feed("a", "with_subagents", 30 * TONE_GAP_MS);
  feed("a", "ended", 40 * TONE_GAP_MS);
  expect(tones).toBe(2);
});

test("an agent's first state and the snapshot after welcome stay silent", async () => {
  apply({ type: "welcome", version: "1", distro: null });
  feed("a", "waiting_you");
  feed("b", "error");
  feed("c", "waiting_permission");
  // A reconnect: disconnected clears the store, so the new snapshot is silent too.
  apply({ type: "disconnected", reason: "gone" });
  apply({ type: "welcome", version: "1", distro: null });
  feed("a", "error", TONE_GAP_MS * 2);
  await settle();
  expect(tones).toBe(0);
  expect(shown).toEqual([]);
});

test("several agents changing at once play one tone", () => {
  for (const id of ["a", "b", "c"]) feed(id, "working");
  feed("a", "waiting_you");
  feed("b", "error", 10);
  feed("c", "waiting_permission", TONE_GAP_MS - 1);
  expect(tones).toBe(1);
  feed("a", "working", TONE_GAP_MS);
  feed("a", "error", TONE_GAP_MS);
  expect(tones).toBe(2);
});

test("finishing (working or with subagents → waiting for you) notifies with the place", async () => {
  useHive.setState({
    projects: {
      p: {
        id: "p",
        name: "shop",
        path: "/r/shop",
        error: null,
        worktrees: [
          { id: "w", name: "feat", path: "/r/shop/w", branch: null, main: false, claude: true },
        ],
      },
    },
  });
  apply({ type: "agent_detected", channel: 1, id: "a", project: "p", worktree: "w", cwd: null });
  apply({ type: "agent_detected", channel: 2, id: "b", project: "p", worktree: null, cwd: null });
  feed("a", "working");
  feed("a", "waiting_you");
  feed("b", "with_subagents");
  feed("b", "waiting_you");
  feed("c", "working");
  feed("c", "waiting_you");
  // Not a finish: from idle, or into another alerting state.
  feed("a", "idle");
  feed("a", "waiting_you");
  feed("b", "working");
  feed("b", "waiting_permission");
  await settle();
  expect(shown).toEqual([
    { title: "Agent finished", body: "shop · feat: waiting for you" },
    { title: "Agent finished", body: "shop: waiting for you" },
    { title: "Agent finished", body: "Waiting for you" },
  ]);
});

test("other messages are ignored", () => {
  notify({ type: "welcome", version: "1", distro: null }, useHive.getState() as HiveState, clock);
  expect(tones).toBe(0);
});
