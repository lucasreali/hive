import type { Settings } from "./protocol";
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
