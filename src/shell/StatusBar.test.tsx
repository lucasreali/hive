import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { version } from "../../package.json";
import { asMac } from "../../test/mac";
import { apply } from "../reduce";
import { DEFAULT_SETTINGS, initialState, showNotice, useHive } from "../store";
import { transport } from "../transport";
import { StatusBar } from "./StatusBar";

/** The empty ring's tooltip, while no window came (21.8). */
const EMPTY = "No usage yet: it appears once an agent of this account answers";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

test("shows the app version at the right end of the status line", () => {
  render(<StatusBar />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" }));
  const bar = screen.getByRole("contentinfo");
  expect(bar.lastElementChild?.textContent).toBe(`v${version}`);
  expect(bar.textContent).toBe(`WSL: Ubuntuconnected${EMPTY}v${version}`);
});

test("shows the version on macOS without the WSL part", () => {
  asMac();
  render(<StatusBar />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(screen.getByRole("contentinfo").textContent).toBe(`macOSconnected${EMPTY}v${version}`);
});

test("shows Windows while the service runs on Windows itself (12.5.4)", () => {
  render(<StatusBar />);
  act(() => apply({ type: "app_mode", mode: "native", wsl: true }));
  expect(screen.getByTitle("Service connection").textContent).toBe("Windowsconnecting");
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(screen.getByRole("contentinfo").textContent).toBe(`Windowsconnected${EMPTY}v${version}`);
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
  expect(ring.hasAttribute("data-empty")).toBe(false);
  // The empty ring once it reset, and none from a service that is gone.
  act(() => apply({ type: "session_usage", usage: null, week: null }));
  expect(screen.getByRole("button", { name: EMPTY })).toBeDefined();
  expect(screen.getByRole("contentinfo").textContent).toBe(
    `WSL: Ubuntuconnected${EMPTY}v${version}`,
  );
  act(() => {
    apply({ type: "session_usage", usage: { used_percentage: 7, resets_at } });
    apply({ type: "disconnected", reason: "gone", bundled: false });
  });
  expect(screen.queryByRole("button")).toBeNull();
  expect(screen.getByRole("contentinfo").textContent).toBe(`WSLdisconnectedv${version}`);
});

test("with no window, a dim empty ring says when the usage appears (21.8)", () => {
  render(<StatusBar />);
  // Nothing before the service is connected.
  expect(screen.queryByRole("button")).toBeNull();
  act(() => apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" }));
  const ring = screen.getByRole("button", { name: EMPTY });
  expect(ring.className).toBe("session-usage");
  expect(ring.hasAttribute("data-empty")).toBe(true);
  expect(ring.hasAttribute("data-warning")).toBe(false);
  expect(ring.querySelector(".usage-track")).not.toBeNull();
  expect(ring.querySelector(".usage-arc")?.getAttribute("stroke-dasharray")).toBe("0 100");
  expect(ring.querySelector(".usage-tip")?.textContent).toBe(EMPTY);
  // A window fills it.
  const resets_at = new Date(2026, 8, 28, 14, 30).getTime() / 1000;
  act(() => apply({ type: "session_usage", usage: { used_percentage: 85, resets_at } }));
  expect(ring.getAttribute("aria-label")).toBe("Session 85% · resets 14:30");
  expect(ring.hasAttribute("data-empty")).toBe(false);
  expect(ring.hasAttribute("data-warning")).toBe(true);
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
  expect(screen.getByRole("contentinfo").textContent).toBe(
    `WSL: Ubuntuconnected${EMPTY}v${version}`,
  );
});

const NO_ENV = {
  git_name: null,
  git_email: null,
  gh_config_dir: null,
  gh_account: null,
  claude_account: null,
};

/** Two spaces, "Work" with the account of folder `work` (22.5), `current` the current one. */
const spaces = (current: string, work: string | null = "/w") =>
  apply({
    type: "spaces",
    spaces: [
      { id: "default", name: "Personal", projects: [], env: NO_ENV },
      { id: "work", name: "Work", projects: [], env: { ...NO_ENV, claude_account: work } },
    ],
    current,
  });

const WORK_ACCOUNT = {
  accounts: [{ name: "Work account", config_dir: "/w" }],
  default_name: "Default",
  share_sessions: false,
};

test("with an account besides the default one, the current space's account is shown (22.5)", () => {
  render(<StatusBar />);
  const shown = () => document.querySelector<HTMLButtonElement>(".space-account");
  act(() => spaces("work"));
  expect(shown()).toBeNull();
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: WORK_ACCOUNT } }));
  expect(shown()?.textContent).toBe("Work account");
  expect(shown()?.title).toBe("Claude account of Work; change it in the space's settings");
  // No select left: it is the space's, changed in its dialog, which a click opens.
  expect(screen.queryByRole("combobox")).toBeNull();
  fireEvent.click(shown() as HTMLElement);
  expect(useHive.getState().modal).toBe("edit-space");
  // Its session usage (12.1) sits right after it.
  act(() => apply({ type: "session_usage", usage: { used_percentage: 7, resets_at: 1 } }));
  const group = shown()?.closest(".statusbar-account");
  expect(group?.lastElementChild?.className).toBe("session-usage");
  // Next to the version, which stays last.
  expect(screen.getByRole("contentinfo").lastElementChild?.textContent).toBe(`v${version}`);
  // Another space, the default account; a folder no account has is the default one too.
  act(() => spaces("default"));
  expect(shown()?.textContent).toBe("Default");
  expect(shown()?.title).toBe("Claude account of Personal; change it in the space's settings");
  act(() => spaces("work", "/gone"));
  expect(shown()?.textContent).toBe("Default");
  // The default account renamed (21.4) shows its name.
  const personal = { ...WORK_ACCOUNT, default_name: "Mine" };
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: personal } }));
  expect(shown()?.textContent).toBe("Mine");
});

test("whether each account logged in is asked once connected, for new accounts and on focus (21.6)", () => {
  const ask = spyOn(transport, "listClaudeLogins").mockResolvedValue();
  render(<StatusBar />);
  expect(ask).not.toHaveBeenCalled();
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(ask).toHaveBeenCalledTimes(1);
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: WORK_ACCOUNT } }));
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

test("the space's account that never logged in says so, with Log in… (21.6)", () => {
  const ask = spyOn(transport, "listClaudeLogins").mockResolvedValue();
  render(<StatusBar />);
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude: WORK_ACCOUNT } });
    spaces("work");
  });
  const logIn = () => screen.queryByRole("button", { name: "Log in…" });
  const mark = () => screen.queryByText("Not logged in");
  // Until the service answers, every account counts as logged in.
  expect(logIn()).toBeNull();
  const logins = (work: boolean, def = true) => [
    { config_dir: null, logged_in: def },
    { config_dir: "/w", logged_in: work },
  ];
  act(() => apply({ type: "claude_logins", logins: logins(false) }));
  expect(mark()?.className).toBe("not-logged-in");
  expect(logIn()?.title).toBe("Select a worktree to open the terminal in");
  // Logged in now: no mark.
  act(() => apply({ type: "claude_logins", logins: logins(true) }));
  expect(mark()).toBeNull();
  expect(logIn()).toBeNull();
  // A space of the default account, never logged in.
  act(() => {
    spaces("default");
    apply({ type: "claude_logins", logins: logins(true, false) });
  });
  expect(mark()).not.toBeNull();
  expect(logIn()).not.toBeNull();
  ask.mockRestore();
});
