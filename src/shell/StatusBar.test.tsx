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

test("holds no message: a failure or a confirmation shows elsewhere, as a toast (10.3)", () => {
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
