import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { version } from "../../package.json";
import { App } from "../App";
import { COMMANDS } from "../shortcuts";
import { apply, DEFAULT_SETTINGS, initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_DIAGNOSTICS, MOCK_REPOS } from "../transport/mock";
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
  fireEvent.keyDown(document.body, { key: ",", ctrlKey: true });
  return screen.getByRole("dialog", { name: "Settings" }) as HTMLDialogElement;
}

const section = (name: string) => fireEvent.click(screen.getByRole("button", { name }));
const input = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
const search = (value: string) =>
  fireEvent.change(screen.getByRole("searchbox", { name: "Search settings" }), {
    target: { value },
  });
const settings = () => useHive.getState().settings;
const labels = () =>
  [...document.querySelectorAll(".settings-body .field")].map((f) => f.textContent);

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
  fireEvent.click(screen.getByRole("button", { name: "Close Esc" }));
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
  section("Agents");
  fireEvent.change(input("Silence before waiting for you (seconds)"), { target: { value: "9" } });
  await waitFor(() => expect(settings().agents.silence_secs).toBe(9));
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
  expect(labels()).toHaveLength(2);
});

test("shortcuts are the command table, read-only", () => {
  open();
  section("Shortcuts");
  const rows = [...document.querySelectorAll(".settings-keys tr")].map((r) => r.textContent);
  expect(rows).toEqual(COMMANDS.map((c) => `${c.label}${c.keys}`));
});

test("about shows both versions, the service's diagnostics and terminals without hooks", async () => {
  open();
  const ask = spyOn(transport, "getDiagnostics");
  act(() => {
    apply({ type: "terminal_opened", channel: 3 });
    apply({ type: "unhooked_agent", channel: 3 });
    apply({ type: "terminal_opened", channel: 4 });
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
  act(() => apply({ type: "diagnostics", settings_file: "/s", wrapper: "/w", claude: null }));
  expect(about()).toContain("claude it runsnot found on PATH");
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

test("Open settings file asks the service, whose answer opens it", async () => {
  open();
  const ask = spyOn(transport, "openSettingsFile");
  fireEvent.click(screen.getByRole("button", { name: "Open settings file" }));
  expect(ask).toHaveBeenCalledTimes(1);
  ask.mockRestore();
});
