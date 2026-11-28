import type { DiffBase, Settings } from "./protocol";
import type { HiveState, PanelView, SpacePlace } from "./store";

// The window's own preferences, kept in its `localStorage` between runs (not service data, #37).

/** How a side panel may be sized, in pixels. */
export const LIMITS = {
  sidebar: { min: 200, max: 480 },
  panel: { min: 280, max: 640 },
  /** The split's left pane, in percent of the terminal area. */
  split: { min: 20, max: 80 },
} as const;
/** Dragged this narrow, the right panel closes instead. */
export const PANEL_CLOSE_AT = 200;
export type Side = keyof typeof LIMITS;
const KEYS = { sidebar: "sidebarWidth", panel: "panelWidth", split: "splitPercent" } as const;
export const widthKey = (side: Side) => KEYS[side];

/** A width kept within the side's limits. */
export const clampWidth = (side: Side, width: number) =>
  Math.round(Math.max(LIMITS[side].min, Math.min(LIMITS[side].max, width)));

/** Where the widths are remembered between runs (a per-window preference, not service data). */
const STORAGE = "hive.widths";

/** The widths remembered from the last run, within the limits; the defaults otherwise. */
export function savedWidths(storage: Pick<Storage, "getItem"> | null = safeStorage()) {
  try {
    const saved = JSON.parse(storage?.getItem(STORAGE) ?? "{}");
    return {
      sidebarWidth: clampWidth("sidebar", Number(saved.sidebarWidth) || 264),
      panelWidth: clampWidth("panel", Number(saved.panelWidth) || 380),
      splitPercent: clampWidth("split", Number(saved.splitPercent) || 50),
    };
  } catch {
    return { sidebarWidth: 264, panelWidth: 380, splitPercent: 50 };
  }
}

export function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** Whether the right panel's view is open and the sidebar shows (28.27), kept with the widths. */
export type Layout = Pick<HiveState, "rightPanel" | "sidebarShown">;

/** The layout of the last run: both open unless closed then (and when none or unreadable). */
export function savedLayout(storage: Pick<Storage, "getItem"> | null = safeStorage()): Layout {
  try {
    const saved = JSON.parse(storage?.getItem(STORAGE) ?? "{}");
    return {
      rightPanel: saved.rightPanel === null ? null : "files",
      sidebarShown: saved.sidebarShown !== false,
    };
  } catch {
    return { rightPanel: "files", sidebarShown: true };
  }
}

/** Remembers the widths of `s`, and its layout (28.27), between runs. */
export function saveWidths(
  s: Pick<HiveState, "sidebarWidth" | "panelWidth" | "splitPercent"> & Layout,
) {
  const { sidebarWidth, panelWidth, splitPercent, rightPanel, sidebarShown } = s;
  const saved = { sidebarWidth, panelWidth, splitPercent, rightPanel, sidebarShown };
  try {
    safeStorage()?.setItem(STORAGE, JSON.stringify(saved));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/** Remembers the layout whenever the right panel opens or closes or the sidebar shows or hides. */
export function saveLayout(s: HiveState, prev: HiveState): void {
  if (s.rightPanel !== prev.rightPanel || s.sidebarShown !== prev.sidebarShown) saveWidths(s);
}

/** Where "Unseen only" (27.1) is remembered between runs (28.27): one switch for every worktree. */
const UNSEEN_STORAGE = "hive.unseenOnly";

/** Whether the Diff view listed only the files not seen in the last run; off when unreadable. */
export function savedUnseenOnly(storage: Pick<Storage, "getItem"> | null = safeStorage()): boolean {
  try {
    return storage?.getItem(UNSEEN_STORAGE) === "true";
  } catch {
    return false;
  }
}

/** Remembers "Unseen only" whenever it changes. */
export function saveUnseenOnly(s: HiveState, prev: HiveState): void {
  if (s.unseenOnly === prev.unseenOnly) return;
  try {
    safeStorage()?.setItem(UNSEEN_STORAGE, String(s.unseenOnly));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/** Where the agents' order is remembered between runs (a per-window preference, #37). */
const ORDER_STORAGE = "hive.agentOrder";
/** How many session ids are remembered; the oldest moves are forgotten first. */
export const ORDER_LIMIT = 500;

/** The agents' order remembered from the last run; empty when none or unreadable. */
export function savedAgentOrder(
  storage: Pick<Storage, "getItem"> | null = safeStorage(),
): string[] {
  try {
    const saved: unknown = JSON.parse(storage?.getItem(ORDER_STORAGE) ?? "[]");
    return Array.isArray(saved)
      ? saved.filter((id): id is string => typeof id === "string").slice(0, ORDER_LIMIT)
      : [];
  } catch {
    return [];
  }
}

/** Remembers the agents' order whenever it changes. */
export function saveAgentOrder(s: HiveState, prev: HiveState): void {
  if (s.agentOrder === prev.agentOrder) return;
  try {
    safeStorage()?.setItem(ORDER_STORAGE, JSON.stringify(s.agentOrder));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/** Where the tab bar's order is remembered between runs (a per-window preference, #37). */
const TAB_ORDER_STORAGE = "hive.tabOrder";

/**
 * The tab bar's order remembered from the last run, empty when none or unreadable. A plain
 * terminal's key names a channel of that run only, so only files and sessions are kept; nor a
 * `browser:` key, of the agent's browser's tab an install before 26.11 remembers.
 */
export function savedTabOrder(storage: Pick<Storage, "getItem"> | null = safeStorage()): string[] {
  try {
    const saved: unknown = JSON.parse(storage?.getItem(TAB_ORDER_STORAGE) ?? "[]");
    return Array.isArray(saved)
      ? saved
          .filter(
            (k): k is string =>
              typeof k === "string" && !k.startsWith("tab:") && !k.startsWith("browser:"),
          )
          .slice(-ORDER_LIMIT)
      : [];
  } catch {
    return [];
  }
}

/** Remembers the tab bar's order whenever it changes. */
export function saveTabOrder(s: HiveState, prev: HiveState): void {
  if (s.tabOrder === prev.tabOrder) return;
  try {
    safeStorage()?.setItem(TAB_ORDER_STORAGE, JSON.stringify(s.tabOrder));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/** Where the theme setting is remembered between runs, for the launch's colour (18.33). */
const THEME_STORAGE = "hive.theme";
type Theme = Settings["appearance"]["theme"];
const THEMES: readonly Theme[] = ["one-dark", "one-light", "system"];

/** The theme setting of the last run; One Dark (the default) when none or unknown. */
export function savedTheme(storage: Pick<Storage, "getItem"> | null = safeStorage()): Theme {
  try {
    const saved = storage?.getItem(THEME_STORAGE);
    return THEMES.find((t) => t === saved) ?? "one-dark";
  } catch {
    return "one-dark";
  }
}

/** Remembers the theme setting whenever it changes. */
export function saveTheme(s: HiveState, prev: HiveState): void {
  const theme = s.settings.appearance.theme;
  if (theme === prev.settings.appearance.theme) return;
  try {
    safeStorage()?.setItem(THEME_STORAGE, theme);
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/**
 * Where each worktree was left (28.19), remembered between runs beside the tab order (#57, a
 * per-window preference, #37): the bar keys of the tabs it showed, the most recent last, and
 * its right panel's view. Keyed by `placeKey` (tabs.ts).
 */
const PLACES_STORAGE = "hive.places";
/** The right panel's views a saved place may name. */
const VIEWS: readonly PanelView[] = [
  "files",
  "changes",
  "comments",
  "sessions",
  "pulls",
  "actions",
  "scripts",
];
/** How many shown tabs each worktree remembers; the oldest are forgotten first. */
export const HISTORY_LIMIT = 20;
/** How many worktrees' places are remembered; the least recently changed are forgotten first. */
export const PLACES_LIMIT = 200;

export type SavedPlaces = Pick<HiveState, "tabHistory" | "panelViews">;

/** `record` with `key` set to `value` and moved last, the oldest keys past the limit dropped. */
export function remembered<T>(record: Record<string, T>, key: string, value: T): Record<string, T> {
  const { [key]: _, ...rest } = record;
  return Object.fromEntries([...Object.entries(rest).slice(1 - PLACES_LIMIT), [key, value]]);
}

/** A plain terminal's key names a channel of one run only (as in `savedTabOrder`). */
const lasting = (key: unknown): key is string =>
  typeof key === "string" && !key.startsWith("tab:") && !key.startsWith("browser:");

/**
 * The places remembered from the last run, none when unreadable: files' and sessions' tabs only,
 * as for the tab order, and only the views the panel has.
 */
export function savedPlaces(storage: Pick<Storage, "getItem"> | null = safeStorage()): SavedPlaces {
  try {
    const saved: Partial<Record<keyof SavedPlaces, unknown>> =
      JSON.parse(storage?.getItem(PLACES_STORAGE) ?? "{}") ?? {};
    const entries = (record: unknown) =>
      typeof record === "object" && record !== null
        ? Object.entries(record).slice(-PLACES_LIMIT)
        : [];
    const tabs = entries(saved.tabHistory).flatMap(([place, keys]): [string, string[]][] => {
      const kept = Array.isArray(keys) ? keys.filter(lasting).slice(-HISTORY_LIMIT) : [];
      return kept.length > 0 ? [[place, kept]] : [];
    });
    const views = entries(saved.panelViews).filter((e): e is [string, PanelView] =>
      VIEWS.includes(e[1] as PanelView),
    );
    return { tabHistory: Object.fromEntries(tabs), panelViews: Object.fromEntries(views) };
  } catch {
    return { tabHistory: {}, panelViews: {} };
  }
}

/** Remembers the places whenever they change. */
export function savePlaces(s: HiveState, prev: HiveState): void {
  if (s.tabHistory === prev.tabHistory && s.panelViews === prev.panelViews) return;
  const { tabHistory, panelViews } = s;
  try {
    safeStorage()?.setItem(PLACES_STORAGE, JSON.stringify({ tabHistory, panelViews }));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/** Where each space's place (11.5) is remembered between runs (28.27), by the store's key. */
const SPACE_PLACES_STORAGE = "hive.spacePlaces";

/** Whether `value` is a place a space may name: a place and its project, or none (null). */
const isSpacePlace = (value: unknown): value is SpacePlace | null => {
  if (value === null) return true;
  const { place, project } = (typeof value === "object" ? value : {}) as Partial<SpacePlace>;
  return typeof place === "string" && (project === null || typeof project === "string");
};

/** The spaces' places of the last run, the most recent `PLACES_LIMIT`; none when unreadable. */
export function savedSpacePlaces(
  storage: Pick<Storage, "getItem"> | null = safeStorage(),
): HiveState["spacePlaces"] {
  try {
    const saved: unknown = JSON.parse(storage?.getItem(SPACE_PLACES_STORAGE) ?? "{}");
    if (typeof saved !== "object" || saved === null) return {};
    const places = Object.entries(saved).filter((e) => isSpacePlace(e[1]));
    return Object.fromEntries(places.slice(-PLACES_LIMIT));
  } catch {
    return {};
  }
}

/** Remembers the spaces' places, the current space's included (`leaveSpace`), the most recent. */
export function saveSpacePlaces(places: HiveState["spacePlaces"]): void {
  const kept = Object.fromEntries(Object.entries(places).slice(-PLACES_LIMIT));
  try {
    safeStorage()?.setItem(SPACE_PLACES_STORAGE, JSON.stringify(kept));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/**
 * Where each host's tabs and trees were left (28.27, rows 4, 5, 6, 9 and 10 of
 * `docs/review/last-place.md`), remembered between runs by `hostKey` ("" for this Mac): its open
 * files (without their edits), its pairs of files and chats by their bar keys, each worktree's
 * diff base, the trees' collapsed projects and worktrees and open folders, and its pinned tabs
 * (29.2) by their bar keys.
 */
const HOST_PLACES_STORAGE = "hive.hostPlaces";
/** How many hosts are remembered. */
export const HOSTS_LIMIT = 50;
/** How many open files each host reopens. */
export const FILES_LIMIT = 100;
/** How many pairs each host remembers. */
export const PAIRS_LIMIT = 50;
/** How many diff bases each host remembers. */
export const BASES_LIMIT = 200;
/** How many collapsed or open tree nodes each host remembers; the oldest are forgotten first. */
export const COLLAPSED_LIMIT = 1000;

/** An open file's tab as it reopens: where it is, and how it showed. */
export type SavedFile = {
  worktree: string;
  path: string;
  preview?: boolean;
  rendered?: boolean;
  asText?: boolean;
};
/** One host's places. */
export type HostPlaces = {
  files: SavedFile[];
  pairs: [string, string][];
  diffBases: HiveState["diffBases"];
  collapsed: HiveState["collapsed"];
  /** The pinned tabs (29.2): files' and chats' bar keys (a plain terminal's is one run's). */
  pins: string[];
};
export const NO_HOST_PLACES: HostPlaces = {
  files: [],
  pairs: [],
  diffBases: {},
  collapsed: {},
  pins: [],
};

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** A saved file, its flags only when true; none when it names no worktree and path. */
function savedFile(value: unknown): SavedFile[] {
  if (!isObject(value) || typeof value.worktree !== "string" || typeof value.path !== "string") {
    return [];
  }
  const flags = (["preview", "rendered", "asText"] as const).filter((f) => value[f] === true);
  const file: SavedFile = { worktree: value.worktree, path: value.path };
  for (const flag of flags) file[flag] = true;
  return [file];
}

/** A pair's or a pin's key: a file's or a chat's (a plain terminal's channel is one run's). */
const pairKey = (key: unknown): key is string =>
  typeof key === "string" && (key.startsWith("file:") || key.startsWith("session:"));

/** A diff base as `DiffBase` holds it; none for anything else. */
function savedBase(value: unknown): DiffBase | undefined {
  if (value === "head" || value === "branch") return value;
  if (!isObject(value)) return undefined;
  const { turn, commit } = value;
  if (isObject(turn) && typeof turn.session === "string")
    return { turn: { session: turn.session } };
  if (isObject(commit) && typeof commit.id === "string") return { commit: { id: commit.id } };
  return undefined;
}

/** One host's places as saved, what cannot be read left out. */
function hostPlaces(value: unknown): HostPlaces {
  if (!isObject(value)) return NO_HOST_PLACES;
  const list = (v: unknown) => (Array.isArray(v) ? v : []);
  const record = (v: unknown) => Object.entries(isObject(v) ? v : {});
  const pairs = list(value.pairs).filter(
    (p): p is [string, string] => Array.isArray(p) && p.length === 2 && p.every(pairKey),
  );
  const bases = record(value.diffBases).flatMap(([path, v]): [string, DiffBase][] => {
    const base = savedBase(v);
    return base ? [[path, base]] : [];
  });
  const collapsed = record(value.collapsed).filter(
    (e): e is [string, boolean] => typeof e[1] === "boolean",
  );
  return {
    files: list(value.files).flatMap(savedFile).slice(-FILES_LIMIT),
    pairs: pairs.slice(-PAIRS_LIMIT),
    diffBases: Object.fromEntries(bases.slice(-BASES_LIMIT)),
    collapsed: Object.fromEntries(collapsed.slice(-COLLAPSED_LIMIT)),
    pins: [...new Set(list(value.pins).filter(pairKey))].slice(-ORDER_LIMIT),
  };
}

/** Each host's places of the last run, by `hostKey`; none when unreadable. */
export function savedHostPlaces(
  storage: Pick<Storage, "getItem"> | null = safeStorage(),
): Record<string, HostPlaces> {
  try {
    const saved: unknown = JSON.parse(storage?.getItem(HOST_PLACES_STORAGE) ?? "{}");
    const hosts = Object.entries(isObject(saved) ? saved : {}).slice(-HOSTS_LIMIT);
    return Object.fromEntries(hosts.map(([host, places]) => [host, hostPlaces(places)]));
  } catch {
    return {};
  }
}

/** The hosts' places as they are written (each list capped as when read). */
const hostsText = (hosts: Record<string, HostPlaces>): string =>
  JSON.stringify(
    Object.fromEntries(
      Object.entries(hosts)
        .slice(-HOSTS_LIMIT)
        .map(([host, places]) => [host, hostPlaces(places)]),
    ),
  );

/** The hosts' places last written, or found at launch: the same places are not written again. */
let hostsWritten = "";

/** Takes `hosts` as the places already saved (at launch: those just read). */
export function hostPlacesSaved(hosts: Record<string, HostPlaces>): void {
  hostsWritten = hostsText(hosts);
}

/** Remembers each host's places (each list capped as when read); the same places write nothing. */
export function saveHostPlaces(hosts: Record<string, HostPlaces>): void {
  const text = hostsText(hosts);
  if (text === hostsWritten) return;
  hostsWritten = text;
  try {
    safeStorage()?.setItem(HOST_PLACES_STORAGE, text);
  } catch {
    // A full or blocked storage only loses the preference.
  }
}
