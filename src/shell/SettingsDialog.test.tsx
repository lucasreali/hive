import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { version } from "../../package.json";
import { asMac } from "../../test/mac";
import { expectClearButton } from "../../test/searchClear";
import { App } from "../App";
import { apply } from "../reduce";
import { COMMANDS } from "../shortcuts";
import { DEFAULT_SETTINGS, initialState, useHive } from "../store";
import { transport } from "../transport";
import {
  MOCK_ACCOUNTS_DIR,
  MOCK_CLAUDE_DIR,
  MOCK_DIAGNOSTICS,
  MOCK_REPOS,
  MOCK_SHARED,
} from "../transport/mock";
import { isMac } from "../window";
import { SECTIONS } from "./SettingsDialog";

beforeAll(async () => {
  // The mock service keeps the settings and answers with them.
  await transport.connect(apply);
  await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(async () => {
  cleanup();
  await transport.setSettings(DEFAULT_SETTINGS);
  await new Promise((resolve) => setTimeout(resolve, 0));
  useHive.setState(initialState, true);
});

function open() {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => apply({ type: "settings", settings: DEFAULT_SETTINGS }));
  fireEvent.keyDown(document.body, { key: ",", ctrlKey: !isMac(), metaKey: isMac() });
  return screen.getByRole("dialog", { name: "Settings" }) as HTMLDialogElement;
}

// Within the dialog: the title bar's bell is also named "Notifications".
const section = (name: string) =>
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name }));
const input = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
/** Opens the new account form (22.6). */
const newAccount = () => fireEvent.click(screen.getByRole("button", { name: "New account" }));
const search = (value: string) =>
  fireEvent.change(screen.getByRole("searchbox", { name: "Search settings" }), {
    target: { value },
  });
const settings = () => useHive.getState().settings;
const labels = () =>
  [...document.querySelectorAll(".settings-body .field")].map((f) => f.textContent);

test("Search settings has the app's clear button, which brings the section back", () => {
  open();
  expectClearButton("Search settings");
  expect(input("Font size").value).toBe("13");
});

test("Ctrl+, opens it on the terminal section, focused on the search; Esc and Close close it", () => {
  const dialog = open();
  expect(dialog.open).toBe(true);
  expect(document.activeElement).toBe(screen.getByRole("searchbox"));
  expect([...dialog.querySelectorAll(".settings-section")].map((b) => b.textContent)).toEqual([
    ...SECTIONS,
  ]);
  expect(screen.getByRole("button", { name: "Terminal" }).getAttribute("aria-current")).toBe(
    "page",
  );
  expect(input("Font size").value).toBe("13");
  expect(dialog.querySelector("button kbd")).toBeNull();
  fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  const reopen = () => fireEvent.keyDown(document.body, { key: ",", ctrlKey: true });
  reopen();
  fireEvent.click(screen.getByTitle("Close (Esc)"));
  expect(screen.queryByRole("dialog")).toBeNull();
  reopen();
  act(() => (screen.getByRole("dialog") as HTMLDialogElement).close());
  expect(useHive.getState().modal).toBeNull();
});

test("a checkbox saves the whole settings at once", async () => {
  open();
  const set = spyOn(transport, "setSettings");
  fireEvent.click(screen.getByLabelText("Copy on select"));
  const expected = structuredClone(DEFAULT_SETTINGS);
  expected.terminal.copy_on_select = true;
  expect(set).toHaveBeenLastCalledWith(expected);
  await waitFor(() => expect(settings().terminal.copy_on_select).toBe(true));
  expect(input("Copy on select").checked).toBe(true);
  fireEvent.click(screen.getByLabelText("Blinking cursor"));
  await waitFor(() => expect(settings().terminal.cursor_blink).toBe(true));
  section("Agents");
  fireEvent.click(screen.getByLabelText("Confirm closing Hive with agents running"));
  await waitFor(() => expect(settings().agents.confirm_close).toBe(false));
  set.mockRestore();
});

test("the Dock bounce setting is offered on macOS only, on by default", async () => {
  open();
  const bounce = "Bounce the Dock icon when an agent needs an action";
  section("Notifications");
  expect(screen.queryByLabelText(bounce)).toBeNull();
  asMac();
  section("Agents");
  section("Notifications");
  expect(input(bounce).checked).toBe(true);
  fireEvent.click(screen.getByLabelText(bounce));
  await waitFor(() => expect(settings().notifications.bounce).toBe(false));
  expect(input(bounce).checked).toBe(false);
});

test("the menu bar icon setting is offered on macOS only, on by default", async () => {
  open();
  const menuBar = "Show agents in the menu bar";
  section("Notifications");
  expect(screen.queryByLabelText(menuBar)).toBeNull();
  asMac();
  section("Agents");
  section("Notifications");
  expect(input(menuBar).checked).toBe(true);
  fireEvent.click(screen.getByLabelText(menuBar));
  await waitFor(() => expect(settings().notifications.menu_bar).toBe(false));
  expect(input(menuBar).checked).toBe(false);
});

test("numbers and texts are saved once typing pauses; non-numbers are not sent", async () => {
  open();
  const set = spyOn(transport, "setSettings");
  fireEvent.change(input("Font size"), { target: { value: "1" } });
  fireEvent.change(input("Font size"), { target: { value: "16" } });
  expect(set).not.toHaveBeenCalled();
  await waitFor(() => expect(settings().terminal.font_size).toBe(16));
  expect(set).toHaveBeenCalledTimes(1);
  fireEvent.change(input("Scrollback lines"), { target: { value: "" } });
  fireEvent.change(input("Scrollback lines"), { target: { value: "2.5" } });
  await new Promise((resolve) => setTimeout(resolve, 500));
  expect(set).toHaveBeenCalledTimes(1);
  fireEvent.change(input("Scrollback lines"), { target: { value: "9000" } });
  await waitFor(() => expect(settings().terminal.scrollback).toBe(9000));
  fireEvent.change(input("Font family"), { target: { value: "Fira Code" } });
  await waitFor(() => expect(settings().terminal.font_family).toBe("Fira Code"));
  section("Notifications");
  fireEvent.change(input("Alert volume"), { target: { value: "0" } });
  await waitFor(() => expect(settings().notifications.volume).toBe(0));
  fireEvent.change(input("Long commands (seconds)"), { target: { value: "30" } });
  await waitFor(() => expect(settings().notifications.command_secs).toBe(30));
  section("Agents");
  fireEvent.change(input("Silence before waiting for you (seconds)"), { target: { value: "9" } });
  await waitFor(() => expect(settings().agents.silence_secs).toBe(9));
  fireEvent.click(
    screen.getByRole("button", { name: "Increase Silence before waiting for you (seconds)" }),
  );
  await waitFor(() => expect(settings().agents.silence_secs).toBe(10));
  set.mockRestore();
});

test("an empty default base is null: the project's current branch", async () => {
  open();
  section("Worktrees");
  const base = input("Default base branch");
  expect(base.placeholder).toBe("Current branch");
  fireEvent.change(base, { target: { value: "dev" } });
  await waitFor(() => expect(settings().worktrees.default_base).toBe("dev"));
  fireEvent.change(base, { target: { value: " " } });
  await waitFor(() => expect(settings().worktrees.default_base).toBeNull());
  expect(base.value).toBe("");
});

test("the agent's browser: on, its program (empty: found) and how many at once (24.6)", async () => {
  open();
  section("Agents");
  fireEvent.click(screen.getByLabelText("Give agents a browser"));
  await waitFor(() => expect(settings().browser.enabled).toBe(true));
  const path = input("Browser program");
  expect(path.placeholder).toBe("Found where it installs");
  fireEvent.change(path, { target: { value: "/opt/chrome" } });
  await waitFor(() => expect(settings().browser.path).toBe("/opt/chrome"));
  fireEvent.change(path, { target: { value: " " } });
  await waitFor(() => expect(settings().browser.path).toBeNull());
  fireEvent.change(input("Browsers open at once"), { target: { value: "2" } });
  await waitFor(() => expect(settings().browser.max).toBe(2));
});

test("selects save the cursor style and the theme", async () => {
  open();
  const pick = (name: string, option: string) => {
    fireEvent.mouseDown(screen.getByRole("combobox", { name }));
    fireEvent.click(screen.getByRole("option", { name: option }));
  };
  pick("Cursor style", "Bar");
  await waitFor(() => expect(settings().terminal.cursor_style).toBe("bar"));
  section("Appearance");
  pick("Theme", "One Light");
  await waitFor(() => expect(settings().appearance.theme).toBe("one-light"));
  expect(document.documentElement.dataset.theme).toBe("one-light");
});

test("the service setting shows only where WSL or Windows is offered, and asks before switching (12.5.4)", () => {
  open();
  const service = () => screen.queryByRole("combobox", { name: "Service" });
  expect(service()).toBeNull();
  search("service");
  expect(screen.getByText("No setting matches.")).toBeTruthy();
  search("");
  act(() => apply({ type: "app_mode", mode: "wsl", wsl: true }));
  expect(service()?.textContent).toContain("WSL");
  const setMode = spyOn(transport, "setMode").mockResolvedValue();
  const pick = () => {
    fireEvent.mouseDown(service() as HTMLElement);
    fireEvent.click(screen.getByRole("option", { name: "Windows" }));
    return screen.getByRole("dialog", { name: "Run Hive on Windows?" });
  };
  fireEvent.click(within(pick()).getByText("Cancel"));
  expect(setMode).not.toHaveBeenCalled();
  expect(useHive.getState().modal).toBe("settings");
  fireEvent.click(within(pick()).getByText("Switch"));
  expect(setMode).toHaveBeenCalledWith("native");
  expect(useHive.getState().modal).toBe("settings");
  act(() => apply({ type: "app_mode", mode: "native", wsl: true }));
  expect(service()?.textContent).toContain("Windows");
  // Without WSL there is nothing to choose.
  act(() => apply({ type: "app_mode", mode: "native", wsl: false }));
  expect(service()).toBeNull();
  setMode.mockRestore();
});

test("the shell is a setting only when the service runs natively on Windows", async () => {
  open();
  expect(screen.queryByRole("combobox", { name: "Shell" })).toBeNull();
  search("shell");
  expect(screen.getByText("No setting matches.")).toBeTruthy();
  search("");
  act(() => apply({ type: "app_mode", mode: "native", wsl: false }));
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Shell" }));
  fireEvent.click(screen.getByRole("option", { name: "Git Bash" }));
  await waitFor(() => expect(settings().terminal.shell).toBe("git_bash"));
  // Back on WSL: gone again.
  act(() => apply({ type: "app_mode", mode: "wsl", wsl: true }));
  expect(screen.queryByRole("combobox", { name: "Shell" })).toBeNull();
});

test("keeping the Mac awake is a setting on a Mac only, which says the lid still sleeps it (17.17)", async () => {
  const label = "Keep the Mac awake while agents work";
  open();
  section("Agents");
  expect(screen.queryByLabelText(label)).toBeNull();
  cleanup();
  useHive.setState(initialState, true);
  const mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
  Object.defineProperty(navigator, "userAgent", { value: mac, configurable: true });
  try {
    render(<App />);
    act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
    act(() => apply({ type: "settings", settings: DEFAULT_SETTINGS }));
    fireEvent.keyDown(document.body, { key: ",", metaKey: true });
    section("Agents");
    expect(input(label).checked).toBe(true);
    expect(labels().join()).toContain("closing the lid on battery still sleeps the Mac");
    fireEvent.click(input(label));
    await waitFor(() => expect(settings().agents.keep_awake).toBe(false));
  } finally {
    Reflect.deleteProperty(navigator, "userAgent");
  }
  expect(navigator.userAgent).not.toContain("Mac");
});

test("the archive script's help names the shell it runs in", () => {
  open();
  section("Projects");
  act(() => apply({ type: "projects", projects: [MOCK_REPOS[0]] }));
  const help = () => input("Archive script").closest(".field")?.textContent;
  expect(help()).toContain("Run by the service with sh -c in the worktree");
  act(() => apply({ type: "app_mode", mode: "native", wsl: false }));
  act(() =>
    apply({
      type: "settings",
      settings: { ...DEFAULT_SETTINGS, terminal: { ...DEFAULT_SETTINGS.terminal, shell: "cmd" } },
    }),
  );
  expect(help()).toContain("Run by the service in Command Prompt (the Shell setting;");
});

test("the projects' hint names PORT and what Vite needs for it (23.3)", () => {
  open();
  section("Projects");
  act(() => apply({ type: "projects", projects: [MOCK_REPOS[0]] }));
  const help = screen.getByRole("combobox", { name: "Project" }).closest(".field")?.textContent;
  expect(help).toContain("They also have that port as $PORT (unless your shell already sets one)");
  expect(help).toContain("Vite needs server.port: Number(process.env.PORT) in its config.");
});

test("the projects section edits each project's scripts", async () => {
  open();
  section("Projects");
  expect(screen.getByText("Add a project to give it scripts.")).toBeDefined();
  const [shop, api] = MOCK_REPOS;
  act(() => apply({ type: "projects", projects: [shop, api] }));
  expect(screen.getByRole("combobox", { name: "Project" }).textContent).toBe(shop.name);
  const scripts = () => settings().projects[shop.id]?.scripts;

  fireEvent.change(input("Setup script"), { target: { value: "bun install" } });
  await waitFor(() => expect(scripts()?.setup).toBe("bun install"));
  fireEvent.change(input("Archive script"), { target: { value: "make clean" } });
  await waitFor(() => expect(scripts()?.archive).toBe("make clean"));
  fireEvent.change(input("Setup script"), { target: { value: " " } });
  await waitFor(() => expect(scripts()?.setup).toBeNull());

  const add = screen.getByRole("button", { name: "Add" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  fireEvent.change(input("New run script name"), { target: { value: " dev " } });
  fireEvent.change(input("New run script command"), { target: { value: "bun dev" } });
  fireEvent.click(add);
  await waitFor(() => expect(scripts()?.run).toEqual([{ name: "dev", command: "bun dev" }]));
  expect(input("New run script name").value).toBe("");
  fireEvent.change(input("Command of dev"), { target: { value: "bun run dev" } });
  await waitFor(() => expect(scripts()?.run[0].command).toBe("bun run dev"));
  fireEvent.change(input("Name"), { target: { value: "serve " } });
  await waitFor(() => expect(scripts()?.run[0].name).toBe("serve"));
  fireEvent.click(screen.getByTitle("Remove serve"));
  await waitFor(() => expect(scripts()?.run).toEqual([]));
  expect(scripts()?.archive).toBe("make clean");

  // Another project has its own.
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "Project" }));
  fireEvent.click(screen.getByRole("option", { name: api.name }));
  expect(input("Archive script").value).toBe("");
});

test("a new account's folder example is a home folder of the platform (18.25)", () => {
  open();
  section("Accounts");
  newAccount();
  fireEvent.click(input("Use an existing folder"));
  expect(input("New account folder").placeholder).toBe("/home/you/.claude-work");
  asMac();
  section("Agents");
  section("Accounts");
  newAccount();
  fireEvent.click(input("Use an existing folder"));
  expect(input("New account folder").placeholder).toBe("/Users/you/.claude-work");
});

test("sharing sessions between accounts is off by default and turns on (21.10)", async () => {
  open();
  section("Accounts");
  const share = input("Share sessions between accounts");
  expect(share.checked).toBe(false);
  expect(screen.getByTitle(/rewind checkpoints from before the move are lost/)).toBeDefined();
  fireEvent.click(share);
  await waitFor(() => expect(settings().claude.share_sessions).toBe(true));
});

test("the accounts section adds, renames, logs in to and removes Claude accounts (12.2)", async () => {
  open();
  section("Accounts");
  const accounts = () => settings().claude.accounts;
  expect(screen.getByTitle("~/.claude").textContent).toBe("~/.claude");
  // No worktree to open a terminal in: "Log in…" waits for one.
  const logIns = () => screen.getAllByRole("button", { name: "Log in…" }) as HTMLButtonElement[];
  expect(logIns()[0]?.disabled).toBe(true);
  // The form opens from "New account" (22.6), its name field focused; Cancel closes it,
  // adding nothing.
  expect(screen.queryByLabelText("New account name")).toBeNull();
  newAccount();
  expect(screen.queryByRole("button", { name: "New account" })).toBeNull();
  expect(document.activeElement).toBe(input("New account name"));
  fireEvent.change(input("New account name"), { target: { value: "Dropped" } });
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByLabelText("New account name")).toBeNull();
  expect(accounts()).toEqual([]);
  newAccount();
  const add = screen.getByRole("button", { name: "Add" }) as HTMLButtonElement;
  expect(input("New account name").value).toBe("");
  expect(add.disabled).toBe(true);
  fireEvent.click(input("Use an existing folder"));
  fireEvent.change(input("New account name"), { target: { value: " Work " } });
  expect(add.disabled).toBe(true);
  // A clicked folder ends with `/`: the service keeps it without (22.2).
  fireEvent.change(input("New account folder"), { target: { value: " /w/ " } });
  fireEvent.click(add);
  await waitFor(() => expect(accounts()).toEqual([{ name: "Work", config_dir: "/w" }]));
  // Added, the form closes and the button is back.
  expect(screen.queryByLabelText("New account name")).toBeNull();
  expect(screen.getByRole("button", { name: "New account" })).toBeDefined();
  // One row per account in the same columns: name, folder (the whole path as its tooltip),
  // status, "Log in…", remove; the default row's remove slot is empty.
  const rows = [...document.querySelectorAll(".account-row")].map((row) =>
    [...row.children].map((cell) => cell.getAttribute("aria-label") ?? cell.textContent),
  );
  expect(rows).toEqual([
    ["Name of the default account", "~/.claude", "", "Log in…", ""],
    // The mock: an added account never logged in.
    ["Name of /w", "/w", "Not logged in", "Log in…", ""],
  ]);
  const cells = document.querySelectorAll(".account-row")[1]?.children;
  expect(cells?.[1]?.getAttribute("title")).toBe("/w");
  expect(cells?.[4]?.getAttribute("title")).toBe("Remove Work");
  expect(document.querySelectorAll(".account-row")[0]?.children[4]?.tagName).toBe("SPAN");
  fireEvent.change(input("Name of /w"), { target: { value: "Job " } });
  await waitFor(() => expect(accounts()[0]?.name).toBe("Job"));

  // With a worktree selected, "Log in…" runs claude there as the account and closes the settings.
  const [shop] = MOCK_REPOS;
  act(() => {
    apply({ type: "projects", projects: [shop] });
    useHive.setState({ selection: shop.id });
  });
  const opened = spyOn(transport, "openTerminal").mockResolvedValue(9);
  const typed = spyOn(transport, "writeTerminal").mockResolvedValue();
  expect(logIns()[1]?.title).toBe("A terminal running claude as Job: type /login there");
  fireEvent.click(logIns()[1] as HTMLButtonElement);
  await waitFor(() => expect(typed).toHaveBeenCalledWith(9, "claude\r"));
  expect(opened.mock.calls[0]?.[0]).toBe(shop.path);
  expect(opened.mock.calls[0]?.[4]).toEqual({ config_dir: "/w" });
  expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
  opened.mockRestore();
  typed.mockRestore();

  // Removed, its folder stays on disk.
  fireEvent.keyDown(document.body, { key: ",", ctrlKey: !isMac(), metaKey: isMac() });
  section("Accounts");
  fireEvent.click(screen.getByTitle("Remove Job"));
  await waitFor(() => expect(accounts()).toEqual([]));
});

test("the new account's folder is browsed from home, hidden folders included (22.2)", async () => {
  open();
  section("Accounts");
  newAccount();
  // Hive makes the folder unless an existing one is chosen (22.3).
  expect(screen.queryByLabelText("New account folder")).toBeNull();
  fireEvent.click(input("Use an existing folder"));
  const folder = () => input("New account folder");
  const folders = () =>
    [...screen.getByRole("list", { name: "Folders" }).querySelectorAll("li")].map(
      (li) => li.textContent,
    );
  await waitFor(() => expect(folder().value).toBe("/home/user/"));
  await waitFor(() => expect(folders()).toContain(".claude-work"));
  expect(screen.queryByText(/A Claude config folder/)).toBeNull();
  // A click enters a folder; the service says it is a Claude config folder.
  fireEvent.click(screen.getByRole("button", { name: ".claude-work" }));
  expect(folder().value).toBe(`${MOCK_CLAUDE_DIR}/`);
  await waitFor(() =>
    expect(screen.getByText(/A Claude config folder/).getAttribute("role")).toBe("status"),
  );
  // ↑ goes up, and a typed path still works.
  fireEvent.click(screen.getByTitle("Up a level"));
  expect(folder().value).toBe("/home/user/");
  fireEvent.change(folder(), { target: { value: "/home/user/.config/gh-work" } });
  fireEvent.change(input("New account name"), { target: { value: "Work" } });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));
  await waitFor(() =>
    expect(settings().claude.accounts).toEqual([
      { name: "Work", config_dir: "/home/user/.config/gh-work" },
    ]),
  );
});

test("a new account named only gets a folder Hive makes, then is offered its log in (22.3)", async () => {
  open();
  section("Accounts");
  const accounts = () => settings().claude.accounts;
  newAccount();
  expect(screen.getByText(/Hive makes its folder/)).toBeDefined();
  const created = spyOn(transport, "createAccount");
  fireEvent.change(input("New account name"), { target: { value: " Work 2 " } });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));
  const config_dir = `${MOCK_ACCOUNTS_DIR}/work-2`;
  await waitFor(() =>
    expect(accounts()).toEqual([{ name: "Work 2", config_dir, hive_folder: true }]),
  );
  expect(created).toHaveBeenCalledWith("Work 2");
  created.mockRestore();
  expect(screen.queryByLabelText("New account name")).toBeNull();
  expect(screen.getByTitle(config_dir)).toBeDefined();
  const offer = screen.getByText(/Work 2 is made: log in to it/);
  expect(offer.getAttribute("role")).toBe("status");
  // Only said: its row has the "Log in…" (22.6).
  expect(within(offer).queryByRole("button")).toBeNull();
  expect(screen.getAllByRole("button", { name: "Log in…" })).toHaveLength(2);
  // A taken name is refused: nothing made, nothing offered for it, the reason shown.
  newAccount();
  fireEvent.change(input("New account name"), { target: { value: "Default" } });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));
  await waitFor(() => expect(settings().claude.accounts).toHaveLength(1));
  await waitFor(() =>
    expect(useHive.getState().settingsError).toBe("Another account is named Default"),
  );
  expect(screen.queryByText(/is made: log in to it/)).toBeNull();
});

test("removing an account Hive made asks whether to delete its folder too (22.3)", async () => {
  open();
  section("Accounts");
  const accounts = () => settings().claude.accounts;
  for (const value of ["Work", "Kept"]) {
    newAccount();
    fireEvent.change(input("New account name"), { target: { value } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(screen.queryByTitle(`Remove ${value}`)).not.toBeNull());
  }
  const removed = spyOn(transport, "removeAccount");
  // It asks first, saying to log out: cancelled, nothing changes.
  fireEvent.click(screen.getByTitle("Remove Work"));
  const ask = () => screen.getByRole("group", { name: "Remove Work" });
  expect(within(ask()).getByText(/\/logout/)).toBeDefined();
  expect(within(ask()).getByText(/Keychain/)).toBeDefined();
  fireEvent.click(within(ask()).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("group", { name: "Remove Work" })).toBeNull();
  expect(removed).not.toHaveBeenCalled();
  // Removed with its folder.
  fireEvent.click(screen.getByTitle("Remove Work"));
  fireEvent.click(within(ask()).getByLabelText("Also delete its folder"));
  fireEvent.click(within(ask()).getByRole("button", { name: "Remove" }));
  await waitFor(() => expect(accounts().map((a) => a.name)).toEqual(["Kept"]));
  expect(removed).toHaveBeenCalledWith(`${MOCK_ACCOUNTS_DIR}/work`, true);
  // Removed, its folder kept: the box starts unticked each time.
  fireEvent.click(screen.getByTitle("Remove Kept"));
  const keep = screen.getByRole("group", { name: "Remove Kept" });
  expect((within(keep).getByLabelText("Also delete its folder") as HTMLInputElement).checked).toBe(
    false,
  );
  fireEvent.click(within(keep).getByRole("button", { name: "Remove" }));
  await waitFor(() => expect(accounts()).toEqual([]));
  expect(removed).toHaveBeenLastCalledWith(`${MOCK_ACCOUNTS_DIR}/kept`, false);
  removed.mockRestore();
});

test("a save queued behind an account request builds on its answer (22.3)", async () => {
  open();
  section("Accounts");
  const sent = spyOn(transport, "setSettings");
  newAccount();
  fireEvent.change(input("New account name"), { target: { value: "Work" } });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));
  // Sent while the account is being made: it waits, and keeps the new account.
  fireEvent.click(input("Share sessions between accounts"));
  await waitFor(() => expect(sent).toHaveBeenCalledTimes(1));
  const saved = sent.mock.calls[0]?.[0];
  expect(saved?.claude.share_sessions).toBe(true);
  expect(saved?.claude.accounts.map((a) => a.name)).toEqual(["Work"]);
  sent.mockRestore();
  await waitFor(() => expect(useHive.getState().settingsPending).toBe(false));
});

test("the default account is renamed in place, and emptied it is Default again (21.4)", async () => {
  open();
  section("Accounts");
  const name = () => input("Name of the default account");
  // Not renamed: an empty field showing "Default" as its placeholder (22.4).
  expect(name().value).toBe("");
  expect(name().placeholder).toBe("Default");
  fireEvent.change(name(), { target: { value: " Personal " } });
  await waitFor(() => expect(settings().claude.default_name).toBe("Personal"));
  expect(name().value).toBe("Personal");
  const [shop] = MOCK_REPOS;
  act(() => {
    apply({ type: "projects", projects: [shop] });
    useHive.setState({ selection: shop.id });
  });
  const logIn = screen.getAllByRole("button", { name: "Log in…" })[0] as HTMLButtonElement;
  expect(logIn.title).toBe("A terminal running claude as Personal: type /login there");
  // The service reads an empty name as "Default", shown as the empty field.
  fireEvent.change(name(), { target: { value: " " } });
  await waitFor(() => expect(settings().claude.default_name).toBe("Default"));
  expect(name().value).toBe("");
  expect(logIn.title).toBe("A terminal running claude as Default: type /login there");
});

test("an emptied default account name stays empty while a new one is typed (22.4)", async () => {
  const claude = {
    accounts: [{ name: "Work", config_dir: "/w" }],
    default_name: "Personal",
    share_sessions: false,
  };
  await transport.setSettings({ ...DEFAULT_SETTINGS, claude });
  open();
  const env = {
    git_name: null,
    git_email: null,
    gh_config_dir: null,
    gh_account: null,
    claude_account: null,
  };
  act(() => {
    apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude } });
    apply({ type: "spaces", spaces: [{ id: "d", name: "D", projects: [], env }], current: "d" });
  });
  section("Accounts");
  const name = () => input("Name of the default account");
  // The status bar's name of the space's account (22.5).
  const select = () => document.querySelector(".space-account") as HTMLElement;
  expect(name().value).toBe("Personal");
  // Emptied and saved: the mock settles it to "Default", the field keeps its empty text.
  name().focus();
  fireEvent.change(name(), { target: { value: "" } });
  await waitFor(() => expect(settings().claude.default_name).toBe("Default"));
  expect(name().value).toBe("");
  expect(name().placeholder).toBe("Default");
  expect(select().textContent).toBe("Default");
  // Typed key by key, each pause saved and settled: no key is lost.
  for (const text of ["W", "Wo", "Wor"]) {
    fireEvent.change(name(), { target: { value: text } });
    await waitFor(() => expect(settings().claude.default_name).toBe(text));
    expect(name().value).toBe(text);
  }
  // A late answer (an earlier save's) while typing does not overwrite the draft.
  fireEvent.change(name(), { target: { value: "Work2" } });
  const late = { ...DEFAULT_SETTINGS, claude: { ...claude, default_name: "Wo" } };
  act(() => apply({ type: "settings", settings: late }));
  expect(name().value).toBe("Work2");
  // Unfocused, the field follows the service again; "Default" shows as the empty field.
  name().blur();
  await waitFor(() => expect(settings().claude.default_name).toBe("Work2"));
  act(() =>
    apply({
      type: "settings",
      settings: { ...DEFAULT_SETTINGS, claude: { ...claude, default_name: "Default" } },
    }),
  );
  expect(name().value).toBe("");
  // Typing "Default" itself saves nothing new.
  const set = spyOn(transport, "setSettings");
  fireEvent.change(name(), { target: { value: "Default" } });
  await new Promise((resolve) => setTimeout(resolve, 450));
  expect(set).not.toHaveBeenCalled();
  set.mockRestore();
});

test("the accounts section marks an account that never logged in (21.6)", async () => {
  const ask = spyOn(transport, "listClaudeLogins");
  const claude = {
    accounts: [{ name: "Work", config_dir: "/w" }],
    default_name: "Default",
    share_sessions: false,
  };
  await transport.setSettings({ ...DEFAULT_SETTINGS, claude });
  open();
  act(() => apply({ type: "settings", settings: { ...DEFAULT_SETTINGS, claude } }));
  const asked = ask.mock.calls.length;
  section("Accounts");
  // Asked again as the section opens: a `/login` may have finished in a Hive terminal.
  expect(ask.mock.calls.length).toBe(asked + 1);
  // The mock: the default account logged in, Work not.
  const marks = () => [...document.querySelectorAll(".account-row")].map((row) => row.textContent);
  await waitFor(() => expect(marks()[1]).toContain("Not logged in"));
  expect(marks()[0]).not.toContain("Not logged in");
  // A badge in its own column, right before the row's "Log in…" (22.6).
  const mark = screen.getByText("Not logged in");
  expect(mark.className).toBe("tab-badge warn");
  expect(mark.parentElement?.nextElementSibling?.textContent).toBe("Log in…");
  ask.mockRestore();
});

test("a run script keeps its fields and the focus while renamed; a late edit finds its row", async () => {
  open();
  section("Projects");
  const [shop] = MOCK_REPOS;
  act(() => apply({ type: "projects", projects: [shop] }));
  const run = () => settings().projects[shop.id]?.scripts.run;
  const add = (name: string, command: string) => {
    fireEvent.change(input("New run script name"), { target: { value: name } });
    fireEvent.change(input("New run script command"), { target: { value: command } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
  };
  add("dev", "bun dev");
  add("test", "bun test");
  await waitFor(() => expect(run()?.map((r) => r.name)).toEqual(["dev", "test"]));

  // Renaming saves once typing pauses: the same field stays, focused.
  const [name] = screen.getAllByLabelText("Name") as HTMLInputElement[];
  name.focus();
  fireEvent.change(name, { target: { value: "serve" } });
  await waitFor(() => expect(run()?.[0]?.name).toBe("serve"));
  expect(document.activeElement).toBe(name);
  expect(name.isConnected).toBe(true);

  // A command typed in the second row, then the first removed before it is saved: the edit
  // still lands on its own row.
  fireEvent.change(input("Command of test"), { target: { value: "bun test --watch" } });
  fireEvent.click(screen.getByTitle("Remove serve"));
  await waitFor(() => expect(run()).toEqual([{ name: "test", command: "bun test --watch" }]));

  // A list stored from elsewhere shows as it is.
  const stored = structuredClone(settings());
  stored.projects[shop.id] = {
    scripts: { setup: null, run: [{ name: "lint", command: "x" }], archive: null },
  };
  act(() => apply({ type: "settings", settings: stored }));
  expect(input("Command of lint").value).toBe("x");
});

test("a save made before the last one is answered builds on it: both land", async () => {
  open();
  const set = spyOn(transport, "setSettings");
  fireEvent.click(screen.getByLabelText("Copy on select"));
  fireEvent.click(screen.getByLabelText("Blinking cursor"));
  // The second waits for the answer to the first.
  expect(set).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(set).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(useHive.getState().settingsPending).toBe(false));
  expect(settings().terminal.copy_on_select).toBe(true);
  expect(settings().terminal.cursor_blink).toBe(true);
  set.mockRestore();
});

test("the service's refusal shows inline, and the typed text stays", async () => {
  open();
  fireEvent.change(input("Font size"), { target: { value: "99" } });
  act(() =>
    apply({ type: "settings_failed", message: "terminal.font_size must be between 8 and 32" }),
  );
  expect(screen.getByRole("alert").textContent).toBe("terminal.font_size must be between 8 and 32");
  expect(input("Font size").value).toBe("99");
});

test("the search lists every matching field across sections", () => {
  open();
  search("cursor");
  expect(labels()).toEqual(["Cursor styleBlock", "Blinking cursor"]);
  expect(screen.getByRole("button", { name: "Terminal" }).getAttribute("aria-current")).toBeNull();
  search("BRANCH");
  expect(labels()).toEqual(["Default base branchEmpty: the project's current branch."]);
  search("nothing like it");
  expect(screen.getByText("No setting matches.")).toBeTruthy();
  // Picking a section clears the search.
  section("Agents");
  expect(screen.getByRole<HTMLInputElement>("searchbox").value).toBe("");
  // Two agents' settings and the agent's browser's three (24.6).
  expect(labels()).toHaveLength(5);
});

const shortcutRows = () =>
  [...document.querySelectorAll(".settings-keys tr")].map((r) => r.textContent);

test("shortcuts are the command table, read-only, without macOS' own", () => {
  open();
  section("Shortcuts");
  expect(shortcutRows()).toEqual(COMMANDS.filter((c) => !c.mac).map((c) => `${c.label}${c.keys}`));
  expect(shortcutRows()).toContain("Go to the next pending agentF8");
});

test("on macOS the shortcuts are named with ⌘, with macOS' own and the terminal's keys", () => {
  asMac();
  open();
  section("Shortcuts");
  const rows = shortcutRows();
  expect(rows).toHaveLength(COMMANDS.length + 10);
  expect(rows).toContain("Command palette⇧⌘P");
  expect(rows).toContain("Go to the next pending agent⇧⌘J or F8");
  expect(rows).toContain("New terminal in this worktree⌘T");
  expect(rows).toContain("New worktree⌘N or ⇧⌘N");
  expect(rows).toContain("Close the tab⌘W");
  expect(rows).toContain("Show the next tab⌃Tab or ⇧⌘]");
  expect(rows).toContain("Show the previous tab⌃⇧Tab or ⇧⌘[");
  expect(rows).toContain("Show tab 9⌘9");
  expect(rows).toContain("Make the terminal font smaller⌘-");
  expect(rows.slice(-10)).toEqual([
    "Clear the terminal⌘K",
    "Jump to the previous prompt⌘↑",
    "Jump to the next prompt⌘↓",
    "Go to the start of the line⌘←",
    "Go to the end of the line⌘→",
    "Delete to the start of the line⌘⌫",
    "Scroll a page upfn↑",
    "Scroll a page downfn↓",
    "Scroll to the topfn←",
    "Scroll to the bottomfn→",
  ]);
});

test("Option as Meta is a macOS setting, off by default, per Option key", async () => {
  open();
  expect(screen.queryByRole("combobox", { name: "Option as Meta" })).toBeNull();
  cleanup();
  asMac();
  open();
  const select = () => screen.getByRole("combobox", { name: "Option as Meta" });
  expect(select().textContent).toContain("Off");
  fireEvent.mouseDown(select());
  fireEvent.click(screen.getByRole("option", { name: "Left Option" }));
  await waitFor(() => expect(settings().terminal.option_as_meta).toBe("left"));
  expect(select().textContent).toContain("Left Option");
});

test("Screen reader is off by default and turns on", async () => {
  open();
  expect(input("Screen reader").checked).toBe(false);
  fireEvent.click(input("Screen reader"));
  await waitFor(() => expect(settings().terminal.screen_reader).toBe(true));
  expect(input("Screen reader").checked).toBe(true);
});

test("about shows both versions, the service's diagnostics and terminals without hooks", async () => {
  open();
  const ask = spyOn(transport, "getDiagnostics");
  act(() => {
    apply({ type: "terminal_opened", channel: 3, worktree: null, account: "Default" });
    apply({ type: "unhooked_agent", channel: 3 });
    apply({ type: "terminal_opened", channel: 4, worktree: null, account: "Default" });
    apply({ type: "unhooked_agent", channel: 4 });
    apply({ type: "terminal_exited", channel: 4, code: 0 });
  });
  section("About");
  expect(ask).toHaveBeenCalledTimes(1);
  const about = () => document.querySelector(".settings-about")?.textContent;
  expect(about()).toContain(`Hive app${version}`);
  expect(about()).toContain("Hive service0.1.0");
  expect(about()).toContain("Terminals without hooks3");
  await waitFor(() => expect(about()).toContain(`Settings file${MOCK_DIAGNOSTICS.settings_file}`));
  expect(about()).toContain(`claude wrapper${MOCK_DIAGNOSTICS.wrapper}`);
  expect(about()).toContain(`claude it runs${MOCK_DIAGNOSTICS.claude}`);
  expect(about()).not.toContain("git");
  const diagnostics = { settings_file: "/s", wrapper: "/w", claude: null, git_error: null };
  act(() => apply({ type: "diagnostics", ...diagnostics }));
  expect(about()).toContain("claude it runsnot found on PATH");
  act(() => apply({ type: "diagnostics", ...diagnostics, git_error: "No tools." }));
  expect(about()).toContain("gitNo tools.");
  act(() => apply({ type: "terminal_exited", channel: 3, code: 0 }));
  expect(about()).toContain("Terminals without hooksnone");
  ask.mockRestore();
});

test("before the answers, about says so", () => {
  open();
  const ask = spyOn(transport, "getDiagnostics").mockResolvedValue();
  act(() => useHive.setState({ connection: { status: "connecting" } }));
  section("About");
  const about = document.querySelector(".settings-about")?.textContent;
  expect(about).toContain("Hive servicenot connected");
  expect(about).toContain("Settings file…");
  expect(about).toContain("claude it runs…");
  ask.mockRestore();
});

test("Accounts lists what the shared folder holds and opens it (21.9)", async () => {
  const ask = spyOn(transport, "getShared");
  const openIt = spyOn(transport, "openShared").mockResolvedValue();
  open();
  section("Accounts");
  expect(ask).toHaveBeenCalledTimes(1);
  const folder = () => document.querySelector(".shared-row .folder-cell") as HTMLElement;
  expect(folder().textContent).toBe("…");
  await waitFor(() => expect(folder().title).toBe(MOCK_SHARED.folder));
  expect(folder().textContent).toBe(MOCK_SHARED.folder);
  // What it holds on one line, an empty kind left out (22.6).
  const held = () => document.querySelector(".shared-held")?.textContent;
  expect(held()).toBe("Skills: /hive-shared:review · Agents: hive-shared:tester · MCP servers: db");
  expect(document.querySelector(".field-error")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Open folder" }));
  expect(openIt).toHaveBeenCalledTimes(1);
  act(() => apply({ type: "shared", ...MOCK_SHARED, error: ".mcp.json: not a file" }));
  expect(document.querySelector(".field-error")?.textContent).toBe(".mcp.json: not a file");
  // Empty, "none" is said once.
  const empty = { skills: [], agents: [], commands: [], mcp_servers: [], error: null };
  act(() => apply({ type: "shared", ...MOCK_SHARED, ...empty }));
  expect(held()).toBe("Skills, commands, agents and MCP servers: none");
  ask.mockRestore();
  openIt.mockRestore();
});

test("Open settings file asks the service, whose answer opens it", async () => {
  open();
  const ask = spyOn(transport, "openSettingsFile");
  fireEvent.click(screen.getByRole("button", { name: "Open settings file" }));
  expect(ask).toHaveBeenCalledTimes(1);
  ask.mockRestore();
});

test("Notifications says when the OS keeps Hive from notifying, with a link to its settings", async () => {
  let permission: "granted" | "denied" = "denied";
  const check = spyOn(transport, "checkNotifications").mockImplementation(async () => {
    apply({ type: "notification_permission", permission });
  });
  const openPane = spyOn(transport, "openNotificationSettings").mockResolvedValue();
  try {
    open();
    expect(check).not.toHaveBeenCalled();
    section("Notifications");
    const note = await screen.findByRole("note");
    expect(note.textContent).toBe(
      "Notifications from Hive are turned off in System Settings: alerts show only in the inbox. Open System Settings",
    );
    fireEvent.click(within(note).getByRole("button", { name: "Open System Settings" }));
    expect(openPane).toHaveBeenCalledTimes(1);
    // Back from System Settings: the window's focus asks again.
    permission = "granted";
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(screen.queryByRole("note")).toBeNull());
    expect(check).toHaveBeenCalledTimes(2);
    expect(input("Alert volume").value).toBe("100");
    // Another section asks nothing and listens no more.
    section("Agents");
    window.dispatchEvent(new Event("focus"));
    expect(check).toHaveBeenCalledTimes(2);
  } finally {
    check.mockRestore();
    openPane.mockRestore();
  }
});
