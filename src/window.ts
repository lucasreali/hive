import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import type { Settings } from "./protocol";

export type WindowAction = "minimize" | "toggleMaximize" | "close";

const never = () => false;
/** Asked on every close request; true keeps the window open. */
let keepOpen: () => boolean = never;

/**
 * Title bar Close, Alt+F4 and the taskbar all become one Tauri close request, which asks
 * `guard` first; true keeps the window open. Outside Tauri (browser, mock transport) only the
 * title bar Close requests it. Returns the cleanup.
 */
export function guardClose(guard: () => boolean): () => void {
  keepOpen = guard;
  const unlisten = isTauri()
    ? getCurrentWindow().onCloseRequested((e) => {
        if (guard()) e.preventDefault();
      })
    : null;
  return () => {
    keepOpen = never;
    void unlisten?.then((stop) => stop());
  };
}

/** Closes without asking. Outside Tauri the page is only marked closed, for browser checks. */
export function closeWindow(): void {
  if (isTauri()) void getCurrentWindow().destroy();
  else document.documentElement.dataset.closed = "";
}

/** Custom title bar buttons. Outside Tauri only Close does something: its close request. */
export function windowAction(action: WindowAction): void {
  if (isTauri()) void getCurrentWindow()[action]();
  else if (action === "close" && !keepOpen()) closeWindow();
}

/**
 * Calls `onChange` with whether the window has the focus, now and on every change: Tauri's
 * focus events in the app, the page's focus and blur outside it. Returns the cleanup.
 */
export function watchFocus(onChange: (focused: boolean) => void): () => void {
  onChange(document.hasFocus());
  if (isTauri()) {
    const unlisten = getCurrentWindow().onFocusChanged(({ payload }) => onChange(payload));
    return () => void unlisten.then((stop) => stop());
  }
  const focus = () => onChange(true);
  const blur = () => onChange(false);
  window.addEventListener("focus", focus);
  window.addEventListener("blur", blur);
  return () => {
    window.removeEventListener("focus", focus);
    window.removeEventListener("blur", blur);
  };
}

/**
 * Calls `onChange` with whether the window is visible (the page's `visibilityState`: not
 * hidden nor minimised), now and on every change. Returns the cleanup.
 */
export function watchVisible(onChange: (visible: boolean) => void): () => void {
  const change = () => onChange(document.visibilityState !== "hidden");
  change();
  document.addEventListener("visibilitychange", change);
  return () => document.removeEventListener("visibilitychange", change);
}

/**
 * The number on the Dock icon (17.16), removed at zero. Only in the macOS app: elsewhere there
 * is no Dock (Windows' taskbar has no badge count). A failure is dropped: the badge is a hint.
 */
export function setDockBadge(count: number): void {
  if (!isTauri() || !isMac()) return;
  getCurrentWindow()
    .setBadgeCount(count > 0 ? count : undefined)
    .catch(() => {});
}

/** One Dock bounce (`Informational`: once, not until Hive is focused); only in the macOS app. */
export function bounceDock(): void {
  if (!isTauri() || !isMac()) return;
  getCurrentWindow()
    .requestUserAttention(UserAttentionType.Informational)
    .catch(() => {});
}

/** Whether Hive runs on macOS, from the WebView's user agent; asked on every call. */
export const isMac = () => navigator.userAgent.includes("Mac");

/** The platform's command key held without the other one: Cmd on macOS, Ctrl elsewhere. */
export const commandKey = (event: KeyboardEvent) =>
  isMac() ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;

/**
 * A text naming keys, with macOS' symbols and names there (18.25): "Ctrl+Shift+O" becomes "⇧⌘O",
 * "Control+Shift+Tab" "⌃⇧Tab", "Shift+Enter" "⇧↩", "Enter" "↩" and "Esc" "esc".
 */
export const keyText = (text: string) =>
  isMac()
    ? text
        .replaceAll("Control+Shift+", "⌃⇧")
        .replaceAll("Control+", "⌃")
        .replaceAll("Ctrl+Shift+", "⇧⌘")
        .replaceAll("Ctrl+", "⌘")
        .replace(/\bShift\+Enter\b/g, "⇧↩")
        .replace(/\bEnter\b/g, "↩")
        .replace(/\bEsc\b/g, "esc")
    : text;

/** An example path in the user's home, as macOS writes it there: "/home/you" is "/Users/you". */
export const homeText = (path: string) => (isMac() ? path.replace(/^\/home\//, "/Users/") : path);

/**
 * Whether a key belongs to a composition (an IME, or a dead key: ´ ~ ^ on a pt-BR layout,
 * 17.14): Enter then ends the composition, never a field's own action. WebKit sends Enter's
 * keydown after `compositionend`, marked only by keyCode 229.
 */
export const composing = (event: KeyboardEvent) => event.isComposing || event.keyCode === 229;

/**
 * Calls `onChange` with whether the window is in full screen, now and on every change: Tauri's
 * resize events in the app (entering and leaving full screen resize the window), the page's
 * full screen outside it. Returns the cleanup.
 */
export function watchFullscreen(onChange: (fullscreen: boolean) => void): () => void {
  if (isTauri()) {
    const window = getCurrentWindow();
    const check = () => void window.isFullscreen().then(onChange);
    check();
    const unlisten = window.onResized(check);
    return () => void unlisten.then((stop) => stop());
  }
  const check = () => onChange(Boolean(document.fullscreenElement));
  check();
  document.addEventListener("fullscreenchange", check);
  return () => document.removeEventListener("fullscreenchange", check);
}

type Theme = Settings["appearance"]["theme"];

/** Each theme's `--bg` (src/styles.css): the window's background behind the page. */
export const THEME_BACKGROUND: Record<Theme, string> = {
  "one-dark": "#282c33",
  "one-light": "#fafafa",
};

/**
 * The window follows Hive's theme: native parts (the window's controls, menus, form controls)
 * in the theme's light or dark appearance, and the theme's background behind the page, so a
 * resize or full screen shows no other colour. The config starts on One Dark's, the default
 * theme, so the launch shows no white. Outside Tauri nothing.
 */
export function nativeTheme(theme: Theme): void {
  if (!isTauri()) return;
  void getCurrentWindow().setTheme(theme === "one-light" ? "light" : "dark");
  void getCurrentWebviewWindow().setBackgroundColor(THEME_BACKGROUND[theme]);
}

/**
 * A double click on the empty title bar on macOS does what the system's "Double-click a
 * window's title bar to" setting says, which the app reads (`title_bar_action`): zoom,
 * minimize or nothing. Outside Tauri nothing.
 */
export async function titleBarDoubleClick(): Promise<void> {
  if (!isTauri()) return;
  const action = await invoke<string>("title_bar_action");
  if (action === "zoom") await getCurrentWindow().toggleMaximize();
  else if (action === "minimize") await getCurrentWindow().minimize();
}
