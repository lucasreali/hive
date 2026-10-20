import { attention } from "./attention";
import type { AgentState, ServiceMessage } from "./protocol";
import { type HiveState, useHive } from "./store";
import { bounceDock, setDockBadge } from "./window";

// The Dock icon (17.16): the bell's count as its badge and one bounce when an agent asks for an
// action while Hive is not focused. The service decides `pending` and `alert` (#37).

/** The states that ask for an action (🟡 permission, plan, answer; 🔴 error): they bounce. */
const ACTION: ReadonlySet<AgentState> = new Set([
  "waiting_permission",
  "waiting_plan",
  "waiting_answer",
  "error",
]);

/**
 * Whether `message` bounces the Dock: an agent's new 🟡 or 🔴 (the service's `alert`, so never
 * the snapshot after `welcome`), still pending, while the window is not focused and the
 * `notifications.bounce` setting is on. A 🟠 (finished, seen or not) never bounces.
 */
export function shouldBounce(message: ServiceMessage, s: HiveState): boolean {
  return (
    message.type === "agent_state" &&
    message.alert !== null &&
    message.pending &&
    ACTION.has(message.state) &&
    !s.focused &&
    s.settings.notifications.bounce
  );
}

/** Bounces the Dock for `message` when `shouldBounce` says so. */
export function bounceFor(message: ServiceMessage, s: HiveState = useHive.getState()): void {
  if (shouldBounce(message, s)) bounceDock();
}

/**
 * Keeps the Dock badge on the bell's count (`attention().unseen`), sent only when it changes
 * and cleared at zero. Returns the unsubscribe.
 */
export function followDock(badge: (count: number) => void = setDockBadge): () => void {
  let shown = 0;
  const sync = (s: HiveState) => {
    const count = attention(s).unseen;
    if (count === shown) return;
    shown = count;
    badge(count);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
