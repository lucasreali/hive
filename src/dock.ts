import { attention } from "./attention";
import type { ServiceMessage } from "./protocol";
import { type HiveState, useHive } from "./store";
import { bounceDock, setDockBadge } from "./window";

// The Dock icon (17.16): the bell's count as its badge and one bounce when an agent asks for an
// action while Hive is not focused. The service decides `pending` and `alert` (#37).

/**
 * Whether `message` bounces the Dock: an agent that entered a state asking for an action (the
 * service's `action` alert: 🟡 or 🔴, never the snapshot after `welcome`), still pending, while
 * the window is not focused and the `notifications.bounce` setting is on. A 🟠 (finished or
 * waiting, seen or not) never bounces.
 */
export function shouldBounce(message: ServiceMessage, s: HiveState): boolean {
  return (
    message.type === "agent_state" &&
    message.alert === "action" &&
    message.pending &&
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
