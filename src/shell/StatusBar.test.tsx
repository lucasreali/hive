import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render, screen } from "@testing-library/react";
import { version } from "../../package.json";
import { asMac } from "../../test/mac";
import { apply } from "../reduce";
import { initialState, showNotice, useHive } from "../store";
import { StatusBar } from "./StatusBar";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

test("shows the app version at the right end of the status line", () => {
  render(<StatusBar />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" }));
  const bar = screen.getByRole("contentinfo");
  expect(bar.lastElementChild?.textContent).toBe(`v${version}`);
  expect(bar.textContent).toBe(`WSL: Ubuntuconnectedv${version}`);
});

test("shows the version on macOS without the WSL part", () => {
  asMac();
  render(<StatusBar />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(screen.getByRole("contentinfo").textContent).toBe(`macOSconnectedv${version}`);
});

test("shows the current account's session usage with its reset in local time (12.1)", () => {
  render(<StatusBar />);
  const resets_at = new Date(2026, 8, 28, 14, 30).getTime() / 1000;
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" });
    apply({ type: "session_usage", usage: { used_percentage: 42, resets_at } });
  });
  const usage = screen.getByTitle("Current session (5-hour limit): 42% used, resets at 14:30");
  expect(usage.textContent).toBe("Session 42% · resets 14:30");
  expect(screen.getByRole("contentinfo").textContent).toBe(
    `WSL: UbuntuconnectedSession 42% · resets 14:30v${version}`,
  );
  // None once it reset, and none from a service that is gone.
  act(() => apply({ type: "session_usage", usage: null }));
  expect(screen.getByRole("contentinfo").textContent).toBe(`WSL: Ubuntuconnectedv${version}`);
  act(() => {
    apply({ type: "session_usage", usage: { used_percentage: 7, resets_at } });
    apply({ type: "disconnected", reason: "gone" });
  });
  expect(screen.getByRole("contentinfo").textContent).toBe(`WSLdisconnectedv${version}`);
});

test("holds no message:a failure or a confirmation shows elsewhere, as a toast (10.3)", () => {
  render(<StatusBar />);
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" });
    apply({ type: "notice", message: "No GitHub token for me" });
    showNotice("info", "Copied /w");
  });
  expect(useHive.getState().notices).toHaveLength(2);
  expect(screen.getByRole("contentinfo").textContent).toBe(`WSL: Ubuntuconnectedv${version}`);
});
