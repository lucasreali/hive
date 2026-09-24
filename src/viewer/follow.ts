import { type HiveState, useHive } from "../store";
import type { Transport } from "../transport";

/**
 * Keeps the open file's text current: asks the service for it when it opens, whenever a new
 * list of its worktree's changes arrives (the file may have changed with it), and after a new
 * `welcome`. Returns the unsubscribe.
 */
export function followOpenFile(transport: Transport): () => void {
  let asked: unknown[] = [];
  const sync = (s: HiveState) => {
    const open = s.connection.status === "connected" ? s.openFile : null;
    const reasons = [open, open && s.changes[open.worktree], s.connection];
    if (reasons.every((reason, i) => reason === asked[i])) return;
    asked = reasons;
    if (open) void transport.openFile(open.worktree, open.path);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
