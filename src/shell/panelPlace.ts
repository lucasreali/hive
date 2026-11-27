import { type RefObject, type UIEvent, useLayoutEffect, useRef } from "react";
import { PLACES_LIMIT } from "../persist";

// Where each right-panel view of each worktree was left (28.19), this run only (UI state, #37):
// its scroll, and a tree's active row. Coming back to the worktree or the view puts them back.

/** A view's place: its scroll in pixels, and a tree's active row by its key and index. */
export type ViewPlace = { top: number; row?: string; index?: number };

const places = new Map<string, ViewPlace>();

/** The key of `view`'s place in `worktree` of `host` (two hosts may have the same path, 26.18). */
export const viewKey = (view: string, host: string | null, worktree: string) =>
  `${view}\n${host ?? ""}\n${worktree}`;

/** The place view `key` was left at, if it was. */
export const viewPlace = (key: string): ViewPlace | undefined => places.get(key);

/** Remembers `change` of view `key`'s place, the least recently changed past the limit forgotten. */
export function keepViewPlace(key: string, change: Partial<ViewPlace>): void {
  const place = { top: 0, ...places.get(key), ...change };
  places.delete(key);
  places.set(key, place);
  for (const old of places.keys()) {
    if (places.size <= PLACES_LIMIT) break;
    places.delete(old);
  }
}

/** Forgets every view's place (the tests start each from none). */
export const forgetViewPlaces = () => places.clear();

/**
 * Puts back the scroll of the element `ref` holds as view `key` was left, when that view shows
 * again: once `ready` (its rows are listed, so the saved scroll fits), and whenever `key`
 * changes (another worktree in the same view). Returns the element's `onScroll`, which keeps
 * the scroll as `key`'s; until it was put back nothing is kept, so the previous worktree's
 * scroll never becomes this one's.
 */
export function useKeptScroll(
  key: string,
  ref: RefObject<HTMLElement | null>,
  ready: boolean,
): (event: UIEvent<HTMLElement>) => void {
  const restored = useRef<string | null>(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element || !ready || restored.current === key) return;
    restored.current = key;
    element.scrollTop = viewPlace(key)?.top ?? 0;
  }, [key, ready, ref]);
  return (event) => {
    if (restored.current === key) keepViewPlace(key, { top: event.currentTarget.scrollTop });
  };
}
