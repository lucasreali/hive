// Keep the service following what the files panel shows: the worktree it lists and the open file.

import { diffBase, type HiveState, panelWorktree, useHive } from "./store";
import type { Transport } from "./transport";

/**
 * Keeps the service watching the worktree the right panel shows (`panelWorktree`), none while it
 * is closed. A new connection starts with no watch, so it is sent again. Returns the unsubscribe.
 */
export function followPanel(transport: Transport): () => void {
  let watched: string | null = null;
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    const open = connected && s.rightPanel === "files";
    const shown = open ? (panelWorktree(s)?.worktree.path ?? null) : null;
    // A new base is watched anew: the service lists its changes against it.
    const base = shown && diffBase(s, shown);
    const key = shown && `${base}:${shown}`;
    if (key === watched) return;
    if (shown && base) void transport.watchWorktree(shown, base);
    else if (connected) void transport.unwatchWorktree();
    watched = key;
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}

/**
 * Tells the service which terminal is in view (none while a file is shown) and whether the
 * window has the focus, whenever either changes and after a new `welcome`: the service decides
 * that an agent finishing there was already seen (hive.md item 5). Returns the unsubscribe.
 */
export function followView(transport: Transport): () => void {
  let sent: unknown[] = [];
  const sync = (s: HiveState) => {
    const view = [s.connection, s.fileShown ? null : s.activeTab, s.focused] as const;
    if (s.connection.status !== "connected" || view.every((v, i) => v === sent[i])) return;
    sent = [...view];
    void transport.setView(view[1], view[2]);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}

/**
 * Keeps the open file's text current: asks the service for it when it opens, whenever a new
 * list of its worktree's changes arrives (the file may have changed with it) or its base
 * changes, after a new `welcome`, and when a save found a newer version on disk. Returns the
 * unsubscribe.
 */
export function followOpenFile(transport: Transport): () => void {
  let asked: unknown[] = [];
  const sync = (s: HiveState) => {
    const open = s.connection.status === "connected" ? s.openFile : null;
    const base = open && diffBase(s, open.worktree);
    const changes = open && s.changes[open.worktree];
    const reasons = [open, changes, base, s.connection, s.edit?.recheck ?? 0];
    if (reasons.every((reason, i) => reason === asked[i])) return;
    asked = reasons;
    if (open && base) void transport.openFile(open.worktree, open.path, base);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
