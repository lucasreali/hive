import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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

const tab = (name: string) => screen.getByRole("tab", { name: new RegExp(`^${name}`) });

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

test("tabs switch, tell same-named worktrees apart, and show exit and missing hooks", () => {
  show();
  act(() => {
    addTab(1, shopMain.path);
    addTab(2, apiMain.path);
    addTab(3, "/somewhere/else");
  });
  // Both are "main": the project tells them apart. An unknown path shows as is.
  const tabs = () => screen.getAllByRole("tab");
  expect(tabs().map((t) => t.textContent)).toEqual(["mainshop", "mainapi", "/somewhere/else"]);
  expect(tabs().map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "false", "true"]);

  fireEvent.click(tabs()[0]);
  expect(tabs()[0].getAttribute("aria-selected")).toBe("true");
  expect(useHive.getState().selection).toBe(shopMain.path);

  act(() => apply({ type: "unhooked_agent", channel: 1 }));
  act(() => apply({ type: "terminal_exited", channel: 2, code: 3 }));
  act(() => apply({ type: "terminal_exited", channel: 3, code: null }));
  expect(screen.getByText("no hooks").getAttribute("title")).toBe(
    "Claude runs in this terminal without Hive's hooks: its state is not observed",
  );
  expect(tabs()[1].textContent).toBe("mainapiexited");
  expect(screen.getAllByText("exited").map((b) => b.getAttribute("title"))).toEqual([
    "Exit code: 3",
    "Exit code: none (killed)",
  ]);
});

test("the close button ends the terminal and removes its tab", () => {
  const close = spyOn(transport, "closeTerminal");
  show();
  act(() => addTab(7, fixLogin.path));
  fireEvent.click(screen.getByRole("button", { name: "Close terminal fix-login" }));
  expect(close).toHaveBeenCalledWith(7);
  expect(screen.queryByRole("tab")).toBeNull();
  close.mockRestore();
});

test("the open file has its tab after the terminals, shown in place of the terminal", () => {
  show();
  act(() => addTab(1, shopMain.path));
  act(() => setOpenFile({ worktree: shopMain.path, path: "src/app.ts" }));
  const host = () => document.querySelector(".terminal-host") as HTMLElement;
  const tabs = () => screen.getAllByRole("tab").map((t) => t.getAttribute("aria-selected"));
  expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["main", "app.ts"]);
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
  // A new terminal is shown in front of the file.
  act(() => addTab(2, fixLogin.path));
  expect(tabs()).toEqual(["false", "true", "false"]);
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
