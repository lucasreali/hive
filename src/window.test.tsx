import { afterEach, expect, test } from "bun:test";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { asMac } from "../test/mac";
import { TitleBar } from "./shell/TitleBar";
import {
  bounceDock,
  closeWindow,
  guardClose,
  isMac,
  keyText,
  nativeTheme,
  setDockBadge,
  THEME_BACKGROUND,
  titleBarDoubleClick,
  watchFocus,
  watchFullscreen,
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
const settle = () => new Promise((r) => setTimeout(r, 0));
/** A command the title bar's actions send, not its full screen watch (`watchFullscreen`). */
const watching = (cmd: string) =>
  !cmd.startsWith("plugin:event|") && cmd !== "plugin:window|is_fullscreen";

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
  mockIPC(
    (cmd) => {
      if (watching(cmd)) calls.push(cmd);
    },
    { shouldMockEvents: true },
  );
  await act(async () => {
    render(<TitleBar />);
  });
  for (const b of buttons) fireEvent.click(screen.getByTitle(b));
  await Promise.resolve();
  expect(calls).toEqual([
    "plugin:window|minimize",
    "plugin:window|toggle_maximize",
    "plugin:window|close",
  ]);
  cleanup();
  await settle();
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

test("the whole empty title bar drags the window, its buttons do not", () => {
  render(<TitleBar />);
  const brand = screen.getByText("Hive").parentElement;
  // Anything inside the brand (the icon, the name) drags; the bar itself drags where it is empty.
  expect(brand?.getAttribute("data-tauri-drag-region")).toBe("deep");
  expect(brand?.parentElement?.getAttribute("data-tauri-drag-region")).toBe("true");
  expect(screen.getByTitle("Close").hasAttribute("data-tauri-drag-region")).toBe(false);
});

const bar = () => document.querySelector("header.titlebar") as HTMLElement;

/** The commands the title bar's double clicks run, with `title_bar_action` answering `action`. */
async function doubleClicks(action: string, run: (header: HTMLElement) => void) {
  g.isTauri = true;
  mockWindows("main");
  const calls: string[] = [];
  mockIPC(
    (cmd) => {
      if (watching(cmd)) calls.push(cmd);
      if (cmd === "title_bar_action") return action;
    },
    { shouldMockEvents: true },
  );
  await act(async () => {
    render(<TitleBar />);
  });
  await act(async () => {
    run(bar());
    for (let i = 0; i < 3; i++) await settle();
  });
  // Unmounted while the mocks are still there: its full screen watch stops listening.
  cleanup();
  await settle();
  return calls;
}

test("macOS: a double click on the empty title bar follows the system's setting", async () => {
  asMac();
  const twice = (el: Element) => fireEvent.mouseUp(el, { detail: 2 });
  expect(await doubleClicks("zoom", twice)).toEqual([
    "title_bar_action",
    "plugin:window|toggle_maximize",
  ]);
  expect(await doubleClicks("minimize", twice)).toEqual([
    "title_bar_action",
    "plugin:window|minimize",
  ]);
  expect(await doubleClicks("none", twice)).toEqual(["title_bar_action"]);
  // On the brand too; not on a button, not a single click, not another mouse button.
  const brand = () => twice(screen.getByText("Hive"));
  expect(await doubleClicks("none", brand)).toEqual(["title_bar_action"]);
  const bell = () => twice(screen.getByTitle(/Notifications/));
  expect(await doubleClicks("zoom", bell)).toEqual([]);
  const once = (header: HTMLElement) => fireEvent.mouseUp(header, { detail: 1 });
  expect(await doubleClicks("zoom", once)).toEqual([]);
  const right = (header: HTMLElement) => fireEvent.mouseUp(header, { detail: 2, button: 2 });
  expect(await doubleClicks("zoom", right)).toEqual([]);
});

test("macOS: Tauri's own double click (a zoom on the document's mouse-up) never runs", () => {
  const reached: number[] = [];
  const listen = (e: MouseEvent) => reached.push(e.detail);
  document.addEventListener("mouseup", listen);
  render(<TitleBar />);
  fireEvent.mouseUp(bar(), { detail: 2 });
  cleanup();
  asMac();
  render(<TitleBar />);
  fireEvent.mouseUp(bar(), { detail: 1 });
  fireEvent.mouseUp(bar(), { detail: 2 });
  document.removeEventListener("mouseup", listen);
  // Elsewhere Tauri's drag region keeps its double click (Windows).
  expect(reached).toEqual([2, 1]);
});

test("outside Tauri a double click on the title bar does nothing", async () => {
  asMac();
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  await titleBarDoubleClick();
  expect(calls).toEqual([]);
});

test("macOS full screen drops the traffic lights' room: the page's full screen outside Tauri", () => {
  asMac();
  render(<TitleBar />);
  expect(bar().hasAttribute("data-fullscreen")).toBe(false);
  Object.defineProperty(document, "fullscreenElement", { value: bar(), configurable: true });
  act(() => {
    document.dispatchEvent(new Event("fullscreenchange"));
  });
  expect(bar().hasAttribute("data-fullscreen")).toBe(true);
  Object.defineProperty(document, "fullscreenElement", { value: null, configurable: true });
  act(() => {
    document.dispatchEvent(new Event("fullscreenchange"));
  });
  expect(bar().hasAttribute("data-fullscreen")).toBe(false);
  cleanup();
  // Gone with the title bar.
  const seen: boolean[] = [];
  const stop = watchFullscreen((f) => seen.push(f));
  stop();
  document.dispatchEvent(new Event("fullscreenchange"));
  expect(seen).toEqual([false]);
});

test("full screen in the app: asked again on every resize", async () => {
  g.isTauri = true;
  mockWindows("main");
  let fullscreen = false;
  mockIPC(
    (cmd) => {
      if (cmd === "plugin:window|is_fullscreen") return fullscreen;
    },
    { shouldMockEvents: true },
  );
  const seen: boolean[] = [];
  const stop = watchFullscreen((f) => seen.push(f));
  await settle();
  fullscreen = true;
  await emit("tauri://resize", { width: 1, height: 1 });
  await settle();
  fullscreen = false;
  await emit("tauri://resize", { width: 2, height: 2 });
  await settle();
  stop();
  await settle();
  await emit("tauri://resize", { width: 3, height: 3 });
  await settle();
  expect(seen).toEqual([false, true, false]);
});

test("the full screen rule drops the left padding (styles.css)", async () => {
  const css = await Bun.file(new URL("./styles.css", import.meta.url)).text();
  expect(css).toMatch(/\.titlebar\[data-mac\]\[data-fullscreen\] \{\s*padding-left: 0;/);
});

test("the window takes Hive's theme: appearance and background", async () => {
  g.isTauri = true;
  mockWindows("main");
  const calls: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
  });
  nativeTheme("one-light");
  nativeTheme("one-dark");
  for (let i = 0; i < 3; i++) await settle();
  const color = (hex: string) => [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
  ];
  const sent = calls.map(([cmd, args]) => {
    const a = args as { value?: unknown; color?: unknown };
    return [cmd, a.value ?? a.color];
  });
  expect(sent).toEqual(
    expect.arrayContaining([
      ["plugin:window|set_theme", "light"],
      ["plugin:window|set_theme", "dark"],
      ["plugin:window|set_background_color", "#fafafa"],
      ["plugin:webview|set_webview_background_color", "#fafafa"],
      ["plugin:window|set_background_color", "#282c33"],
      ["plugin:webview|set_webview_background_color", "#282c33"],
    ]),
  );
  expect(sent).toHaveLength(6);
  expect(color(THEME_BACKGROUND["one-light"])).toEqual([250, 250, 250]);
});

test("outside Tauri the theme stays in the page", () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  nativeTheme("one-light");
  expect(calls).toEqual([]);
});

test("each theme's window background is its --bg, and the window starts on One Dark's", async () => {
  const css = await Bun.file(new URL("./styles.css", import.meta.url)).text();
  const dark = css.slice(css.indexOf(":root {"));
  const light = css.slice(css.indexOf(':root[data-theme="one-light"]'));
  const bg = (block: string) => block.match(/--bg: (#[0-9a-f]{6});/)?.[1];
  expect(bg(dark)).toBe(THEME_BACKGROUND["one-dark"]);
  expect(bg(light)).toBe(THEME_BACKGROUND["one-light"]);
  for (const config of ["tauri.conf.json", "tauri.macos.conf.json"]) {
    const file = Bun.file(new URL(`../src-tauri/${config}`, import.meta.url));
    const window = (await file.json()).app.windows[0];
    expect(window.backgroundColor).toBe(THEME_BACKGROUND["one-dark"]);
  }
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

test("the Dock badge and bounce reach only the macOS app's window, failures dropped", async () => {
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, (args as { value?: unknown }).value]);
    if (cmd === "plugin:window|set_badge_count" && (args as { value?: number }).value === 9)
      throw new Error("no Dock");
  });
  // Outside Tauri, and in the Windows app, there is no Dock.
  setDockBadge(2);
  bounceDock();
  g.isTauri = true;
  mockWindows("main");
  setDockBadge(2);
  bounceDock();
  expect(calls).toEqual([]);
  asMac();
  setDockBadge(2);
  setDockBadge(0);
  setDockBadge(9);
  bounceDock();
  await Promise.resolve();
  expect(calls).toEqual([
    ["plugin:window|set_badge_count", 2],
    ["plugin:window|set_badge_count", undefined],
    ["plugin:window|set_badge_count", 9],
    ["plugin:window|request_user_attention", { type: "Informational" }],
  ]);
});
