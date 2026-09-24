import { type AgentState, type HiveState, type ServiceMessage, useHive } from "./store";
import { showNotification } from "./window";

// Presentation of state changes the service sent (hive.md item 5, #37): a tone when an agent
// enters an alerting state, an OS notification when it finishes, unless the service says it is
// not pending (it finished in view of the focused window: already seen). Nothing here computes a
// state.

const ALERTING: AgentState[] = ["waiting_permission", "waiting_you", "error"];
const BUSY: AgentState[] = ["working", "with_subagents"];
/** Agents changing within this window share one tone. */
export const TONE_GAP_MS = 500;

let lastTone = Number.NEGATIVE_INFINITY;
let audio: AudioContext | undefined;

/** A short synthesized beep; no audio file. */
function tone(): void {
  audio ??= new AudioContext();
  void audio.resume();
  const t = audio.currentTime;
  const osc = audio.createOscillator();
  const gain = audio.createGain();
  osc.frequency.value = 880;
  gain.gain.setValueAtTime(0.15, t);
  gain.gain.exponentialRampToValueAtTime(0.001, t + 0.2);
  osc.connect(gain).connect(audio.destination);
  osc.start(t);
  osc.stop(t + 0.2);
}

/** "project · worktree" for the agent, as far as the service placed it. */
function place(s: HiveState, id: string): string {
  const agent = s.agents[id];
  const project = agent?.project ? s.projects?.[agent.project] : undefined;
  const worktree = project?.worktrees.find((w) => w.id === agent?.worktree);
  return [project?.name, worktree?.name].filter(Boolean).join(" · ");
}

/**
 * Call before `apply(message)`, so the store still holds the previous state. An agent's first
 * state is not a change and stays silent; so is the snapshot after `welcome`, which always
 * lands in an empty store (a fresh page, or after `disconnected` cleared it).
 */
export function notify(
  message: ServiceMessage,
  s: HiveState = useHive.getState(),
  now = performance.now(),
): void {
  if (message.type !== "agent_state") return;
  const before = s.agentStates[message.id]?.state;
  const after = message.state;
  if (before === undefined || before === after) return;
  if (ALERTING.includes(after) && now - lastTone >= TONE_GAP_MS) {
    lastTone = now;
    tone();
  }
  if (after === "waiting_you" && BUSY.includes(before) && message.pending) {
    const where = place(s, message.id);
    void showNotification(
      "Agent finished",
      where ? `${where}: waiting for you` : "Waiting for you",
    );
  }
}
