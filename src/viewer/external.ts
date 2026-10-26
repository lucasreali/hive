import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ServiceMessage } from "../protocol";
import { setEditorNotice, showNotice, useHive } from "../store";
import { transport } from "../transport";
import { isMac } from "../window";
import { isFor } from "./buffer";

type EditorTarget = Extract<ServiceMessage, { type: "editor_target" }>;

/**
 * The `editor_target` answers asked for and not yet received, by worktree, path and `reveal`:
 * whether the answer shows in the open file's view (else as a toast).
 */
const pending = new Map<string, boolean>();
const key = (worktree: string, path: string, reveal: boolean) =>
  JSON.stringify([worktree, path, reveal]);

/**
 * Asks the service for the Windows path of a worktree's file (an empty `path`: its folder), to
 * open with its default app, or to show in the Finder or the Explorer when `reveal` (D19.1).
 * The file view's button shows the answer in the view; a menu's (`inView` false), as a toast.
 */
export function openInEditor(
  worktree: string,
  path: string,
  reveal = false,
  inView = path !== "" && !reveal,
): void {
  pending.set(key(worktree, path, reveal), inView);
  void transport.openInEditor(worktree, path, reveal);
}

/** Asks the service for the settings file's Windows path. */
export function openSettingsFile(): void {
  pending.set(key("", "", false), false);
  void transport.openSettingsFile();
}

/**
 * An `editor_target` from the service: opened once, and only while the app is waiting for it.
 * Anything else is dropped, so a service can never make the app open a path by itself.
 */
export function openTarget(target: EditorTarget, tauri = isTauri()): Promise<void> | undefined {
  const asked = key(target.worktree, target.path, target.reveal);
  const inView = pending.get(asked);
  if (inView === undefined) return;
  pending.delete(asked);
  return inView ? openExternal(target, tauri) : openFolder(target, tauri);
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
 * A menu's answer: a worktree's folder (an empty `path`) opened in the Explorer or the Finder,
 * a file or folder revealed there (`reveal`), or a file opened with its default app ("Open with
 * Default App"). Anything that goes wrong shows as an error toast.
 */
export async function openFolder(target: EditorTarget, tauri = isTauri()): Promise<void> {
  if (!target.windows_path) return showNotice("error", target.error ?? "No path");
  // No worktree: the settings file ("Open settings file"), opened the same way.
  const manager = isMac() ? "the Finder" : "the Explorer";
  const what =
    target.worktree === ""
      ? "the settings file"
      : target.reveal || target.path === ""
        ? manager
        : "the file's default app";
  if (!tauri) return showNotice("error", `Only the Hive app opens ${what}: ${target.windows_path}`);
  try {
    await openPath(target.windows_path, target.reveal);
  } catch (error) {
    showNotice("error", String(error));
  }
}
