// Keep the service following what the files panel shows: the worktree it lists and the open file.

import type { DiffBase } from "./protocol";
import {
  diffBase,
  type HiveState,
  type OpenFile,
  panelWorktree,
  tableShown,
  terminalShown,
  useHive,
} from "./store";
import { asideFile, fileKey, paneState } from "./tabs";
import type { Transport } from "./transport";

/**
 * Keeps the service watching the worktree the right panel shows (`panelWorktree`), none while it
 * is closed. A new connection starts with no watch, so it is sent again. Also tells it which of
 * the listing's ignored folders (`dir/`) the Files tree has open (14.2), whenever that changes
 * and after every new watch (which starts with none). Returns the unsubscribe.
 */
export function followPanel(transport: Transport): () => void {
  let watched: string | null = null;
  let expanded: string[] = [];
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    const open = connected && s.rightPanel === "files";
    const shown = open ? (panelWorktree(s)?.worktree.path ?? null) : null;
    // A new base is watched anew: the service lists its changes against it.
    const base = shown && diffBase(s, shown);
    const key = shown && `${base}:${shown}`;
    if (key !== watched) {
      if (shown && base) void transport.watchWorktree(shown, base);
      else if (connected) void transport.unwatchWorktree();
      watched = key;
      expanded = [];
    }
    const listing = shown && s.worktreeFiles?.path === shown ? s.worktreeFiles : null;
    const folders = (listing?.ignored ?? [])
      .filter((p) => p.endsWith("/") && s.collapsed[`files:${shown}/${p.slice(0, -1)}`] === false)
      .map((p) => p.slice(0, -1));
    const same = folders.length === expanded.length && folders.every((p, i) => p === expanded[i]);
    if (!shown || same) return;
    expanded = folders;
    void transport.expandIgnored(shown, folders);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}

/**
 * Tells the service which terminal is in view (none while a file is shown), whether the window
 * has the focus and whether it is visible, whenever one changes and after a new `welcome`: the
 * service decides that an agent waiting for you there was already seen (hive.md item 5, 15.6),
 * and pauses its health check while the window is hidden (D18.11). Returns the unsubscribe.
 */
export function followView(transport: Transport): () => void {
  let sent: unknown[] = [];
  const sync = (s: HiveState) => {
    const view = [
      s.connection,
      terminalShown(s) ? s.activeTab : null,
      s.focused,
      s.visible,
    ] as const;
    if (s.connection.status !== "connected" || view.every((v, i) => v === sent[i])) return;
    sent = [...view];
    void transport.setView(view[1], view[2], view[3]);
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
  const reasons = (s: HiveState, open: OpenFile | null, recheck: number) => {
    const base = open && diffBase(s, open.worktree);
    // A folder project's folder has no changes (23.1): each new listing of it stands in.
    const folder = open && s.projects?.[open.worktree]?.folder;
    const changes = open && (folder ? s.worktreeFiles : s.changesSeen[open.worktree]);
    return [open && fileKey(open), changes, base, s.connection, recheck];
  };
  const ask = (open: OpenFile | null, seen: unknown[], before: unknown[]) => {
    if (seen.every((reason, i) => reason === before[i])) return before;
    if (open) void transport.openFile(open.worktree, open.path, seen[2] as DiffBase);
    return seen;
  };
  let asked: unknown[] = [];
  // A pair's other file (25.1), followed as the open one is.
  let askedAside: unknown[] = [];
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    const open = connected ? s.openFile : null;
    asked = ask(open, reasons(s, open, s.edit?.recheck ?? 0), asked);
    const aside = connected ? asideFile(s) : null;
    askedAside = ask(aside, reasons(s, aside, aside?.edit?.recheck ?? 0), askedAside);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}

/**
 * Keeps the open table file's table current (23.6) while it shows as one: asks the service for
 * it when it shows, whenever a new list of its worktree's changes arrives and after a new
 * `welcome`. Returns the unsubscribe.
 */
export function followOpenTable(transport: Transport): () => void {
  let asked: unknown[] = [];
  let askedAside: unknown[] = [];
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    const open = connected && tableShown(s) ? s.openFile : null;
    const reasons = [open, open && s.changesSeen[open.worktree], s.connection];
    if (!reasons.every((reason, i) => reason === asked[i])) {
      asked = reasons;
      if (open) void transport.openTable(open.worktree, open.path);
    }
    // A pair's other file (25.1), shown as its table.
    const file = connected ? asideFile(s) : null;
    const aside = file && tableShown(paneState(s, file)) ? file : null;
    const seen = [aside && fileKey(aside), aside && s.changesSeen[aside.worktree], s.connection];
    if (seen.every((reason, i) => reason === askedAside[i])) return;
    askedAside = seen;
    if (aside) void transport.openTable(aside.worktree, aside.path);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
