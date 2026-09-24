import { type HiveState, panelWorktree, useHive } from "./store";
import type { Transport } from "./transport";

/**
 * Keeps the service watching the worktree the files panel shows (`panelWorktree`): one at a
 * time, none while the panel is closed. A new connection starts with no watch, so it is sent
 * again. Returns the unsubscribe.
 */
export function followPanel(transport: Transport): () => void {
  let watched: string | null = null;
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    const shown = connected ? panelWorktree(s) : null;
    if (shown === watched) return;
    if (shown) void transport.watchWorktree(shown);
    else if (connected) void transport.unwatchWorktree();
    watched = shown;
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
