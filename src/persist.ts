import type { HiveState } from "./store";

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

/** Remembers the widths of `s` between runs. */
export function saveWidths(s: Pick<HiveState, "sidebarWidth" | "panelWidth" | "splitPercent">) {
  const { sidebarWidth, panelWidth, splitPercent } = s;
  try {
    safeStorage()?.setItem(STORAGE, JSON.stringify({ sidebarWidth, panelWidth, splitPercent }));
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
 * terminal's key names a channel of that run only, so only files and sessions are kept.
 */
export function savedTabOrder(storage: Pick<Storage, "getItem"> | null = safeStorage()): string[] {
  try {
    const saved: unknown = JSON.parse(storage?.getItem(TAB_ORDER_STORAGE) ?? "[]");
    return Array.isArray(saved)
      ? saved
          .filter((k): k is string => typeof k === "string" && !k.startsWith("tab:"))
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
