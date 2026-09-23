import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, openModal, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

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

function open() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  act(() => openModal("worktree-picker"));
  return screen.getByRole("dialog", { name: "Open a terminal in worktree" }) as HTMLDialogElement;
}

const field = () => screen.getByPlaceholderText("Open a terminal in worktree…");
const rows = () =>
  [...document.querySelectorAll(".picker-row")].map((r) => [
    r.querySelector(".picker-name")?.textContent,
    r.querySelector(".picker-project")?.textContent,
    r.getAttribute("aria-pressed"),
  ]);
const key = (key: string) => fireEvent.keyDown(field(), { key });

test("lists every worktree of every project, the first one picked", () => {
  const dialog = open();
  expect(dialog.open).toBe(true);
  expect(rows()).toEqual([
    ["main", "shop", "true"],
    ["fix-login", "shop", "false"],
    ["feat-checkout", "shop", "false"],
    ["main", "api", "false"],
    ["refactor-auth", "api", "false"],
  ]);
  expect(document.querySelector(".picker-path")?.textContent).toBe(shop.worktrees[0].path);
  expect(dialog.querySelector("footer")?.textContent).toBe("↑↓navigateEnteropen terminalEscclose");
});

test("filters by worktree or project name; arrows move within the list", () => {
  open();
  fireEvent.change(field(), { target: { value: "API" } });
  expect(rows()).toEqual([
    ["main", "api", "true"],
    ["refactor-auth", "api", "false"],
  ]);
  key("ArrowUp");
  expect(rows()[0][2]).toBe("true");
  key("ArrowDown");
  key("ArrowDown");
  expect(rows()[1][2]).toBe("true");
  // Other keys are the field's.
  key("a");
  expect(rows()[1][2]).toBe("true");
  // Typing starts again from the first match.
  fireEvent.change(field(), { target: { value: "login" } });
  expect(rows()).toEqual([["fix-login", "shop", "true"]]);
  fireEvent.change(field(), { target: { value: "nothing" } });
  expect(rows()).toEqual([]);
  expect(screen.getByText("No worktrees found")).toBeDefined();
  // Enter with no match does nothing.
  key("Enter");
  expect(useHive.getState().modal).toBe("worktree-picker");
});

test("Enter opens a terminal in the picked worktree and closes the picker", async () => {
  const openTerminal = spyOn(transport, "openTerminal");
  open();
  key("ArrowDown");
  key("Enter");
  expect(useHive.getState().modal).toBeNull();
  const path = shop.worktrees[1].path;
  await waitFor(() => expect(useHive.getState().tabs.map((t) => t.cwd)).toEqual([path]));
  expect(openTerminal.mock.calls[0][0]).toBe(path);
  openTerminal.mockRestore();
});

test("hovering picks a row and a click opens it", async () => {
  const row = within(open()).getByRole("button", { name: /refactor-auth/ });
  fireEvent.mouseEnter(row);
  expect(rows()[4][2]).toBe("true");
  fireEvent.click(row);
  await waitFor(() =>
    expect(useHive.getState().tabs.map((t) => t.cwd)).toEqual([api.worktrees[1].path]),
  );
});

test("Esc closes it", () => {
  const dialog = open();
  act(() => dialog.dispatchEvent(new Event("close")));
  expect(useHive.getState().modal).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
});
