import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "../App";
import { addTab, apply, initialState, select, setEdit, setOpenFile, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { toText } from "../viewer/buffer";

const [shop, api] = MOCK_REPOS;
const [shopMain, fixLogin] = shop.worktrees;
const [apiMain] = api.worktrees;

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

function show() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
}

const bar = () => within(screen.getByRole("tablist", { name: "Open terminals and files" }));
const tab = (name: string) => bar().getByRole("tab", { name: new RegExp(`^${name}`) });

test("New terminal opens one in the selected worktree, shown in its tab", async () => {
  const open = spyOn(transport, "openTerminal");
  show();
  const button = screen.getByTitle("New terminal (Ctrl+Shift+T)") as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "fix-login" }));
  fireEvent.click(button);
  expect(open.mock.calls[0][0]).toBe(fixLogin.path);
  await waitFor(() => expect(tab("fix-login").getAttribute("aria-selected")).toBe("true"));
  expect(tab("fix-login").closest(".tab")?.getAttribute("title")).toBe(fixLogin.path);
  open.mockRestore();
});

test("with an agent selected (F8), New terminal opens one in the agent's worktree", () => {
  const open = spyOn(transport, "openTerminal");
  show();
  const button = screen.getByTitle("New terminal (Ctrl+Shift+T)") as HTMLButtonElement;
  const agent = { type: "agent_detected", channel: 9, cwd: "/x" } as const;
  act(() => {
    apply({ ...agent, id: "s", project: shop.id, worktree: fixLogin.id });
    select("s");
  });
  fireEvent.click(button);
  expect(open.mock.calls.map((call) => call[0])).toEqual([fixLogin.path]);
  // An agent outside every project has no worktree to open one in.
  act(() => {
    apply({ ...agent, id: "out", project: null, worktree: null });
    select("out");
  });
  expect(button.disabled).toBe(true);
  open.mockRestore();
});

test("tabs switch and show exit and missing hooks", () => {
  show();
  act(() => {
    addTab(1, shopMain.path);
    addTab(2, apiMain.path);
    addTab(3, "/somewhere/else");
  });
  // Only the selected worktree's tabs show: the last one opened selected its own place.
  const tabs = () => bar().getAllByRole("tab");
  expect(tabs().map((t) => t.textContent)).toEqual(["/somewhere/else"]);
  // With nothing selected every tab shows, by its worktree's name (no project: a tab bar
  // shows one worktree's tabs).
  act(() => select(null));
  expect(tabs().map((t) => t.textContent)).toEqual(["main", "main", "/somewhere/else"]);
  expect(tabs().map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "false", "true"]);

  fireEvent.click(tabs()[0]);
  expect(tabs().map((t) => t.getAttribute("aria-selected"))).toEqual(["true"]);
  expect(useHive.getState().selection).toBe(shopMain.path);
  act(() => select(null));

  act(() => apply({ type: "unhooked_agent", channel: 1 }));
  act(() => apply({ type: "terminal_exited", channel: 2, code: 3 }));
  act(() => apply({ type: "terminal_exited", channel: 3, code: null }));
  expect(screen.getByText("no hooks").getAttribute("title")).toBe(
    "Claude runs in this terminal without Hive's hooks: its state is not observed",
  );
  expect(tabs()[1].textContent).toBe("mainexited");
  expect(screen.getAllByText("exited").map((b) => b.getAttribute("title"))).toEqual([
    "Exit code: 3",
    "Exit code: none (killed)",
  ]);
});

test("a tab running Claude shows the agent's state and its session's name", () => {
  show();
  act(() => addTab(4, fixLogin.path));
  const tab = () => bar().getByRole("tab");
  expect(tab().querySelector(".tab-name")?.textContent).toBe("fix-login");
  const agent = { type: "agent_detected", channel: 4, id: "s", project: shop.id } as const;
  act(() => apply({ ...agent, worktree: fixLogin.id, cwd: fixLogin.path }));
  // Idle until its first state; the worktree's name until the session has one.
  expect(tab().querySelector("[role=img]")?.getAttribute("aria-label")).toBe("idle");
  act(() => {
    apply({
      type: "agent_state",
      id: "s",
      state: "working",
      urgency: 2,
      pending: false,
      subagents: [],
      activity: null,
      since_ms: 0,
    });
    apply({ type: "agent_title", channel: 4, id: "s", title: "Fix the login redirect" });
  });
  expect(tab().querySelector("[role=img]")?.getAttribute("aria-label")).toBe("working");
  expect(tab().querySelector(".tab-name")?.textContent).toBe("Fix the login redirect");
  expect(tab().closest(".tab")?.getAttribute("title")).toBe(
    `Fix the login redirect\n${fixLogin.path}`,
  );
  expect(
    screen.getByRole("button", { name: "Close terminal Fix the login redirect" }),
  ).toBeDefined();
  // The session ended: the tab is the worktree's again.
  act(() => apply({ type: "agent_removed", channel: 4, id: "s" }));
  expect(tab().querySelector(".tab-name")?.textContent).toBe("fix-login");
  expect(useHive.getState().agentTitles).toEqual({});
});

test("the close button ends the terminal and removes its tab", () => {
  const close = spyOn(transport, "closeTerminal");
  show();
  act(() => addTab(7, fixLogin.path));
  fireEvent.click(screen.getByRole("button", { name: "Close terminal fix-login" }));
  expect(close).toHaveBeenCalledWith(7);
  expect(bar().queryByRole("tab")).toBeNull();
  close.mockRestore();
});

test("the open file has its tab after the terminals, shown in place of the terminal", () => {
  show();
  act(() => addTab(1, shopMain.path));
  act(() => setOpenFile({ worktree: shopMain.path, path: "src/app.ts" }));
  const host = () => document.querySelector(".terminal-host") as HTMLElement;
  const tabs = () =>
    bar()
      .getAllByRole("tab")
      .map((t) => t.getAttribute("aria-selected"));
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["main", "app.ts"]);
  expect(tab("app.ts").closest(".tab")?.getAttribute("title")).toBe("src/app.ts");
  expect(tabs()).toEqual(["false", "true"]);
  expect(host().hidden).toBe(true);
  expect(screen.getByRole("region", { name: "src/app.ts" })).toBeDefined();

  fireEvent.click(tab("main"));
  expect(tabs()).toEqual(["true", "false"]);
  expect(host().hidden).toBe(false);
  expect(screen.queryByRole("region", { name: "src/app.ts" })).toBeNull();

  fireEvent.click(tab("app.ts"));
  expect(tabs()).toEqual(["false", "true"]);
  // A terminal in another worktree selects it: only its tab shows, the file's goes with main.
  act(() => addTab(2, fixLogin.path));
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["fix-login"]);
  expect(screen.queryByRole("region", { name: "src/app.ts" })).toBeNull();
  act(() => select(shopMain.id));
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["main", "app.ts"]);
  expect(tabs()).toEqual(["true", "false"]);
  // Leaving the worktree while its file is shown hides the file; coming back shows a terminal.
  fireEvent.click(tab("app.ts"));
  act(() => select(fixLogin.id));
  expect(useHive.getState().fileShown).toBe(false);
  act(() => select(shopMain.id));
  expect(tabs()).toEqual(["true", "false"]);
});

test("a worktree without terminals says so and opens one", () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(5);
  show();
  act(() => addTab(1, shopMain.path));
  act(() => select(fixLogin.id));
  expect(bar().queryByRole("tab")).toBeNull();
  expect(screen.getByText("No terminal in fix-login")).toBeDefined();
  expect((document.querySelector(".terminal-host") as HTMLElement).hidden).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "New terminal" }));
  expect(open.mock.calls[0]?.[0]).toBe(fixLogin.path);
  open.mockRestore();
});

test("unsaved edits put a dot in place of the file tab's ×, named for screen readers", () => {
  show();
  act(() => setOpenFile({ worktree: shopMain.path, path: "src/app.ts" }));
  const buffer = {
    worktree: shopMain.path,
    path: "src/app.ts",
    doc: toText("one\n"),
    saved: toText("one\n"),
    version: "v",
    conflict: null,
    saving: null,
    error: null,
    recheck: 0,
  };
  act(() => setEdit(buffer));
  const clean = screen.getByRole("button", { name: "Close file app.ts" });
  expect(clean.hasAttribute("data-dirty")).toBe(false);
  expect(clean.querySelector(".dirty")).toBeNull();

  act(() => setEdit({ ...buffer, doc: toText("two\n") }));
  const dirty = screen.getByRole("button", { name: "Close file app.ts (unsaved changes)" });
  expect(dirty.hasAttribute("data-dirty")).toBe(true);
  expect(dirty.querySelector(".dirty")).not.toBeNull();
  // The dot is on the × only, not a second marker in the label.
  expect(tab("app.ts").querySelector(".dirty")).toBeNull();
});
