import { isTauri } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { type ServiceMessage, setEditorNotice, useHive } from "../store";
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
