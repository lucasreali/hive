import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import { newPull } from "../pulls";
import { apply, initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const checkout = shop.worktrees[2];

function open() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  act(() => newPull(shop.id, checkout.id));
  return screen.getByRole("dialog", { name: "Create pull request" });
}

test("creates from the worktree's branch into the main worktree's by default", () => {
  const create = spyOn(transport, "createPull").mockResolvedValue();
  const dialog = open();
  expect(dialog.textContent).toContain(
    `From ${checkout.branch}, which must be pushed to GitHub first.`,
  );
  const title = within(dialog).getByLabelText("Title") as HTMLInputElement;
  const submit = within(dialog).getByRole("button", { name: "Create" }) as HTMLButtonElement;
  expect(document.activeElement).toBe(title);
  expect((within(dialog).getByLabelText("Into") as HTMLInputElement).value).toBe("main");
  expect(submit.disabled).toBe(true);
  fireEvent.change(title, { target: { value: "Checkout" } });
  fireEvent.change(within(dialog).getByLabelText("Into"), { target: { value: "develop" } });
  fireEvent.change(within(dialog).getByLabelText("Description"), { target: { value: "Why" } });
  fireEvent.click(within(dialog).getByLabelText("Draft"));
  fireEvent.click(submit);
  expect(create).toHaveBeenCalledWith(checkout.path, "Checkout", "Why", "develop", true);
  expect(submit.textContent).toBe("Creating…");
  expect(submit.disabled).toBe(true);

  // gh's refusal shows here; an action's on a pull request does not.
  act(() => apply({ type: "pull_failed", project: shop.id, number: 3, message: "other" }));
  expect(within(dialog).queryByRole("alert")).toBeNull();
  const message =
    "gh pr create failed: pull request create failed: GraphQL: Head sha can't be blank";
  act(() => apply({ type: "pull_failed", project: shop.id, number: null, message }));
  expect(within(dialog).getByRole("alert").textContent).toBe(message);
  expect(submit.textContent).toBe("Create");

  // Opened: the dialog closes and the status bar says so.
  act(() =>
    apply({ type: "pull_done", project: shop.id, number: 21, message: "Opened pull request #21" }),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(useHive.getState().notice).toBe("Opened pull request #21");
  create.mockRestore();
});

test("Cancel closes; a worktree that is gone shows nothing", () => {
  const dialog = open();
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  act(() => newPull(shop.id, "/gone"));
  expect(screen.queryByRole("dialog")).toBeNull();
  // A project whose main worktree is on no branch offers "main".
  const detached = {
    ...shop,
    worktrees: shop.worktrees.map((w) => (w.main ? { ...w, branch: null } : w)),
  };
  act(() => apply({ type: "projects", projects: [detached] }));
  act(() => newPull(shop.id, checkout.id));
  const again = screen.getByRole("dialog", { name: "Create pull request" });
  expect((within(again).getByLabelText("Into") as HTMLInputElement).value).toBe("main");
  fireEvent.click(within(again).getByTitle("Close (Esc)"));
  expect(useHive.getState().modal).toBeNull();
});
