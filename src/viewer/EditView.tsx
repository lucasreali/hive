import { useEffect, useRef, useState } from "react";
import {
  clearGotoLine,
  leaveView,
  saveFileView,
  setEdit,
  setSelectedLines,
  takePlace,
  useHive,
} from "../store";
import { type EditBuffer, isFor, resolve } from "./buffer";
import { createEditor, type Editor, restoreView, revealLine, snapshot } from "./editor";
import { saveOpenFile } from "./save";

export { saveOpenFile };

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
    const { edit, openFiles } = useHive.getState();
    const file = edit as EditBuffer;
    const created = createEditor(parent.current as HTMLDivElement, path, file.doc, {
      change: changed,
      save: saveOpenFile,
      select: setSelectedLines,
    });
    editor.current = created;
    // Shown again in its tab (8.21): as it was left.
    restoreView(created.view, openFiles.find((f) => isFor(f, file))?.view, takePlace(file));
    return () => {
      const left = snapshot(created.view);
      saveFileView(file, left);
      // Left for the rendered Markdown (21.3): it starts at the same line.
      leaveView(file, true, left.top);
      created.destroy();
      setSelectedLines(null);
    };
  }, [path]);
  useEffect(() => editor.current?.load(doc), [doc]);
  // A search result asked for a line: shown once the buffer is there.
  const goto = useHive((s) => s.gotoLine);
  useEffect(() => {
    const view = editor.current?.view;
    if (!view || !goto || goto.path !== path || goto.worktree !== edit.worktree) return;
    revealLine(view, goto.line);
    clearGotoLine();
  }, [goto, path, edit.worktree]);
  // The base text the service sent with the file (`HEAD` or the merge-base), for the markers.
  const base = useHive((s) =>
    s.file?.worktree === edit.worktree && s.file.path === path ? s.file.base : null,
  );
  useEffect(() => editor.current?.setBase(base), [base]);
  const original = comparing && conflict ? (conflict.content ?? "") : null;
  useEffect(() => editor.current?.compare(original), [original]);
  return (
    <>
      <BufferNotes edit={edit} comparing={comparing} onCompare={setComparing} />
      <div className="code-view" ref={parent} />
    </>
  );
}

/**
 * Over a view of the buffer: the conflict banner when the file changed on disk under unsaved
 * edits (Reload, Keep mine and, where the text shows, View diff), and why a save failed.
 */
export function BufferNotes({
  edit,
  comparing = false,
  onCompare,
}: {
  edit: EditBuffer;
  comparing?: boolean;
  onCompare?: (comparing: boolean) => void;
}) {
  const { conflict } = edit;
  const choose = (keepMine: boolean) => {
    onCompare?.(false);
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
          {onCompare && (
            <button
              type="button"
              className="conflict-choice"
              aria-pressed={comparing}
              onClick={() => onCompare(!comparing)}
            >
              View diff
            </button>
          )}
        </div>
      )}
      {edit.error && <div className="files-error">{edit.error}</div>}
    </>
  );
}
