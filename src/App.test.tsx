import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";
import { App } from "./App";
import { apply, initialState, useHive } from "./store";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

test("renders the connection status from the store", () => {
  const { container } = render(<App />);
  const main = container.querySelector("main") as HTMLElement;
  expect(main.dataset.connection).toBe("connecting");
  act(() => apply({ type: "welcome", version: "0.1.0" }));
  expect(main.dataset.connection).toBe("connected");
});
