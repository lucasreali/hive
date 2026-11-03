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

test("shows the current account's session usage as a ring, with its reset in local time (12.1, 15.3)", () => {
  render(<StatusBar />);
  const resets_at = new Date(2026, 8, 28, 14, 30).getTime() / 1000;
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" });
    apply({ type: "session_usage", usage: { used_percentage: 42, resets_at }, week: null });
  });
  // No text in the bar: the ring, its tooltip and its accessible name.
  const ring = screen.getByRole("button", { name: "Session 42% · resets 14:30" });
  expect(ring.querySelector(".usage-arc")?.getAttribute("stroke-dasharray")).toBe("42 100");
  expect(ring.querySelector(".usage-arc")?.getAttribute("pathLength")).toBe("100");
  expect(ring.querySelector(".usage-tip")?.textContent).toBe("Session 42% · resets 14:30");
  expect(ring.hasAttribute("data-warning")).toBe(false);
  // None once it reset, and none from a service that is gone.
  act(() => apply({ type: "session_usage", usage: null, week: null }));
  expect(screen.queryByRole("button")).toBeNull();
  expect(screen.getByRole("contentinfo").textContent).toBe(`WSL: Ubuntuconnectedv${version}`);
  act(() => {
    apply({ type: "session_usage", usage: { used_percentage: 7, resets_at } });
    apply({ type: "disconnected", reason: "gone", bundled: false });
  });
  expect(screen.getByRole("contentinfo").textContent).toBe(`WSLdisconnectedv${version}`);
});

test("the ring's tooltip adds the weekly window when the service sends one (15.3)", () => {
  render(<StatusBar />);
  const resets_at = new Date(2026, 8, 28, 14, 30).getTime() / 1000;
  // A Monday.
  const week = { used_percentage: 41, resets_at: new Date(2026, 9, 5, 9, 0).getTime() / 1000 };
  act(() => apply({ type: "session_usage", usage: { used_percentage: 80, resets_at }, week }));
  const lines = ["Session 80% · resets 14:30", "Week 41% · resets Mon 09:00"];
  const ring = screen.getByRole("button", { name: lines.join(", ") });
  const tip = ring.querySelector(".usage-tip");
  expect([...(tip?.children ?? [])].map((line) => line.textContent)).toEqual(lines);
  // The warning colour from 80%.
  expect(ring.hasAttribute("data-warning")).toBe(true);
  act(() => apply({ type: "session_usage", usage: { used_percentage: 79, resets_at }, week }));
  expect(ring.hasAttribute("data-warning")).toBe(false);
  // An older service sends no week: the session alone.
  act(() => apply({ type: "session_usage", usage: { used_percentage: 100, resets_at } }));
  expect(ring.getAttribute("aria-label")).toBe("Session 100% · resets 14:30");
  expect(ring.querySelector(".usage-arc")?.getAttribute("stroke-dasharray")).toBe("100 100");
  expect(useHive.getState().weekUsage).toBeNull();
});

test("the ring's tooltip adds the pace the service forecasts (17.23)", () => {
  render(<StatusBar />);
  const time = (day: number, hour: number, minute = 0) =>
    new Date(2026, 8, day, hour, minute).getTime() / 1000;
  const usage = { used_percentage: 60, resets_at: time(28, 14, 30) };
  const week = { used_percentage: 70, resets_at: time(28, 9) };
  const paced = { limit_at: time(28, 13, 50), week_limit_at: time(27, 18) };
  act(() => apply({ type: "session_usage", usage, week, ...paced }));
  const lines = [
    "Session 60% · resets 14:30",
    "At this pace: limit at 13:50",
    "Week 70% · resets Mon 09:00",
    "At this pace: limit Sun 18:00",
  ];
  const ring = screen.getByRole("button", { name: lines.join(", ") });
  const tip = () => [...(ring.querySelector(".usage-tip")?.children ?? [])];
  expect(tip().map((line) => line.textContent)).toEqual(lines);
  // Lasting until the reset (or an older service): no pace; a service that is gone, none.
  act(() => apply({ type: "session_usage", usage, week, limit_at: null }));
  expect(tip().map((line) => line.textContent)).toEqual([lines[0], lines[2]]);
  act(() => {
    apply({ type: "session_usage", usage, week, ...paced });
    apply({ type: "disconnected", reason: "gone", bundled: false });
  });
  expect(useHive.getState().usageLimits).toEqual({ session: null, week: null });
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
  const claude = {
    accounts: [{ name: "Work", config_dir: "/w" }],
    account: "/w",
    default_name: "Default",
  };
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
  // The default account renamed (21.4) shows its name.
  const personal = { ...claude, account, default_name: "Personal" };
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: personal } }));
  expect(select()?.textContent).toBe("Personal");
  fireEvent.mouseDown(select() as HTMLElement);
  expect(screen.getByRole("option", { name: "Personal" })).toBeTruthy();
  set.mockRestore();
});

test("whether each account logged in is asked once connected, for new accounts and on focus (21.6)", () => {
  const ask = spyOn(transport, "listClaudeLogins").mockResolvedValue();
  render(<StatusBar />);
  expect(ask).not.toHaveBeenCalled();
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(ask).toHaveBeenCalledTimes(1);
  const claude = {
    accounts: [{ name: "Work", config_dir: "/w" }],
    account: null,
    default_name: "Default",
  };
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude } }));
  expect(ask).toHaveBeenCalledTimes(2);
  // Back from the browser, where a `/login` finished.
  act(() => {
    window.dispatchEvent(new Event("focus"));
  });
  expect(ask).toHaveBeenCalledTimes(3);
  cleanup();
  window.dispatchEvent(new Event("focus"));
  expect(ask).toHaveBeenCalledTimes(3);
  ask.mockRestore();
});

test("an account that never logged in says so in the select, with Log in… when current (21.6)", () => {
  const ask = spyOn(transport, "listClaudeLogins").mockResolvedValue();
  render(<StatusBar />);
  const claude = {
    accounts: [{ name: "Work", config_dir: "/w" }],
    account: "/w",
    default_name: "Default",
  };
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude } });
  });
  const select = () => screen.getByRole("combobox", { name: "Claude account" });
  const logIn = () => screen.queryByRole("button", { name: "Log in…" });
  // Until the service answers, every account counts as logged in.
  expect(select().textContent).toBe("Work");
  expect(logIn()).toBeNull();
  const logins = (work: boolean, def = true) => [
    { config_dir: null, logged_in: def },
    { config_dir: "/w", logged_in: work },
  ];
  act(() => apply({ type: "claude_logins", logins: logins(false) }));
  expect(select().textContent).toBe("Work (not logged in)");
  expect(screen.getByText("Not logged in").className).toBe("not-logged-in");
  expect(logIn()?.title).toBe("Select a worktree to open the terminal in");
  fireEvent.mouseDown(select());
  const options = screen.getAllByRole("option").map((o) => o.textContent);
  expect(options).toEqual(["Default", "Work (not logged in)"]);
  fireEvent.keyDown(select(), { key: "Escape" });
  // Logged in now: no mark.
  act(() => apply({ type: "claude_logins", logins: logins(true) }));
  expect(select().textContent).toBe("Work");
  expect(logIn()).toBeNull();
  // The default account, current and never logged in.
  const def = { ...claude, account: null };
  act(() => {
    apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: def } });
    apply({ type: "claude_logins", logins: logins(true, false) });
  });
  expect(select().textContent).toBe("Default (not logged in)");
  expect(logIn()).not.toBeNull();
  // Renamed (21.4), it keeps the mark.
  const personal = { ...def, default_name: "Personal" };
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: personal } }));
  expect(select().textContent).toBe("Personal (not logged in)");
  ask.mockRestore();
});
