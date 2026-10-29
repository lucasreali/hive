import { afterEach, expect, spyOn, test } from "bun:test";
import { stubStorage } from "../test/storage";
import { savedTheme } from "./persist";
import { appliedTheme, DEFAULT_SETTINGS, initialState, setSystemTheme, useHive } from "./store";
import { watchColorScheme } from "./window";

// 18.33: a System theme following the system's appearance, and the last theme kept for launch.

afterEach(() => useHive.setState(initialState, true));

const withTheme = (theme: "one-dark" | "one-light" | "system") => ({
  ...DEFAULT_SETTINGS,
  appearance: { theme },
});

test("the theme shown is the setting's, or the system's for System", () => {
  const s = (theme: "one-dark" | "one-light" | "system", systemTheme: "one-dark" | "one-light") =>
    appliedTheme({ settings: withTheme(theme), systemTheme });
  expect(s("one-light", "one-dark")).toBe("one-light");
  expect(s("one-dark", "one-light")).toBe("one-dark");
  expect(s("system", "one-light")).toBe("one-light");
  expect(s("system", "one-dark")).toBe("one-dark");
  setSystemTheme("one-light");
  expect(useHive.getState().systemTheme).toBe("one-light");
});

test("the theme setting is remembered whenever it changes, and read back at launch", () => {
  expect(savedTheme()).toBe("one-dark");
  useHive.setState({ settings: withTheme("one-light") });
  expect(localStorage.getItem("hive.theme")).toBe("one-light");
  expect(savedTheme()).toBe("one-light");
  useHive.setState({ settings: withTheme("system") });
  expect(savedTheme()).toBe("system");
  // Other settings changing leave it alone.
  localStorage.setItem("hive.theme", "one-dark");
  useHive.setState({ settings: { ...withTheme("system"), worktrees: { default_base: "x" } } });
  expect(localStorage.getItem("hive.theme")).toBe("one-dark");
  // Unknown or unreadable: the default.
  localStorage.setItem("hive.theme", "solarized");
  expect(savedTheme()).toBe("one-dark");
  expect(savedTheme(null)).toBe("one-dark");
  const broken = {
    getItem: () => {
      throw new Error("blocked");
    },
  };
  expect(savedTheme(broken)).toBe("one-dark");
  // A full or blocked storage only loses the preference.
  const full = stubStorage(true);
  expect(() => useHive.setState({ settings: withTheme("one-light") })).not.toThrow();
  expect(full).toHaveBeenLastCalledWith("hive.theme", "one-light");
});

test("the system's appearance is followed live (prefers-color-scheme)", () => {
  const listeners = new Set<() => void>();
  const query = {
    matches: true,
    addEventListener: (_: string, f: () => void) => listeners.add(f),
    removeEventListener: (_: string, f: () => void) => listeners.delete(f),
  };
  const media = spyOn(window, "matchMedia").mockImplementation(
    () => query as unknown as MediaQueryList,
  );
  try {
    const seen: string[] = [];
    const stop = watchColorScheme((t) => seen.push(t));
    expect(media).toHaveBeenCalledWith("(prefers-color-scheme: dark)");
    query.matches = false;
    for (const f of listeners) f();
    stop();
    expect(listeners.size).toBe(0);
    expect(seen).toEqual(["one-dark", "one-light"]);
  } finally {
    media.mockRestore();
  }
});
