import { type OpenFile, setSaveAfterAsks, useHive } from "../store";
import { asksPending, fileTabState, patchEdits } from "../tabs";
import { transport } from "../transport";
import { failed, startSave } from "./buffer";

/** Sends file `f`'s buffer, open or kept in its tab, with the version it was based on. */
export function saveBuffer(f: OpenFile): void {
  const { edit } = fileTabState(useHive.getState(), f);
  const next = edit && startSave(edit);
  if (!next) return;
  useHive.setState((s) => patchEdits(s, f, () => next));
  transport
    .saveFile(next.worktree, next.path, next.doc.toString(), next.version)
    .catch((error: unknown) => {
      // Never sent (e.g. not connected), so no answer will come.
      useHive.setState((s) => patchEdits(s, f, (b) => failed(b, "io", String(error))));
    });
}

/**
 * Ctrl+S and the Save button, for the open file. While the service works on its table's edits
 * (25.4) it waits for them, so the save holds them; another file's edits are no reason to.
 */
export function saveOpenFile(): void {
  const s = useHive.getState();
  if (!s.edit) return;
  const file = { worktree: s.edit.worktree, path: s.edit.path };
  if (asksPending(s, file)) setSaveAfterAsks(file);
  else saveBuffer(file);
}
