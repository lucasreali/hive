import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { version } from "../../package.json";
import { asMac } from "../../test/mac";
import { apply } from "../reduce";
import { DEFAULT_SETTINGS, initialState, showNotice, useHive } from "../store";
import { transport } from "../transport";
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

test("shows Windows while the service runs on Windows itself (12.5.4)", () => {
  render(<StatusBar />);
  act(() => apply({ type: "app_mode", mode: "native", wsl: true }));
  expect(screen.getByTitle("Service connection").textContent).toBe("Windowsconnecting");
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(screen.getByRole("contentinfo").textContent).toBe(`Windowsconnectedv${version}`);
  // In WSL mode, its distribution as before.
  act(() => apply({ type: "app_mode", mode: "wsl", wsl: true }));
  act(() => apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" }));
  expect(screen.getByTitle("WSL connection").textContent).toBe("WSL: Ubuntuconnected");
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
    apply({ type: "disconnected", reason: "gone", bundled: false });
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

test("with an account besides the default one, a select picks the current account (12.2)", () => {
  const set = spyOn(transport, "setSettings").mockResolvedValue();
  render(<StatusBar />);
  const select = () => screen.queryByRole("combobox", { name: "Claude account" });
  expect(select()).toBeNull();
  const claude = { accounts: [{ name: "Work", config_dir: "/w" }], account: "/w" };
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude } }));
  expect(select()?.textContent).toBe("Work");
  // Its session usage (12.1) sits right after it.
  act(() => apply({ type: "session_usage", usage: { used_percentage: 7, resets_at: 1 } }));
  const group = select()?.closest(".statusbar-account");
  expect(group?.lastElementChild?.className).toBe("session-usage");
  // Next to the version, which stays last.
  expect(screen.getByRole("contentinfo").lastElementChild?.textContent).toBe(`v${version}`);
  fireEvent.mouseDown(select() as HTMLElement);
  fireEvent.click(screen.getByRole("option", { name: "Default" }));
  const account = null;
  expect(set).toHaveBeenCalledWith({ ...DEFAULT_SETTINGS, claude: { ...claude, account } });
  act(() =>
    apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: { ...claude, account } } }),
  );
  expect(select()?.textContent).toBe("Default");
  fireEvent.mouseDown(select() as HTMLElement);
  fireEvent.click(screen.getByRole("option", { name: "Work" }));
  expect(set).toHaveBeenLastCalledWith({ ...DEFAULT_SETTINGS, claude });
  set.mockRestore();
});
