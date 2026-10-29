import { mock } from "bun:test";

/**
 * Makes the window's `localStorage` one whose `setItem` is the returned mock (writing nothing,
 * or failing as `fails` says) until the test ends (setup.ts); its `clear` is the real storage's
 * (the tests' `afterEach`). A spy on `Storage.prototype` is not enough: once happy-dom's `setItem` has run,
 * its calls no longer reach the spy.
 */
export function stubStorage(fails = false) {
  const real = localStorage;
  const setItem = mock((_key: string, _value: string) => {
    if (fails) throw new Error("full");
  });
  const stub = { setItem, clear: () => real.clear() };
  Object.defineProperty(window, "localStorage", { get: () => stub, configurable: true });
  return setItem;
}
