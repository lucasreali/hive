import { getChunks, getOriginalDoc } from "@codemirror/merge";
import type { Extension } from "@codemirror/state";
import { type BlockInfo, BlockType, type EditorView, ViewPlugin } from "@codemirror/view";
import type { LineSide } from "../protocol";
import { type LineAsk, type OpenFile, useHive } from "../store";

// "Who wrote this line?" (27.5) in the editor: a right click on a line number opens Hive's
// small menu (the text keeps its native menu, #53), and the palette asks about the cursor's
// line. Only where a line is on screen is worked out here; the answer is the service's.

/** A line of the view's text or of its diff's old side (the removed lines' widgets). */
export type GutterLine = { line: number; side: LineSide };

/**
 * The line at `height` (from the document's top) in `view`: a text line counts on the worktree
 * side; a line of a removed chunk (a unified diff's widget) on the base side, only when
 * `oldSide` (the diff's original is the base, not the disk's text). Null for anything else.
 */
export function lineAtHeight(
  view: EditorView,
  height: number,
  oldSide: boolean,
): GutterLine | null {
  const block = view.lineBlockAtHeight(height);
  const parts: readonly BlockInfo[] = Array.isArray(block.type) ? block.type : [block];
  const part = parts.find((b) => height >= b.top && height < b.bottom) ?? parts[parts.length - 1];
  if (part.type === BlockType.Text) {
    return { line: view.state.doc.lineAt(part.from).number, side: "worktree" };
  }
  if (!oldSide || part.type !== BlockType.WidgetBefore) return null;
  const chunk = getChunks(view.state)?.chunks.find((c) => c.fromB === part.from && c.fromA < c.toA);
  if (!chunk) return null;
  const original = getOriginalDoc(view.state);
  const first = original.lineAt(chunk.fromA).number;
  const count = original.lineAt(Math.max(chunk.fromA, chunk.endA)).number - first + 1;
  const offset = Math.floor(((height - part.top) / part.height) * count);
  return { line: first + Math.min(Math.max(offset, 0), count - 1), side: "base" };
}

/**
 * The line number menu for file `f`'s view: a right click in the gutter opens it at the
 * pointer, on the line there; anywhere else the native menu stays. On the view's wrapper:
 * CodeMirror's own handlers hear its text only, not the gutter.
 */
export function lineMenu(f: OpenFile, oldSide: boolean): Extension {
  return ViewPlugin.define((view) => {
    const open = (event: MouseEvent) => {
      const target = event.target as Element;
      if (!target.closest(".cm-gutters")) return;
      const at = lineAtHeight(view, event.clientY - view.documentTop, oldSide);
      if (!at) return;
      event.preventDefault();
      const ask: LineAsk = { ...f, ...at, x: event.clientX, y: event.clientY };
      useHive.setState({ lineMenu: ask, provenance: null });
    };
    view.dom.addEventListener("contextmenu", open);
    return { destroy: () => view.dom.removeEventListener("contextmenu", open) };
  });
}

/** The open files' views, for the palette's "Who wrote this line?" (the cursor's line). */
const views = new Map<string, EditorView>();
const keyOf = (f: OpenFile) => `${f.worktree}\n${f.path}`;

/** Keeps `view` as file `f`'s until the returned function lets it go. */
export function trackView(f: OpenFile, view: EditorView): () => void {
  views.set(keyOf(f), view);
  return () => {
    if (views.get(keyOf(f)) === view) views.delete(keyOf(f));
  };
}

/**
 * The cursor's line of file `f`'s view as a question, placed under that line (or at the view's
 * corner when it is not laid out); null when the file has no view.
 */
export function cursorAsk(f: OpenFile): LineAsk | null {
  const view = views.get(keyOf(f));
  if (!view) return null;
  const head = view.state.selection.main.head;
  const line = view.state.doc.lineAt(head).number;
  const box = view.coordsAtPos(head);
  const corner = view.dom.getBoundingClientRect();
  return {
    ...f,
    line,
    side: "worktree",
    x: box?.left ?? corner.left,
    y: box?.bottom ?? corner.top,
  };
}
