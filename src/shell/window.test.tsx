import { afterEach, expect, test } from "bun:test";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { TitleBar } from "./TitleBar";

const g = globalThis as { isTauri?: boolean };

afterEach(() => {
  cleanup();
  clearMocks();
  delete g.isTauri;
});

const buttons = ["Minimize", "Maximize", "Close"];

test("window buttons do nothing outside Tauri", () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  render(<TitleBar />);
  for (const b of buttons) fireEvent.click(screen.getByTitle(b));
  expect(calls).toEqual([]);
});

test("window buttons drive the Tauri window", async () => {
  g.isTauri = true;
  mockWindows("main");
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  render(<TitleBar />);
  for (const b of buttons) fireEvent.click(screen.getByTitle(b));
  await Promise.resolve();
  expect(calls).toEqual([
    "plugin:window|minimize",
    "plugin:window|toggle_maximize",
    "plugin:window|close",
  ]);
});

test("the title bar drags the window", () => {
  render(<TitleBar />);
  expect(screen.getByText("Hive").parentElement?.hasAttribute("data-tauri-drag-region")).toBe(true);
});
