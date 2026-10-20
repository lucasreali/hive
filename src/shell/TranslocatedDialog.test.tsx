import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { notice } from "../../test/notice";
import { App } from "../App";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { UNQUARANTINE } from "./TranslocatedDialog";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const dialog = () => screen.queryByRole("dialog", { name: "Move Hive to Applications" });
const updateButton = () => screen.getByRole("button", { name: "Restart to update to v0.2.0" });

test("the first UI of a translocated run opens the notice once; OK closes it", () => {
  render(<App />);
  act(() => apply({ type: "translocated", notice: true }));
  const shown = dialog() as HTMLDialogElement;
  expect(shown.open).toBe(true);
  expect(shown.textContent).toContain("read-only copy");
  expect(shown.querySelector("code")?.textContent).toBe(UNQUARANTINE);
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "OK" }));
  fireEvent.click(screen.getByRole("button", { name: "OK" }));
  expect(dialog()).toBeNull();
  // A reloaded UI learns it again, without the notice.
  act(() => apply({ type: "translocated", notice: false }));
  expect(dialog()).toBeNull();
  expect(useHive.getState().translocated).toBe(true);
});

test("the header button and Esc close the notice", () => {
  render(<App />);
  act(() => apply({ type: "translocated", notice: true }));
  fireEvent.click(screen.getByTitle("Close (Esc)"));
  expect(dialog()).toBeNull();
  act(() => apply({ type: "translocated", notice: true }));
  act(() => (dialog() as HTMLDialogElement).close());
  expect(dialog()).toBeNull();
});

test("the command is copied, saying so", async () => {
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  render(<App />);
  act(() => apply({ type: "translocated", notice: true }));
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy command" })));
  expect(writeText).toHaveBeenCalledWith(UNQUARANTINE);
  expect(notice()).toBe("Copied the command");
  writeText.mockRestore();
});

test("translocated, the update button explains the move instead of installing", () => {
  const install = spyOn(transport, "installUpdate").mockResolvedValue();
  render(<App />);
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    apply({ type: "translocated", notice: false });
    apply({ type: "update_ready", version: "0.2.0" });
  });
  expect(updateButton().title).toBe("Move Hive to Applications to update");
  fireEvent.click(updateButton());
  expect((dialog() as HTMLDialogElement).open).toBe(true);
  expect(install).not.toHaveBeenCalled();
  install.mockRestore();
});
