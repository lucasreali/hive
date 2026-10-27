// Timed refreshes that pause while the window is hidden (D18.11).

import { useEffect, useRef } from "react";
import { useHive } from "./store";

/**
 * Calls `ask` every `ms` while the window is visible (null: never), and once as soon as it shows
 * again; `now` also calls it once at start. Starts again when `deps` change. A window merely out
 * of focus keeps refreshing.
 */
export function useRefresh(ask: () => void, ms: number | null, now: boolean, deps: unknown[]) {
  const visible = useHive((s) => s.visible);
  const missed = useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `deps` say when to start again.
  useEffect(() => {
    if (ms === null) return;
    if (!visible) {
      missed.current = true;
      return;
    }
    if (now || missed.current) ask();
    missed.current = false;
    const timer = setInterval(ask, ms);
    return () => clearInterval(timer);
  }, [visible, ms, ...deps]);
}
