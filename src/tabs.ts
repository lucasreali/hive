import { ORDER_LIMIT } from "./persist";
import type { FileText } from "./protocol";
import type { FileTab, HiveState, OpenFile, Split, Tab } from "./store";
import { type EditBuffer, fromDisk, isFor, startEdit } from "./viewer/buffer";

// The bar, tab and file-tab selectors: pure functions of the store's state.

/**
 * The edit buffer after an answer for the open file while editing: the buffer updated from
 * it, or started from it; unchanged for any other file or when not editing.
 */
export function editFor(s: HiveState, file: FileText | null): EditBuffer | null {
  if (!s.editing || !file || !s.openFile || !isFor(file, s.openFile)) return s.edit;
  return s.edit ? fromDisk(s.edit, file) : startEdit(file);
}

/** The key of a file's tab in `tabOrder`. */
export const fileKey = (f: OpenFile) => `file:${f.worktree}\n${f.path}`;

/** `order` with `key` last, unless it already has a place; the oldest go past the limit. */
export const withKey = (order: string[], key: string) =>
  order.includes(key) ? order : [...order, key].slice(-ORDER_LIMIT);

/** File `f`'s own state: live for the open file, else kept in its tab. */
export function fileTabState(
  s: HiveState,
  f: OpenFile,
): { editing: boolean; edit: EditBuffer | null } {
  if (s.openFile && isFor(f, s.openFile)) return { editing: s.editing, edit: s.edit };
  const tab = s.openFiles.find((t) => isFor(t, f));
  return { editing: tab?.editing ?? false, edit: tab?.edit ?? null };
}

export function opened(s: HiveState, file: OpenFile, editing: boolean, line?: number) {
  const { worktree, path } = file;
  const shown = {
    fileShown: true,
    gotoLine: line ? { worktree, path, line } : null,
  };
  if (s.openFile && isFor(file, s.openFile)) return shown;
  // The file shown until now keeps its live state in its tab.
  const openFiles = s.openFiles.map((f) =>
    s.openFile && isFor(f, s.openFile) ? { ...f, editing: s.editing, edit: s.edit } : f,
  );
  const tab = openFiles.find((f) => isFor(f, file));
  return {
    ...shown,
    openFiles: tab
      ? openFiles
      : [...openFiles, { worktree, path, editing, edit: null, view: null }],
    tabOrder: withKey(s.tabOrder, fileKey(file)),
    openFile: { worktree, path },
    editing: tab ? tab.editing : editing,
    edit: tab ? tab.edit : null,
    editorNotice: null,
  };
}

/**
 * Closes file `f`'s tab. When it was shown, the tab beside it in the bar (the right one, else
 * the left one) shows instead.
 */
export function dropFile(s: HiveState, f: OpenFile): Partial<HiveState> {
  const items = barItems(s);
  const i = items.findIndex((t) => "path" in t && isFor(t, f));
  const rest = items.filter((_, j) => j !== i);
  const base = {
    openFiles: s.openFiles.filter((t) => !isFor(t, f)),
    tabOrder: s.tabOrder.filter((k) => k !== fileKey(f)),
  };
  if (!s.openFile || !isFor(f, s.openFile)) return base;
  const closed = {
    ...base,
    openFile: null,
    fileShown: false,
    editing: false,
    edit: null,
    editorNotice: null,
    gotoLine: null,
  };
  const next = s.fileShown ? rest[Math.min(Math.max(i, 0), rest.length - 1)] : undefined;
  if (next && "path" in next) return { ...closed, ...opened({ ...s, ...closed }, next, false) };
  return next ? { ...closed, activeTab: next.id } : closed;
}

/** The split shown: the stored one while the active tab is one of its panes, else none. */
export function shownSplit(s: HiveState): Split | null {
  const split = s.split;
  return split && (s.activeTab === split.left || s.activeTab === split.right) ? split : null;
}

/** The terminals the terminal area shows, left to right. */
export function shownTerminals(s: HiveState): number[] {
  const split = shownSplit(s);
  if (split) return [split.left, split.right];
  return s.activeTab === null ? [] : [s.activeTab];
}

/**
 * The worktree a terminal tab belongs to, as the service placed its cwd (`terminal_opened`;
 * once that worktree is gone, its project's); outside every project, or until the service
 * answers, its cwd.
 */
export const tabWorktree = (s: HiveState, tab: Tab): string =>
  s.terminals[tab.id]?.worktree ?? tab.cwd;

/**
 * Whose tabs the tab bar shows: the selected worktree (a selected project stands for its main
 * worktree); a selected agent's terminal's worktree. Null (nothing selected) shows every tab.
 */
export function tabsPlace(s: HiveState): string | null {
  const agent = s.agents[s.selection ?? ""];
  if (!agent) return s.selection;
  const tab = s.tabs.find((t) => t.id === agent.terminal);
  return tab ? tabWorktree(s, tab) : agent.worktree;
}

/** The terminal tabs of the place the tab bar shows, in the bar's order. */
export function visibleTabs(s: HiveState): Tab[] {
  const place = tabsPlace(s);
  return inBarOrder(s, place === null ? s.tabs : s.tabs.filter((t) => tabWorktree(s, t) === place));
}

/** A tab of the bar: a terminal or an open file. */
export type BarItem = Tab | FileTab;

/**
 * A tab's key in `tabOrder` (8.21): a file's by its path; a terminal's by its Claude
 * session while one runs in it (so it keeps its place when the session is resumed after a
 * reload), else by its channel.
 */
export function barKey(s: HiveState, item: BarItem): string {
  if ("path" in item) return fileKey(item);
  const agent = Object.values(s.agents).find((a) => a.terminal === item.id);
  return agent ? `session:${agent.id}` : `tab:${item.id}`;
}

/** `items` in the bar's order (`tabOrder`); any it does not hold follow, as given (a stable sort). */
export function inBarOrder<T extends BarItem>(s: HiveState, items: T[]): T[] {
  const rank = (item: T) => {
    let i = s.tabOrder.indexOf(barKey(s, item));
    if (i < 0 && !("path" in item)) i = s.tabOrder.indexOf(`tab:${item.id}`);
    return i < 0 ? s.tabOrder.length : i;
  };
  return [...items].sort((a, b) => rank(a) - rank(b));
}

/** The tabs of the bar, terminals and files mixed, in its order. */
export function barItems(s: HiveState): BarItem[] {
  const place = tabsPlace(s);
  const files = s.openFiles.filter((f) => place === null || f.worktree === place);
  return inBarOrder(s, [...visibleTabs(s), ...files]);
}

/** Whether the open file's tab belongs to the place the tab bar shows. */
export function fileVisible(s: HiveState): boolean {
  const place = tabsPlace(s);
  return !!s.openFile && (place === null || s.openFile.worktree === place);
}
