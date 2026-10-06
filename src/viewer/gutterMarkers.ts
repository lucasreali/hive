import { Chunk } from "@codemirror/merge";
import { type Extension, RangeSet, StateField, type Text } from "@codemirror/state";
import { EditorView, GutterMarker, gutter } from "@codemirror/view";
import { toText } from "./buffer";

// Change markers in the editor's gutter (14.4): the buffer's lines added, modified and deleted
// against the base text the service sent (`HEAD`, or the merge-base). Drawing a diff the
// service sent is presentation (#37); the chunks follow the buffer as it is typed.

export type MarkerKind = "added" | "modified" | "deleted";

/**
 * Each changed line of `doc` (1-based) and how it changed. A deletion marks the line the
 * removed lines sat before (the empty line after a final newline when they ended the file).
 */
export function markerLines(
  chunks: readonly Chunk[],
  doc: Text,
): { line: number; kind: MarkerKind }[] {
  const marks: { line: number; kind: MarkerKind }[] = [];
  for (const { fromA, toA, fromB, toB } of chunks) {
    const first = doc.lineAt(fromB).number;
    if (fromB === toB) {
      marks.push({ line: first, kind: "deleted" });
      continue;
    }
    const kind = fromA === toA ? "added" : "modified";
    // `toB` is one past the chunk's last line break (it may point past the end of the text).
    const last = doc.lineAt(toB - 1).number;
    for (let line = first; line <= last; line++) marks.push({ line, kind });
  }
  return marks;
}

class Marker extends GutterMarker {
  constructor(override readonly elementClass: string) {
    super();
  }
}

const MARKERS = {
  added: new Marker("cm-change-added"),
  modified: new Marker("cm-change-modified"),
  deleted: new Marker("cm-change-deleted"),
};

const draw = (chunks: readonly Chunk[], doc: Text) =>
  RangeSet.of(
    markerLines(chunks, doc).map(({ line, kind }) => MARKERS[kind].range(doc.line(line).from)),
  );

const look = EditorView.theme({
  ".cm-changeGutter .cm-gutterElement": { width: "3px", padding: "0", marginRight: "3px" },
  ".cm-changeGutter .cm-change-added": { background: "var(--git-added)" },
  ".cm-changeGutter .cm-change-modified": { background: "var(--git-modified)" },
  // Between the lines: a small wedge on the top edge of the line the removed ones sat before.
  ".cm-changeGutter .cm-change-deleted": {
    background: "linear-gradient(var(--git-deleted), var(--git-deleted)) top / 100% 6px no-repeat",
  },
});

/**
 * The gutter of change markers against `base`; none without a base (a new or untracked file,
 * no repository, binary). Put it before `lineNumbers()` so it sits left of them.
 */
export function changeMarkers(base: string | null): Extension {
  if (base === null) return [];
  const a = toText(base);
  // A new field per base, so a new base rebuilds the chunks when reconfigured.
  const chunks = StateField.define<readonly Chunk[]>({
    create: (state) => Chunk.build(a, state.doc),
    update: (value, tr) => (tr.docChanged ? Chunk.updateB(value, a, tr.newDoc, tr.changes) : value),
  });
  return [
    chunks,
    look,
    gutter({
      class: "cm-changeGutter",
      markers: (view) => draw(view.state.field(chunks), view.state.doc),
    }),
  ];
}
