import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, openModal, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;

function open() {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => apply({ type: "projects", projects: [] }));
  fireEvent.click(screen.getByRole("button", { name: "Add project Ctrl+Shift+O" }));
  return screen.getByRole("dialog", { name: "Add project" }) as HTMLDialogElement;
}

const field = () => screen.getByLabelText("Folder in WSL") as HTMLInputElement;
const submit = () => screen.getByRole("button", { name: "Add project Enter" }) as HTMLButtonElement;

test("the empty state opens a modal dialog focused on the folder field", () => {
  const dialog = open();
  expect(dialog.open).toBe(true);
  expect(document.activeElement).toBe(field());
  expect(submit().disabled).toBe(true);
  fireEvent.change(field(), { target: { value: "  " } });
  expect(submit().disabled).toBe(true);
  expect(dialog.textContent).toContain("A git repository, or any folder inside one");
});

test("the typed path goes to the service, and the answer closes the dialog", async () => {
  const add = spyOn(transport, "addProject");
  open();
  fireEvent.change(field(), { target: { value: shop.path } });
  fireEvent.click(submit());
  expect(add).toHaveBeenCalledWith(shop.path);
  add.mockRestore();
  // Tests run outside Tauri, so the mock service answers.
  await waitFor(() => expect(useHive.getState().modal).toBeNull());
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByRole("button", { name: "shop" })).toBeDefined();
});

test("editing the field clears the refusal", () => {
  open();
  fireEvent.change(field(), { target: { value: "/" } });
  act(() =>
    apply({ type: "add_project_failed", path: "/", error: "not_a_git_repository", message: "m" }),
  );
  fireEvent.change(field(), { target: { value: "" } });
  expect(screen.queryByRole("alert")).toBeNull();
  expect(field().getAttribute("aria-invalid")).toBe("false");
});

test("a refused path is explained under the field until the dialog reopens", () => {
  open();
  const message = "cannot open /x: No such file or directory (os error 2)";
  act(() => apply({ type: "add_project_failed", path: "/x", error: "not_found", message }));
  expect(screen.getByRole("alert").textContent).toBe(message);
  expect(field().getAttribute("aria-invalid")).toBe("true");
  act(() => openModal("add-project"));
  expect(screen.queryByRole("alert")).toBeNull();
});

test("close, cancel and Esc close the dialog", () => {
  for (const how of ["Close (Esc)", "Cancel", "Escape"]) {
    const dialog = open();
    if (how === "Escape") fireEvent(dialog, new Event("close"));
    else if (how === "Cancel") fireEvent.click(screen.getByRole("button", { name: "Cancel Esc" }));
    else fireEvent.click(screen.getByTitle(how));
    expect(useHive.getState().modal).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    cleanup();
  }
});

test("the dialog stays closed while the connection is blocked", () => {
  open();
  act(() => apply({ type: "disconnected", reason: "gone" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});
