import type { Agent, AgentStatus } from "./protocol";
import { type HiveState, mostUrgent, pendingAgents, treeAgents, unseenPending } from "./store";

/**
 * What needs the user, read from the service's `pending` and `urgency` (#37): the one source for
 * the Dock (17.16) and the menu bar icon (17.22). Nothing here decides a state.
 */
export type Attention = {
  /** The pending agents in tree order: the bell's list and F8's cycle. */
  pending: Agent[];
  /** The bell's count (8.5): pending agents not seen since the bell was last opened. */
  unseen: number;
  /** The most urgent status among every agent (a pending one first at equal urgency), or null. */
  urgent: AgentStatus | null;
};

export function attention(s: HiveState): Attention {
  return {
    pending: pendingAgents(s),
    unseen: unseenPending(s),
    urgent: mostUrgent(s, treeAgents(s)),
  };
}
