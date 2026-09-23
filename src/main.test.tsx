import { expect, test } from "bun:test";
import { act } from "@testing-library/react";
import { initialState, useHive } from "./store";

test("mounts the app into #root", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  const { root } = await act(() => import("./main"));
  expect(document.querySelector("#root header")?.textContent).toBe("Hive");
  // Outside Tauri the mock transport connects.
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(document.querySelector("#root footer")?.textContent).toBe("WSL: Ubuntuconnected");
  // Unmounted, so this app does not keep reacting to the store in later tests.
  act(() => root.unmount());
  document.body.innerHTML = "";
  useHive.setState(initialState, true);
});
