// Keep the service following what the files panel shows: the worktree it lists and the open file.

import type { DiffBase } from "./protocol";
import {
  askBufferTable,
  clearTableAsks,
  diffBase,
  dropTableAsks,
  type HiveState,
  type OpenFile,
  panelWorktree,
  setSaveAfterAsks,
  setTableSentFrom,
  type TableAsk,
  tableShown,
  terminalShown,
  useHive,
} from "./store";
import { asideFile, asksPending, fileKey, fileTabState, paneState } from "./tabs";
import type { Transport } from "./transport";
import { isDirty, isFor } from "./viewer/buffer";
import { saveBuffer } from "./viewer/save";

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
 * Keeps the service sending the open table file's table (23.6) while it shows as one: when it
 * starts showing, on each new `changes` of its worktree, after a new `welcome`, and once its
 * buffer has no more unsaved edits. A table with unsaved edits (25.4) is its buffer's, not the
 * disk's: asked only when it starts showing, from that text.
 */
export function followOpenTable(transport: Transport): () => void {
  let asked: unknown[] = [];
  let askedAside: unknown[] = [];
  const sync = (s: HiveState) => {
    const connected = s.connection.status === "connected";
    // A table with unsaved edits is its buffer's (25.4), asked only when it starts showing.
    const ask = (file: OpenFile, dirty: boolean, shows: boolean) => {
      if (!dirty) void transport.openTable(file.worktree, file.path);
      else if (shows) askBufferTable(file);
    };
    const open = connected && tableShown(s) ? s.openFile : null;
    const dirty = !!open && !!s.edit && isFor(s.edit, open) && isDirty(s.edit);
    const reasons = [open, open && !dirty && s.changesSeen[open.worktree], s.connection, dirty];
    if (!reasons.every((reason, i) => reason === asked[i])) {
      const shows = open !== asked[0] || s.connection !== asked[2];
      asked = reasons;
      if (open) ask(open, dirty, shows);
    }
    // A pair's other file (25.1), shown as its table.
    const file = connected ? asideFile(s) : null;
    const aside = file && tableShown(paneState(s, file)) ? file : null;
    const asideDirty = !!aside?.edit && isDirty(aside.edit);
    const seen = [
      aside && fileKey(aside),
      aside && !asideDirty && s.changesSeen[aside.worktree],
      s.connection,
      asideDirty,
    ];
    if (seen.every((reason, i) => reason === askedAside[i])) return;
    const shows = seen[0] !== askedAside[0] || s.connection !== askedAside[2];
    askedAside = seen;
    if (aside) ask(aside, asideDirty, shows);
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}

/**
 * Sends what is asked of a table being edited (25.4), one at a time, each with the text the last
 * answer left, noted so its answer applies to that text only; a save that waited for its file's
 * then goes. A connection lost answers nothing, so they are dropped; one that cannot be sent (too
 * large for a frame) says so through `transport` and under the header, and those of its file and
 * a save waiting on them go with it.
 */
export function followTableEdits(transport: Transport): () => void {
  let sent: TableAsk | null = null;
  const sync = (s: HiveState) => {
    if (s.connection.status !== "connected") {
      if (s.tableAsks.length > 0 || s.saveAfterAsks) clearTableAsks();
      return;
    }
    const waiting = s.saveAfterAsks;
    if (waiting && !asksPending(s, waiting)) {
      setSaveAfterAsks(null);
      saveBuffer(waiting);
    }
    const next = s.tableAsks[0] ?? null;
    if (next === sent) return;
    sent = next;
    if (!next) return;
    const { edit } = fileTabState(s, next);
    // The file was closed meanwhile.
    if (!edit) return void dropTableAsks(next);
    setTableSentFrom(edit.doc);
    transport
      .editTable(next.worktree, next.path, edit.doc.toString(), next.edit)
      .catch((error: unknown) => dropTableAsks(next, String(error)));
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
