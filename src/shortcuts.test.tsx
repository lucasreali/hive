import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { App } from "./App";
import { nextPending, shortcut } from "./shortcuts";
import { apply, initialState, openModal, select, useHive } from "./store";
import { closeTerminal, openTerminal, terminal } from "./terminals";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

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

function app() {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => apply({ type: "projects", projects: [shop, api] }));
}

type Keys = { key: string; ctrlKey?: boolean; shiftKey?: boolean; altKey?: boolean };
const ctrlShift = (key: string): Keys => ({ key, ctrlKey: true, shiftKey: true });
/** Presses keys with the focus outside any terminal; true when the app took them. */
const press = (keys: Keys, target: Element = document.body) => {
  let taken = false;
  act(() => {
    taken = !fireEvent.keyDown(target, keys);
  });
  return taken;
};

test("Ctrl+Shift+T opens the worktree picker", () => {
  app();
  expect(press(ctrlShift("T"))).toBe(true);
  expect(useHive.getState().modal).toBe("worktree-picker");
});

test("Ctrl+Shift+N opens the new worktree dialog for the current project", () => {
  app();
  // Nothing selected: the first project.
  press(ctrlShift("N"));
  expect(useHive.getState()).toMatchObject({ modal: "new-worktree", modalProject: shop.id });
  // A selected project, or the project of a selected worktree.
  for (const selection of [api.id, api.worktrees[1].id]) {
    act(() => openModal(null));
    act(() => select(selection));
    press(ctrlShift("n"));
    expect(useHive.getState()).toMatchObject({ modal: "new-worktree", modalProject: api.id });
  }
});

test("Ctrl+Shift+N without projects opens the add project dialog", () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [] }));
  press(ctrlShift("N"));
  expect(useHive.getState().modal).toBe("add-project");
});

test("Ctrl+Shift+B toggles the files panel and Ctrl+Shift+O opens add project", () => {
  app();
  press(ctrlShift("B"));
  expect(useHive.getState().rightPanel).toBe("files");
  press(ctrlShift("B"));
  expect(useHive.getState().rightPanel).toBeNull();
  press(ctrlShift("O"));
  expect(useHive.getState().modal).toBe("add-project");
});

test("F8 goes to the next pending agent, which no agent is before Stage 2", () => {
  app();
  expect(shortcut(new KeyboardEvent("keydown", { key: "F8" }))).toBe(nextPending);
  const before = useHive.getState();
  expect(press({ key: "F8" })).toBe(true);
  expect(useHive.getState()).toBe(before);
});

test("other keys are not shortcuts", () => {
  app();
  for (const keys of [
    { key: "T", ctrlKey: true },
    { key: "O", ctrlKey: true },
    { key: "b", altKey: true },
    { key: "t" },
    { key: "F8", shiftKey: true },
    { key: "T", ctrlKey: true, shiftKey: true, altKey: true },
    ctrlShift("C"),
    ctrlShift("V"),
    ctrlShift("X"),
  ]) {
    expect(press(keys)).toBe(false);
  }
  expect(useHive.getState()).toMatchObject({ modal: null, rightPanel: null });
});

test("nothing runs under the connection block or while a dialog is open", () => {
  app();
  act(() => openModal("add-project"));
  expect(press(ctrlShift("B"))).toBe(false);
  act(() => openModal(null));
  for (const block of [
    { type: "disconnected", reason: "gone" } as const,
    {
      type: "version_mismatch",
      protocol: 2,
      version: "0.2.0",
      app_protocol: 1,
      app_version: "0.1.0",
    } as const,
  ]) {
    act(() => apply(block));
    expect(press(ctrlShift("B"))).toBe(false);
    expect(press(ctrlShift("T"))).toBe(false);
    expect(press({ key: "F8" })).toBe(false);
  }
  expect(useHive.getState()).toMatchObject({ modal: null, rightPanel: null });
});

test("in a terminal, shortcuts run once and never reach it; other keys do", async () => {
  const write = spyOn(transport, "writeTerminal");
  app();
  const id = await act(() => openTerminal(shop.worktrees[1].path));
  const textarea = terminal(id)?.textarea as HTMLTextAreaElement;
  press(ctrlShift("B"), textarea);
  expect(useHive.getState().rightPanel).toBe("files");
  press({ key: "F8", code: "F8", keyCode: 119 } as Keys, textarea);
  expect(write).not.toHaveBeenCalled();
  // Ctrl+O is Claude Code's: it goes to the terminal.
  press({ key: "o", code: "KeyO", keyCode: 79, ctrlKey: true } as Keys, textarea);
  await waitFor(() => expect(write).toHaveBeenCalledWith(id, "\x0f"));
  expect(useHive.getState()).toMatchObject({ modal: null, rightPanel: "files" });
  write.mockRestore();
});

test("the app stops listening when it unmounts", () => {
  app();
  cleanup();
  press(ctrlShift("B"));
  expect(useHive.getState().rightPanel).toBeNull();
});
