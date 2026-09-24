import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

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

/** An OS notification through Tauri's plugin; nothing outside Tauri (browser, mock transport). */
export async function showNotification(title: string, body: string): Promise<void> {
  if (!isTauri()) return;
  if (!(await isPermissionGranted()) && (await requestPermission()) !== "granted") return;
  sendNotification({ title, body });
}

/** Whether Hive runs on macOS, from the WebView's user agent; asked on every call. */
export const isMac = () => navigator.userAgent.includes("Mac");

/** The platform's command key held without the other one: Cmd on macOS, Ctrl elsewhere. */
export const commandKey = (event: KeyboardEvent) =>
  isMac() ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;

/** A text naming Ctrl shortcuts, with macOS' symbols there: "Ctrl+Shift+O" becomes "⇧⌘O". */
export const keyText = (text: string) =>
  isMac() ? text.replaceAll("Ctrl+Shift+", "⇧⌘").replaceAll("Ctrl+", "⌘") : text;
