import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "../App";
import { COMMANDS } from "../shortcuts";
import { apply, DEFAULT_SETTINGS, initialState, NO_SCRIPTS, openModal, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";
import { FILE_LIMIT, fuzzy, type PaletteItem, rank } from "./Palette";

beforeAll(async () => {
  await transport.connect(apply);
  await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const fixLogin = shop.worktrees[1];

function open() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  act(() => openModal("palette"));
  return screen.getByRole("dialog", { name: "Command palette" }) as HTMLDialogElement;
}

const field = () => screen.getByRole("textbox", { name: /Search commands/ });
const type = (value: string) => fireEvent.change(field(), { target: { value } });
const key = (key: string) => fireEvent.keyDown(field(), { key });
/** Each group's name and its rows' labels. */
const groups = () =>
  [...document.querySelectorAll(".palette section")].map((g) => [
    g.getAttribute("aria-label"),
    [...g.querySelectorAll(".picker-name")].map((n) => n.textContent),
  ]);
const picked = () => document.querySelector('.picker-row[aria-pressed="true"] .picker-name');

test("fuzzy matches a subsequence in any case, favouring runs and word starts", () => {
  expect(fuzzy("", "anything")).toBe(0);
  expect(fuzzy("xyz", "settings")).toBeNull();
  expect(fuzzy("OS", "Open settings")).not.toBeNull();
  // Consecutive letters beat scattered ones; a word start beats the middle of a word.
  expect(fuzzy("set", "Open settings") as number).toBeGreaterThan(
    fuzzy("set", "some extra tail") as number,
  );
  expect(fuzzy("s", "a/src") as number).toBeGreaterThan(fuzzy("s", "abs") as number);
  const item = (label: string, detail?: string): PaletteItem => ({ label, detail, run() {} });
  const ranked = rank([item("some extra tail"), item("x"), item("Open", "settings")], "set");
  expect(ranked.map((i) => i.label)).toEqual(["Open", "some extra tail"]);
});

test("lists every command with its keys but itself, then agents and worktrees", () => {
  const dialog = open();
  expect(dialog.open).toBe(true);
  const [commands, places] = groups();
  expect(commands).toEqual([
    "Commands",
    [...COMMANDS.filter((c) => c.id !== "palette").map((c) => c.label), "Remove merged worktrees…"],
  ]);
  expect(places).toEqual([
    "Agents and worktrees",
    ["main", "fix-login", "feat-checkout", "main", "refactor-auth"],
  ]);
  expect(document.querySelector(".picker-row kbd")?.textContent).toBe("Ctrl+,");
  expect(picked()?.textContent).toBe("Open settings");
  expect(dialog.querySelector("footer")?.textContent).toBe("↑↓navigateEnterrunEscclose");
});

test("filters as typed, arrows move across groups, Enter runs the row", () => {
  open();
  type("refac");
  expect(groups()).toEqual([["Agents and worktrees", ["refactor-auth"]]]);
  type("settings");
  expect(picked()?.textContent).toBe("Open settings");
  key("ArrowUp");
  expect(picked()?.textContent).toBe("Open settings");
  key("ArrowDown");
  key("ArrowDown");
  key("ArrowDown");
  const rows = document.querySelectorAll(".picker-row").length;
  expect(picked()).toBe(document.querySelectorAll(".picker-row .picker-name")[rows - 1] ?? null);
  key("Tab");
  type("open settings");
  key("Enter");
  expect(useHive.getState().modal).toBe("settings");
});

test("says so when nothing matches; Enter then does nothing", () => {
  open();
  type("zzzqqq");
  expect(screen.getByText("No matches")).toBeDefined();
  key("Enter");
  expect(useHive.getState().modal).toBe("palette");
});

test("the mouse picks and runs a row; a click outside closes", () => {
  const dialog = open();
  type("remove merged");
  const row = screen.getByRole("button", { name: /Remove merged worktrees…/ });
  fireEvent.mouseEnter(row);
  expect(row.getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(row);
  expect(useHive.getState()).toMatchObject({ modal: "remove-merged", modalProject: shop.id });
  act(() => openModal("palette"));
  fireEvent.click(screen.getByRole("dialog", { name: "Command palette" }));
  expect(useHive.getState().modal).toBeNull();
  expect(dialog.isConnected).toBe(false);
});

test("Enter on a worktree selects it and expands its project", () => {
  open();
  act(() => useHive.setState({ collapsed: { [shop.id]: true } }));
  type("fix-login");
  key("Enter");
  expect(useHive.getState()).toMatchObject({
    modal: null,
    selection: fixLogin.id,
    collapsed: { [shop.id]: false },
  });
});

test("the selected worktree's run scripts are commands that type them into a new terminal", async () => {
  const terminal = spyOn(transport, "openTerminal").mockResolvedValue(8);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.projects[shop.id] = {
    scripts: { ...NO_SCRIPTS, run: [{ name: "dev", command: "bun dev" }] },
  };
  act(() => apply({ type: "settings", settings }));
  open();
  expect(screen.queryByRole("button", { name: /^Run: dev/ })).toBeNull();
  act(() => useHive.setState({ selection: fixLogin.id }));
  act(() => openModal("palette"));
  type("run dev");
  expect(picked()?.textContent).toBe("Run: dev");
  key("Enter");
  expect(terminal.mock.calls[0]?.[0]).toBe(fixLogin.path);
  await waitFor(() => expect(write).toHaveBeenCalledWith(8, "bun dev\r"));
  terminal.mockRestore();
  write.mockRestore();
});

test("worktrees are the current space's only", () => {
  const env = { claude_config_dir: null, git_name: null, git_email: null, gh_config_dir: null };
  open();
  act(() =>
    apply({
      type: "spaces",
      spaces: [
        { id: "default", name: "Default", projects: [shop.id], env },
        { id: "space-1", name: "Work", projects: [api.id], env },
      ],
      current: "space-1",
    }),
  );
  expect(groups()[1]).toEqual(["Agents and worktrees", ["main", "refactor-auth"]]);
});

test("agents show their name, state and worktree; Enter goes to the agent", () => {
  open();
  act(() => {
    useHive.setState({ tabs: [{ id: 7, cwd: fixLogin.path }], collapsed: { [shop.id]: true } });
    apply({
      type: "agent_detected",
      channel: 7,
      id: "s1",
      project: shop.id,
      worktree: fixLogin.id,
      cwd: fixLogin.path,
    });
    apply({
      type: "agent_detected",
      channel: 8,
      id: "s2",
      project: null,
      worktree: null,
      cwd: "/elsewhere",
    });
    apply({ type: "agent_title", channel: 7, id: "s1", title: "Fix the login" });
    apply({ type: "agent_state", id: "s1", ...agentStatus("waiting_you"), subagents: [] });
  });
  const details = [...document.querySelectorAll(".palette section")][1]?.querySelectorAll(
    ".picker-path",
  );
  expect(details?.[0]?.textContent).toBe("waiting for you · fix-login");
  expect(details?.[1]?.textContent).toBe("/elsewhere");
  type("fix the");
  key("Enter");
  expect(useHive.getState()).toMatchObject({
    modal: null,
    selection: "s1",
    activeTab: 7,
    collapsed: { [shop.id]: false, [`worktree:${fixLogin.id}`]: false },
  });
});

test("Send review shows once the open file has comments for the active terminal", () => {
  open();
  const file = { worktree: fixLogin.path, path: "a.ts" };
  act(() =>
    useHive.setState({
      tabs: [{ id: 3, cwd: fixLogin.path }],
      activeTab: 3,
      openFile: file,
      comments: { [fixLogin.path]: [{ path: "a.ts", from: 1, to: 1, text: "why?" }] },
    }),
  );
  type("send review");
  fireEvent.click(screen.getByRole("button", { name: /^Send review/ }));
  expect(useHive.getState().modal).toBeNull();
});

test("files: the selected worktree's lines holding the text, once typing pauses", async () => {
  const search = spyOn(transport, "searchFiles").mockImplementation(async () => {});
  open();
  act(() => useHive.setState({ selection: fixLogin.id }));
  type("tok");
  type("token");
  expect(screen.getByText("Searching files…")).toBeDefined();
  await waitFor(() => expect(search).toHaveBeenCalledTimes(1));
  expect(search).toHaveBeenCalledWith(fixLogin.path, "token");
  const matches = [
    { path: "src/auth/session.ts", line: 3, text: "  a token  " },
    { path: "docs/notes.md", line: 9, text: "token" },
    ...Array.from({ length: FILE_LIMIT }, (_, i) => ({ path: `f${i}`, line: 1, text: "token" })),
  ];
  act(() => {
    apply({ type: "changes", path: fixLogin.path, files: [], added: 0, removed: 0, error: null });
    apply({
      type: "search_results",
      worktree: fixLogin.path,
      query: "token",
      matches,
      truncated: false,
      error: null,
    });
  });
  const files = groups().find(([name]) => name === "Files")?.[1] as string[];
  expect(files).toHaveLength(FILE_LIMIT);
  expect(files[0]).toBe("src/auth/session.ts:3");
  expect(screen.getByText("a token")).toBeDefined();
  key("Enter");
  expect(useHive.getState()).toMatchObject({
    modal: null,
    openFile: { worktree: fixLogin.path, path: "src/auth/session.ts" },
    // Unchanged: editable text, as the Files panel opens it.
    editing: true,
    gotoLine: { line: 3 },
  });
  search.mockRestore();
});
