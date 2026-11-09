import { Change, Chunk, diff } from "@codemirror/merge";
import { type Extension, RangeSet, StateField, type Text } from "@codemirror/state";
import { EditorView, GutterMarker, gutter } from "@codemirror/view";
import { toText } from "./buffer";

// Change markers in the editor's gutter (14.4): the buffer's lines added, modified and deleted
// against the base text the service sent (`HEAD`, or the merge-base). Drawing a diff the
// service sent is presentation (#37); the chunks follow the buffer as it is typed.

export type MarkerKind = "added" | "modified" | "deleted";

/** The ids of distinct lines: UTF-16 code units below the surrogates, which `diff` keeps paired. */
const LINE_IDS = 0xd800;
/** Changed lines the line diff compares exactly; past that it is coarser, never slower. */
const LINE_SCAN_LIMIT = 2000;
/** The limit `@codemirror/merge`'s own views diff characters with. */
const CHAR_SCAN_LIMIT = 500;

/**
 * The differences of two texts, whole lines at a time (25.7): each distinct line becomes one
 * character, so the cost follows the changed lines. The markers only need lines, and a diff of
 * characters is quadratic in the changed characters: 5 s on the main thread, which stalled the
 * whole window, for 3 KB added to a 240 KB `TODO.md` of 5 KB lines. A text with more distinct
 * lines than there are ids falls back to the characters, limited.
 */
export function lineDiff(a: string, b: string): readonly Change[] {
  const ids = new Map<string, string>();
  const encode = (text: string) => {
    const starts = [0];
    const codes: string[] = [];
    for (let at = 0; at < text.length; at = starts[starts.length - 1] as number) {
      const end = text.indexOf("\n", at);
      const to = end < 0 ? text.length : end + 1;
      const line = text.slice(at, to);
      let id = ids.get(line);
      if (id === undefined) {
        id = String.fromCharCode(ids.size % LINE_IDS);
        ids.set(line, id);
      }
      codes.push(id);
      starts.push(to);
    }
    return { starts, codes: codes.join("") };
  };
  const [oldLines, newLines] = [encode(a), encode(b)];
  if (ids.size > LINE_IDS) return diff(a, b, { scanLimit: CHAR_SCAN_LIMIT });
  return diff(oldLines.codes, newLines.codes, { scanLimit: LINE_SCAN_LIMIT }).map(
    (c) =>
      new Change(
        oldLines.starts[c.fromA] as number,
        oldLines.starts[c.toA] as number,
        newLines.starts[c.fromB] as number,
        newLines.starts[c.toB] as number,
      ),
  );
}

/** How the markers' chunks are diffed: by lines, for `Chunk.build` and `Chunk.updateB`. */
export const MARKER_DIFF = { override: lineDiff };

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
    create: (state) => Chunk.build(a, state.doc, MARKER_DIFF),
    update: (value, tr) =>
      tr.docChanged ? Chunk.updateB(value, a, tr.newDoc, tr.changes, MARKER_DIFF) : value,
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
