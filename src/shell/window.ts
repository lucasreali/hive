import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type WindowAction = "minimize" | "toggleMaximize" | "close";

/** Custom title bar buttons. Outside Tauri (browser, mock transport) they do nothing. */
export function windowAction(action: WindowAction): void {
  if (isTauri()) void getCurrentWindow()[action]();
}
