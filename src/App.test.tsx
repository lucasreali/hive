import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "./App";
import { apply, initialState, useHive } from "./store";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

test("renders the shell regions", () => {
  render(<App />);
  expect(screen.getByRole("banner").textContent).toBe("Hive");
  expect(screen.getByRole("navigation", { name: "Projects" }).textContent).toContain("No projects");
  expect(screen.getByRole("region", { name: "Terminals" })).toBeDefined();
  expect(screen.getByRole("contentinfo").textContent).toBe("WSLconnecting");
  expect(screen.queryByRole("complementary")).toBeNull();
});

test("status bar follows the connection status", () => {
  render(<App />);
  const status = screen.getByTitle("WSL connection");
  expect(status.dataset.status).toBe("connecting");
  act(() => apply({ type: "welcome", version: "0.1.0" }));
  expect(status.dataset.status).toBe("connected");
  expect(status.textContent).toBe("WSLconnected");
  act(() => apply({ type: "version_mismatch", protocol: 2, version: "0.2.0" }));
  expect(status.textContent).toBe("WSLversion mismatch");
  act(() => apply({ type: "disconnected", reason: "gone" }));
  expect(status.textContent).toBe("WSLdisconnected");
});

test("the files button toggles the right panel", () => {
  render(<App />);
  const toggle = screen.getByTitle("Files and diff (Ctrl+Shift+B)");
  expect(toggle.getAttribute("aria-pressed")).toBe("false");

  fireEvent.click(toggle);
  expect(useHive.getState().rightPanel).toBe("files");
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  expect(screen.getByRole("complementary", { name: "Files and diff" }).textContent).toContain(
    "Select a project or agent to see its files.",
  );

  fireEvent.click(toggle);
  expect(screen.queryByRole("complementary")).toBeNull();

  fireEvent.click(toggle);
  fireEvent.click(screen.getByTitle("Collapse (Ctrl+Shift+B)"));
  expect(useHive.getState().rightPanel).toBeNull();
  expect(screen.queryByRole("complementary")).toBeNull();
});

test("the project button opens the add-project dialog", () => {
  render(<App />);
  fireEvent.click(screen.getByTitle("Add project (Ctrl+Shift+O)"));
  expect(useHive.getState().modal).toBe("add-project");
});

test("new terminal waits for the worktree picker", () => {
  render(<App />);
  expect((screen.getByTitle("New terminal (Ctrl+Shift+T)") as HTMLButtonElement).disabled).toBe(
    true,
  );
});
