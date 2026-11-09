import { setEdit, setSaveAfterAsks, useHive } from "../store";
import { transport } from "../transport";
import { failed, isFor, startSave } from "./buffer";

/**
 * Ctrl+S and the Save button: sends the buffer's text with the version it was based on. While
 * the service works on a table's edits (25.4) it waits for them, so the save holds them.
 */
export function saveOpenFile(): void {
  const { edit, tableAsks } = useHive.getState();
  if (tableAsks.length > 0) {
    setSaveAfterAsks(true);
    return;
  }
  const next = edit && startSave(edit);
  if (!next) return;
  setEdit(next);
  transport
    .saveFile(next.worktree, next.path, next.doc.toString(), next.version)
    .catch((error: unknown) => {
      // Never sent (e.g. not connected), so no answer will come.
      const { edit } = useHive.getState();
      if (edit && isFor(edit, next)) setEdit(failed(edit, "io", String(error)));
    });
}
