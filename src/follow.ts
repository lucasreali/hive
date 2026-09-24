// Keep the service following what the files panel shows: the worktree it lists and the open file.

import { type HiveState, panelWorktree, useHive } from "./store";
import type { Transport } from "./transport";

/**
 * Keeps the service watching the worktree the right panel shows (`panelWorktree`): one at a
 * time, none while the panel is closed. A new connection starts with no watch, so it is sent
 * again. Returns the unsubscribe.
 */
export function followPanel(transport: Transport): () => void {
  let watched: string | null = null;
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    const open = connected && s.rightPanel === "files";
    const shown = open ? (panelWorktree(s)?.worktree.path ?? null) : null;
    if (shown === watched) return;
    if (shown) void transport.watchWorktree(shown);
    else if (connected) void transport.unwatchWorktree();
    watched = shown;
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}

/**
 * Keeps the open file's text current: asks the service for it when it opens, whenever a new
 * list of its worktree's changes arrives (the file may have changed with it), after a new
 * `welcome`, and when a save found a newer version on disk. Returns the unsubscribe.
 */
export function followOpenFile(transport: Transport): () => void {
  let asked: unknown[] = [];
  const sync = (s: HiveState) => {
    const open = s.connection.status === "connected" ? s.openFile : null;
    const reasons = [open, open && s.changes[open.worktree], s.connection, s.edit?.recheck ?? 0];
    if (reasons.every((reason, i) => reason === asked[i])) return;
    asked = reasons;
    if (open) void transport.openFile(open.worktree, open.path);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
