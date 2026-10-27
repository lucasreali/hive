import { expect, spyOn, test } from "bun:test";
import { act } from "@testing-library/react";
import { version } from "../package.json";
import { DEFAULT_SETTINGS, initialState, useHive } from "./store";

test("mounts the app into #root", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  // The last run's System theme, in a light system: the page starts light (18.33).
  useHive.setState({ settings: { ...DEFAULT_SETTINGS, appearance: { theme: "system" } } });
  const media = spyOn(window, "matchMedia");
  const { root } = await act(() => import("./main"));
  expect(media).toHaveBeenCalledWith("(prefers-color-scheme: dark)");
  media.mockRestore();
  expect(document.documentElement.dataset.theme).toBe("one-light");
  // Off macOS the custom scrollbars and pointer stay (18.36).
  expect(document.documentElement.dataset.mac).toBeUndefined();
  expect(document.querySelector("#root header")?.textContent).toBe("Hive");
  // Outside Tauri the mock transport connects.
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  const footer = document.querySelector("#root footer")?.textContent;
  expect(footer).toStartWith("WSL: UbuntuconnectedSession 42% · resets ");
  expect(footer).toEndWith(`v${version}`);
  // Unmounted, so this app does not keep reacting to the store in later tests.
  act(() => root.unmount());
  document.body.innerHTML = "";
  useHive.setState(initialState, true);
});
