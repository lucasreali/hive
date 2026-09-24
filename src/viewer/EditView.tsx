import { useEffect, useRef, useState } from "react";
import { setEdit, setSelectedLines, useHive } from "../store";
import { transport } from "../transport";
import { type EditBuffer, failed, isFor, resolve, startSave } from "./buffer";
import { createEditor, type Editor } from "./editor";

/** Ctrl+S and the Save button: sends the buffer's text with the version it was based on. */
export function saveOpenFile(): void {
  const { edit } = useHive.getState();
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

/** The user's edits reach the buffer; ignored once it is gone (the file closed). */
function changed(doc: EditBuffer["doc"]): void {
  const { edit } = useHive.getState();
  if (edit) setEdit({ ...edit, doc });
}

/**
 * The open file as editable text (3.5, #31), with the conflict banner when the file changed
 * on disk while the buffer had unsaved edits: Reload (take the disk's text), Keep mine (the
 * next save overwrites it) or View diff (the edits against the disk, read-only). The
 * CodeMirror view is created once per file, outside React state (#30); a reload only swaps
 * its text. Key it by the file.
 */
export function EditView({ edit }: { edit: EditBuffer }) {
  const parent = useRef<HTMLDivElement>(null);
  const editor = useRef<Editor | null>(null);
  const [comparing, setComparing] = useState(false);
  const { path, doc, conflict } = edit;
  useEffect(() => {
    const { edit } = useHive.getState();
    const created = createEditor(parent.current as HTMLDivElement, path, (edit as EditBuffer).doc, {
      change: changed,
      save: saveOpenFile,
      select: setSelectedLines,
    });
    editor.current = created;
    return () => {
      created.destroy();
      setSelectedLines(null);
    };
  }, [path]);
  useEffect(() => editor.current?.load(doc), [doc]);
  const original = comparing && conflict ? (conflict.content ?? "") : null;
  useEffect(() => editor.current?.compare(original), [original]);
  const choose = (keepMine: boolean) => {
    setComparing(false);
    setEdit(resolve(edit, keepMine));
  };
  return (
    <>
      {conflict && (
        <div className="conflict-banner" role="alert">
          <span className="conflict-text">
            {conflict.content === null ? "Deleted on disk." : "Changed on disk."}
          </span>
          <button type="button" className="conflict-choice" onClick={() => choose(false)}>
            Reload
          </button>
          <button
            type="button"
            className="conflict-choice"
            title="The next save overwrites the file"
            onClick={() => choose(true)}
          >
            Keep mine
          </button>
          <button
            type="button"
            className="conflict-choice"
            aria-pressed={comparing}
            onClick={() => setComparing(!comparing)}
          >
            View diff
          </button>
        </div>
      )}
      {edit.error && <div className="files-error">{edit.error}</div>}
      <div className="code-view" ref={parent} />
    </>
  );
}
