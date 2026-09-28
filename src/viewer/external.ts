import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ServiceMessage } from "../protocol";
import { setEditorNotice, setNotice, useHive } from "../store";
import { transport } from "../transport";
import { isMac } from "../window";
import { isFor } from "./buffer";

type EditorTarget = Extract<ServiceMessage, { type: "editor_target" }>;

/** The `editor_target` answers asked for and not yet received, by worktree and path. */
const pending = new Set<string>();
const key = (worktree: string, path: string) => JSON.stringify([worktree, path]);

/** Asks the service for the Windows path of a worktree's file (an empty `path`: its folder). */
export function openInEditor(worktree: string, path: string): void {
  pending.add(key(worktree, path));
  void transport.openInEditor(worktree, path);
}

/** Asks the service for the settings file's Windows path. */
export function openSettingsFile(): void {
  pending.add(key("", ""));
  void transport.openSettingsFile();
}

/**
 * An `editor_target` from the service: opened once, and only while the app is waiting for it.
 * Anything else is dropped, so a service can never make the app open a path by itself.
 */
export function openTarget(target: EditorTarget, tauri = isTauri()): Promise<void> | undefined {
  if (!pending.delete(key(target.worktree, target.path))) return;
  return target.path === "" ? openFolder(target, tauri) : openExternal(target, tauri);
}

/**
 * Opens `path` with the system's default app, or shows it in the Explorer or the Finder when
 * `reveal`. The app side opens only a path the service just sent, once (9.10): the webview has
 * no permission to open a path itself.
 */
export const openPath = (path: string, reveal = false) =>
  invoke<void>("open_path", { path, reveal });

/**
 * "Open in external editor": the service's answer opens the file's Windows path with its
 * Windows default app (the opener plugin). Outside Tauri (browser, mock transport) nothing
 * opens and the file view says so. An answer for a file no longer open is dropped.
 */
export async function openExternal(target: EditorTarget, tauri = isTauri()): Promise<void> {
  const open = useHive.getState().openFile;
  if (!open || !isFor(open, target)) return;
  if (!target.windows_path) return setEditorNotice(target.error);
  if (!tauri) {
    return setEditorNotice(`Only the Hive app opens an external editor: ${target.windows_path}`);
  }
  try {
    await openPath(target.windows_path);
  } catch (error) {
    setEditorNotice(String(error));
  }
}

/**
 * "Open in Explorer" (a worktree's menu): the service's answer for the folder (an empty
 * `path`) opens its Windows path with Windows' default app for folders, the Explorer.
 * Anything that goes wrong shows in the status bar.
 */
export async function openFolder(target: EditorTarget, tauri = isTauri()): Promise<void> {
  if (!target.windows_path) return setNotice(target.error);
  // No worktree: the settings file ("Open settings file"), opened the same way.
  const what =
    target.worktree === "" ? "the settings file" : isMac() ? "the Finder" : "the Explorer";
  if (!tauri) return setNotice(`Only the Hive app opens ${what}: ${target.windows_path}`);
  try {
    await openPath(target.windows_path);
  } catch (error) {
    setNotice(String(error));
  }
}
