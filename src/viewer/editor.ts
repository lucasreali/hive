import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { unifiedMergeView } from "@codemirror/merge";
import {
  Compartment,
  EditorSelection,
  EditorState,
  type Extension,
  Prec,
  type Text,
} from "@codemirror/state";
import { oneDark } from "@codemirror/theme-one-dark";
import { Decoration, drawSelection, EditorView, keymap, lineNumbers } from "@codemirror/view";
import type { Lines } from "../store";

/** What the viewer shows: `content`, and as a unified diff against `original` when given. */
export type Doc = { content: string; original: string | null };

/** The prototype's look (screen 1g) over One Dark (#23): its surfaces and diff colors. */
const theme = Prec.highest(
  EditorView.theme(
    {
      "&": { height: "100%", fontSize: "12px", backgroundColor: "var(--bg)" },
      "&.cm-focused": { outline: "none" },
      ".cm-scroller": { fontFamily: "var(--font-code)", lineHeight: "18px" },
      ".cm-content": { padding: "4px 0" },
      ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--text-4)", border: "none" },
      ".cm-lineNumbers .cm-gutterElement": { minWidth: "32px", padding: "0 6px 0 4px" },
      "&.cm-merge-b .cm-changedLine": { backgroundColor: "rgba(161, 193, 129, 0.12)" },
      "&.cm-merge-b .cm-changedText": { background: "rgba(161, 193, 129, 0.25)" },
      ".cm-deletedChunk": { backgroundColor: "rgba(208, 114, 119, 0.12)" },
      ".cm-deletedChunk .cm-deletedText": { background: "rgba(208, 114, 119, 0.25)" },
      "&.cm-merge-b .cm-changedLineGutter": { background: "var(--state-idle)" },
      ".cm-deletedLineGutter": { background: "var(--state-error)" },
      // Lines with a review comment (6.7): a bar on the left, over any diff color.
      ".cm-commented": { boxShadow: "inset 3px 0 0 var(--accent)" },
      ".cm-collapsedLines": {
        color: "var(--accent)",
        background: "rgba(116, 173, 232, 0.08)",
        fontFamily: "var(--font-sans)",
        fontSize: "var(--fs-meta)",
        // The default "⦚" marks are not in IBM Plex.
        "&:before, &:after": { display: "none" },
      },
    },
    { dark: true },
  ),
);

/**
 * The lines the main selection covers, or null when it is empty. In a diff the document is the
 * new file (removed lines are widgets, not selectable text), so these are new-file lines. A
 * selection ending at the start of a line does not take that line.
 */
export function selectedLines(state: EditorState): Lines | null {
  const { from, to } = state.selection.main;
  if (from === to) return null;
  const first = state.doc.lineAt(from).number;
  const end = state.doc.lineAt(to);
  return { from: first, to: end.from === to ? end.number - 1 : end.number };
}

/**
 * The file name's language for `view`, loaded on demand: `extension()` is what to put in a new
 * state (empty until loaded), and a view still open gets it once it loads.
 */
function highlighting(view: EditorView, path: string) {
  const language = new Compartment();
  let support: Extension = [];
  let destroyed = false;
  const found = LanguageDescription.matchFilename(languages, path.split("/").pop() ?? path);
  void found?.load().then((loaded) => {
    support = loaded;
    if (!destroyed) view.dispatch({ effects: language.reconfigure(loaded) });
  });
  return {
    extension: () => language.of(support),
    destroy() {
      destroyed = true;
      view.destroy();
    },
  };
}

/** Replaces the view's state, keeping its scroll position. */
function replaceState(view: EditorView, state: EditorState) {
  const top = view.scrollDOM.scrollTop;
  view.setState(state);
  view.scrollDOM.scrollTop = top;
}

/** A view's selection and scroll, kept in a file's tab to show it again as it was left (8.21). */
export type ViewSnapshot = { selection: EditorSelection; top: number };

export const snapshot = (view: EditorView): ViewSnapshot => ({
  selection: view.state.selection,
  top: view.scrollDOM.scrollTop,
});

/** Puts a snapshot back into `view`, its selection kept within the text; none does nothing. */
export function restoreView(view: EditorView, saved: unknown): void {
  if (!saved) return;
  const { selection, top } = saved as ViewSnapshot;
  const clamp = (n: number) => Math.min(n, view.state.doc.length);
  const ranges = selection.ranges.map((r) => EditorSelection.range(clamp(r.anchor), clamp(r.head)));
  view.dispatch({ selection: EditorSelection.create(ranges, selection.mainIndex) });
  view.scrollDOM.scrollTop = top;
}

const commented = Decoration.line({ class: "cm-commented" });

/** Marks every line of `ranges` that the document has (the text may have changed since). */
export function commentMarks(ranges: Lines[]): Extension {
  return EditorView.decorations.of(({ state: { doc } }) => {
    const lines = new Set<number>();
    for (const { from, to } of ranges) {
      for (let n = Math.max(1, from); n <= Math.min(to, doc.lines); n++) lines.add(n);
    }
    const sorted = [...lines].sort((a, b) => a - b);
    return Decoration.set(sorted.map((n) => commented.range(doc.line(n).from)));
  });
}

/**
 * A read-only CodeMirror view for one open file; `show` replaces what it shows, `mark` the
 * lines marked as commented.
 */
export type Viewer = {
  view: EditorView;
  show(doc: Doc): void;
  mark(lines: Lines[]): void;
  destroy(): void;
};

/**
 * Creates the view in `parent` for the file at `path`, highlighted by its name's language
 * (loaded on demand). CodeMirror lives outside React (#30): the component only calls `show`
 * with each new answer and `destroy` when the file closes. `show` keeps the scroll position.
 * `onSelect` hears the selected lines (null when none) whenever they may have changed.
 */
export function createViewer(
  parent: HTMLElement,
  path: string,
  onSelect: (lines: Lines | null) => void = () => {},
): Viewer {
  const view = new EditorView({ parent });
  const language = highlighting(view, path);
  const marks = new Compartment();
  let marked: Lines[] = [];
  return {
    view,
    show({ content, original }) {
      const diff =
        original === null
          ? []
          : unifiedMergeView({
              original,
              mergeControls: false,
              collapseUnchanged: { margin: 3, minSize: 6 },
            });
      replaceState(
        view,
        EditorState.create({
          doc: content,
          extensions: [
            EditorState.readOnly.of(true),
            lineNumbers(),
            oneDark,
            theme,
            language.extension(),
            diff,
            marks.of(commentMarks(marked)),
            EditorView.updateListener.of((u) => u.selectionSet && onSelect(selectedLines(u.state))),
          ],
        }),
      );
      onSelect(null);
    },
    mark(lines) {
      marked = lines;
      view.dispatch({ effects: marks.reconfigure(commentMarks(lines)) });
    },
    destroy: language.destroy,
  };
}

/** Selects line `line` (1-based, clamped to the text) and scrolls it to the middle. */
export function revealLine(view: EditorView, line: number): void {
  const doc = view.state.doc;
  const at = doc.line(Math.max(1, Math.min(line, doc.lines)));
  view.dispatch({
    selection: { anchor: at.from, head: at.to },
    effects: EditorView.scrollIntoView(at.from, { y: "center" }),
  });
}

/** What the editable view tells its owner. */
export type EditorEvents = {
  /** The user changed the text. */
  change(doc: Text): void;
  /** Ctrl+S, only while the editor has the focus (never the terminal's key). */
  save(): void;
  /** The selected lines (null when none), for the terminal reference (3.4). */
  select(lines: Lines | null): void;
};

/** An editable CodeMirror view for the open file's buffer. */
export type Editor = {
  view: EditorView;
  /** Shows `doc` unless it already is the view's (a reload); keeps scroll and selection. */
  load(doc: Text): void;
  /** A read-only unified diff of the text against `original` (on disk), or null to edit. */
  compare(original: string | null): void;
  destroy(): void;
};

/**
 * Creates the editable view in `parent` for the file at `path`, holding `doc`. Lines split on
 * "\n" only, so a "\r" stays in the text and saves unchanged. Standard keys, undo, and Tab
 * indents (Esc, then Tab, leaves the editor).
 */
export function createEditor(
  parent: HTMLElement,
  path: string,
  doc: Text,
  on: EditorEvents,
): Editor {
  const view = new EditorView({ parent });
  const language = highlighting(view, path);
  const comparing = new Compartment();
  const create = (doc: Text, selection?: EditorSelection) =>
    EditorState.create({
      doc,
      selection,
      extensions: [
        EditorState.lineSeparator.of("\n"),
        history(),
        keymap.of([
          {
            key: "Mod-s",
            run: () => {
              on.save();
              return true;
            },
          },
          indentWithTab,
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        drawSelection(),
        lineNumbers(),
        oneDark,
        theme,
        language.extension(),
        comparing.of([]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) on.change(u.state.doc);
          if (u.selectionSet) on.select(selectedLines(u.state));
        }),
      ],
    });
  view.setState(create(doc));
  return {
    view,
    load(doc) {
      if (doc === view.state.doc) return;
      const { ranges, mainIndex } = view.state.selection;
      const clamp = (n: number) => Math.min(n, doc.length);
      const kept = ranges.map((r) => EditorSelection.range(clamp(r.anchor), clamp(r.head)));
      const state = create(doc, EditorSelection.create(kept, mainIndex));
      replaceState(view, state);
      on.select(selectedLines(state));
    },
    compare(original) {
      const diff =
        original === null
          ? []
          : [EditorState.readOnly.of(true), unifiedMergeView({ original, mergeControls: false })];
      view.dispatch({ effects: comparing.reconfigure(diff) });
    },
    destroy: language.destroy,
  };
}
