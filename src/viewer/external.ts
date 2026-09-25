import { isTauri } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { type ServiceMessage, setEditorNotice, setNotice, useHive } from "../store";
import { isMac } from "../window";
import { isFor } from "./buffer";

type EditorTarget = Extract<ServiceMessage, { type: "editor_target" }>;

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
