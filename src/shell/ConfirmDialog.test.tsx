import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import { apply, ask, initialState, useHive } from "../store";
import { askDiscard } from "./ConfirmDialog";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const dialog = (name: string) => screen.queryByRole("dialog", { name }) as HTMLDialogElement;
const button = (name: string) => screen.getByRole("button", { name });

function asking() {
  const run = mock(() => {});
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => askDiscard("TODO.md", run));
  return run;
}

test("the question shows in the app, Cancel focused, with no key hints", () => {
  const native = spyOn(window, "confirm");
  asking();
  const asked = dialog("Discard changes?");
  expect(asked.open).toBe(true);
  expect(asked.textContent).toContain("Your unsaved changes to TODO.md will be lost.");
  expect(document.activeElement).toBe(button("Cancel"));
  expect(asked.querySelector("kbd")).toBeNull();
  expect(native).not.toHaveBeenCalled();
  native.mockRestore();
});

test("the action runs once and closes the dialog", () => {
  const run = asking();
  fireEvent.click(button("Discard"));
  expect(run).toHaveBeenCalledTimes(1);
  expect(dialog("Discard changes?")).toBeNull();
  expect(useHive.getState().modal).toBeNull();
});

test("the action may ask again", () => {
  asking();
  const second = mock(() => {});
  act(() =>
    ask({
      title: "First?",
      text: "",
      action: "Go",
      run: () => ask({ title: "Second?", text: "", action: "Go", run: second }),
    }),
  );
  fireEvent.click(button("Go"));
  expect(dialog("Second?").open).toBe(true);
  fireEvent.click(button("Go"));
  expect(second).toHaveBeenCalledTimes(1);
});

test("Cancel, the header button and Esc do nothing", () => {
  const run = asking();
  fireEvent.click(button("Cancel"));
  expect(dialog("Discard changes?")).toBeNull();
  act(() => askDiscard("TODO.md", run));
  fireEvent.click(screen.getByTitle("Close (Esc)"));
  expect(dialog("Discard changes?")).toBeNull();
  act(() => askDiscard("TODO.md", run));
  act(() => dialog("Discard changes?").close());
  expect(dialog("Discard changes?")).toBeNull();
  expect(run).not.toHaveBeenCalled();
});
