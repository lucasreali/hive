import {
  type RefObject,
  type UIEvent,
  useCallback,
  useLayoutEffect,
  useReducer,
  useRef,
} from "react";
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

/** Forgets every view's place and every kept choice (the tests start each from none). */
export const forgetViewPlaces = () => {
  places.clear();
  choices.clear();
};

/**
 * Choices kept for this run only (28.27, UI state, #37), by key: a view's search or filter per
 * worktree (`viewKey`), the Settings section, the space switcher's open hosts.
 */
const choices = new Map<string, unknown>();
/** How many choices are kept; the least recently changed are forgotten first. */
export const CHOICES_LIMIT = 1000;

/** Keeps `value` as choice `key`, the least recently changed past the limit forgotten. */
function keepChoice(key: string, value: unknown): void {
  choices.delete(key);
  choices.set(key, value);
  for (const old of choices.keys()) {
    if (choices.size <= CHOICES_LIMIT) break;
    choices.delete(old);
  }
}

/** Choice `key`, `initial` until one was made. */
const choice = <T>(key: string, initial: T): T =>
  choices.has(key) ? (choices.get(key) as T) : initial;

/**
 * A choice kept for this run as `key` (28.27): its value, `initial` until one was made, and its
 * setter (a value, or a function of the current one). Another `key` (another worktree) reads
 * that one's choice; a component shown again finds the choice it was left with.
 */
export function useKept<T>(key: string, initial: T): [T, (next: T | ((was: T) => T)) => void] {
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const set = useCallback(
    (next: T | ((was: T) => T)) => {
      const was = choice(key, initial);
      keepChoice(key, typeof next === "function" ? (next as (was: T) => T)(was) : next);
      rerender();
    },
    [key, initial],
  );
  return [choice(key, initial), set];
}

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
