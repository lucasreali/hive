import type { AgentState, ServiceMessage } from "./protocol";
import { goToAgent } from "./shortcuts";
import { addToInbox, type HiveState, spaceOf, useHive } from "./store";
import { transport } from "./transport";

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

/** A short synthesized beep at `volume` percent; no audio file. */
function tone(volume: number): void {
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

/**
 * Presents a message's alert. The service sets `alert` only on the message whose state changed,
 * never on an agent's first state, the snapshot after `welcome` or an interrupt.
 */
export function notify(
  message: ServiceMessage,
  s: HiveState = useHive.getState(),
  now = performance.now(),
  wall = Date.now(),
): void {
  if (message.type !== "agent_state" || !message.alert) return;
  const { id, state } = message;
  const text = alertText(s, message);
  addToInbox({ agent: id, state, at: wall, text: text.title, space: spaceName(s, id) });
  const volume = s.settings.notifications.volume;
  if (volume > 0 && now - lastTone >= TONE_GAP_MS) {
    lastTone = now;
    tone(volume);
  }
  if (message.notify) void transport.showNotification(text.title, text.body, id);
}

/**
 * A click on an agent's OS notification (13.5): goes to the agent as an inbox item does,
 * switching space when needed. One whose agent is gone changes nothing; the app side already
 * brought the window up.
 */
export function goToClicked(id: string): void {
  const agent = useHive.getState().agents[id];
  if (agent) goToAgent(agent);
}
