import { afterEach, expect, test } from "bun:test";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { TitleBar } from "./TitleBar";
import { closeWindow, guardClose, windowAction } from "./window";

const g = globalThis as { isTauri?: boolean };

afterEach(() => {
  cleanup();
  clearMocks();
  delete g.isTauri;
  delete document.documentElement.dataset.closed;
});

const buttons = ["Minimize", "Maximize", "Close"];
const closed = () => document.documentElement.dataset.closed !== undefined;

test("outside Tauri the window buttons call nothing; Close only marks the page", () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  render(<TitleBar />);
  for (const b of buttons.slice(0, 2)) fireEvent.click(screen.getByTitle(b));
  expect(closed()).toBe(false);
  fireEvent.click(screen.getByTitle("Close"));
  expect(closed()).toBe(true);
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

test("outside Tauri the title bar Close asks the guard, then marks the page closed", () => {
  let keep = true;
  const stop = guardClose(() => keep);
  windowAction("close");
  expect(closed()).toBe(false);
  keep = false;
  windowAction("close");
  expect(closed()).toBe(true);
  delete document.documentElement.dataset.closed;
  keep = true;
  stop();
  // Without a guard nothing keeps the window open.
  windowAction("close");
  expect(closed()).toBe(true);
});

test("every Tauri close request asks the guard; a confirmed close destroys the window", async () => {
  g.isTauri = true;
  mockWindows("main");
  const calls: string[] = [];
  mockIPC(
    (cmd) => {
      calls.push(cmd);
    },
    { shouldMockEvents: true },
  );
  const settle = () => new Promise((r) => setTimeout(r, 0));
  let keep = true;
  const stop = guardClose(() => keep);
  await settle();
  await emit("tauri://close-requested");
  await settle();
  expect(calls).toEqual([]);
  keep = false;
  await emit("tauri://close-requested");
  await settle();
  expect(calls).toEqual(["plugin:window|destroy"]);
  stop();
  await settle();
  await emit("tauri://close-requested");
  await settle();
  expect(calls).toEqual(["plugin:window|destroy"]);
  closeWindow();
  await settle();
  expect(calls).toEqual(["plugin:window|destroy", "plugin:window|destroy"]);
  expect(closed()).toBe(false);
});

test("the title bar drags the window", () => {
  render(<TitleBar />);
  expect(screen.getByText("Hive").parentElement?.hasAttribute("data-tauri-drag-region")).toBe(true);
});
