import { expect, test } from "bun:test";
import { act } from "@testing-library/react";

test("mounts the app into #root", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  await act(() => import("./main"));
  expect(document.querySelector("#root main")?.textContent).toBe("Hive");
});
