import type { AgentState, ServiceMessage } from "./protocol";
import { goToAgent } from "./shortcuts";
import { activateTab, addToInbox, findWorktree, type HiveState, spaceOf, useHive } from "./store";
import { transport } from "./transport";
import { usageAlertText } from "./usage";
import { isMac } from "./window";

// Presentation of the alerts the service decided (hive.md item 5, 2.4, 13.4, #37): a tone and
// an inbox item for each `agent_state` with an `alert`, and an OS notification when the service
// says `notify` (the agent is not in view of the focused window). Nothing here computes a state
// or a transition.

/** What an alert says after the agent's name; "finished" when it finished. */
const ALERT_TEXT: Partial<Record<AgentState, string>> = {
  waiting_permission: "is waiting for permission",
  waiting_plan: "is waiting for plan approval",
  waiting_answer: "is waiting for your answer",
  waiting_you: "is waiting for you",
  error: "failed",
};
/** Agents changing within this window share one tone. */
export const TONE_GAP_MS = 500;

let lastTone = Number.NEGATIVE_INFINITY;
let audio: AudioContext | undefined;

/**
 * The alert tone at `volume` percent. On macOS the app plays it (17.19): App Nap throttles a
 * hidden webview, so its own tone could come late or not at all; the webview's is the fallback.
 */
function tone(volume: number): void {
  if (isMac()) transport.playTone(volume).catch(() => webTone(volume));
  else webTone(volume);
}

/** A short synthesized beep at `volume` percent; no audio file. */
function webTone(volume: number): void {
  audio ??= new AudioContext();
  void audio.resume();
  const t = audio.currentTime;
  const osc = audio.createOscillator();
  const gain = audio.createGain();
  osc.frequency.value = 880;
  gain.gain.setValueAtTime((0.15 * volume) / 100, t);
  gain.gain.exponentialRampToValueAtTime(0.001, t + 0.2);
  osc.connect(gain).connect(audio.destination);
  osc.start(t);
  osc.stop(t + 0.2);
}

/**
 * "space · project · worktree" for the agent, as far as the service placed it; the space is
 * named only when there are several (6.14: agents of every space alert).
 */
export function agentPlace(s: HiveState, id: string): string {
  const agent = s.agents[id];
  const project = agent?.project ? s.projects?.[agent.project] : undefined;
  const worktree = project?.worktrees.find((w) => w.id === agent?.worktree);
  return [spaceName(s, id), project?.name, worktree?.name].filter(Boolean).join(" · ");
}

/** The name of the agent's space, only when there are several (6.14). */
export function spaceName(s: HiveState, id: string): string | undefined {
  if ((s.spaces?.length ?? 0) < 2) return undefined;
  return spaceOf(s, s.agents[id]?.project ?? null)?.name;
}

type AgentStateMessage = Extract<ServiceMessage, { type: "agent_state" }>;

/** An alert's text: the OS notification's and, its title, the inbox item's. */
export type AlertText = { agent: string; title: string; body: string };

/**
 * The text of an `agent_state` with an alert: the title names the agent (its title, else
 * "Claude") and what happened ("fix login finished"); the body is where it is.
 */
export function alertText(s: HiveState, message: AgentStateMessage): AlertText {
  const { id, state, alert } = message;
  const what = alert === "finished" ? "finished" : ALERT_TEXT[state];
  return { agent: id, title: `${s.agentTitles[id] ?? "Claude"} ${what}`, body: agentPlace(s, id) };
}

/** The tone, at most one per `TONE_GAP_MS`. */
function toneAt(s: HiveState, now: number): void {
  const volume = s.settings.notifications.volume;
  if (volume > 0 && now - lastTone >= TONE_GAP_MS) {
    lastTone = now;
    tone(volume);
  }
}

/**
 * Presents a message's alert. The service sets `alert` only on the message whose state changed,
 * never on an agent's first state, the snapshot after `welcome` or an interrupt; it sends a
 * terminal's alerts (17.25) only when they alert.
 */
export function notify(
  message: ServiceMessage,
  s: HiveState = useHive.getState(),
  now = performance.now(),
  wall = Date.now(),
): void {
  if (message.type === "usage_alert") {
    usageAlert(message, wall);
    return;
  }
  if (message.type === "pull_ended") {
    offerRemoval(s, message, wall);
    return;
  }
  if (message.type === "command_finished" || message.type === "terminal_notification") {
    const text = terminalAlertText(s, message);
    const failed = message.type === "command_finished" && !!message.exit;
    addToInbox({ terminal: message.channel, failed, at: wall, text: text.inbox });
    toneAt(s, now);
    const notifies = message.type === "command_finished" || message.notify;
    if (notifies) void transport.showNotification(text.title, text.body, text.agent);
    return;
  }
  if (message.type !== "agent_state" || !message.alert) return;
  const { id, state } = message;
  const text = alertText(s, message);
  addToInbox({ agent: id, state, at: wall, text: text.title, space: spaceName(s, id) });
  toneAt(s, now);
  if (message.notify) void transport.showNotification(text.title, text.body, id);
}

type PullEnded = Extract<ServiceMessage, { type: "pull_ended" }>;

/**
 * A worktree's pull request merged or closed (17.27): an inbox item offering to remove the
 * worktree ("#12 merged: remove fix-login?"), naming its project (and space, when there are
 * several). No tone and no OS notification: nothing waits on the user.
 */
function offerRemoval(s: HiveState, m: PullEnded, wall: number): void {
  const project = s.projects?.[m.project];
  const worktree = project?.worktrees.find((w) => w.id === m.worktree);
  const several = (s.spaces?.length ?? 0) > 1;
  const space = several ? spaceOf(s, m.project)?.name : undefined;
  addToInbox({
    offer: { worktree: m.worktree, number: m.number, title: m.title, state: m.state },
    at: wall,
    text: `#${m.number} ${m.state}: remove ${worktree?.name ?? "its worktree"}?`,
    space: [space, project?.name].filter(Boolean).join(" · ") || undefined,
  });
}

/**
 * A window of the account's usage reached 80% or 95% (17.23; the service sends it once per
 * threshold and window): an inbox item and an OS notification, focused or not, with no agent
 * (a click only brings the window up) and no tone.
 */
function usageAlert(message: Extract<ServiceMessage, { type: "usage_alert" }>, wall: number) {
  const text = usageAlertText(message);
  addToInbox({ agent: null, state: null, at: wall, text: text.title });
  void transport.showNotification(text.title, text.body, "");
}

/** What a terminal's OS notification carries as its `agent`, so its click finds the terminal. */
const TERMINAL_CLICK = "terminal:";

/** A terminal's name in its alerts: its worktree's, else its folder's. */
export function terminalName(s: HiveState, channel: number): string {
  const worktree = findWorktree(s.projects, s.terminals[channel]?.worktree ?? null);
  const cwd = s.tabs.find((t) => t.id === channel)?.cwd ?? "";
  return worktree?.name ?? (cwd.slice(cwd.lastIndexOf("/") + 1) || "Terminal");
}

/** "12s", "3m 4s", "1h 2m": how long a command ran. */
export function duration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${Math.floor(secs / 3600)}h ${Math.floor(secs / 60) % 60}m`;
}

type TerminalAlert = Extract<
  ServiceMessage,
  { type: "command_finished" } | { type: "terminal_notification" }
>;

/**
 * A terminal alert's text (17.25): the OS notification's title and body, and the inbox item's.
 * A long command says how it ended and how long it ran. A program's notification is titled with
 * the terminal's name, never the program's own title, so a program cannot pass its text off as
 * Hive's (an agent's alert); its title and body follow ("make — done").
 */
export function terminalAlertText(s: HiveState, m: TerminalAlert): AlertText & { inbox: string } {
  const name = terminalName(s, m.channel);
  const agent = `${TERMINAL_CLICK}${m.channel}`;
  if (m.type === "command_finished") {
    const how = m.exit ? `Command failed (exit ${m.exit})` : "Command finished";
    const title = `${how} in ${name}`;
    const body = `after ${duration(m.secs)}`;
    return { agent, title, body, inbox: `${title} ${body}` };
  }
  const body = [m.title, m.body].filter((part) => part).join(" — ");
  const inbox = body ? `${name}: ${body}` : name;
  return { agent, title: name, body, inbox };
}

/**
 * A click on an OS notification (13.5): goes to its agent as an inbox item does, switching
 * space when needed, or shows its terminal (17.25). One whose agent or terminal is gone changes
 * nothing; the app side already brought the window up.
 */
export function goToClicked(id: string): void {
  const s = useHive.getState();
  const agent = s.agents[id];
  const tab = s.tabs.find((t) => `${TERMINAL_CLICK}${t.id}` === id);
  if (agent) goToAgent(agent);
  else if (tab) activateTab(tab);
}
