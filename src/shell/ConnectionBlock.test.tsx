import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, setOpenFile, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const dialog = () => screen.queryByRole("alertdialog");
const workspace = () => screen.getByRole("main");

test("no block while connecting or connected", () => {
  render(<App />);
  expect(dialog()).toBeNull();
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  expect(dialog()).toBeNull();
  expect(workspace().inert).toBe(false);
});

test("a version mismatch blocks the workspace with both versions and the fix", () => {
  render(<App />);
  act(() =>
    apply({
      type: "version_mismatch",
      protocol: 2,
      version: "0.2.0",
      app_protocol: 1,
      app_version: "0.1.0",
    }),
  );
  const block = dialog() as HTMLElement;
  expect(block.getAttribute("aria-modal")).toBe("true");
  expect(block.textContent).toContain("The app and the hive service versions differ");
  expect(block.textContent).toContain("App0.1.0 (protocol 1)Service0.2.0 (protocol 2)");
  expect(block.querySelector("pre")?.textContent).toBe(
    "cargo install --path crates/hive\npkill -f 'hive daemon'",
  );
  expect(workspace().inert).toBe(true);
  // The title bar stays usable so the window can be closed.
  expect(screen.getByTitle("Close").closest("[inert]")).toBeNull();
  expect(document.activeElement?.textContent).toBe("Reconnect");
});

test("a disconnect shows the reason, and reconnect connects again", async () => {
  render(<App />);
  act(() => apply({ type: "disconnected", reason: "wsl.exe: distro not found" }));
  const block = dialog() as HTMLElement;
  expect(block.textContent).toContain("Lost the connection to the hive service");
  expect(block.querySelector("pre")?.textContent).toBe("wsl.exe: distro not found");
  expect(workspace().inert).toBe(true);

  fireEvent.click(screen.getByText("Reconnect"));
  expect(useHive.getState().connection.status).toBe("connecting");
  expect(dialog()).toBeNull();
  // Tests run outside Tauri, so the mock transport answers with `welcome`.
  await waitFor(() => expect(useHive.getState().connection.status).toBe("connected"));
  expect(workspace().inert).toBe(false);
});

test("after a reconnect, messages reach the same handler as at startup", async () => {
  render(<App />);
  act(() => apply({ type: "disconnected", reason: "gone" }));
  fireEvent.click(screen.getByText("Reconnect"));
  await waitFor(() => expect(useHive.getState().connection.status).toBe("connected"));
  // `editor_target` is handled outside the store: only the full handler shows the notice.
  const [shop] = MOCK_REPOS;
  act(() => setOpenFile({ worktree: shop.path, path: "README.md" }));
  await transport.openInEditor(shop.path, "README.md");
  await waitFor(() =>
    expect(useHive.getState().editorNotice).toStartWith(
      "Only the Hive app opens an external editor",
    ),
  );
});
