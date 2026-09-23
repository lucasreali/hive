import { expect, test } from "bun:test";
import { act } from "@testing-library/react";

test("mounts the app into #root", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  await act(() => import("./main"));
  expect(document.querySelector("#root header")?.textContent).toBe("Hive");
  document.body.innerHTML = "";
});
