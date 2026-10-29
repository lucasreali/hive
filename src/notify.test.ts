import { afterAll, afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { asMac } from "../test/mac";
import {
  alertText,
  duration,
  goToClicked,
  notify,
  TONE_GAP_MS,
  terminalAlertText,
  terminalName,
} from "./notify";
import type { AgentState, Alert } from "./protocol";
import type { PullState } from "./pulls";
import { apply } from "./reduce";
import {
  addToInbox,
  DEFAULT_SETTINGS,
  type HiveState,
  INBOX_LIMIT,
  initialState,
  useHive,
} from "./store";
import { transport } from "./transport";
import { agentStatus, MOCK_REPOS } from "./transport/mock";

const g = globalThis as Record<string, unknown>;
let tones = 0;
/** The gain each tone started at. */
let gains: number[] = [];
let shown: { title: string; body: string; agent: string }[] = [];
/** The audio context's resumes and suspends, in order. */
let audioCalls: string[] = [];
/** The last tone's oscillator, to end it. */
let lastOsc: { onended: (() => void) | null } | undefined;
// Advances past the rate limit between tests; each test moves its own clock from here.
let clock = 1_000_000;

class FakeAudio {
  currentTime = 0;
  destination = {};
  resume = () => {
    audioCalls.push("resume");
    return Promise.resolve();
  };
  suspend = () => {
    audioCalls.push("suspend");
    return Promise.resolve();
  };
  createGain = () => ({
    gain: { setValueAtTime: (v: number) => gains.push(v), exponentialRampToValueAtTime() {} },
    connect: (d: unknown) => d,
  });
  createOscillator = () => {
    const osc = {
      frequency: { value: 0 },
      onended: null as (() => void) | null,
      connect: (n: unknown) => n,
      start: () => tones++,
      stop() {},
    };
    lastOsc = osc;
    return osc;
  };
}

// The app side shows them (13.5); the test transport would only keep the last agent.
const show = spyOn(transport, "showNotification").mockImplementation(async (title, body, agent) => {
  shown.push({ title, body, agent });
});
afterAll(() => show.mockRestore());

beforeEach(() => {
  tones = 0;
  gains = [];
  shown = [];
  audioCalls = [];
  clock += 10 * TONE_GAP_MS;
  g.AudioContext = FakeAudio;
});

afterEach(() => {
  useHive.setState(initialState, true);
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
    feed("a", s, "action", tones * TONE_GAP_MS * 2);
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
  feed("a", "error", "action");
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications.volume = 40;
  apply({ type: "settings", settings });
  feed("a", "error", "action", TONE_GAP_MS);
  settings.notifications.volume = 0;
  apply({ type: "settings", settings: structuredClone(settings) });
  feed("a", "error", "action", 2 * TONE_GAP_MS);
  expect([tones, gains]).toEqual([2, [0.15, 0.06]]);
});

test("the audio context runs only while a tone plays (19.17)", () => {
  feed("a", "error", "action");
  expect(audioCalls).toEqual(["resume"]);
  lastOsc?.onended?.();
  expect(audioCalls).toEqual(["resume", "suspend"]);
  feed("a", "error", "action", TONE_GAP_MS);
  lastOsc?.onended?.();
  expect(audioCalls).toEqual(["resume", "suspend", "resume", "suspend"]);
});

test("on macOS the app plays the tone; the webview's only when the app cannot (17.19)", async () => {
  asMac();
  const played: number[] = [];
  let refuse = false;
  const play = spyOn(transport, "playTone").mockImplementation(async (volume) => {
    played.push(volume);
    if (refuse) throw new Error("no afplay");
  });
  try {
    const settings = structuredClone(DEFAULT_SETTINGS);
    settings.notifications.volume = 40;
    apply({ type: "settings", settings });
    feed("a", "error", "action");
    await settle();
    expect([played, tones]).toEqual([[40], 0]);
    refuse = true;
    feed("a", "error", "action", TONE_GAP_MS);
    await settle();
    expect([played, tones, gains]).toEqual([[40, 40], 1, [0.06]]);
  } finally {
    play.mockRestore();
  }
});

test("several agents changing at once play one tone", () => {
  feed("a", "waiting_you", "finished");
  feed("b", "error", "action", 10);
  feed("c", "waiting_permission", "action", TONE_GAP_MS - 1);
  expect(tones).toBe(1);
  feed("a", "error", "action", TONE_GAP_MS);
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
  feed("a", "waiting_permission", "action");
  feed("b", "waiting_plan", "action");
  feed("b", "waiting_answer", "action");
  feed("c", "error", "action");
  // Waiting for you without having finished (e.g. from idle).
  feed("c", "waiting_you", "waiting");
  await settle();
  expect(shown).toEqual([
    { title: "fix login finished", body: "shop · feat", agent: "a" },
    { title: "fix login is waiting for permission", body: "shop · feat", agent: "a" },
    { title: "Claude is waiting for plan approval", body: "shop", agent: "b" },
    { title: "Claude is waiting for your answer", body: "shop", agent: "b" },
    { title: "Claude failed", body: "", agent: "c" },
    { title: "Claude is waiting for you", body: "", agent: "c" },
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

test("a usage alert goes to the inbox and the system, with no agent and no tone (17.23)", async () => {
  const resets_at = new Date(2026, 8, 28, 14, 30).getTime() / 1000;
  const alert = { type: "usage_alert", window: "session", threshold: 80, resets_at } as const;
  notify(alert, useHive.getState(), clock, 4321);
  apply(alert);
  await settle();
  expect(tones).toBe(0);
  expect(shown).toEqual([{ title: "Session usage at 80%", body: "Resets 14:30", agent: "" }]);
  const { inbox } = useHive.getState();
  expect(inbox.map(({ id: _, ...item }) => item)).toEqual([
    { agent: null, state: null, text: "Session usage at 80%", at: 4321 },
  ]);
  // The window's day for the week.
  notify({ ...alert, window: "week", threshold: 95 }, useHive.getState(), clock);
  await settle();
  expect(shown[1]).toEqual({ title: "Weekly usage at 95%", body: "Resets Mon 14:30", agent: "" });
});

test("other messages are ignored", () => {
  notify({ type: "welcome", version: "1", distro: null }, useHive.getState() as HiveState, clock);
  expect(tones).toBe(0);
});

test("every alert is kept in the inbox, the newest first, at most INBOX_LIMIT", () => {
  apply({ type: "agent_title", channel: 1, id: "a", title: "fix login" });
  feed("a", "waiting_you", "finished");
  feed("a", "waiting_permission", "action");
  feed("b", "waiting_you", "waiting");
  feed("b", "error", "action");
  // Muted: no tone, but still an alert.
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications.volume = 0;
  apply({ type: "settings", settings });
  notify(state("b", "waiting_permission", "action"), useHive.getState(), clock, 1234);
  const { inbox } = useHive.getState();
  expect(inbox.map((i) => "agent" in i && [i.id, i.agent, i.state, i.text])).toEqual([
    [5, "b", "waiting_permission", "Claude is waiting for permission"],
    [4, "b", "error", "Claude failed"],
    [3, "b", "waiting_you", "Claude is waiting for you"],
    [2, "a", "waiting_permission", "fix login is waiting for permission"],
    [1, "a", "waiting_you", "fix login finished"],
  ]);
  expect(inbox[0]?.at).toBe(1234);
  feed("a", "waiting_plan", "action");
  feed("a", "waiting_answer", "action");
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

test("a worktree's pull request that ended is an inbox item offering to remove it (17.27)", () => {
  const [shop, api] = MOCK_REPOS;
  const fix = shop.worktrees[1];
  const ended = (worktree: string, state: PullState, project = shop.id) => ({
    type: "pull_ended" as const,
    project,
    worktree,
    number: 12,
    title: "Fix the login",
    state,
  });
  apply({ type: "projects", projects: [shop, api] });
  for (const m of [ended(fix.id, "merged"), ended("/gone", "closed", "/unknown")]) {
    notify(m, useHive.getState(), clock, 99);
    apply(m);
  }
  // No tone and no OS notification: nothing waits on the user.
  expect([tones, shown]).toEqual([0, []]);
  expect(useHive.getState().inbox).toEqual([
    {
      id: 2,
      at: 99,
      text: "#12 closed: remove its worktree?",
      space: undefined,
      offer: { worktree: "/gone", number: 12, title: "Fix the login", state: "closed" },
    },
    {
      id: 1,
      at: 99,
      text: `#12 merged: remove ${fix.name}?`,
      space: "shop",
      offer: { worktree: fix.id, number: 12, title: "Fix the login", state: "merged" },
    },
  ]);

  // With several spaces, the item names the worktree's.
  const env = { git_name: null, git_email: null, gh_config_dir: null, gh_account: null };
  const spaces = [
    { id: "default", name: "Home", projects: [shop.id], env },
    { id: "w", name: "Work", projects: [api.id], env },
  ];
  apply({ type: "spaces", spaces, current: "w" });
  notify(ended(fix.id, "merged"), useHive.getState(), clock, 100);
  expect(useHive.getState().inbox[0]?.space).toBe("Home · shop");
});

// 17.25: whether a terminal's command or program alerts is the service's (`osc.rs`, tested
// there); here, only what each alert shows.

test("a command's time reads in seconds, minutes or hours", () => {
  expect([0, 59, 60, 125, 3599, 3600, 7322].map(duration)).toEqual([
    "0s",
    "59s",
    "1m 0s",
    "2m 5s",
    "59m 59s",
    "1h 0m",
    "2h 2m",
  ]);
});

test("a terminal is named by its worktree, else its folder", () => {
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const main = shop.worktrees[0] as (typeof shop.worktrees)[number];
  apply({ type: "projects", projects: [shop] });
  useHive.setState({
    tabs: [
      { id: 1, cwd: main.path },
      { id: 2, cwd: "/home/u/notes" },
      { id: 3, cwd: "/" },
    ],
  });
  apply({ type: "terminal_opened", channel: 1, worktree: main.id });
  const s = useHive.getState();
  expect([1, 2, 3, 4].map((id) => terminalName(s, id))).toEqual([
    main.name,
    "notes",
    "Terminal",
    "Terminal",
  ]);
});

test("a long command and a program's notification go to the inbox, with a tone and an OS notification", () => {
  useHive.setState({ tabs: [{ id: 7, cwd: "/home/u/api" }] });
  const command = (exit: number | null, secs: number) =>
    ({ type: "command_finished", channel: 7, exit, secs }) as const;
  const note = (title: string, body: string, notify: boolean) =>
    ({ type: "terminal_notification", channel: 7, title, body, notify }) as const;
  const messages = [
    command(0, 12),
    command(2, 75),
    command(null, 3600),
    note("make", "done", true),
    note("", "build ok", false),
    note("deploy", "", true),
  ];
  messages.forEach((m, i) => {
    notify(m, useHive.getState(), clock + i * 2 * TONE_GAP_MS, 1000 + i);
  });
  expect(tones).toBe(6);
  const inbox = [...useHive.getState().inbox].reverse();
  expect(inbox.map((i) => ("terminal" in i ? [i.terminal, i.failed, i.at, i.text] : i))).toEqual([
    [7, false, 1000, "Command finished in api after 12s"],
    [7, true, 1001, "Command failed (exit 2) in api after 1m 15s"],
    [7, false, 1002, "Command finished in api after 1h 0m"],
    [7, false, 1003, "api: make — done"],
    [7, false, 1004, "api: build ok"],
    [7, false, 1005, "api: deploy"],
  ]);
  // In view of the focused window a program's notification is not an OS one.
  expect(shown).toEqual([
    { title: "Command finished in api", body: "after 12s", agent: "terminal:7" },
    { title: "Command failed (exit 2) in api", body: "after 1m 15s", agent: "terminal:7" },
    { title: "Command finished in api", body: "after 1h 0m", agent: "terminal:7" },
    { title: "api", body: "make — done", agent: "terminal:7" },
    { title: "api", body: "deploy", agent: "terminal:7" },
  ]);
});

test("a program's notification never takes over the title: the terminal's name always leads", () => {
  useHive.setState({ tabs: [{ id: 7, cwd: "/home/u/api" }] });
  const s = useHive.getState();
  const forged = "Claude needs your permission";
  for (const [title, body] of [
    [forged, ""],
    [forged, "shop · main"],
    ["", forged],
    ["", ""],
  ]) {
    const text = terminalAlertText(s, {
      type: "terminal_notification",
      channel: 7,
      title,
      body,
      notify: true,
    });
    expect(text.title).toBe("api");
    expect(text.inbox.startsWith("api")).toBe(true);
    expect(text.inbox).not.toBe(forged);
    expect(text.agent).toBe("terminal:7");
  }
  const text = terminalAlertText(s, {
    type: "terminal_notification",
    channel: 7,
    title: forged,
    body: "shop · main",
    notify: true,
  });
  expect(text).toEqual({
    agent: "terminal:7",
    title: "api",
    body: `${forged} — shop · main`,
    inbox: `api: ${forged} — shop · main`,
  });
});

test("clicking a terminal's notification shows that terminal while it is open", () => {
  useHive.setState({
    tabs: [
      { id: 7, cwd: "/a" },
      { id: 8, cwd: "/b" },
    ],
    activeTab: 8,
  });
  goToClicked("terminal:7");
  expect(useHive.getState().activeTab).toBe(7);
  useHive.setState({ activeTab: 8 });
  for (const id of ["terminal:9", "terminal:", "7", "gone-agent"]) goToClicked(id);
  expect(useHive.getState().activeTab).toBe(8);
});
