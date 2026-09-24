import { afterEach, expect, test } from "bun:test";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { asMac } from "../test/mac";
import { TitleBar } from "./shell/TitleBar";
import {
  closeWindow,
  guardClose,
  isMac,
  keyText,
  showNotification,
  watchFocus,
  windowAction,
} from "./window";

const g = globalThis as { isTauri?: boolean };

afterEach(() => {
  cleanup();
  clearMocks();
  delete g.isTauri;
  delete document.documentElement.dataset.closed;
});

const buttons = ["Minimize", "Maximize", "Close"];

test("shortcut texts use Ctrl, or macOS' symbols there", () => {
  const send = "Send (Ctrl+Shift+L), save (Ctrl+S)";
  expect(isMac()).toBe(false);
  expect(keyText(send)).toBe(send);
  asMac();
  expect(isMac()).toBe(true);
  expect(keyText(send)).toBe("Send (⇧⌘L), save (⌘S)");
});
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

test("OS notifications go through Tauri's plugin, asking permission while undecided", async () => {
  const w = window as unknown as Record<string, unknown>;
  const shown: string[] = [];
  let permission = "default";
  let answer = "denied";
  w.Notification = class {
    static get permission() {
      return permission;
    }
    static requestPermission = () => Promise.resolve(answer);
    constructor(title: string, options: { body: string }) {
      shown.push(`${title}: ${options.body}`);
    }
  };
  let granted = false;
  mockIPC((cmd) => (cmd === "plugin:notification|is_permission_granted" ? granted : undefined));
  // Outside Tauri: nothing.
  await showNotification("t", "outside");
  g.isTauri = true;
  await showNotification("t", "refused");
  answer = "granted";
  await showNotification("t", "asked");
  granted = true;
  await showNotification("t", "already");
  permission = "denied";
  answer = "denied";
  await showNotification("t", "blocked");
  expect(shown).toEqual(["t: asked", "t: already"]);
  delete w.Notification;
});

test("the title bar drags the window", () => {
  render(<TitleBar />);
  expect(screen.getByText("Hive").parentElement?.hasAttribute("data-tauri-drag-region")).toBe(true);
});

test("window focus: the page's focus and blur outside Tauri", () => {
  const seen: boolean[] = [];
  const stop = watchFocus((f) => seen.push(f));
  window.dispatchEvent(new Event("blur"));
  window.dispatchEvent(new Event("focus"));
  stop();
  window.dispatchEvent(new Event("blur"));
  expect(seen).toEqual([document.hasFocus(), false, true]);
});

test("window focus: Tauri's focus events in the app", async () => {
  g.isTauri = true;
  mockWindows("main");
  mockIPC(() => {}, { shouldMockEvents: true });
  const settle = () => new Promise((r) => setTimeout(r, 0));
  const seen: boolean[] = [];
  const stop = watchFocus((f) => seen.push(f));
  await settle();
  await emit("tauri://blur");
  await emit("tauri://focus");
  stop();
  await settle();
  await emit("tauri://blur");
  expect(seen).toEqual([document.hasFocus(), false, true]);
});
