import { HISTORY_LIMIT, ORDER_LIMIT, remembered } from "./persist";
import type { FileText } from "./protocol";
import type { FileTab, HiveState, OpenFile, PaneId, Split, Tab } from "./store";
import { type EditBuffer, fromDisk, isDirty, isFor, startEdit } from "./viewer/buffer";

// The bar, tab and file-tab selectors: pure functions of the store's state.

/**
 * The edit buffer after an answer for the open file while editing: the buffer updated from
 * it, or started from it; unchanged for any other file or when not editing.
 */
export function editFor(s: HiveState, file: FileText | null): EditBuffer | null {
  if (!s.editing || !file || !s.openFile || !isFor(file, s.openFile)) return s.edit;
  return s.edit ? fromDisk(s.edit, file, asksPending(s, file)) : startEdit(file);
}

/** The service is still to answer a table's edit of file `f` (25.4). */
export const asksPending = (s: HiveState, f: OpenFile) => s.tableAsks.some((a) => isFor(a, f));

/** `change` on file `f`'s edit buffer, live or kept in its tab. */
export function patchEdits(
  s: HiveState,
  f: OpenFile,
  change: (b: EditBuffer) => EditBuffer,
): Pick<HiveState, "edit" | "openFiles"> {
  const mine = (b: EditBuffer | null): b is EditBuffer => !!b && isFor(b, f);
  return {
    edit: mine(s.edit) ? change(s.edit) : s.edit,
    openFiles: s.openFiles.map((t) => (mine(t.edit) ? { ...t, edit: change(t.edit) } : t)),
  };
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
  // A terminal it is paired with (25.1) is the pane beside it.
  const pair = splitOf(s, fileKey(file));
  const beside = [pair?.left, pair?.right].find((p) => typeof p === "number");
  const shown = {
    fileShown: true,
    gotoLine: line ? { worktree, path, line } : null,
    ...(beside === undefined ? {} : { activeTab: beside }),
  };
  if (s.openFile && isFor(file, s.openFile)) return shown;
  // The file shown until now keeps its live state in its tab; in a pair, its answers too, as its
  // pane still shows them (25.1).
  const openFiles = s.openFiles.map((f) => (s.openFile && isFor(f, s.openFile) ? left(s, f) : f));
  const tab = openFiles.find((f) => isFor(f, file));
  // A new tab is the preview tab (11.1): it takes the old preview's place in its worktree's bar;
  // one of another worktree closes. One with unsaved edits is kept, whatever its flag says.
  const old = tab ? undefined : openFiles.find((f) => f.preview && !(f.edit && isDirty(f.edit)));
  const key = fileKey(file);
  const order = !old
    ? s.tabOrder
    : old.worktree === worktree
      ? s.tabOrder.map((k) => (k === fileKey(old) ? key : k))
      : s.tabOrder.filter((k) => k !== fileKey(old));
  return {
    ...shown,
    openFiles: tab
      ? openFiles
      : [
          ...openFiles.filter((f) => f !== old),
          { worktree, path, editing, edit: null, view: null, preview: true },
        ],
    tabOrder: withKey(order, key),
    openFile: { worktree, path },
    editing: tab ? tab.editing : editing,
    edit: tab ? tab.edit : null,
    editorNotice: null,
    ...(tab?.text ? { file: tab.text } : {}),
    ...(tab?.table ? { table: tab.table } : {}),
  };
}

/** The tab of `f`, the open file until now: its live state, and in a pair its last answers. */
function left(s: HiveState, f: FileTab): FileTab {
  const state = { ...f, editing: s.editing, edit: s.edit };
  if (!splitOf(s, fileKey(f))) return state;
  const text = s.file && isFor(s.file, f) ? s.file : f.text;
  return { ...state, text, table: s.table && isFor(s.table, f) ? s.table : f.table };
}

/**
 * The state as a pane showing file `f` reads it (25.1): the store's for the open file; for
 * another (a pane of a pair, without the focus), what its tab keeps. Its view stays as it is when
 * the focus moves in or out: the same values only change place.
 */
export function paneState(s: HiveState, f: OpenFile): HiveState {
  if (s.openFile && isFor(f, s.openFile)) return s;
  const tab = s.openFiles.find((t) => isFor(t, f));
  return {
    ...s,
    openFile: f,
    editing: tab?.editing ?? false,
    edit: tab?.edit ?? null,
    file: tab?.text ?? null,
    table: tab?.table ?? null,
    selectedLines: null,
    editorNotice: null,
  };
}

/** `files` with file `f`'s tab kept (11.1): no longer the preview tab. */
export const kept = (files: FileTab[], f: OpenFile): FileTab[] =>
  files.some((t) => t.preview && isFor(t, f))
    ? files.map((t) => (isFor(t, f) ? { ...t, preview: false } : t))
    : files;

/**
 * Closes file `f`'s tab. When it was shown, the other pane of its pair shows instead, else the
 * tab of its worktree shown most recently (28.19), else the tab beside it in the bar (the right
 * one, else the left one).
 */
export function dropFile(s: HiveState, f: OpenFile): Partial<HiveState> {
  const items = barItems(s);
  const i = items.findIndex((t) => "path" in t && isFor(t, f));
  const rest = items.filter((_, j) => j !== i);
  const split = splitOf(s, fileKey(f));
  const base = {
    openFiles: s.openFiles.filter((t) => !isFor(t, f)),
    tabOrder: s.tabOrder.filter((k) => k !== fileKey(f)),
    pinnedTabs: s.pinnedTabs.filter((k) => k !== fileKey(f)),
    splits: s.splits.filter((p) => p !== split),
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
  // The other pane of its pair (25.1), else the tab its worktree showed last (28.19), else the
  // tab beside it.
  const other = split && (split.left === fileKey(f) ? split.right : split.left);
  const next = !s.fileShown
    ? undefined
    : other !== null
      ? rest.find((t) => paneIdOf(t) === other)
      : (lastShown(s, f.worktree, rest) ?? rest[Math.min(Math.max(i, 0), rest.length - 1)]);
  return { ...closed, ...shownInstead({ ...s, ...closed }, next) };
}

/** What shows `next`, the tab beside one that closed (none: nothing changes). */
export function shownInstead(s: HiveState, next: BarItem | undefined): Partial<HiveState> {
  if (!next) return {};
  if ("path" in next) return opened(s, next, false);
  return { activeTab: next.id };
}

/** The pair tab `id` is in (20.5: one at most), or none. */
export const splitOf = (s: HiveState, id: PaneId | null): Split | null =>
  s.splits.find((p) => p.left === id || p.right === id) ?? null;

/** The tab shown, the focused pane in a pair: the open file while it shows, else the active terminal. */
export function shownPane(s: HiveState): PaneId | null {
  return s.fileShown && s.openFile && fileVisible(s) ? fileKey(s.openFile) : s.activeTab;
}

/** The split shown: the shown tab's pair (that tab is the focused pane), else none. */
export const shownSplit = (s: HiveState): Split | null => splitOf(s, shownPane(s));

/** The tabs the terminal area shows, left to right. */
export function shownPanes(s: HiveState): PaneId[] {
  const split = shownSplit(s);
  if (split) return [split.left, split.right];
  const pane = shownPane(s);
  return pane === null ? [] : [pane];
}

/** The terminals the terminal area shows, left to right. */
export const shownTerminals = (s: HiveState): number[] =>
  shownPanes(s).filter((p) => typeof p === "number");

/** The tab of the file pane `pane` names, if it is an open file's. */
export const paneFile = (s: HiveState, pane: PaneId): FileTab | undefined =>
  s.openFiles.find((f) => fileKey(f) === pane);

/**
 * The file shown in a pane that is not the open file (a pair's other pane, 25.1): its text and
 * table are asked for beside the open file's. At most one: of a pair's two files, the focused
 * one is the open file.
 */
export function asideFile(s: HiveState): FileTab | null {
  const files = shownPanes(s).flatMap((pane) => paneFile(s, pane) ?? []);
  return files.find((f) => !s.openFile || !isFor(f, s.openFile)) ?? null;
}

/** What a tab of the bar is as a pane: a terminal by its id, a file by its key. */
export function paneIdOf(item: BarItem): PaneId {
  return "path" in item ? fileKey(item) : item.id;
}

/** The bar key of pane `pane`. */
export const paneKey = (s: HiveState, pane: PaneId): string =>
  typeof pane === "number" ? barKey(s, { id: pane, cwd: "" }) : pane;

/**
 * The worktree a terminal tab belongs to, as the service placed its cwd (`terminal_opened`;
 * once that worktree is gone, its project's); outside every project, or until the service
 * answers, its cwd.
 */
export function tabWorktree(s: HiveState, tab: Tab): string {
  const place = s.terminals[tab.id]?.worktree ?? tab.cwd;
  // Another host's path, even one a local worktree also has, is no place of the shown host's
  // (26.18): NUL is in no path.
  return onShownHost(s, tab) ? place : `\0${tab.host ?? ""}\0${place}`;
}

/** Whether the tab's terminal runs on the host shown (26.18). */
export const onShownHost = (s: HiveState, tab: Tab): boolean => (tab.host ?? null) === s.host;

/**
 * Whose tabs the tab bar shows: the selected worktree (a selected project stands for its main
 * worktree); a selected agent's terminal's worktree. Null (nothing selected) shows every tab,
 * except in an empty space: none.
 */
export function tabsPlace(s: HiveState): string | null {
  // An empty space has no place, so no tabs (11.5): "" is no tab's worktree.
  const space = s.spaces?.find((x) => x.id === s.currentSpace);
  if (s.selection === null && space?.projects.length === 0) return "";
  const agent = s.agents[s.selection ?? ""];
  if (!agent) return s.selection;
  const tab = s.tabs.find((t) => t.id === agent.terminal);
  return tab ? tabWorktree(s, tab) : agent.worktree;
}

/** The terminal tabs of the place the tab bar shows, in the bar's order. */
export function visibleTabs(s: HiveState): Tab[] {
  const place = tabsPlace(s);
  const tabs = s.tabs.filter((t) => onShownHost(s, t));
  return inBarOrder(s, place === null ? tabs : tabs.filter((t) => tabWorktree(s, t) === place));
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
    if (i < 0 && "id" in item) i = s.tabOrder.indexOf(`tab:${item.id}`);
    return i < 0 ? s.tabOrder.length : i;
  };
  return [...items].sort((a, b) => rank(a) - rank(b));
}

/** The worktree a tab of the bar belongs to. */
export function itemWorktree(s: HiveState, item: BarItem): string {
  return "path" in item ? item.worktree : tabWorktree(s, item);
}

/**
 * The tab `key` (a `barKey`) names when it can be dropped on the panes shown: a terminal or a
 * file of their worktree (20.5, 25.1).
 */
export function droppable(s: HiveState, key: string): PaneId | null {
  const all = [...s.tabs, ...s.openFiles];
  const item = all.find((t) => barKey(s, t) === key);
  const shown = all.find((t) => paneIdOf(t) === shownPane(s));
  return item && shown && itemWorktree(s, item) === itemWorktree(s, shown) ? paneIdOf(item) : null;
}

/** The tabs of the bar, terminals and files mixed, in its order. */
export function barItems(s: HiveState): BarItem[] {
  const place = tabsPlace(s);
  const files = s.openFiles.filter((f) => place === null || f.worktree === place);
  return inBarOrder(s, [...visibleTabs(s), ...files]);
}

/**
 * The key of worktree `place` of the shown host in `tabHistory` and `panelViews` (28.19): two
 * hosts may each have the same path (26.18).
 */
export const placeKey = (s: Pick<HiveState, "host">, place: string) => `${s.host ?? ""}\n${place}`;

/** The keys a tab is remembered by: its bar key, and a terminal's channel too (`inBarOrder`). */
export const keysOf = (s: HiveState, item: BarItem): string[] =>
  "path" in item ? [fileKey(item)] : [barKey(s, item), `tab:${item.id}`];

/**
 * `tabHistory` with the tab shown now (the focused pane) last in its worktree's (28.19), the
 * oldest past `HISTORY_LIMIT` forgotten; unchanged when it is already last, or none shows.
 */
export function withShown(s: HiveState): HiveState["tabHistory"] {
  const pane = shownPane(s);
  const all = [...s.tabs.filter((t) => onShownHost(s, t)), ...s.openFiles];
  const item = pane === null ? undefined : all.find((t) => paneIdOf(t) === pane);
  if (!item) return s.tabHistory;
  const place = placeKey(s, itemWorktree(s, item));
  const [key, channel] = keysOf(s, item) as [string, string?];
  const list = s.tabHistory[place] ?? [];
  if (list.at(-1) === key) return s.tabHistory;
  const rest = list.filter((k) => k !== key && k !== channel);
  return remembered(s.tabHistory, place, [...rest, key].slice(-HISTORY_LIMIT));
}

/**
 * Of `items`, the one worktree `place` showed last (28.19), by its history of shown tabs (this
 * run's, or `history`); none when it showed none of them.
 */
export function lastShown<T extends BarItem>(
  s: HiveState,
  place: string,
  items: T[],
  history = s.tabHistory,
): T | undefined {
  const list = history[placeKey(s, place)] ?? [];
  for (let i = list.length - 1; i >= 0; i--) {
    const item = items.find((t) => keysOf(s, t).includes(list[i] as string));
    if (item) return item;
  }
  return undefined;
}

/** Whether the open file's tab belongs to the place the tab bar shows. */
export function fileVisible(s: HiveState): boolean {
  const place = tabsPlace(s);
  return !!s.openFile && (place === null || s.openFile.worktree === place);
}

/** Whether the tab is pinned (29.2): by any of its keys, a terminal's channel or its session. */
export const isPinned = (s: HiveState, item: BarItem): boolean =>
  keysOf(s, item).some((k) => s.pinnedTabs.includes(k));

/**
 * The tab pinned, or unpinned when it is (29.2). A pinned file is kept (11.1): no longer the
 * preview, as a pinned tab is never replaced either.
 */
export function withPin(s: HiveState, item: BarItem): Partial<HiveState> {
  const keys = keysOf(s, item);
  if (isPinned(s, item)) return { pinnedTabs: s.pinnedTabs.filter((k) => !keys.includes(k)) };
  const pinnedTabs = keys.reduce(withKey, s.pinnedTabs);
  return "path" in item ? { pinnedTabs, openFiles: kept(s.openFiles, item) } : { pinnedTabs };
}

/**
 * The pins with every key of a pinned terminal of the host shown (29.2): a chat resumed after a
 * restart, pinned by its session, gains its channel, so the pin stays once `claude` exits; a
 * terminal pinned before a session started in it gains the session, so the pin is kept between
 * runs. The same list when nothing is missing.
 */
export function pinsFollowed(s: HiveState): string[] {
  let pins = s.pinnedTabs;
  for (const tab of s.tabs.filter((t) => onShownHost(s, t))) {
    const keys = keysOf(s, tab);
    if (keys.some((k) => pins.includes(k))) pins = keys.reduce(withKey, pins);
  }
  return pins;
}

/** The tab of the bar shown now, the focused pane's in a pair; none when nothing shows. */
export function shownTab(s: HiveState): BarItem | undefined {
  const pane = shownPane(s);
  return barItems(s).find((t) => paneIdOf(t) === pane);
}

/** The worktree at `path` and its project, as the service reports them. */
export function worktreeAt(s: HiveState, path: string) {
  for (const project of Object.values(s.projects ?? {})) {
    const worktree = project.worktrees.find((w) => w.id === path);
    if (worktree) return { worktree, project };
  }
  return null;
}

/** What a terminal's tab is called: its agent's session name, else its worktree's name. */
function tabLabel(s: HiveState, tab: Tab): string {
  const agent = Object.values(s.agents).find((a) => a.terminal === tab.id)?.id;
  return (agent && s.agentTitles[agent]) || (worktreeAt(s, tab.cwd)?.worktree.name ?? tab.cwd);
}

/** A file's name, as its tab shows it. */
export const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** What a tab of the bar is called. */
export function itemLabel(s: HiveState, item: BarItem): string {
  if ("path" in item) return fileName(item.path);
  return tabLabel(s, item);
}
